pub(super) fn read_http_request(stream: &mut std::net::TcpStream) -> RecordedRequest {
    let mut buffer = Vec::new();
    let mut temp = [0u8; 4096];
    let header_end;
    loop {
        let read = std::io::Read::read(stream, &mut temp).unwrap();
        assert!(read > 0, "mock server connection closed before headers");
        buffer.extend_from_slice(&temp[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
    }

    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines.next().unwrap();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap().to_string();
    let path = request_parts.next().unwrap().to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = std::io::Read::read(stream, &mut temp).unwrap();
        assert!(read > 0, "mock server connection closed before body");
        body.extend_from_slice(&temp[..read]);
    }
    body.truncate(content_length);
    let origin = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(beebeeb_types::backup::ORIGIN_HEADER)
            .then(|| value.trim().to_string())
    });

    RecordedRequest {
        method,
        path,
        body,
        origin,
    }
}

pub(super) fn http_json(status: &str, body: serde_json::Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// LD-0139: [`http_json`] with a `Date` header `ahead_s` seconds ahead of this machine's clock, as a server
/// answers (the notice check reads the server's clock from it).
pub(super) fn http_json_dated(status: &str, body: serde_json::Value, ahead_s: i64) -> String {
    let server_now = chrono::Utc::now() + chrono::Duration::seconds(ahead_s);
    let date = server_now.format("%a, %d %b %Y %H:%M:%S GMT");
    http_json(status, body).replacen("\r\n", &format!("\r\nDate: {date}\r\n"), 1)
}

/// [`http_json`] for a binary body (`application/octet-stream`).
pub(super) fn http_bytes(status: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
    response.extend_from_slice(body);
    response
}

pub(super) fn upload_mock_response(request: &RecordedRequest, fail_chunk: bool, fail_thumbnails: bool) -> String {
    match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/api/v1/uploads/init") => {
            // SF-0001 (L13): an honest server answers with the item id the
            // request carried; the client chose it.
            let file_id = serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| body["file_id"].as_str().map(str::to_string))
                .unwrap_or_else(|| "server-file-1".to_string());
            http_json(
                "200 OK",
                serde_json::json!({
                    "file_id": file_id,
                    "tenant_id": "tenant-1",
                    "object_version_id": "object-init-1",
                    "upload_session_id": "upload-session-1",
                    "chunk_size_bytes": 4 * 1024 * 1024,
                    "chunk_count": 1,
                    "storage_format_version": 1,
                    "storage_pool_id": "pool-1",
                    "region": "local"
                }),
            )
        }
        ("PATCH", path) if path.starts_with("/api/v1/files/") => http_json("200 OK", serde_json::json!({ "id": path })),
        ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") if fail_chunk => {
            http_json("500 Internal Server Error", serde_json::json!({ "error": "boom" }))
        }
        ("PUT", "/api/v1/uploads/upload-session-1/chunks/0") => http_json(
            "200 OK",
            serde_json::json!({ "index": 0, "size": request.body.len() as i64, "skipped": false }),
        ),
        ("POST", "/api/v1/uploads/upload-session-1/complete") => http_json(
            "200 OK",
            serde_json::json!({
                "file_id": "server-file-1",
                "version_number": 1,
                "current_object_version_id": "object-complete-1",
                "size_bytes": 19,
                "mime_type": "text/plain"
            }),
        ),
        ("POST", path) if path.ends_with("/upload/abandon") => {
            http_json("200 OK", serde_json::json!({ "abandoned": true }))
        }
        ("PUT", path) if path.starts_with("/api/v1/files/") && path.contains("/thumbnail") => {
            if fail_thumbnails {
                http_json(
                    "500 Internal Server Error",
                    serde_json::json!({ "error": "thumbnail boom" }),
                )
            } else {
                http_json("200 OK", serde_json::json!({ "message": "thumbnail uploaded" }))
            }
        }
        _ => http_json(
            "404 Not Found",
            serde_json::json!({ "error": format!("unexpected {} {}", request.method, request.path) }),
        ),
    }
}

// ── hydrate_file_range (task 1024) ───────────────────────────────────────

/// `hydrate_file_range` parses `file_id` as a UUID up front, so every
/// hydration test needs a real one — this is an arbitrary fixed value, not
/// tied to any real file.
pub(super) const TEST_FILE_ID: &str = "d6c090b4-69c8-437e-a61b-2023ea99ef89";
