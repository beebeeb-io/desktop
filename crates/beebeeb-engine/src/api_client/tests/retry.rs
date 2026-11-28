//! Retry tests: the shared 429 pause, `retry_decision` (previews only) and the transport retries.

use super::*;

// ── old per-request 429 decision (pure helper; previews.rs is its last caller) ──

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
    headers.insert("x-seconds", "1718000000".parse().unwrap());
    headers.insert("x-http-date", "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap());
    assert_eq!(header_secs(&headers, "retry-after"), Some(7));
    assert_eq!(header_secs(&headers, "x-seconds"), Some(1718000000));
    // Non-numeric (HTTP-date) header → None (default wait covers it).
    assert_eq!(header_secs(&headers, "x-http-date"), None);
    // Absent header → None.
    assert_eq!(header_secs(&headers, "nope"), None);
}

// ── shared 429 pause (rate_limit_*) ────────────────────────────────────────

const TOO_MANY: &str =
    "HTTP/1.1 429 Too Many Requests\r\nretry-after: 1\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";

/// Answers the first request 429 (`Retry-After: 1`) and every later one 200. Returns the base URL and the arrival time
/// of every request.
fn refuse_once_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<std::time::Instant>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let arrivals = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = arrivals.clone();
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let first = {
                let mut log = log.lock().unwrap();
                log.push(std::time::Instant::now());
                log.len() == 1
            };
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(if first { TOO_MANY } else { OK_JSON }.as_bytes());
            });
        }
    });
    (url, arrivals)
}

#[test]
fn rate_limit_burst_of_gets_sends_at_most_one_request_during_the_pause() {
    let (base, arrivals) = refuse_once_server();
    let client = link_client(base).with_rate_limits(Duration::from_millis(300), Duration::from_secs(5));
    run_async(async {
        // The first GET meets the 429 ...
        let first = client.get_typed::<serde_json::Value>("/x");
        let burst = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            // ... and the other nine are issued while the pause (at least the 1 s `Retry-After`) runs.
            futures_util::future::join_all((0..9).map(|_| client.get_typed::<serde_json::Value>("/x"))).await
        };
        let (first, rest) = tokio::join!(first, burst);
        first.expect("the refused GET is sent again and succeeds");
        for r in rest {
            r.expect("a GET issued during the pause waits and succeeds");
        }
    });
    let arrivals = arrivals.lock().unwrap();
    let t0 = arrivals[0];
    let during = arrivals
        .iter()
        .skip(1)
        .filter(|at| at.duration_since(t0) < Duration::from_millis(950))
        .count();
    assert!(during <= 1, "{during} requests reached the server during the pause");
    assert_eq!(arrivals.len(), 11, "one refusal, ten answered");
}

#[test]
fn rate_limit_post_that_is_refused_is_not_resent() {
    let (base, arrivals) = refuse_once_server();
    let client = link_client(base.clone());
    let url = format!("{base}/x");
    let err = run_async(client.send_with_retry(|| client.client.post(&url))).unwrap_err();
    assert!(err.to_string().starts_with("HTTP 429"), "{err}");
    assert_eq!(arrivals.lock().unwrap().len(), 1);
}

#[test]
fn rate_limit_write_with_an_idempotency_key_is_resent_after_the_pause() {
    let (base, arrivals) = refuse_once_server();
    let client = link_client(base.clone()).with_rate_limits(Duration::from_millis(50), Duration::from_secs(5));
    let url = format!("{base}/x");
    run_async(client.send_with_retry(|| client.client.put(&url).header("Idempotency-Key", "k-1")))
        .expect("an idempotency key makes the repeat safe");
    assert_eq!(arrivals.lock().unwrap().len(), 2);
}

#[test]
fn rate_limit_wait_follows_the_core_rule() {
    let (floor, cap) = (RATE_FLOOR, RATE_CAP);
    let no_jitter = |streak, hint| rate_limit_wait(streak, hint, floor, cap, 0);
    // The ladder of `beebeeb_core::retry::rate_limit_wait_ms`: 2, 4, 8, 16, 32, 60 s.
    for (streak, secs) in [
        (0, 2),
        (1, 4),
        (2, 8),
        (3, 16),
        (4, 32),
        (5, 60),
        (30, 60),
        (u32::MAX, 60),
    ] {
        assert_eq!(no_jitter(streak, None), Duration::from_secs(secs), "streak {streak}");
    }
    assert_eq!(
        no_jitter(0, Some(10)),
        Duration::from_secs(10),
        "Retry-After above the floor"
    );
    assert_eq!(no_jitter(0, Some(1)), Duration::from_secs(2), "the floor still holds");
    // The 60 s cap bounds the ladder, not the server's wait; that one stops at MAX_RETRY_AFTER_SECONDS.
    assert_eq!(no_jitter(0, Some(60)), Duration::from_secs(60));
    assert_eq!(no_jitter(0, Some(120)), Duration::from_secs(120));
    assert_eq!(no_jitter(0, Some(3600)), Duration::from_secs(300));
    // The jitter adds 0-25 % on top, after the cap, and never shortens a wait.
    assert_eq!(rate_limit_wait(0, None, floor, cap, 1000), Duration::from_millis(2_500));
    assert_eq!(rate_limit_wait(0, None, floor, cap, 500), Duration::from_millis(2_250));
    assert_eq!(rate_limit_wait(0, None, floor, cap, 9999), Duration::from_millis(2_500));
    assert_eq!(rate_limit_wait(5, None, floor, cap, 1000), Duration::from_secs(75));
    assert_eq!(rate_limit_wait(0, Some(60), floor, cap, 1000), Duration::from_secs(75));
    assert_eq!(rate_limit_wait(0, Some(30), floor, cap, 400), Duration::from_secs(33));
}

#[test]
fn rate_limit_refusals_of_one_burst_count_once() {
    let pause = RatePause::with_limits(Duration::from_secs(2), Duration::from_secs(60));
    run_async(async {
        for _ in 0..10 {
            pause.refused(None);
        }
    });
    assert_eq!(pause.state.lock().unwrap().streak, 1);
    pause.answered();
    assert_eq!(pause.state.lock().unwrap().streak, 0);
}

#[test]
fn rate_limit_retries_after_429_only_for_safe_requests() {
    let c = reqwest::Client::new();
    let req = |b: reqwest::RequestBuilder| b.build().unwrap();
    assert!(retries_after_429(&req(c.get("http://h/x"))));
    for b in [c.post("http://h/x"), c.put("http://h/x"), c.delete("http://h/x")] {
        assert!(!retries_after_429(&req(b)));
    }
    assert!(retries_after_429(&req(c
        .post("http://h/x")
        .header("Idempotency-Key", "k"))));
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

const OK_JSON: &str =
    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";
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
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst) as u32,
        1 + MAX_TRANSPORT_RETRIES
    );
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
    client.client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let started = std::time::Instant::now();
    assert!(run_async(client.get_typed::<serde_json::Value>("/x")).is_err());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "bounded: {:?}",
        started.elapsed()
    );
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
    assert!(
        transport_retry_decision(0, true, false, false).is_some(),
        "connect failure retries any method"
    );
    assert!(
        transport_retry_decision(0, false, true, false).is_none(),
        "a POST that may have been sent is not resent"
    );
    assert!(transport_retry_decision(0, false, true, true).is_some());
    assert!(transport_retry_decision(MAX_TRANSPORT_RETRIES, true, true, true).is_none());
}

// ── bulk GET (the whole-tree snapshot of a large account) ──────────────────

/// A snapshot body of `nodes` rows, about 250 bytes each (a 41,510-item account is about 10 MB).
fn snapshot_body(nodes: usize) -> Vec<u8> {
    let rows: Vec<serde_json::Value> = (0..nodes)
        .map(|i| {
            serde_json::json!({
                "id": format!("00000000-0000-0000-0000-{i:012}"), "parent_id": null, "is_folder": false,
                "name_encrypted": "x".repeat(120), "size_bytes": i, "updated_at": "2026-11-24T10:00:00Z",
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({ "seq_id": 7, "nodes": rows })).unwrap()
}

/// Serve `body` on one connection in `pieces` pieces, `gap` apart; with `stall_after` it stops after that many
/// pieces and holds the connection open without finishing.
fn slow_body_server(body: Vec<u8>, pieces: usize, gap: Duration, stall_after: Option<usize>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        let mut buf = [0u8; 4096];
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.read(&mut buf);
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let size = body.len().div_ceil(pieces);
        for (i, piece) in body.chunks(size).enumerate() {
            if stall_after == Some(i) {
                std::thread::sleep(Duration::from_secs(10));
                return;
            }
            if stream.write_all(piece).is_err() || stream.flush().is_err() {
                return;
            }
            std::thread::sleep(gap);
        }
    });
    url
}

#[test]
fn a_snapshot_that_arrives_slowly_but_steadily_is_not_cut_off_by_a_total_cap() {
    // 41,510 rows, served over ~2 s in 20 pieces. A total cap of 1 s (the app's 30 s, shrunk) cuts it off,
    // as it did on the 0.9.0 account; the bulk GET, with 500 ms of allowed silence, reads all of it.
    let body = snapshot_body(41_510);
    let base = slow_body_server(body.clone(), 20, Duration::from_millis(100), None);
    let mut capped = link_client(base);
    capped.client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let err = run_async(capped.get_typed::<crate::api_client::SyncSnapshot>("/x")).unwrap_err();
    assert!(err.to_string().contains("decoding"), "{err}");

    let base = slow_body_server(body, 20, Duration::from_millis(100), None);
    let client = link_client(base);
    let snap =
        run_async(client.get_typed_bulk_with::<crate::api_client::SyncSnapshot>("/x", Duration::from_millis(500)))
            .expect("a steady body is read to the end");
    assert_eq!(snap.nodes.len(), 41_510);
}

#[test]
fn a_snapshot_whose_body_stalls_fails_in_bounded_time() {
    let base = slow_body_server(snapshot_body(2_000), 10, Duration::from_millis(50), Some(3));
    let client = link_client(base);
    let started = std::time::Instant::now();
    let result =
        run_async(client.get_typed_bulk_with::<crate::api_client::SyncSnapshot>("/x", Duration::from_millis(400)));
    assert!(result.is_err());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "bounded: {:?}",
        started.elapsed()
    );
}

#[test]
fn get_file_rides_out_a_5xx_and_records_a_401() {
    let (base, hits) = scripted_server(vec![Some(UNAVAILABLE), Some(OK_JSON)]);
    let client = link_client(base);
    run_async(client.get_file("f1")).expect("get_file retries a 503 like every typed GET");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);

    let base = one_shot_server(Some(
        "HTTP/1.1 401 Unauthorized\r\ncontent-length: 2\r\nconnection: close\r\n\r\nno",
    ));
    let client = link_client(base);
    let err = run_async(client.get_file("f1")).unwrap_err();
    assert!(err.to_string().contains("401"), "{err}");
    assert!(
        client.link().snapshot().last_ok_at.is_some(),
        "the 401 answer reaches the link monitor"
    );
}
