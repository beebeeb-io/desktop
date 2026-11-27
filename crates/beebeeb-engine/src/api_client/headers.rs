//! Writer-provenance headers, the shared 429 pause and the bounded retry policy.
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

// ── 429 backoff/retry tuning (account-data GETs) ───────────────────────────────
//
// The account-data endpoints (auth/me, billing/*, account/*, clients/*) can 429
// under load. The client must ride out a transient 429 rather than surfacing a
// "Couldn't load" to the view, but must stay BOUNDED — it is not a hot loop and
// must never hang a view waiting on a hammered endpoint.
//
/// Extra attempts after the first 429 (so up to 1 + this total requests).
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

// ── Shared 429 pause (one per client) ──────────────────────────────────────────
//
// A 429 means the account is over its window, so every request of this client waits, not only the refused one.
// The wait is the server's `Retry-After` or a 2 s floor that doubles with each refusal in a row, plus full
// jitter, capped at 60 s. `x-ratelimit-reset` is not read. Only requests that are safe to send twice (a GET, or
// a write carrying an idempotency key) are sent again on their own; the rest surface the 429.

/// The shortest pause after a 429; doubles with each refusal in a row.
pub(super) const RATE_FLOOR: Duration = Duration::from_secs(2);

/// The longest pause after a 429, whatever the server asks for.
pub(super) const RATE_CAP: Duration = Duration::from_secs(60);

/// The header that makes a write safe to send twice.
pub(super) const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Whether a request refused with 429 may be sent again without the caller's say: it was not processed, but a
/// write the server may have half-applied is only repeated when its idempotency key lets the server dedupe it.
pub(super) fn retries_after_429(request: &reqwest::Request) -> bool {
    matches!(
        *request.method(),
        reqwest::Method::GET | reqwest::Method::HEAD | reqwest::Method::OPTIONS
    ) || request.headers().contains_key(IDEMPOTENCY_KEY_HEADER)
}

/// The pause after the `streak`-th refusal in a row (0 for the first). `jitter` is in `0.0..=1.0`: the wait is
/// `base * (1 + jitter)`, where `base` is the larger of the doubled floor and `Retry-After`, never above `cap`.
pub(super) fn rate_limit_wait(
    pub(super) streak: u32,
    retry_after: Option<u64>,
    floor: Duration,
    cap: Duration,
    jitter: f64,
) -> Duration {
    let doubled = floor.saturating_mul(1u32 << streak.min(16));
    let hint = Duration::from_secs(retry_after.unwrap_or(0));
    let base = doubled.max(hint).min(cap);
    base.mul_f64(1.0 + jitter.clamp(0.0, 1.0)).min(cap)
}

pub(super) struct PauseState {
    pub(super) until: Option<tokio::time::Instant>,
    pub(super) streak: u32,
}

/// What a client knows about being refused: shared by every request it sends (and its backup-pass twin).
pub(super) struct RatePause {
    pub(super) state: std::sync::Mutex<PauseState>,
    floor: Duration,
    cap: Duration,
}

impl RatePause {
    pub(super) fn new() -> Self {
        Self::with_limits(RATE_FLOOR, RATE_CAP)
    }

    pub(super) fn with_limits(floor: Duration, cap: Duration) -> Self {
        Self {
            state: std::sync::Mutex::new(PauseState { until: None, streak: 0 }),
            floor,
            cap,
        }
    }

    /// Returns once no pause is running.
    pub(super) async fn wait(&self) {
        loop {
            let until = self.state.lock().unwrap_or_else(|e| e.into_inner()).until;
            match until {
                Some(at) if at > tokio::time::Instant::now() => tokio::time::sleep_until(at).await,
                _ => return,
            }
        }
    }

    /// A 429 came back. Starts the pause, unless one is already running: the refusals of one burst are one event
    /// and do not double the wait again.
    pub(super) fn refused(&self, retry_after: Option<u64>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = tokio::time::Instant::now();
        if state.until.is_some_and(|at| at > now) {
            return;
        }
        let jitter: f64 = rand::Rng::gen_range(&mut rand::thread_rng(), 0.0..=1.0);
        let wait = rate_limit_wait(state.streak, retry_after, self.floor, self.cap, jitter);
        state.until = Some(now + wait);
        state.streak = state.streak.saturating_add(1);
    }

    /// A request got an answer that is not a 429: the streak ends.
    pub(super) fn answered(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).streak = 0;
    }
}

// ── Transport / 5xx retry (bounded) ────────────────────────────────────────────

/// Extra attempts after a connect failure, a timeout or a 502/503/504 (so up to 1 + this requests).
pub(super) const MAX_TRANSPORT_RETRIES: u32 = 2;

/// First backoff; doubles each retry (250 ms, 500 ms). Worst case adds under a second to a call.
pub(super) const TRANSPORT_RETRY_BASE_MS: u64 = 250;

/// GET/HEAD/PUT/DELETE/OPTIONS are safe to send twice; POST/PATCH are not.
pub(super) fn is_idempotent(method: &reqwest::Method) -> bool {
    !matches!(*method, reqwest::Method::POST | reqwest::Method::PATCH)
}

/// Pure decision after a failure. `connect`: the request never left. `maybe_sent`: a timeout or a 5xx
/// answer, where the server may have acted. Connect failures retry for any method; the rest only for
/// idempotent ones. Returns the wait before the next try, or `None` to give up.
pub(super) fn transport_retry_decision(
    attempt: u32,
    connect: bool,
    maybe_sent: bool,
    idempotent: bool,
) -> Option<Duration> {
    if attempt >= MAX_TRANSPORT_RETRIES || !(connect || (maybe_sent && idempotent)) {
        return None;
    }
    Some(Duration::from_millis(TRANSPORT_RETRY_BASE_MS << attempt))
}
