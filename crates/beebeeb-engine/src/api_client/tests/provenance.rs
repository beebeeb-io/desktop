//! Writer-provenance header tests: names, values and version fallback.

use super::*;

// ── Writer-provenance headers (task 1436) ───────────────────────────────
//
// `provenance_headers()` is the single source of `default_headers` for
// `ApiClient::new` AND every ad-hoc `reqwest::Client` built in `lib.rs`'s
// Tauri commands (desktop_login, desktop_login_2fa,
// desktop_confirm_action, desktop_storage_summary) — asserting on it here
// covers every one of those call sites without needing a real HTTP mock
// (desktop's `tokio` dependency has no "net" feature, unlike the CLI's
// axum-mock pattern in PR #14; a genuinely wired-up request is exercised
// at the local-.app-bundle verification rung instead).
#[test]
fn provenance_headers_names_this_client_desktop() {
    let headers = provenance_headers();
    assert_eq!(
        headers.get("X-Beebeeb-Client").and_then(|v| v.to_str().ok()),
        Some("desktop"),
        "X-Beebeeb-Client must be exactly \"desktop\" (the server's writer-provenance column reads this verbatim)"
    );
}

#[test]
fn provenance_headers_carries_the_real_release_version() {
    let headers = provenance_headers();
    let version = headers
        .get("X-Beebeeb-Client-Version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(!version.is_empty(), "X-Beebeeb-Client-Version must not be empty");
    // Must be non-empty and NOT silently fall back to the pinned, never-bumped
    // Cargo.toml version unless BEEBEEB_RELEASE_VERSION is genuinely unset —
    // matches lib.rs's real_app_version() fallback exactly.
    let expected = option_env!("BEEBEEB_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    assert_eq!(version, expected);
}

/// Raw-TCP mock that captures every header off the wire, so a test can
/// prove `ApiClient::new`'s ACTUAL `reqwest::Client` sends the
/// writer-provenance headers on a real request — not just that
/// `provenance_headers()` builds the right `HeaderMap` in isolation.
/// Desktop's `tokio` dependency has no axum-mock setup like the CLI's PR
/// #14 (no `axum` dependency here), so this uses the same
/// `std::net::TcpListener` technique `engine_bridge.rs`'s
/// `UploadMockServer` already relies on — no new dependency needed.
struct HeaderMockServer {
    base_url: String,
    headers: std::sync::Arc<std::sync::Mutex<Option<std::collections::HashMap<String, String>>>>,
    handle: std::thread::JoinHandle<()>,
}

impl HeaderMockServer {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let headers = std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured = std::sync::Arc::clone(&headers);
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = Vec::new();
            let mut temp = [0u8; 4096];
            let header_end;
            loop {
                let read = std::io::Read::read(&mut stream, &mut temp).unwrap();
                assert!(read > 0, "header mock connection closed before headers");
                buffer.extend_from_slice(&temp[..read]);
                if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = pos + 4;
                    break;
                }
            }
            let raw = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let mut map = std::collections::HashMap::new();
            for line in raw.lines().skip(1) {
                if let Some((name, value)) = line.split_once(':') {
                    map.insert(name.trim().to_lowercase(), value.trim().to_string());
                }
            }
            *captured.lock().unwrap() = Some(map);
            let body = serde_json::json!({ "files": [] }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            std::io::Write::write_all(&mut stream, response.as_bytes()).unwrap();
        });
        Self {
            base_url,
            headers,
            handle,
        }
    }

    fn finish(self) -> std::collections::HashMap<String, String> {
        self.handle.join().unwrap();
        self.headers
            .lock()
            .unwrap()
            .take()
            .expect("mock never received a request")
    }
}

#[tokio::test]
async fn api_client_list_files_sends_provenance_headers_on_the_wire() {
    let server = HeaderMockServer::start();
    let client = ApiClient::new(server.base_url.clone(), "tok".into(), [0u8; 32]);
    client.list_files(None).await.expect("mock request should succeed");
    let headers = server.finish();
    assert_eq!(
        headers.get("x-beebeeb-client").map(String::as_str),
        Some("desktop"),
        "X-Beebeeb-Client missing or wrong on a real ApiClient request"
    );
    let expected_version = option_env!("BEEBEEB_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    assert_eq!(
        headers.get("x-beebeeb-client-version").map(String::as_str),
        Some(expected_version),
        "X-Beebeeb-Client-Version missing or stale on a real ApiClient request"
    );
}
