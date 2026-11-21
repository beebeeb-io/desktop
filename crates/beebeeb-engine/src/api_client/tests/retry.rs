//! `retry_decision` tests: bounded 429 backoff and server hints.

use super::*;

// ── 429 backoff/retry decision (pure helper) ───────────────────────────────

#[test]
fn retry_decision_first_429_uses_default_when_no_hint() {
    // First 429 (attempt 0), nothing spent, no server hint → default wait.
    assert_eq!(retry_decision(0, 0, None, None), Some(RETRY_DEFAULT_WAIT_SECS));
}

#[test]
fn retry_decision_honours_retry_after_capped() {
    // A small Retry-After is used verbatim …
    assert_eq!(retry_decision(0, 0, Some(2), None), Some(2));
    // … but an absurd one is clamped to the per-attempt cap.
    assert_eq!(retry_decision(0, 0, Some(120), None), Some(RETRY_WAIT_CAP_SECS));
}

#[test]
fn retry_decision_uses_reset_when_retry_after_absent() {
    // `x-ratelimit-reset` delta (seconds-until-reset) drives the wait.
    assert_eq!(retry_decision(0, 0, None, Some(3)), Some(3));
}

#[test]
fn retry_decision_takes_larger_of_two_hints() {
    // When BOTH hints are present we wait the longer of the two.
    assert_eq!(retry_decision(0, 0, Some(2), Some(4)), Some(4));
    assert_eq!(retry_decision(0, 0, Some(4), Some(2)), Some(4));
}

#[test]
fn retry_decision_stops_after_max_attempts() {
    // Once we've already retried MAX_429_RETRIES times, give up.
    assert_eq!(retry_decision(MAX_429_RETRIES, 0, Some(1), None), None);
}

#[test]
fn retry_decision_stops_when_budget_exhausted() {
    // Spent at/over the total budget → no more sleeping.
    assert_eq!(retry_decision(1, RETRY_TOTAL_BUDGET_SECS, Some(1), None), None);
}

#[test]
fn retry_decision_trims_wait_to_remaining_budget() {
    // Cap says 5s, but only 2s of budget remain → wait is trimmed to 2s so
    // cumulative sleep never exceeds RETRY_TOTAL_BUDGET_SECS.
    let spent = RETRY_TOTAL_BUDGET_SECS - 2;
    assert_eq!(retry_decision(1, spent, Some(5), None), Some(2));
}

#[test]
fn header_secs_parses_numeric_and_rejects_dates() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("retry-after", "  7 ".parse().unwrap());
    headers.insert("x-ratelimit-reset", "1718000000".parse().unwrap());
    headers.insert("x-http-date", "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap());
    assert_eq!(header_secs(&headers, "retry-after"), Some(7));
    assert_eq!(header_secs(&headers, "x-ratelimit-reset"), Some(1718000000));
    // Non-numeric (HTTP-date) header → None (default wait covers it).
    assert_eq!(header_secs(&headers, "x-http-date"), None);
    // Absent header → None.
    assert_eq!(header_secs(&headers, "nope"), None);
}

// ── transport / 5xx retries on a fake server ───────────────────────────────

/// Serve each raw response in turn, one per connection (`None` = hang up), then stop. Returns the base URL
/// and a counter of connections accepted.
fn scripted_server(script: Vec<Option<&'static str>>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
        for step in script {
            let Ok((mut stream, _)) = listener.accept() else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let _ = stream.read(&mut buf);
            if let Some(raw) = step {
                let _ = stream.write_all(raw.as_bytes());
            }
        }
    });
    (url, hits)
}

const OK_JSON: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";
const UNAVAILABLE: &str = "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 4\r\nconnection: close\r\n\r\ndown";

#[test]
fn a_reset_connection_is_retried_and_recovers() {
    // First connection is dropped without an answer (a reset); the second answers.
    let (base, hits) = scripted_server(vec![None, Some(OK_JSON)]);
    let client = link_client(base);
    run_async(client.get_typed::<serde_json::Value>("/x")).expect("recovers after a reset");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn a_5xx_burst_shorter_than_the_bound_recovers() {
    let (base, hits) = scripted_server(vec![Some(UNAVAILABLE), Some(UNAVAILABLE), Some(OK_JSON)]);
    let client = link_client(base);
    run_async(client.get_typed::<serde_json::Value>("/x")).expect("recovers after two 503s");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[test]
fn a_5xx_burst_longer_than_the_bound_stops_with_the_status() {
    let (base, hits) = scripted_server(vec![Some(UNAVAILABLE); 6]);
    let client = link_client(base);
    let err = run_async(client.get_typed::<serde_json::Value>("/x")).unwrap_err();
    assert!(err.to_string().starts_with("HTTP 503"), "{err}");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst) as u32, 1 + MAX_TRANSPORT_RETRIES);
}

#[test]
fn a_hung_server_times_out_within_the_bound() {
    // Accepts and never answers: every attempt times out; the call must end, not hang.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let held = std::thread::spawn(move || {
        let mut streams = Vec::new();
        listener.set_nonblocking(false).unwrap();
        while streams.len() < 3 {
            if let Ok((s, _)) = listener.accept() {
                streams.push(s);
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    });
    let mut client = link_client(base);
    client.client = reqwest::Client::builder().timeout(Duration::from_millis(200)).build().unwrap();
    let started = std::time::Instant::now();
    assert!(run_async(client.get_typed::<serde_json::Value>("/x")).is_err());
    assert!(started.elapsed() < Duration::from_secs(3), "bounded: {:?}", started.elapsed());
    drop(held);
}

#[test]
fn a_post_is_not_resent_after_a_5xx() {
    let (base, hits) = scripted_server(vec![Some(UNAVAILABLE), Some(OK_JSON)]);
    let client = link_client(base.clone());
    let url = format!("{base}/x");
    let err = run_async(client.send_with_retry(|| client.client.post(&url))).unwrap_err();
    assert!(err.to_string().starts_with("HTTP 503"), "{err}");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn transport_retry_decision_is_bounded_and_method_aware() {
    assert!(transport_retry_decision(0, true, false, false).is_some(), "connect failure retries any method");
    assert!(transport_retry_decision(0, false, true, false).is_none(), "a POST that may have been sent is not resent");
    assert!(transport_retry_decision(0, false, true, true).is_some());
    assert!(transport_retry_decision(MAX_TRANSPORT_RETRIES, true, true, true).is_none());
}
