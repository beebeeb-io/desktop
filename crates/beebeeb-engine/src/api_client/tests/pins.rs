//! The pinned-folders preference against a loopback server.

use super::*;
use std::io::{Read, Write};

/// Serve each `(status, body)` in turn on loopback; the raw request text of each is sent back on the channel.
fn serve(responses: Vec<(&'static str, String)>) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen);
                if let Some(at) = text.find("\r\n\r\n") {
                    let want = text[..at]
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if seen.len() >= at + 4 + want {
                        break;
                    }
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&seen).to_string());
            let raw = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(raw.as_bytes());
        }
    });
    (url, rx)
}

#[test]
fn pinned_ids_read_web_shape_bare_array_and_junk() {
    assert_eq!(pinned_ids_of(&serde_json::json!({ "folder_ids": ["a", "b", "a"] })), ["a", "b"]);
    assert_eq!(pinned_ids_of(&serde_json::json!(["x"])), ["x"]);
    assert!(pinned_ids_of(&serde_json::Value::Null).is_empty());
    assert!(pinned_ids_of(&serde_json::json!({ "folder_ids": 5 })).is_empty());
}

#[test]
fn pinned_folders_gets_the_preference() {
    let (base, rx) = serve(vec![("200 OK", r#"{"value":{"folder_ids":["f1","f2"]}}"#.into())]);
    let client = ApiClient::new(base, "tok".into(), [0u8; 32]);
    let ids = run_async(client.pinned_folders()).unwrap();
    assert_eq!(ids, ["f1", "f2"]);
    let req = rx.recv().unwrap();
    assert!(req.starts_with("GET /api/v1/preferences/pinned_folders "), "{req}");
}

#[test]
fn pinned_folders_error_is_not_empty() {
    let (base, _rx) = serve(vec![("500 Internal Server Error", "{}".into())]);
    let client = ApiClient::new(base, "tok".into(), [0u8; 32]);
    assert!(run_async(client.pinned_folders()).is_err());
}

#[test]
fn set_pinned_folders_puts_the_web_shape() {
    let (base, rx) = serve(vec![("200 OK", "{}".into())]);
    let client = ApiClient::new(base, "tok".into(), [0u8; 32]);
    run_async(client.set_pinned_folders(&["f1".to_string()])).unwrap();
    let req = rx.recv().unwrap();
    assert!(req.starts_with("PUT /api/v1/preferences/pinned_folders "), "{req}");
    assert!(req.to_ascii_lowercase().contains("authorization: bearer tok"));
    assert!(req.ends_with(r#"{"folder_ids":["f1"]}"#), "{req}");
}
