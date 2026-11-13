use super::{encode_base64, provision_vault_key_from_phrase};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct SeenRequest {
    path: String,
    authorization: Option<String>,
    body: String,
}

#[derive(Clone, Copy)]
enum Reply {
    /// Behave like the server: 200 `{valid:true}` when the submitted check
    /// equals the stored one, else 400 `invalid_recovery_phrase`.
    CompareAgainstStored,
    /// Server fault — must never be read as "phrase is valid".
    InternalError,
}

/// Minimal loop-accepting HTTP/1.1 mock of
/// `POST /api/v1/auth/verify-recovery-check` (same raw-TCP technique as
/// `api_client.rs`'s `HeaderMockServer`; desktop has no axum dev-dep).
struct VerifyMockServer {
    base_url: String,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
}

impl VerifyMockServer {
    fn start(stored_check_b64: Option<String>, reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_thread = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buffer = Vec::new();
                let mut temp = [0u8; 4096];
                let header_end = loop {
                    let read = stream.read(&mut temp).unwrap_or(0);
                    if read == 0 {
                        break None;
                    }
                    buffer.extend_from_slice(&temp[..read]);
                    if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(pos + 4);
                    }
                };
                let Some(header_end) = header_end else { continue };
                let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or_default().to_string();
                let mut content_length = 0usize;
                let mut authorization = None;
                for line in head.lines().skip(1) {
                    if let Some((name, value)) = line.split_once(':') {
                        match name.trim().to_ascii_lowercase().as_str() {
                            "content-length" => content_length = value.trim().parse().unwrap_or(0),
                            "authorization" => authorization = Some(value.trim().to_string()),
                            _ => {}
                        }
                    }
                }
                while buffer.len() < header_end + content_length {
                    let read = stream.read(&mut temp).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&temp[..read]);
                }
                let body = String::from_utf8_lossy(&buffer[header_end..]).to_string();
                let submitted = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v.get("recovery_check").and_then(|c| c.as_str()).map(str::to_string));
                seen_thread.lock().unwrap().push(SeenRequest {
                    path: path.clone(),
                    authorization,
                    body,
                });
                let (status, payload) = match reply {
                    Reply::InternalError => ("500 Internal Server Error", r#"{"error":"internal"}"#.to_string()),
                    Reply::CompareAgainstStored => {
                        let matches = path == "/api/v1/auth/verify-recovery-check"
                            && stored_check_b64.is_some()
                            && submitted == stored_check_b64;
                        if matches {
                            ("200 OK", r#"{"valid":true}"#.to_string())
                        } else {
                            ("400 Bad Request", r#"{"error":"invalid_recovery_phrase"}"#.to_string())
                        }
                    }
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self { base_url, seen }
    }

    fn requests(&self) -> Vec<SeenRequest> {
        self.seen.lock().unwrap().clone()
    }
}

fn check_b64_for_phrase(phrase: &str) -> String {
    let key = beebeeb_core::recovery::recover_from_phrase(phrase).expect("valid phrase");
    encode_base64(&*beebeeb_core::opaque::compute_recovery_check(&key))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
}

/// The account's own phrase (P1) and another checksum-valid phrase (P2).
fn two_phrases() -> (String, String) {
    let (p1, _) = beebeeb_core::recovery::generate_recovery_phrase().unwrap();
    let (p2, _) = beebeeb_core::recovery::generate_recovery_phrase().unwrap();
    assert_ne!(p1, p2);
    (p1, p2)
}

#[tokio::test]
async fn wrong_but_valid_phrase_is_rejected_and_never_persisted() {
    let (p1, p2) = two_phrases();
    // Precondition of the bug: P2 is a perfectly valid phrase on its own.
    assert!(beebeeb_core::recovery::recover_from_phrase(&p2).is_ok());
    let server = VerifyMockServer::start(Some(check_b64_for_phrase(&p1)), Reply::CompareAgainstStored);
    let persisted: Mutex<Option<[u8; 32]>> = Mutex::new(None);

    let result = provision_vault_key_from_phrase(&client(), &server.base_url, "sess-tok", &p2, |key| {
        *persisted.lock().unwrap() = Some(key);
        Ok(())
    })
    .await;

    assert_eq!(
        result.err().as_deref(),
        Some("Incorrect recovery phrase. Check your words and try again."),
        "a valid phrase that is not the account's must be refused"
    );
    assert!(
        persisted.lock().unwrap().is_none(),
        "the wrong key must never reach the keychain"
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "exactly one verify call: {requests:?}");
    assert_eq!(requests[0].path, "/api/v1/auth/verify-recovery-check");
}

#[tokio::test]
async fn account_phrase_is_verified_with_session_then_persisted() {
    let (p1, _) = two_phrases();
    let stored = check_b64_for_phrase(&p1);
    let server = VerifyMockServer::start(Some(stored.clone()), Reply::CompareAgainstStored);
    let persisted: Mutex<Option<[u8; 32]>> = Mutex::new(None);

    let key = provision_vault_key_from_phrase(&client(), &server.base_url, "sess-tok", &p1, |key| {
        *persisted.lock().unwrap() = Some(key);
        Ok(())
    })
    .await
    .expect("the account's own phrase must unlock");

    let expected = *beebeeb_core::recovery::recover_from_phrase(&p1).unwrap().to_bytes();
    assert_eq!(*key, expected);
    assert_eq!(*persisted.lock().unwrap(), Some(expected));
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "exactly one verify call: {requests:?}");
    assert_eq!(requests[0].path, "/api/v1/auth/verify-recovery-check");
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer sess-tok"));
    let body: serde_json::Value = serde_json::from_str(&requests[0].body).unwrap();
    assert_eq!(body["recovery_check"].as_str(), Some(stored.as_str()));
}

#[tokio::test]
async fn account_without_stored_check_is_refused_like_web() {
    // Server returns 400 invalid_recovery_phrase when no check is on file
    // (server routes/recovery.rs); web treats that as a mismatch (task
    // 0874/0875) and so does desktop.
    let (p1, _) = two_phrases();
    let server = VerifyMockServer::start(None, Reply::CompareAgainstStored);
    let persisted: Mutex<Option<[u8; 32]>> = Mutex::new(None);

    let result = provision_vault_key_from_phrase(&client(), &server.base_url, "sess-tok", &p1, |key| {
        *persisted.lock().unwrap() = Some(key);
        Ok(())
    })
    .await;

    assert_eq!(
        result.err().as_deref(),
        Some("Incorrect recovery phrase. Check your words and try again.")
    );
    assert!(persisted.lock().unwrap().is_none());
}

#[tokio::test]
async fn verifier_failure_fails_closed() {
    let (p1, _) = two_phrases();
    let persisted: Mutex<Option<[u8; 32]>> = Mutex::new(None);

    // Server error: never read as "valid".
    let server = VerifyMockServer::start(Some(check_b64_for_phrase(&p1)), Reply::InternalError);
    let result = provision_vault_key_from_phrase(&client(), &server.base_url, "sess-tok", &p1, |key| {
        *persisted.lock().unwrap() = Some(key);
        Ok(())
    })
    .await;
    let err = result.expect_err("a 500 from the verifier must not unlock");
    assert!(
        !err.starts_with("Incorrect recovery phrase"),
        "server fault is not a wrong phrase: {err}"
    );
    assert_eq!(server.requests().len(), 1);

    // Unreachable server: bind then drop a listener so the port refuses.
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_url = format!("http://{}", dead.local_addr().unwrap());
    drop(dead);
    let result = provision_vault_key_from_phrase(&client(), &dead_url, "sess-tok", &p1, |key| {
        *persisted.lock().unwrap() = Some(key);
        Ok(())
    })
    .await;
    assert!(result.is_err(), "a network error must not unlock");

    assert!(
        persisted.lock().unwrap().is_none(),
        "nothing may be persisted when unverified"
    );
}
