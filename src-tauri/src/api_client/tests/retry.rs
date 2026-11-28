//! 429 wait-rule tests: the floor ladder, server hints and the bounded retry.

use super::*;

// ── 429 wait rule (pure helpers) ───────────────────────────────────────────

#[test]
fn the_wait_climbs_the_floor_ladder_without_a_hint() {
    let waits: Vec<u64> = (0..6).map(|streak| rate_limit_wait_ms(streak, None, 0)).collect();
    assert_eq!(waits, [2_000, 4_000, 8_000, 16_000, 32_000, 60_000]);
    // The ladder stays at the cap, also for a streak that would overflow the shift.
    assert_eq!(rate_limit_wait_ms(40, None, 0), 60_000);
    assert_eq!(rate_limit_wait_ms(u32::MAX, None, 0), 60_000);
}

#[test]
fn the_wait_is_the_larger_of_ladder_and_retry_after() {
    // A hint below the floor does not shorten the wait.
    assert_eq!(rate_limit_wait_ms(0, Some(1), 0), 2_000);
    assert_eq!(rate_limit_wait_ms(0, Some(7), 0), 7_000);
    assert_eq!(rate_limit_wait_ms(0, Some(60), 0), 60_000);
    assert_eq!(rate_limit_wait_ms(3, Some(7), 0), 16_000);
}

#[test]
fn the_wait_adds_jitter_up_to_25_percent() {
    assert_eq!(rate_limit_wait_ms(0, Some(10), 1000), 12_500);
    assert_eq!(rate_limit_wait_ms(0, Some(10), 400), 11_000);
    assert_eq!(rate_limit_wait_ms(0, None, 1000), 2_500);
    // Out-of-range jitter is clamped.
    assert_eq!(rate_limit_wait_ms(0, Some(10), 9000), 12_500);
}

#[test]
fn a_hint_above_60s_is_a_policy_refusal() {
    assert!(!is_policy_refusal(None, false));
    assert!(!is_policy_refusal(Some(60), false));
    assert!(is_policy_refusal(Some(61), false));
    assert!(is_policy_refusal(Some(120), false));
}

#[test]
fn a_ban_is_a_policy_refusal() {
    assert!(is_policy_refusal(Some(1), true));
    assert!(is_policy_refusal(None, true));
    assert!(is_banned_body(r#"{"error":"banned"}"#));
    assert!(!is_banned_body(r#"{"error":"rate_limited"}"#));
    assert!(!is_banned_body("not json"));
}

#[test]
fn retries_stop_after_max_attempts() {
    assert!(may_retry(MAX_429_RETRIES - 1, 0, 2_000));
    assert!(!may_retry(MAX_429_RETRIES, 0, 2_000));
}

#[test]
fn the_budget_covers_the_whole_ladder_and_one_full_window_with_jitter() {
    // The four ladder waits a call can make (2 + 4 + 8 + 16 s, each with the maximum jitter) fit …
    let mut spent = 0;
    for attempt in 0..MAX_429_RETRIES {
        let wait = rate_limit_wait_ms(attempt, None, 1000);
        assert!(may_retry(attempt, spent, wait), "attempt {attempt}");
        spent += wait;
    }
    assert_eq!(spent, 37_500);
    // … and so does one 60 s window with the maximum jitter, but not a second one.
    assert!(may_retry(0, 0, rate_limit_wait_ms(0, Some(60), 1000)));
    assert!(!may_retry(1, 75_000, rate_limit_wait_ms(1, Some(60), 0)));
}

#[test]
fn header_secs_parses_numeric_and_rejects_dates() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("retry-after", "  7 ".parse().unwrap());
    headers.insert("x-http-date", "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap());
    assert_eq!(header_secs(&headers, "retry-after"), Some(7));
    // Non-numeric (HTTP-date) header → None (the floor ladder covers it).
    assert_eq!(header_secs(&headers, "x-http-date"), None);
    // Absent header → None.
    assert_eq!(header_secs(&headers, "nope"), None);
}

#[test]
fn only_idempotent_methods_are_replayed() {
    use reqwest::Method;
    for m in [Method::GET, Method::HEAD, Method::PUT, Method::DELETE] {
        assert!(is_idempotent(&m), "{m}");
    }
    assert!(!is_idempotent(&Method::POST));
    assert!(!is_idempotent(&Method::PATCH));
}

#[test]
fn body_retry_after_is_read() {
    assert_eq!(body_retry_after(r#"{"error":"rate_limited","retry_after":7}"#), Some(7));
    assert_eq!(body_retry_after("{}"), None);
    assert_eq!(body_retry_after("nope"), None);
}

#[test]
fn a_post_429_is_returned_not_retried_and_pauses_the_client() {
    // The server answers ONE request; a retry would fail as a transport error instead.
    let base = one_shot_server(Some(
        "HTTP/1.1 429 Too Many Requests\r\nretry-after: 1\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
    ));
    let client = link_client(base);
    let err = run_async(client.send_with_retry(|| client.client.post(client.url("/x")))).unwrap_err();
    assert!(matches!(err, DesktopApiError::HttpStatus { status, .. } if status.as_u16() == 429), "{err}");
    assert!(client.pause_until.lock().unwrap().is_some(), "the 429 pauses the whole client");
}

#[test]
fn a_policy_refusal_is_rate_limited_and_says_temporarily_blocked() {
    let base = one_shot_server(Some(
        "HTTP/1.1 429 Too Many Requests\r\nretry-after: 600\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
    ));
    let client = link_client(base);
    let err = run_async(client.send_with_retry(|| client.client.get(client.url("/x")))).unwrap_err();
    assert!(matches!(err, DesktopApiError::RateLimited { banned: false, retry_after: Some(600) }), "{err}");
    assert!(err.to_string().contains("temporarily blocked"));
    assert!(err.to_string().starts_with("HTTP 429"));
}
