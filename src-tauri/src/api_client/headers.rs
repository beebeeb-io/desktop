//! Writer-provenance headers and the bounded 429 retry policy.
//!
//! Owns: the `X-Beebeeb-Client*` values every request carries and
//! the 429 wait arithmetic (`rate_limit_wait_ms`). Must not: touch the network or the
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
// The client pause rule. A 429 pauses the whole client for
// `rate_limit_wait_ms(streak, hint, jitter)`: the floor ladder (2, 4, 8, 16, 32,
// 60 s by streak) or the server's `Retry-After`, whichever is longer, plus
// 0-25 % jitter so clients do not return in step. A hint above
// `RETRY_AFTER_MAX_SECS`, or a body with `error: "banned"`, is a policy refusal:
// not waited out, the 429 is returned to the caller. `x-ratelimit-reset` is not
// read: it describes another tier than the one that refused.

/// Extra attempts after the first 429 (so up to 1 + this total requests).
pub(super) const MAX_429_RETRIES: u32 = 4;

/// Longest `Retry-After` we wait for. Anything above is a policy refusal.
pub(super) const RETRY_AFTER_MAX_SECS: u64 = 60;

/// Upper bound of the jitter added to a wait, in percent of the wait.
pub(super) const RETRY_JITTER_MAX_PCT: u64 = 25;

/// Cap on the CUMULATIVE sleep across all retries for one call, in ms: one full
/// window plus its maximum jitter (60 s + 25 %).
pub(super) const RETRY_TOTAL_BUDGET_MS: u64 = RETRY_AFTER_MAX_SECS * 1000 * (100 + RETRY_JITTER_MAX_PCT) / 100;

/// First step of the wait ladder, in ms (core's `RATE_LIMIT_WAIT_FLOOR_MS`).
pub(super) const RATE_LIMIT_WAIT_FLOOR_MS: u64 = 2_000;

/// Top of the wait ladder, in ms (core's `RATE_LIMIT_WAIT_CAP_MS`).
pub(super) const RATE_LIMIT_WAIT_CAP_MS: u64 = 60_000;

/// How long a 429 pauses the client, in ms. The app's own copy of
/// `beebeeb_core::retry::rate_limit_wait_ms`, with the same numbers; call core's
/// once the pin has it.
///
/// - `streak` is 0 for the first 429 of a run of refusals.
/// - `retry_after_seconds` is the server's hint, if any.
/// - `jitter_permille` (0..=1000) scales the 0-25 % jitter.
///
/// Streaks 0..5 without a hint give 2, 4, 8, 16, 32, 60 s; `Retry-After: 60` gives 60-75 s.
pub(super) fn rate_limit_wait_ms(streak: u32, retry_after_seconds: Option<u64>, jitter_permille: u32) -> u64 {
    let ladder = RATE_LIMIT_WAIT_FLOOR_MS
        .checked_shl(streak)
        .filter(|ms| ms >> streak == RATE_LIMIT_WAIT_FLOOR_MS)
        .map_or(RATE_LIMIT_WAIT_CAP_MS, |ms| ms.min(RATE_LIMIT_WAIT_CAP_MS));
    let hinted = retry_after_seconds.unwrap_or(0).min(RETRY_AFTER_MAX_SECS) * 1000;
    let base = ladder.max(hinted);
    base + base * RETRY_JITTER_MAX_PCT * u64::from(jitter_permille.min(1000)) / 100_000
}

/// True when a 429 is a policy refusal (a ban, or a hint above
/// `RETRY_AFTER_MAX_SECS`): never waited out, never retried.
pub(super) fn is_policy_refusal(hint: Option<u64>, banned: bool) -> bool {
    banned || hint.is_some_and(|secs| secs > RETRY_AFTER_MAX_SECS)
}

/// True when a call that already made `attempt` retries and slept `spent_ms`
/// may sleep `wait_ms` more and try again: at most `MAX_429_RETRIES` retries,
/// and a cumulative sleep within `RETRY_TOTAL_BUDGET_MS`.
pub(super) fn may_retry(attempt: u32, spent_ms: u64, wait_ms: u64) -> bool {
    attempt < MAX_429_RETRIES && spent_ms.saturating_add(wait_ms) <= RETRY_TOTAL_BUDGET_MS
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

/// Current UNIX time in seconds, for the link record. Falls back to 0 on the
/// (impossible) pre-epoch clock rather than panicking.
pub(super) fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse a header value as whole seconds (`u64`). Tolerant of surrounding
/// whitespace; returns `None` if the header is absent or non-numeric (e.g. an
/// HTTP-date `Retry-After`, which we don't try to parse — the floor ladder
/// covers that case).
pub(super) fn header_secs(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}
