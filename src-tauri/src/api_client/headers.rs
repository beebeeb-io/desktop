//! Writer-provenance headers and the bounded 429 retry policy.
//!
//! Owns: the `X-Beebeeb-Client*` values every request carries and
//! `retry_decision`'s backoff arithmetic. Must not: touch the network or the
//! client's credentials. Tests: `api_client/tests/` (provenance and retry).

use super::*;

// ── Writer-provenance headers (task 1436, the desktop/web half of 1392) ────────
//
// The server records `X-Beebeeb-Client` / `X-Beebeeb-Client-Version` on every
// `object_versions` row (server PR #23 / task 1369) so a blast-radius query can
// tell which client + build wrote a version. No user data goes in either value.
//
// The version sent is `BEEBEEB_RELEASE_VERSION` (set by release.yml before
// `cargo build`; this is what `real_app_version()` in `lib.rs` shows in
// Settings and compares against update manifests), falling back to the Cargo
// package version for local/dev builds where CI hasn't set it. Cargo.toml's
// `version` is pinned at `0.1.0` and never bumped for releases, so sending the
// raw `CARGO_PKG_VERSION` (as the CLI does — `cli` has no separate
// release-version indirection, `bb --version` IS `CARGO_PKG_VERSION`) would
// make every desktop-written row report "0.1.0" forever, defeating the point
// of the column. This deliberately deviates from the CLI's exact pattern for
// that reason.
pub(super) const CLIENT_HEADER_NAME: &str = "X-Beebeeb-Client";

pub(super) const CLIENT_HEADER_VALUE: &str = "desktop";

pub(super) const CLIENT_VERSION_HEADER_NAME: &str = "X-Beebeeb-Client-Version";

pub(super) const CLIENT_VERSION_HEADER_VALUE: &str = match option_env!("BEEBEEB_RELEASE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// Builds a `HeaderMap` carrying the writer-provenance headers, for
/// `reqwest::ClientBuilder::default_headers`. Shared by [`ApiClient::new`] and
/// every ad-hoc `reqwest::Client` the Tauri commands in `lib.rs` build for
/// one-off auth calls (login, 2FA, confirm-action, storage summary), so no
/// call site can forget to tag its requests.
pub(crate) fn provenance_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CLIENT_HEADER_NAME, HeaderValue::from_static(CLIENT_HEADER_VALUE));
    headers.insert(
        CLIENT_VERSION_HEADER_NAME,
        HeaderValue::from_static(CLIENT_VERSION_HEADER_VALUE),
    );
    headers
}

// ── 429 backoff/retry tuning ───────────────────────────────────────────────────
//
// A 429 carries the server's window in `Retry-After`. The client waits that long
// (plus 0-25 % jitter so clients do not return in step), up to
// `RETRY_AFTER_MAX_SECS`. A longer hint, or a body with `error: "banned"`, is a
// policy refusal: not waited out, the 429 is returned to the caller.

/// Extra attempts after the first 429 (so up to 1 + this total requests).
pub(super) const MAX_429_RETRIES: u32 = 4;

/// Longest `Retry-After` we wait for. Anything above is a policy refusal.
pub(super) const RETRY_AFTER_MAX_SECS: u64 = 60;

/// Upper bound of the jitter added to a wait, in percent of the wait.
pub(super) const RETRY_JITTER_MAX_PCT: u64 = 25;

/// Cap on the CUMULATIVE sleep across all retries for one call, in ms: one full
/// window plus its maximum jitter (60 s + 25 %).
pub(super) const RETRY_TOTAL_BUDGET_MS: u64 = RETRY_AFTER_MAX_SECS * 1000 * (100 + RETRY_JITTER_MAX_PCT) / 100;

/// Backoff used when the server gives no usable hint (no Retry-After / reset).
pub(super) const RETRY_DEFAULT_WAIT_SECS: u64 = 1;

/// What to do with a 429.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RetryDecision {
    /// Sleep this many milliseconds, then retry.
    Wait(u64),
    /// Out of attempts or budget: return the 429.
    GiveUp,
    /// The server asked for more than we wait, or banned us: return the 429.
    Refuse,
}

/// True when a 429 body is the server's `{"error":"banned"}`.
pub(super) fn is_banned_body(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body).is_ok_and(|v| v["error"] == "banned")
}

/// `retry_after` seconds from a 429 body (`{"error":"rate_limited","retry_after":N}`), if present.
pub(super) fn body_retry_after(body: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["retry_after"].as_u64())
}

/// Only these methods may be replayed after a 429; a POST is not.
pub(super) fn is_idempotent(method: &reqwest::Method) -> bool {
    use reqwest::Method;
    matches!(*method, Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS)
}

/// Current UNIX time in seconds (for interpreting `x-ratelimit-reset`, which is
/// an absolute epoch). Falls back to 0 on the (impossible) pre-epoch clock so
/// the reset delta just reads as "0s" rather than panicking.
pub(super) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse a header value as whole seconds (`u64`). Tolerant of surrounding
/// whitespace; returns `None` if the header is absent or non-numeric (e.g. an
/// HTTP-date `Retry-After`, which we don't try to parse — the default wait
/// covers that case).
pub(super) fn header_secs(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Pure backoff decision for a 429 response.
///
/// - `attempt` is the number of retries ALREADY made (0 on the first 429).
/// - `spent_ms` is the cumulative milliseconds already slept for this call.
/// - `retry_after` / `reset_in` are the server's hints (seconds), if any.
/// - `banned` is true when the body said `error: "banned"`.
/// - `jitter_pct` is the 0-25 % jitter to add (clamped to 25).
///
/// The wait is `max(retry_after, reset_in)` (default `RETRY_DEFAULT_WAIT_SECS`)
/// plus jitter. A hint above `RETRY_AFTER_MAX_SECS` or a ban is `Refuse`. We
/// give up at `MAX_429_RETRIES`, or when the wait would pass the total budget.
pub(super) fn retry_decision(
    attempt: u32,
    spent_ms: u64,
    retry_after: Option<u64>,
    reset_in: Option<u64>,
    banned: bool,
    jitter_pct: u64,
) -> RetryDecision {
    let hint = match (retry_after, reset_in) {
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => RETRY_DEFAULT_WAIT_SECS,
    };
    if banned || hint > RETRY_AFTER_MAX_SECS {
        return RetryDecision::Refuse;
    }
    if attempt >= MAX_429_RETRIES {
        return RetryDecision::GiveUp;
    }
    let base_ms = hint * 1000;
    let wait_ms = base_ms + base_ms * jitter_pct.min(RETRY_JITTER_MAX_PCT) / 100;
    if spent_ms.saturating_add(wait_ms) > RETRY_TOTAL_BUDGET_MS {
        return RetryDecision::GiveUp;
    }
    RetryDecision::Wait(wait_ms)
}
