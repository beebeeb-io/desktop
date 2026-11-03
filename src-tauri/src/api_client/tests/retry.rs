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
