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

// ── Capability declaration (core 0.3.0, chunk format v2) ───────────────────────
//
// Core's rule (its `CLAUDE.md`, "Transfers"): `X-Beebeeb-Capabilities` goes only on the requests whose
// server behaviour it gates, never globally. For this client those are the upload session (init, chunk,
// complete) and the metadata update (`PATCH /api/v1/files/{id}`): the server refuses a v2-protocol write
// without `file-write-v2` once v1 writes end, and a new name for a version-protected file without
// `version-metadata-v1`. The value is the one core's own client sends (`beebeeb-client`
// `NATIVE_CAPABILITIES`, `beebeeb-transfer` `V2_WRITE_CAPABILITIES`); neither constant is public, so it
// is spelled here and pinned by a test.
pub(super) const CAPABILITIES_HEADER_NAME: &str = beebeeb_core::direct_transfer::CAPABILITIES_HEADER;

pub(super) const WRITE_CAPABILITIES: &str = "file-write-v2, version-metadata-v1";

/// Builds a `HeaderMap` carrying the writer-provenance headers, for
/// `reqwest::ClientBuilder::default_headers`. Shared by [`ApiClient::new`] and
/// every ad-hoc `reqwest::Client` the Tauri commands in `lib.rs` build for
/// one-off auth calls (login, 2FA, confirm-action, storage summary), so no
/// call site can forget to tag its requests.
pub fn provenance_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CLIENT_HEADER_NAME, HeaderValue::from_static(CLIENT_HEADER_VALUE));
    headers.insert(
        CLIENT_VERSION_HEADER_NAME,
        HeaderValue::from_static(CLIENT_VERSION_HEADER_VALUE),
    );
    headers
}

/// Extra attempts after the first 429 (so up to 1 + this total requests).
// ── 429 backoff/retry tuning (account-data GETs) ───────────────────────────────
//
// The account-data endpoints (auth/me, billing/*, account/*, clients/*) can 429
// under load. The client must ride out a transient 429 rather than surfacing a
// "Couldn't load" to the view, but must stay BOUNDED — it is not a hot loop and
// must never hang a view waiting on a hammered endpoint.

pub(super) const MAX_429_RETRIES: u32 = 3;

/// Hard cap on any SINGLE backoff wait, regardless of what the server asks for.
/// Protects against an absurd `Retry-After`/`x-ratelimit-reset` value pinning a
/// view. ~5s keeps the UI responsive while still letting a short window clear.
pub(super) const RETRY_WAIT_CAP_SECS: u64 = 5;

/// Cap on the CUMULATIVE time spent sleeping across all retries for one call.
pub(super) const RETRY_TOTAL_BUDGET_SECS: u64 = 8;

/// Backoff used when the server gives no usable hint (no Retry-After / reset).
pub(super) const RETRY_DEFAULT_WAIT_SECS: u64 = 1;

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

/// Pure backoff decision for a 429 response. Returns `Some(wait_secs)` to retry
/// after sleeping `wait_secs`, or `None` to give up (surface the error).
///
/// - `attempt` is the number of retries ALREADY made (0 on the first 429).
/// - `spent` is the cumulative seconds already slept for this call.
/// - `retry_after` / `reset_in` are the server's hints (seconds), if any.
///
/// The chosen wait is `max(retry_after, reset_in)` (the server may send either),
/// defaulting to `RETRY_DEFAULT_WAIT_SECS`, then clamped to `RETRY_WAIT_CAP_SECS`
/// and further trimmed so total sleep never exceeds `RETRY_TOTAL_BUDGET_SECS`.
/// We stop once `attempt` reaches `MAX_429_RETRIES` or the budget is exhausted.
pub(super) fn retry_decision(attempt: u32, spent: u64, retry_after: Option<u64>, reset_in: Option<u64>) -> Option<u64> {
    if attempt >= MAX_429_RETRIES || spent >= RETRY_TOTAL_BUDGET_SECS {
        return None;
    }
    // Take the larger of the two server hints; fall back to the default.
    let hint = match (retry_after, reset_in) {
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => RETRY_DEFAULT_WAIT_SECS,
    };
    // Per-attempt cap.
    let wait = hint.min(RETRY_WAIT_CAP_SECS);
    // Don't blow the cumulative budget: trim the wait to what's left.
    let remaining = RETRY_TOTAL_BUDGET_SECS.saturating_sub(spent);
    let wait = wait.min(remaining);
    Some(wait)
}
