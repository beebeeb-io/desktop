//! Writer-provenance headers, the shared 429 pause and the bounded retry policy.
//!
//! Owns: the `X-Beebeeb-Client*` values every request carries, the 429 pause
//! (`RatePause`) and the transport retry arithmetic. Must not: touch the network or the
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

// ── 429 retry bounds ───────────────────────────────────────────────────────────
//
/// Extra attempts after the first 429 (so up to 1 + this total requests).
pub(super) const MAX_429_RETRIES: u32 = 3;

// The three constants below and `retry_decision` are the old per-request 429 backoff. Only `previews.rs`
// (`preview_retry_wait`) still calls it; everything else waits on the shared `RatePause` further down.

/// Hard cap on any SINGLE backoff wait of `retry_decision`, regardless of what the server asks for.
pub(super) const RETRY_WAIT_CAP_SECS: u64 = 5;

/// Cap on the CUMULATIVE time spent sleeping across all retries for one call.
pub(super) const RETRY_TOTAL_BUDGET_SECS: u64 = 8;

/// Backoff used when the server gives no usable hint (no `Retry-After`).
pub(super) const RETRY_DEFAULT_WAIT_SECS: u64 = 1;

/// Current UNIX time in seconds. Falls back to 0 on the (impossible) pre-epoch clock rather than panicking.
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

// ── Shared 429 pause (one per account and API base) ────────────────────────────
//
// A 429 means the account is over its window, so every request of the account waits, not only the refused one and
// not only those of the client that met it: about 30 call sites build a client for one call, and they all take the
// account's pause from `shared_rate_pause`.
// The wait is defined by `beebeeb_core::retry::rate_limit_wait_ms` (the epic's Client pause rule); this is the
// engine's own copy of it until its core pin moves to a revision that has the function:
//
//   base = max(min(2 s << streak, 60 s), min(Retry-After, MAX_RETRY_AFTER_SECONDS))
//   wait = base + 0-25 % of base (random)
//
// The 60 s cap bounds the doubling floor only: a server wait above 60 s is waited in full. `x-ratelimit-reset` is
// not read. A 429 that arrives while a pause runs does not start or double a new one, and the streak ends on the
// first answer that is not a 429. Only requests that are safe to send twice (a GET, or a write carrying an
// idempotency key) are sent again on their own; the rest surface the 429.
//
// A policy refusal (`is_policy_refusal`: a ban, or a wait above 60 s) is not a pause: nobody sleeps through it and
// nothing is sent again. The refused request gets its 429 back at once, with the wait.

/// The shortest pause after a 429; doubles with each refusal in a row. Core: `RATE_LIMIT_WAIT_FLOOR_MS`.
pub(super) const RATE_FLOOR: Duration = Duration::from_secs(2);

/// Where the doubling floor stops. Not a cap on the server's `Retry-After`. Core: `RATE_LIMIT_WAIT_CAP_MS`.
pub(super) const RATE_CAP: Duration = Duration::from_secs(60);

/// The most jitter added on top of a pause, in percent of it. Core: `CLIENT_429_WAIT_JITTER_PERCENT`.
pub(super) const RATE_JITTER_PERCENT: u64 = 25;

/// A server wait above this many seconds is a policy refusal, not a pause.
pub(super) const POLICY_REFUSAL_SECS: u64 = beebeeb_core::retry::CLIENT_429_POLICY_REFUSAL_SECONDS;

/// Whether a 429 is a policy refusal: the body says `"error": "banned"`, or the wait (`hint`: the larger of the
/// `Retry-After` header and the body's `retry_after`) is above [`POLICY_REFUSAL_SECS`].
pub(super) fn is_policy_refusal(hint: Option<u64>, body: &str) -> bool {
    hint.is_some_and(|secs| secs > POLICY_REFUSAL_SECS) || is_banned(body)
}

pub(super) fn is_banned(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body).is_ok_and(|value| value["error"] == "banned")
}

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

/// The pause after the `streak`-th refusal in a row (0 for the first): `beebeeb_core::retry::rate_limit_wait_ms`
/// with the floor and its cap as arguments. `jitter_permille` (0..=1000) picks how much of the 25 % is added.
pub(super) fn rate_limit_wait(
    streak: u32,
    retry_after: Option<u64>,
    floor: Duration,
    cap: Duration,
    jitter_permille: u32,
) -> Duration {
    let millis = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
    let (floor_ms, cap_ms) = (millis(floor), millis(cap));
    let ladder = floor_ms.saturating_mul(1u64 << streak.min(32)).min(cap_ms);
    let hint = retry_after
        .unwrap_or(0)
        .min(beebeeb_types::limits::MAX_RETRY_AFTER_SECONDS)
        .saturating_mul(1000);
    let base = ladder.max(hint);
    let jitter = base.saturating_mul(RATE_JITTER_PERCENT * u64::from(jitter_permille.min(1000))) / 100_000;
    Duration::from_millis(base.saturating_add(jitter))
}

pub(super) struct PauseState {
    pub(super) until: Option<tokio::time::Instant>,
    pub(super) streak: u32,
}

/// What an account's clients know about being refused: shared by every request any of them sends.
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
    /// and do not double the wait again. A wait above [`POLICY_REFUSAL_SECS`] starts none: it is a policy refusal,
    /// which the caller returns instead of waiting out.
    pub(super) fn refused(&self, retry_after: Option<u64>) {
        if retry_after.is_some_and(|secs| secs > POLICY_REFUSAL_SECS) {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = tokio::time::Instant::now();
        if state.until.is_some_and(|at| at > now) {
            return;
        }
        let jitter: u32 = rand::Rng::gen_range(&mut rand::thread_rng(), 0..=1000);
        let wait = rate_limit_wait(state.streak, retry_after, self.floor, self.cap, jitter);
        state.until = Some(now + wait);
        state.streak = state.streak.saturating_add(1);
    }

    /// A request got an answer that is not a 429: the streak ends.
    pub(super) fn answered(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).streak = 0;
    }
}

// ── The registry: one pause per (API base, account) in the process ─────────────

/// How long a pause nobody holds is kept after it ran out, so that a run of one-call clients keeps its streak. In
/// tests an unheld pause goes at once: a later test may be handed the same loopback port.
const UNHELD_PAUSE_KEEP: Duration = if cfg!(any(test, feature = "test-support")) {
    Duration::ZERO
} else {
    Duration::from_secs(300)
};

/// The API base and a SHA-256 of the session token. The token is what every client of one signed-in account has in
/// common; the digest keeps the token itself out of a map that outlives the clients.
type PauseKey = (String, [u8; 32]);

type PauseRegistry = std::sync::Mutex<std::collections::HashMap<PauseKey, std::sync::Arc<RatePause>>>;

fn pause_registry() -> &'static PauseRegistry {
    static REGISTRY: std::sync::OnceLock<PauseRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn pause_key(base_url: &str, token: &str) -> PauseKey {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"beebeeb-engine/rate-pause/v1\0");
    hasher.update(token.as_bytes());
    (base_url.trim_end_matches('/').to_string(), hasher.finalize().into())
}

/// The pause of the account `token` signs in to at `base_url`: the same one for every client built with them.
pub(super) fn shared_rate_pause(base_url: &str, token: &str) -> std::sync::Arc<RatePause> {
    let mut registry = pause_registry().lock().unwrap_or_else(|e| e.into_inner());
    // Entries of sessions that ended (no client left, pause long over) go here, so the map cannot grow.
    let now = tokio::time::Instant::now();
    registry.retain(|_, pause| {
        std::sync::Arc::strong_count(pause) > 1 || {
            let state = pause.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .until
                .is_some_and(|at| at > now || (state.streak > 0 && at + UNHELD_PAUSE_KEEP > now))
        }
    });
    registry
        .entry(pause_key(base_url, token))
        .or_insert_with(|| std::sync::Arc::new(RatePause::new()))
        .clone()
}

/// Sign-out: the account's pause and streak are dropped. A client still alive keeps the one it holds; the next
/// client built for this base and token starts clean.
pub(super) fn forget_rate_pause(base_url: &str, token: &str) {
    pause_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&pause_key(base_url, token));
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
