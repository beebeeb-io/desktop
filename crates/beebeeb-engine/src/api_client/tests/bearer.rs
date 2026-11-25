//! dc-re-b2: every request an `ApiClient` sends carries the session's bearer token, and a unit-test client never
//! reaches a server off this computer. A loopback server records each request's Authorization header; the token is
//! a fixture, never a real one, and is never printed.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use super::*;

const TOKEN: &str = "fixture-session-token";

/// (request line, Authorization header) of each request the server saw.
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A loopback server that answers every request `200 {}` and records what it saw.
fn recording_server() -> (String, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let record = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).to_string();
            let mut lines = head.lines();
            let line = lines.next().unwrap_or_default().to_string();
            let mut auth = None;
            let mut body_len = 0usize;
            for header in lines {
                if let Some((name, value)) = header.split_once(':') {
                    match name.trim().to_ascii_lowercase().as_str() {
                        "authorization" => auth = Some(value.trim().to_string()),
                        "content-length" => body_len = value.trim().parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            let mut body = vec![0u8; body_len];
            let _ = stream.read_exact(&mut body);
            record.lock().unwrap().push((line, auth));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
            );
        }
    });
    (base, seen)
}

#[test]
fn every_request_of_the_engine_client_carries_the_bearer_token() {
    let (base, seen) = recording_server();
    let client = ApiClient::new(base, TOKEN.into(), [0u8; 32]);
    let backup = client.for_backup_pass();
    let on_body = |_: BodyBytes| {};
    run_async(async {
        let _ = client.get_file("f1").await;
        let _ = client.list_files(None).await;
        let _ = client.list_files(Some("folder1")).await;
        let _ = client.sync_snapshot().await;
        let _ = client.sync_ops(7).await;
        let _ = client.download_chunk("f1", 0, &on_body).await;
        let _ = client.direct_download_offer().await;
        let _ = backup.get_file("f2").await;
    });
    let seen = seen.lock().unwrap().clone();
    let expected = format!("Bearer {TOKEN}");
    let lines: Vec<&str> = seen.iter().map(|(line, _)| line.as_str()).collect();
    for path in [
        "/api/v1/files/f1 ",
        "/api/v1/files ",
        "/api/v1/files?parent_id=folder1 ",
        "/api/v1/sync/snapshot ",
        "/api/v1/sync/ops?since=7 ",
        "/api/v1/files/f1/chunks/0 ",
        "/api/v1/files/f2 ",
    ] {
        assert!(
            lines.iter().any(|line| line.contains(path)),
            "{path} was sent: {lines:?}"
        );
    }
    for (line, auth) in &seen {
        // Never print the header's value: only whether it was right.
        assert!(
            auth.as_deref() == Some(expected.as_str()),
            "{line}: no or another bearer token"
        );
    }
}

/// The fixtures that build a bridge on the production base with a made-up token never reach it: the request fails on
/// this computer, it is not answered (production answered such requests 401).
#[test]
fn a_test_client_for_a_base_off_this_computer_never_leaves_it() {
    let client = ApiClient::new("https://api.beebeeb.io".into(), "token".into(), [0u8; 32]);
    let started = std::time::Instant::now();
    let error = run_async(client.get_file("00000000-0000-4000-8000-000000000000")).unwrap_err();
    assert!(!error.to_string().starts_with("HTTP "), "a server answered: {error}");
    assert!(started.elapsed() < Duration::from_secs(10), "it failed at once");
}
