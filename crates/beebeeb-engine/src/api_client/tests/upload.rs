//! Upload tests: refusal classification and chunk response parsing.

use super::*;

/// The typed refusal inside an upload error, wherever in the chain it sits: the
/// same place a caller's `downcast_ref` used to reach.
fn refusal(error: &DesktopApiError) -> &UploadRefused {
    error
        .causes()
        .into_iter()
        .find_map(|cause| cause.downcast_ref::<UploadRefused>())
        .expect("typed")
}

/// Spec §8.4: a refused upload request keeps the server's code by type, matched exactly, and
/// its text is the status error's alone: the body (its code, its numbers, its message) never
/// reaches the error's text, which is what a log line or the queue's `last_error` could show.
#[tokio::test]
async fn an_upload_refusal_keeps_the_servers_code_and_never_its_body() {
    use std::io::{Read, Write};
    async fn refused(status: &'static str, body: &'static str) -> DesktopApiError {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let response = reqwest::Client::new().get(&url).send().await.unwrap();
        let error = upload_response(response).await.unwrap_err();
        server.join().unwrap();
        error
    }
    let quota = refused(
        "413 Payload Too Large",
        r#"{"error":"quota_exceeded","limit_bytes":5368709120,"used_bytes":5368709120,"is_trial_cap":false}"#,
    )
    .await;
    let typed = refusal(&quota);
    assert_eq!(typed.code, Some(UploadRefusalCode::QuotaExceeded));
    assert_eq!(typed.status(), Some(413));
    let text = quota.to_string();
    assert!(text.contains("413 Payload Too Large"), "{text}");
    assert!(
        !text.contains("quota_exceeded") && !text.contains("5368709120"),
        "the body never reaches the text: {text}"
    );
    assert!(
        quota
            .causes()
            .into_iter()
            .any(|cause| cause.downcast_ref::<reqwest::Error>().is_some()),
        "the status error stays in the chain"
    );
    // Matched exactly: another spelling, or another code, is no code.
    for body in [
        r#"{"error":"Quota_Exceeded"}"#,
        r#"{"error":"quota exceeded"}"#,
        r#"{"error":"payload_too_large"}"#,
        "not json",
    ] {
        let error = refused("413 Payload Too Large", body).await;
        assert_eq!(refusal(&error).code, None, "{body}");
    }
    for (code, expected) in [
        ("object_budget_exceeded", UploadRefusalCode::ObjectBudgetExceeded),
        ("billing_read_only", UploadRefusalCode::BillingReadOnly),
        ("account_lapsed", UploadRefusalCode::AccountLapsed),
        ("plan_required", UploadRefusalCode::PlanRequired),
        ("trial_cancelled_read_only", UploadRefusalCode::TrialCancelledReadOnly),
    ] {
        assert_eq!(UploadRefusalCode::from_code(code), Some(expected));
    }
    // The share gate's `trial_ended` is no upload refusal: no upload route sends it.
    assert_eq!(UploadRefusalCode::from_code("trial_ended"), None);
}

/// A commit (`complete`) carries no idempotency key, so a replay could commit twice: answered 429 it is sent exactly
/// once and the 429 is its error, and the next call of the same client is held until the pause ends. The server names
/// the wait in the body only (`retry_after`, no `Retry-After` header), so the body's hint is what starts the pause.
#[test]
fn a_commit_answered_429_is_sent_once_and_the_next_call_waits_for_the_pause() {
    use std::io::{Read, Write};
    const REFUSED: &str = "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\ncontent-length: 40\r\nconnection: close\r\n\r\n{\"error\":\"rate_limited\",\"retry_after\":1}";
    const DONE: &str =
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";
    assert_eq!(REFUSED.split("\r\n\r\n").nth(1).map(str::len), Some(40));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // The arrival time and request line of every request the server saw.
    let arrivals = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(std::time::Instant, String)>::new()));
    let log = arrivals.clone();
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let at = std::time::Instant::now();
            let mut buf = [0u8; 4096];
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let read = stream.read(&mut buf).unwrap_or(0);
            let line = String::from_utf8_lossy(&buf[..read])
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let first = {
                let mut log = log.lock().unwrap();
                log.push((at, line));
                log.len() == 1
            };
            let _ = stream.write_all(if first { REFUSED } else { DONE }.as_bytes());
        }
    });
    // A floor far below the hint: only the body's `retry_after: 1` can make the pause a second long.
    let client = link_client(base).with_rate_limits(Duration::from_millis(50), Duration::from_secs(5));
    run_async(async {
        let error = client
            .complete_upload_session("s1", None)
            .await
            .expect_err("the refused commit is an error");
        let status = error
            .chain()
            .find_map(|cause| match cause.downcast_ref::<DesktopApiError>() {
                Some(DesktopApiError::HttpStatus { status, .. }) => Some(status.as_u16()),
                _ => None,
            });
        assert_eq!(status, Some(429), "{error}");
        // Well inside the pause: the commit was not sent again.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            arrivals.lock().unwrap().len(),
            1,
            "a refused commit is sent exactly once"
        );
        // The next call of the same client goes out only when the pause is over.
        client
            .complete_upload_session("s2", None)
            .await
            .expect("the call after the pause is answered");
    });
    let arrivals = arrivals.lock().unwrap();
    assert_eq!(arrivals.len(), 2, "one refused commit, one call after the pause");
    assert!(
        arrivals[0].1.starts_with("POST /api/v1/uploads/s1/complete "),
        "{}",
        arrivals[0].1
    );
    assert!(
        arrivals[1].1.starts_with("POST /api/v1/uploads/s2/complete "),
        "{}",
        arrivals[1].1
    );
    let gap = arrivals[1].0.duration_since(arrivals[0].0);
    assert!(
        gap >= Duration::from_millis(950),
        "the next call was sent {gap:?} after the 429, inside the pause"
    );
}
