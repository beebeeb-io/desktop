//! `retry_decision` tests: bounded 429 backoff and server hints.

use super::*;

// ── 429 backoff/retry decision (pure helper) ───────────────────────────────

use RetryDecision::*;

#[test]
fn retry_decision_first_429_uses_default_when_no_hint() {
    assert_eq!(
        retry_decision(0, 0, None, None, false, 0),
        Wait(RETRY_DEFAULT_WAIT_SECS * 1000)
    );
}

#[test]
fn retry_decision_honours_retry_after_up_to_60s() {
    assert_eq!(retry_decision(0, 0, Some(2), None, false, 0), Wait(2000));
    assert_eq!(retry_decision(0, 0, Some(60), None, false, 0), Wait(60_000));
}

#[test]
fn retry_decision_refuses_retry_after_above_60s() {
    assert_eq!(retry_decision(0, 0, Some(61), None, false, 0), Refuse);
    assert_eq!(retry_decision(0, 0, Some(120), None, false, 0), Refuse);
    assert_eq!(retry_decision(0, 0, Some(5), Some(300), false, 0), Refuse);
}

#[test]
fn retry_decision_refuses_banned() {
    assert_eq!(retry_decision(0, 0, Some(1), None, true, 0), Refuse);
    assert!(is_banned_body(r#"{"error":"banned"}"#));
    assert!(!is_banned_body(r#"{"error":"rate_limited"}"#));
    assert!(!is_banned_body("not json"));
}

#[test]
fn retry_decision_adds_jitter_up_to_25_percent() {
    assert_eq!(retry_decision(0, 0, Some(10), None, false, 25), Wait(12_500));
    assert_eq!(retry_decision(0, 0, Some(10), None, false, 10), Wait(11_000));
    // Out-of-range jitter is clamped.
    assert_eq!(retry_decision(0, 0, Some(10), None, false, 90), Wait(12_500));
}

#[test]
fn retry_decision_uses_reset_and_larger_hint() {
    assert_eq!(retry_decision(0, 0, None, Some(3), false, 0), Wait(3000));
    assert_eq!(retry_decision(0, 0, Some(2), Some(4), false, 0), Wait(4000));
    assert_eq!(retry_decision(0, 0, Some(4), Some(2), false, 0), Wait(4000));
}

#[test]
fn retry_decision_stops_after_max_attempts() {
    assert_eq!(retry_decision(MAX_429_RETRIES, 0, Some(1), None, false, 0), GiveUp);
}

#[test]
fn retry_decision_budget_covers_a_full_window_with_jitter() {
    // One 60 s window with the maximum jitter fits the budget …
    assert_eq!(retry_decision(0, 0, Some(60), None, false, 25), Wait(75_000));
    // … a second one does not.
    assert_eq!(retry_decision(1, 75_000, Some(60), None, false, 0), GiveUp);
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
