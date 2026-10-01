//! What the engine says about itself, and what the popover reads back (task 1683
//! slice 2; spec `docs/specs/2026-09-30-macos-menubar-popover.md` section 9, rows
//! "Sync state", "Offline vs server did not answer", "Last successful check",
//! "Overall progress").
//!
//! Before this slice `runner::emit_status` sent `{state, sync_root, error,
//! files_remaining}` with `files_remaining` hard-coded to `null`, and the only
//! states were `running`, `idle`, `paused`, `error`, `stopped`. There was no
//! `syncing` and no `offline`. This module owns the payload and the small state
//! machine that decides which one to send, as pure code so every state's exact
//! payload is a unit test.
//!
//! ## The payload (superset of the old one; old consumers ignore the extra keys)
//!
//! ```json
//! { "state": "syncing", "legacy_state": "idle", "sync_root": "/…", "error": null,
//!   "files_remaining": 14, "bytes_total": 1288490189, "bytes_done": 489626271,
//!   "last_tick_ok_at": 1760000000,
//!   "reason": { "code": "timeout", "detail": "timeout after 30 s" } }
//! ```
//!
//! - `state`: `running` | `idle` | `syncing` | `paused` | `offline` | `error` | `stopped`.
//! - `legacy_state`: what the OLD emitter would have said (`syncing` and `offline`
//!   are new; an old tick that returned `Ok` said `idle`). The tray tooltip and
//!   `sync_status` read this, so nothing a user sees today changes before the
//!   flip (slice 6). Absent on payloads that never had a new state (the pause
//!   toggle's `{state}`), where `state` itself is the legacy value.
//! - `last_tick_ok_at`: unix seconds of the last tick that returned `Ok` AND in
//!   which no request failed at the link level. `null` until there is one. Never
//!   faked: the popover's "Last checked 2 min ago" tooltip is dropped when null.
//! - `reason`: a stable machine `code` and an optional mono `detail` (at most 30
//!   characters). Present for `offline` and `error`.
//!
//! ## Why `syncing` needs a pulse, not just the end of the tick
//!
//! The runner's tick awaits `process_due_operations`, which runs every due upload
//! to completion before the tick returns. After the tick there is almost never
//! anything in flight, so a state derived only from the post-tick counts would
//! read "idle" for the whole time a transfer ran. [`StatusTracker::pulse`] is
//! called once a second by a side task and reports `syncing` while work is in
//! flight, including between ticks (a Finder hydrate).

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::link_health::{self, LinkFailure, LinkFailureKind, LinkReason, LinkSnapshot};
use crate::surfaces::phase::Connectivity;

/// The Tauri event name. Unchanged.
pub const EVENT: &str = "engine-status";

/// `reason.code` for a whole-engine failure that is not a link failure (the state
/// directory could not be prepared, the lock is held, a tick failed for a local
/// reason). Spec section 7: "Whole-engine error: state d".
pub const REASON_ENGINE_ERROR: &str = "engine_error";
/// `reason.code` when the server ANSWERED with 401. Not a connectivity problem:
/// the popover shows state e2 once `auth_expired` flips, not d.
pub const REASON_AUTH: &str = "auth";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Idle,
    Syncing,
    Paused,
    Offline,
    Error,
    Stopped,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Running => "running",
            State::Idle => "idle",
            State::Syncing => "syncing",
            State::Paused => "paused",
            State::Offline => "offline",
            State::Error => "error",
            State::Stopped => "stopped",
        }
    }
}

/// How much work is in flight right now (see `transfer_progress` for where the
/// numbers come from). `bytes_total` counts finished bytes of the current batch
/// plus what is still to go, so `bytes_done / bytes_total` never runs backwards
/// as files finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Activity {
    pub files_remaining: u32,
    pub bytes_total: u64,
    pub bytes_done: u64,
}

/// Turn the state DB's backlog and the transfer board into the event's numbers.
///
/// - `files_remaining`: queued uploads that are due plus files being downloaded.
///   (An upload stays queued while it runs, so the running one is counted once.)
/// - `bytes_total`: bytes finished in this batch plus what is still to go. What is
///   still to go is the larger of the DB's sizes and what the board knows for the
///   files moving now, because a brand-new file's row can still say 0 bytes.
/// - `bytes_done`: finished bytes plus the bytes moved so far in flight. Never
///   above the total.
pub fn compute_activity(
    backlog: &crate::state_db::TransferBacklog,
    board: &crate::transfer_progress::TransferBoard,
) -> Activity {
    // A transfer that is moving but has no queue row (a one-shot upload such as Keep Mine) still
    // counts: the board knows it even though the backlog does not.
    let moving = u32::try_from(board.active().len()).unwrap_or(u32::MAX);
    let files_remaining = backlog.upload_files.saturating_add(backlog.download_files).max(moving);
    let finished = board.finished_bytes();
    let in_flight_done = board.in_flight_done();
    let in_flight_total: u64 = board.active().iter().map(|(_, t)| t.total).sum();
    let backlog_bytes = backlog.upload_bytes.saturating_add(backlog.download_bytes);
    let bytes_total = finished.saturating_add(backlog_bytes.max(in_flight_total));
    let bytes_done = finished.saturating_add(in_flight_done).min(bytes_total);
    Activity {
        files_remaining,
        bytes_total,
        bytes_done,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub code: &'static str,
    pub detail: Option<String>,
}

impl Reason {
    fn engine_error() -> Self {
        Self {
            code: REASON_ENGINE_ERROR,
            detail: None,
        }
    }

    fn auth() -> Self {
        Self {
            code: REASON_AUTH,
            detail: None,
        }
    }
}

impl From<LinkReason> for Reason {
    fn from(reason: LinkReason) -> Self {
        Self {
            code: reason.code(),
            detail: reason.detail(),
        }
    }
}

/// One `engine-status` event, before it is turned into JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusPayload {
    pub state: State,
    /// What the pre-slice-2 emitter would have called this (see module docs).
    pub legacy_state: &'static str,
    pub sync_root: Option<String>,
    pub error: Option<String>,
    pub activity: Option<Activity>,
    pub last_tick_ok_at: Option<i64>,
    pub reason: Option<Reason>,
}

impl StatusPayload {
    pub fn to_json(&self) -> Value {
        json!({
            "state": self.state.as_str(),
            "legacy_state": self.legacy_state,
            "sync_root": self.sync_root,
            "error": self.error,
            "files_remaining": self.activity.map(|a| a.files_remaining),
            "bytes_total": self.activity.map(|a| a.bytes_total),
            "bytes_done": self.activity.map(|a| a.bytes_done),
            "last_tick_ok_at": self.last_tick_ok_at,
            "reason": self.reason.as_ref().map(|r| json!({ "code": r.code, "detail": r.detail })),
        })
    }
}

/// The payload for the states that carry nothing else (`running`, `stopped`,
/// `paused`, and the runner's start-up failures): what `emit_status` always sent,
/// plus the new keys at their empty values. A plain `error` is a whole-engine
/// failure (reason `engine_error`).
pub fn plain_payload(state: &str, sync_root: Option<String>, error: Option<&str>) -> Value {
    let (state, reason) = match state {
        "running" => (State::Running, None),
        "idle" => (State::Idle, None),
        "syncing" => (State::Syncing, None),
        "paused" => (State::Paused, None),
        "offline" => (State::Offline, None),
        "error" => (State::Error, Some(Reason::engine_error())),
        _ => (State::Stopped, None),
    };
    StatusPayload {
        state,
        legacy_state: state.as_str(),
        sync_root,
        error: error.map(str::to_string),
        activity: None,
        last_tick_ok_at: None,
        reason,
    }
    .to_json()
}

// ── What the old consumers read ─────────────────────────────────────────────

/// The state the pre-slice-2 emitter would have put in `state`: `legacy_state`
/// when the payload has one, otherwise `state` itself (the pause toggle's bare
/// `{"state": "paused"}`). The tray tooltip and `sync_status` read this, so the
/// new `syncing` and `offline` values change nothing a user sees before slice 6.
pub fn legacy_state(payload: &Value) -> &str {
    payload
        .get("legacy_state")
        .or_else(|| payload.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Collapse the mirrored engine state into the tri-state string the settings pages
/// render (`sync_status`'s `engine`). Moved here unchanged so a test can pin it.
pub fn sync_status_engine(raw: &str) -> &'static str {
    match raw {
        "running" | "idle" | "syncing" => "running",
        "error" => "error",
        _ => "stopped",
    }
}

/// The tray tooltip for one `engine-status` payload (moved here unchanged from
/// `attach_tray_status_listener`, now reading [`legacy_state`]).
pub fn tray_tooltip(payload: &Value) -> String {
    let files_remaining = payload.get("files_remaining").and_then(Value::as_u64);
    let error = payload.get("error").and_then(Value::as_str);
    match legacy_state(payload) {
        "idle" => "Beebeeb · Synced".to_string(),
        "syncing" => match files_remaining {
            Some(0) | None => "Beebeeb · Syncing…".to_string(),
            Some(1) => "Beebeeb · Syncing 1 file…".to_string(),
            Some(n) => format!("Beebeeb · Syncing {n} files…"),
        },
        "paused" => "Beebeeb · Paused".to_string(),
        "offline" => "Beebeeb · Offline".to_string(),
        "error" => match error {
            Some(msg) if !msg.is_empty() => format!("Beebeeb · Error: {msg}"),
            _ => "Beebeeb · Error".to_string(),
        },
        "stopped" => "Beebeeb · Not signed in".to_string(),
        other => format!("Beebeeb · {other}"),
    }
}

// ── Tick outcome ────────────────────────────────────────────────────────────

/// Why a tick counts as down. `Offline` and `NoAnswer` are the two link
/// failures of spec state d2 and d; `Engine` is a whole-engine failure that is
/// not the link (state d copy, reason `engine_error`); `Auth` is the server
/// answering 401.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Down {
    Offline(Reason),
    NoAnswer(Reason),
    Engine(Reason),
    Auth(Reason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Returned `Ok` and no request in it failed at the link level.
    Synced,
    /// `error` is the tick's own `Err` text, `None` when the tick returned `Ok`
    /// but a request inside it failed at the link level (`sync_tick` swallows a
    /// failed `GET /sync/ops`; see `link_health`).
    Down { down: Down, error: Option<String> },
}

fn down_from_link(failure: LinkFailure) -> Down {
    let reason = Reason::from(failure.reason);
    match failure.kind {
        LinkFailureKind::Offline => Down::Offline(reason),
        LinkFailureKind::ServerDidNotAnswer => Down::NoAnswer(reason),
    }
}

/// Decide what a finished tick means. Pure.
///
/// - `Err`: the error's own class first (a timeout, a refused connection, a 5xx
///   in its text), then a link failure recorded during the tick, then 401, then
///   "the engine failed for a reason that is not the link".
/// - `Ok`: a link failure recorded at or after `tick_started_at` still makes the
///   tick a failure; an older one (a previous tick's, not yet cleared because this
///   tick made no request) does not.
pub fn tick_outcome(result: Result<(), &anyhow::Error>, link: &LinkSnapshot, tick_started_at: i64) -> Outcome {
    let fresh_link_failure = link
        .failure
        .filter(|(_, at)| *at >= tick_started_at)
        .map(|(failure, _)| failure);
    match result {
        Ok(()) => match fresh_link_failure {
            Some(failure) => Outcome::Down {
                down: down_from_link(failure),
                error: None,
            },
            None => Outcome::Synced,
        },
        Err(e) => {
            let text = e.to_string();
            let down = if let Some(failure) = link_health::classify_error(e).or(fresh_link_failure) {
                down_from_link(failure)
            } else if matches!(
                crate::engine_bridge::classify_operation_error(&text),
                crate::engine_bridge::OperationFailureClass::Auth
            ) {
                Down::Auth(Reason::auth())
            } else {
                Down::Engine(Reason::engine_error())
            };
            Outcome::Down {
                down,
                error: Some(text),
            }
        }
    }
}

// ── The tracker ─────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Inner {
    sync_root: Option<String>,
    state: State,
    error: Option<String>,
    reason: Option<Reason>,
    last_tick_ok_at: Option<i64>,
    /// `true` once the pulse has reported `syncing`, so it knows to report the
    /// way back to `idle` (and only then).
    showing_syncing: bool,
    /// What the pre-popover emitter last said for this state (`running` until the first tick
    /// ends, then `idle`, `error`, `paused`, `stopped`). The pulse repeats it, so a user who
    /// reads the tray tooltip sees the same words while a file moves as before this slice.
    legacy_state: &'static str,
}

/// Owns "what did we last tell the UI", so a one-second pulse and the tick can
/// both emit without contradicting each other. Every method takes the one lock,
/// so the pulse's read of the activity and the tick's final state are ordered.
#[derive(Debug)]
pub struct StatusTracker {
    inner: Mutex<Inner>,
}

impl StatusTracker {
    pub fn new(sync_root: Option<String>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                sync_root,
                state: State::Running,
                error: None,
                reason: None,
                last_tick_ok_at: None,
                showing_syncing: false,
                legacy_state: "running",
            }),
        }
    }

    fn payload(inner: &Inner, legacy_state: &'static str, activity: Option<Activity>) -> StatusPayload {
        StatusPayload {
            state: inner.state,
            legacy_state,
            sync_root: inner.sync_root.clone(),
            error: inner.error.clone(),
            activity,
            last_tick_ok_at: inner.last_tick_ok_at,
            reason: inner.reason.clone(),
        }
    }

    /// The engine has started (`running`): nothing is known yet.
    pub fn running(&self) -> StatusPayload {
        let mut inner = self.inner.lock().expect("status tracker poisoned");
        inner.state = State::Running;
        inner.error = None;
        inner.reason = None;
        inner.last_tick_ok_at = None;
        inner.showing_syncing = false;
        inner.legacy_state = "running";
        Self::payload(&inner, "running", None)
    }

    /// A tick was skipped because the user paused sync.
    pub fn paused(&self) -> StatusPayload {
        let mut inner = self.inner.lock().expect("status tracker poisoned");
        inner.state = State::Paused;
        inner.error = None;
        inner.reason = None;
        inner.showing_syncing = false;
        inner.legacy_state = "paused";
        Self::payload(&inner, "paused", None)
    }

    pub fn stopped(&self) -> StatusPayload {
        let mut inner = self.inner.lock().expect("status tracker poisoned");
        inner.state = State::Stopped;
        inner.error = None;
        inner.reason = None;
        inner.last_tick_ok_at = None;
        inner.showing_syncing = false;
        inner.legacy_state = "stopped";
        Self::payload(&inner, "stopped", None)
    }

    /// A tick finished. `now` is unix seconds; `activity` is the work still in
    /// flight after the tick.
    pub fn finish_tick(&self, outcome: &Outcome, now: i64, activity: Activity) -> StatusPayload {
        let mut inner = self.inner.lock().expect("status tracker poisoned");
        inner.showing_syncing = false;
        match outcome {
            Outcome::Synced => {
                inner.state = State::Idle;
                inner.error = None;
                inner.reason = None;
                inner.last_tick_ok_at = Some(now);
                inner.legacy_state = "idle";
                Self::payload(&inner, "idle", Some(activity))
            }
            Outcome::Down { down, error } => {
                let (state, reason) = match down {
                    Down::Offline(reason) => (State::Offline, reason),
                    Down::NoAnswer(reason) | Down::Engine(reason) | Down::Auth(reason) => (State::Error, reason),
                };
                inner.state = state;
                inner.error = error.clone();
                inner.reason = Some(reason.clone());
                // The old emitter said `error` for a tick that returned `Err` and
                // `idle` for one that returned `Ok`, whatever the link did.
                let legacy = if error.is_some() { "error" } else { "idle" };
                inner.legacy_state = legacy;
                Self::payload(&inner, legacy, None)
            }
        }
    }

    /// Called about once a second. Returns the event to emit, if the picture
    /// changed. `paused` is the live pause flag. `transfer_in_flight` is the
    /// in-memory board's answer ("is any file moving right now"), which costs
    /// nothing: `read_activity` (a few state-DB queries) is only called when a
    /// transfer is moving or the last pulse reported syncing, so an idle engine
    /// does no work here. It runs under the lock.
    pub fn pulse(
        &self,
        paused: bool,
        transfer_in_flight: bool,
        read_activity: impl FnOnce() -> Activity,
    ) -> Option<StatusPayload> {
        let mut inner = self.inner.lock().expect("status tracker poisoned");
        if inner.state == State::Stopped {
            // The engine has been told to stop. A pulse that is still running in the last moments
            // of teardown must not announce `paused` or `syncing` after `stopped`.
            return None;
        }
        if paused {
            if inner.state == State::Paused {
                return None;
            }
            inner.state = State::Paused;
            inner.error = None;
            inner.reason = None;
            inner.showing_syncing = false;
            inner.legacy_state = "paused";
            return Some(Self::payload(&inner, "paused", None));
        }
        if inner.state == State::Paused {
            // Resumed: the pause toggle already told the UI `idle`; the next tick
            // decides the rest, and work in flight shows as syncing meanwhile.
            inner.state = State::Idle;
            inner.legacy_state = "idle";
        }
        if !matches!(inner.state, State::Running | State::Idle | State::Syncing) {
            // Offline, error and stopped outrank syncing (spec section 3).
            return None;
        }
        if !transfer_in_flight && !inner.showing_syncing {
            return None;
        }
        let activity = read_activity();
        if activity.files_remaining > 0 {
            inner.state = State::Syncing;
            inner.showing_syncing = true;
            return Some(Self::payload(&inner, inner.legacy_state, Some(activity)));
        }
        if inner.showing_syncing {
            inner.state = State::Idle;
            inner.showing_syncing = false;
            return Some(Self::payload(&inner, inner.legacy_state, Some(activity)));
        }
        None
    }
}

// ── What the UI side keeps ──────────────────────────────────────────────────

/// The latest `engine-status` the listener saw, for the snapshot command to read
/// on first paint (the same idea as `AccountRuntime::engine_state`, with the new
/// fields).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineStatusView {
    pub state: String,
    pub files_remaining: Option<u32>,
    pub bytes_total: Option<u64>,
    pub bytes_done: Option<u64>,
    pub last_tick_ok_at: Option<i64>,
    pub reason_code: Option<String>,
    pub reason_detail: Option<String>,
}

impl EngineStatusView {
    /// Fold one payload in. A key that is absent (the pause toggle sends only
    /// `{state}`) clears the progress and the reason, and keeps
    /// `last_tick_ok_at`: a pause does not change when the last check worked.
    pub fn apply(&mut self, payload: &Value) {
        if let Some(state) = payload.get("state").and_then(Value::as_str) {
            self.state = state.to_string();
        }
        self.files_remaining = payload
            .get("files_remaining")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());
        self.bytes_total = payload.get("bytes_total").and_then(Value::as_u64);
        self.bytes_done = payload.get("bytes_done").and_then(Value::as_u64);
        // The key is always present on a tracker payload (a number, or null when there has been
        // no check yet, or when the engine has just stopped or started), so the payload is the
        // truth. It is absent only on the pause toggle's bare `{state}`, which says nothing
        // about checks.
        if let Some(at) = payload.get("last_tick_ok_at") {
            self.last_tick_ok_at = at.as_i64();
        }
        let reason = payload.get("reason").filter(|r| r.is_object());
        self.reason_code = reason
            .and_then(|r| r.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string);
        self.reason_detail = reason
            .and_then(|r| r.get("detail"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    /// The connectivity the popover phase reducer takes. `error` with an `auth`
    /// reason is the server answering 401: not a connectivity problem.
    pub fn connectivity(&self) -> Connectivity {
        match (self.state.as_str(), self.reason_code.as_deref()) {
            ("offline", _) => Connectivity::Offline,
            ("error", Some(REASON_AUTH)) => Connectivity::Online,
            ("error", _) => Connectivity::ServerDidNotAnswer,
            _ => Connectivity::Online,
        }
    }

    pub fn is_syncing(&self) -> bool {
        self.state == "syncing"
    }
}

/// The view, shared between the listener (writer) and the snapshot command.
pub type SharedStatusView = Arc<Mutex<EngineStatusView>>;

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link_health::{LinkFailure, LinkFailureKind, LinkReason};

    const ROOT: &str = "/Users/sam/Beebeeb";

    fn tracker() -> StatusTracker {
        StatusTracker::new(Some(ROOT.to_string()))
    }

    fn busy(files: u32, total: u64, done: u64) -> Activity {
        Activity {
            files_remaining: files,
            bytes_total: total,
            bytes_done: done,
        }
    }

    fn offline_failure() -> LinkFailure {
        LinkFailure {
            kind: LinkFailureKind::Offline,
            reason: LinkReason::Connect,
        }
    }

    fn timeout_failure() -> LinkFailure {
        LinkFailure {
            kind: LinkFailureKind::ServerDidNotAnswer,
            reason: LinkReason::Timeout,
        }
    }

    fn link_with(failure: Option<(LinkFailure, i64)>) -> LinkSnapshot {
        LinkSnapshot {
            last_ok_at: None,
            failure,
        }
    }

    // ── the payload, one test per state ─────────────────────────────────────

    #[test]
    fn a_running_payload_is_the_old_shape_with_empty_new_keys() {
        let p = tracker().running().to_json();
        assert_eq!(
            p,
            json!({
                "state": "running", "legacy_state": "running", "sync_root": ROOT, "error": null,
                "files_remaining": null, "bytes_total": null, "bytes_done": null,
                "last_tick_ok_at": null, "reason": null
            })
        );
    }

    #[test]
    fn an_idle_payload_after_a_good_tick_carries_the_check_time_and_no_reason() {
        let t = tracker();
        let p = t
            .finish_tick(&Outcome::Synced, 1_760_000_000, Activity::default())
            .to_json();
        assert_eq!(
            p,
            json!({
                "state": "idle", "legacy_state": "idle", "sync_root": ROOT, "error": null,
                "files_remaining": 0, "bytes_total": 0, "bytes_done": 0,
                "last_tick_ok_at": 1_760_000_000_i64, "reason": null
            })
        );
    }

    #[test]
    fn a_syncing_payload_carries_files_and_bytes_and_still_says_idle_to_old_consumers() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 100, Activity::default());
        let p = t
            .pulse(false, true, || busy(14, 1_288_490_189, 489_626_271))
            .expect("work in flight")
            .to_json();
        assert_eq!(
            p,
            json!({
                "state": "syncing", "legacy_state": "idle", "sync_root": ROOT, "error": null,
                "files_remaining": 14, "bytes_total": 1_288_490_189_u64, "bytes_done": 489_626_271_u64,
                "last_tick_ok_at": 100, "reason": null
            })
        );
    }

    #[test]
    fn a_paused_payload_is_paused_with_no_progress() {
        let p = tracker().paused().to_json();
        assert_eq!(p["state"], "paused");
        assert_eq!(p["legacy_state"], "paused");
        assert_eq!(p["files_remaining"], Value::Null);
        assert_eq!(p["reason"], Value::Null);
    }

    #[test]
    fn an_offline_payload_from_a_failed_tick_keeps_the_old_error_state_for_old_consumers() {
        let t = tracker();
        let err = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)).context("sync_ops");
        let outcome = tick_outcome(Err(&err), &link_with(None), 0);
        let p = t.finish_tick(&outcome, 500, Activity::default()).to_json();
        assert_eq!(p["state"], "offline");
        assert_eq!(p["legacy_state"], "error", "the old emitter said error for an Err tick");
        assert_eq!(p["error"], "sync_ops");
        assert_eq!(p["reason"], json!({ "code": "connect", "detail": null }));
        assert_eq!(
            p["last_tick_ok_at"],
            Value::Null,
            "a failed check is not a successful check"
        );
    }

    #[test]
    fn an_offline_payload_from_a_swallowed_failure_keeps_idle_for_old_consumers() {
        // `sync_tick` returned Ok but its `GET /sync/ops` failed at the link.
        let t = tracker();
        let outcome = tick_outcome(Ok(()), &link_with(Some((offline_failure(), 50))), 40);
        let p = t.finish_tick(&outcome, 60, Activity::default()).to_json();
        assert_eq!(p["state"], "offline");
        assert_eq!(p["legacy_state"], "idle", "the old emitter said idle for an Ok tick");
        assert_eq!(p["error"], Value::Null);
        assert_eq!(p["last_tick_ok_at"], Value::Null);
    }

    #[test]
    fn a_server_that_did_not_answer_is_error_with_the_timeout_line() {
        let t = tracker();
        let outcome = tick_outcome(Ok(()), &link_with(Some((timeout_failure(), 50))), 40);
        let p = t.finish_tick(&outcome, 60, Activity::default()).to_json();
        assert_eq!(p["state"], "error");
        assert_eq!(
            p["reason"],
            json!({ "code": "timeout", "detail": "timeout after 30 s" })
        );
    }

    #[test]
    fn a_5xx_carries_the_status_in_the_mono_line() {
        let t = tracker();
        let err = anyhow::anyhow!("HTTP 503 Service Unavailable: down");
        let p = t
            .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 1, Activity::default())
            .to_json();
        assert_eq!(p["state"], "error");
        assert_eq!(p["reason"], json!({ "code": "http_5xx", "detail": "HTTP 503" }));
        assert_eq!(p["legacy_state"], "error");
    }

    #[test]
    fn an_error_that_is_not_the_link_is_an_engine_error() {
        let t = tracker();
        let err = anyhow::anyhow!("sync root vanished");
        let p = t
            .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 1, Activity::default())
            .to_json();
        assert_eq!(p["state"], "error");
        assert_eq!(p["reason"], json!({ "code": "engine_error", "detail": null }));
        assert_eq!(p["error"], "sync root vanished");
    }

    #[test]
    fn a_401_is_the_server_answering_not_a_connectivity_failure() {
        let t = tracker();
        let err = anyhow::anyhow!("HTTP 401 Unauthorized: bad token");
        let p = t
            .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 1, Activity::default())
            .to_json();
        assert_eq!(p["state"], "error");
        assert_eq!(p["reason"]["code"], "auth");
        let mut view = EngineStatusView::default();
        view.apply(&p);
        assert_eq!(view.connectivity(), Connectivity::Online);
    }

    #[test]
    fn a_stopped_payload_is_stopped() {
        let p = tracker().stopped().to_json();
        assert_eq!(p["state"], "stopped");
        assert_eq!(p["legacy_state"], "stopped");
    }

    #[test]
    fn a_plain_error_is_a_whole_engine_failure() {
        let p = plain_payload("error", Some(ROOT.into()), Some("open state.db: locked"));
        assert_eq!(p["state"], "error");
        assert_eq!(p["legacy_state"], "error");
        assert_eq!(p["error"], "open state.db: locked");
        assert_eq!(p["reason"]["code"], "engine_error");
        let running = plain_payload("running", Some(ROOT.into()), None);
        assert_eq!(running["reason"], Value::Null);
    }

    // ── the tick outcome ────────────────────────────────────────────────────

    #[test]
    fn an_ok_tick_with_no_failure_is_synced() {
        assert_eq!(tick_outcome(Ok(()), &link_with(None), 100), Outcome::Synced);
    }

    #[test]
    fn a_link_failure_from_before_the_tick_does_not_fail_it() {
        // The previous tick's failure, not cleared because this tick made no request.
        assert_eq!(
            tick_outcome(Ok(()), &link_with(Some((offline_failure(), 99))), 100),
            Outcome::Synced
        );
        // At the same second counts (the tick started at 100, the request failed at 100).
        assert!(matches!(
            tick_outcome(Ok(()), &link_with(Some((offline_failure(), 100))), 100),
            Outcome::Down {
                down: Down::Offline(_),
                ..
            }
        ));
    }

    #[test]
    fn the_errors_own_class_beats_a_link_failure_recorded_in_the_tick() {
        let err = anyhow::anyhow!("HTTP 502 Bad Gateway");
        let outcome = tick_outcome(Err(&err), &link_with(Some((offline_failure(), 10))), 0);
        assert!(
            matches!(
                outcome,
                Outcome::Down {
                    down: Down::NoAnswer(_),
                    ..
                }
            ),
            "{outcome:?}"
        );
    }

    // ── the tracker: pulse ──────────────────────────────────────────────────

    #[test]
    fn the_pulse_says_nothing_while_idle_and_nothing_is_in_flight() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 1, Activity::default());
        assert_eq!(t.pulse(false, true, Activity::default), None);
        assert_eq!(t.pulse(false, true, Activity::default), None);
    }

    #[test]
    fn the_pulse_reports_syncing_while_work_is_in_flight_and_idle_once_when_it_ends() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 1, Activity::default());
        let s = t.pulse(false, true, || busy(3, 300, 100)).unwrap();
        assert_eq!(s.state, State::Syncing);
        assert_eq!(s.activity, Some(busy(3, 300, 100)));
        // Progress changes are reported as they happen.
        let s = t.pulse(false, true, || busy(2, 300, 250)).unwrap();
        assert_eq!(s.activity, Some(busy(2, 300, 250)));
        // The way back: one idle, then silence.
        let s = t.pulse(false, true, Activity::default).unwrap();
        assert_eq!(s.state, State::Idle);
        assert_eq!(t.pulse(false, true, Activity::default), None);
    }

    #[test]
    fn offline_error_and_stopped_outrank_syncing() {
        for outcome in [
            Outcome::Down {
                down: Down::Offline(Reason::from(LinkReason::Connect)),
                error: None,
            },
            Outcome::Down {
                down: Down::NoAnswer(Reason::from(LinkReason::Timeout)),
                error: None,
            },
        ] {
            let t = tracker();
            t.finish_tick(&outcome, 1, Activity::default());
            assert_eq!(t.pulse(false, true, || busy(5, 10, 1)), None, "{outcome:?}");
        }
        let t = tracker();
        t.stopped();
        assert_eq!(t.pulse(false, true, || busy(5, 10, 1)), None);
    }

    #[test]
    fn pausing_wins_over_a_syncing_pulse_at_once_and_is_reported_once() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 1, Activity::default());
        assert_eq!(
            t.pulse(false, true, || busy(3, 300, 100)).unwrap().state,
            State::Syncing
        );
        let paused = t
            .pulse(true, true, || busy(3, 300, 150))
            .expect("pause must be reported");
        assert_eq!(paused.state, State::Paused);
        assert_eq!(t.pulse(true, true, || busy(3, 300, 200)), None, "already told");
    }

    #[test]
    fn resuming_lets_the_pulse_report_work_in_flight_before_the_next_tick() {
        let t = tracker();
        t.paused();
        assert_eq!(t.pulse(false, true, Activity::default), None);
        let s = t
            .pulse(false, true, || busy(1, 10, 0))
            .expect("resumed with work in flight");
        assert_eq!(s.state, State::Syncing);
    }

    #[test]
    fn the_end_of_a_tick_overrides_a_syncing_pulse() {
        let t = tracker();
        assert_eq!(t.pulse(false, true, || busy(2, 20, 5)).unwrap().state, State::Syncing);
        let p = t.finish_tick(&Outcome::Synced, 9, Activity::default());
        assert_eq!(p.state, State::Idle);
        assert_eq!(p.last_tick_ok_at, Some(9));
        // The pulse does not repeat an idle the tick already sent.
        assert_eq!(t.pulse(false, true, Activity::default), None);
    }

    #[test]
    fn a_failed_check_keeps_the_time_of_the_last_good_one() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 100, Activity::default());
        let down = Outcome::Down {
            down: Down::Offline(Reason::from(LinkReason::Connect)),
            error: None,
        };
        let p = t.finish_tick(&down, 200, Activity::default());
        assert_eq!(p.last_tick_ok_at, Some(100));
    }

    #[test]
    fn a_late_pulse_never_announces_anything_after_stopped() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 1, Activity::default());
        t.stopped();
        // Sync was paused when the engine stopped, and work is still marked in flight.
        assert_eq!(t.pulse(true, true, || busy(3, 30, 1)), None, "not `paused` after `stopped`");
        assert_eq!(t.pulse(false, true, || busy(3, 30, 1)), None, "not `syncing` after `stopped`");
        // A new session starts from `running` and works again.
        t.running();
        assert_eq!(t.pulse(false, true, || busy(3, 30, 1)).unwrap().state, State::Syncing);
    }

    #[test]
    fn the_pulse_does_not_read_the_activity_when_it_cannot_matter() {
        let t = tracker();
        t.stopped();
        let mut called = false;
        let _ = t.pulse(false, true, || {
            called = true;
            Activity::default()
        });
        assert!(!called);
    }

    // ── the view ────────────────────────────────────────────────────────────

    #[test]
    fn the_view_folds_each_payload_and_keeps_the_last_check_across_a_bare_pause() {
        let t = tracker();
        let mut view = EngineStatusView::default();
        view.apply(&t.finish_tick(&Outcome::Synced, 777, Activity::default()).to_json());
        assert_eq!(view.state, "idle");
        assert_eq!(view.last_tick_ok_at, Some(777));
        view.apply(&t.pulse(false, true, || busy(4, 40, 10)).unwrap().to_json());
        assert!(view.is_syncing());
        assert_eq!(
            (view.files_remaining, view.bytes_total, view.bytes_done),
            (Some(4), Some(40), Some(10))
        );
        // The pause toggle in lib.rs sends only `{"state": "paused"}`.
        view.apply(&json!({ "state": "paused" }));
        assert_eq!(view.state, "paused");
        assert_eq!(view.files_remaining, None);
        assert_eq!(view.last_tick_ok_at, Some(777));
        assert_eq!(view.reason_code, None);
    }

    #[test]
    fn a_stopped_or_restarted_engine_has_no_last_check() {
        let t = tracker();
        let mut view = EngineStatusView::default();
        view.apply(&t.finish_tick(&Outcome::Synced, 777, Activity::default()).to_json());
        assert_eq!(view.last_tick_ok_at, Some(777));
        // Lock or sign-out: the engine stops.
        view.apply(&t.stopped().to_json());
        assert_eq!(view.last_tick_ok_at, None, "a stopped engine has no last check");
        // The next session starts: still nothing until its first good tick.
        view.apply(&t.finish_tick(&Outcome::Synced, 900, Activity::default()).to_json());
        assert_eq!(view.last_tick_ok_at, Some(900));
        view.apply(&t.running().to_json());
        assert_eq!(view.last_tick_ok_at, None, "a restarted engine has not checked yet");
    }

    #[test]
    fn the_view_maps_each_state_to_the_phase_connectivity() {
        let cases: [(Value, Connectivity); 6] = [
            (json!({ "state": "idle" }), Connectivity::Online),
            (json!({ "state": "syncing" }), Connectivity::Online),
            (
                json!({ "state": "offline", "reason": { "code": "connect", "detail": null } }),
                Connectivity::Offline,
            ),
            (
                json!({ "state": "error", "reason": { "code": "timeout", "detail": "timeout after 30 s" } }),
                Connectivity::ServerDidNotAnswer,
            ),
            (
                json!({ "state": "error", "reason": { "code": "engine_error", "detail": null } }),
                Connectivity::ServerDidNotAnswer,
            ),
            (
                json!({ "state": "error", "reason": { "code": "auth", "detail": null } }),
                Connectivity::Online,
            ),
        ];
        for (payload, expect) in cases {
            let mut view = EngineStatusView::default();
            view.apply(&payload);
            assert_eq!(view.connectivity(), expect, "{payload}");
        }
    }

    // ── no work while idle ──────────────────────────────────────────────────

    #[test]
    fn an_idle_engine_with_nothing_moving_does_not_touch_the_database() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 1, Activity::default());
        let mut reads = 0;
        for _ in 0..50 {
            let _ = t.pulse(false, false, || {
                reads += 1;
                Activity::default()
            });
        }
        assert_eq!(reads, 0, "50 quiet pulses must read the state DB 0 times");
    }

    #[test]
    fn once_syncing_the_pulse_keeps_reading_until_it_has_reported_the_way_back() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 1, Activity::default());
        // A file is moving: read and report syncing.
        assert!(t.pulse(false, true, || busy(2, 20, 5)).is_some());
        // Between two files nothing is on the board, but work is still queued.
        let s = t.pulse(false, false, || busy(1, 10, 10)).expect("still syncing");
        assert_eq!(s.state, State::Syncing);
        // The queue drained and nothing moves: one idle, then quiet with no reads.
        assert_eq!(t.pulse(false, false, Activity::default).unwrap().state, State::Idle);
        let mut reads = 0;
        assert_eq!(
            t.pulse(false, false, || {
                reads += 1;
                Activity::default()
            }),
            None
        );
        assert_eq!(reads, 0);
    }

    // ── the numbers ─────────────────────────────────────────────────────────

    fn backlog(up_files: u32, up_bytes: u64, down_files: u32, down_bytes: u64) -> crate::state_db::TransferBacklog {
        crate::state_db::TransferBacklog {
            upload_files: up_files,
            upload_bytes: up_bytes,
            download_files: down_files,
            download_bytes: down_bytes,
            ..Default::default()
        }
    }

    #[test]
    fn the_overall_numbers_add_finished_bytes_to_what_is_left() {
        use crate::transfer_progress::{Direction, TransferBoard};
        let board = TransferBoard::new();
        let a = board.begin("a", Direction::Up, 400);
        a.update(400);
        a.finish();
        let b = board.begin("b", Direction::Up, 600);
        b.update(150);
        // 400 finished; b (600) and c (1000) still queued.
        let a = compute_activity(&backlog(2, 1_600, 0, 0), &board);
        assert_eq!(a.files_remaining, 2);
        assert_eq!(a.bytes_total, 400 + 1_600);
        assert_eq!(a.bytes_done, 400 + 150);
    }

    #[test]
    fn the_overall_bar_never_runs_backwards_when_a_file_finishes() {
        use crate::transfer_progress::{Direction, TransferBoard};
        let board = TransferBoard::new();
        let b = board.begin("b", Direction::Up, 500);
        b.update(400);
        let before = compute_activity(&backlog(2, 1_000, 0, 0), &board);
        b.update(500);
        b.finish();
        // b left the queue: 1 file and 500 bytes remain.
        let after = compute_activity(&backlog(1, 500, 0, 0), &board);
        assert_eq!(
            before.bytes_total, after.bytes_total,
            "the total is the batch, not what is left"
        );
        assert!(after.bytes_done > before.bytes_done);
    }

    #[test]
    fn a_new_file_whose_row_says_zero_bytes_uses_the_size_the_board_knows() {
        use crate::transfer_progress::{Direction, TransferBoard};
        let board = TransferBoard::new();
        let g = board.begin("new", Direction::Up, 800);
        g.update(300);
        let a = compute_activity(&backlog(1, 0, 0, 0), &board);
        assert_eq!(a.bytes_total, 800);
        assert_eq!(a.bytes_done, 300);
        assert!(a.bytes_done <= a.bytes_total);
    }

    #[test]
    fn a_moving_upload_with_no_queue_row_still_counts_as_a_file_left() {
        use crate::transfer_progress::{Direction, TransferBoard};
        // Keep Mine uploads directly, without enqueuing an operation.
        let board = TransferBoard::new();
        let g = board.begin("keep-mine", Direction::Up, 700);
        g.update(200);
        let a = compute_activity(&backlog(0, 0, 0, 0), &board);
        assert_eq!(a.files_remaining, 1);
        assert_eq!((a.bytes_total, a.bytes_done), (700, 200));
        // A queued file that is also on the board is one file, not two.
        let a = compute_activity(&backlog(1, 700, 0, 0), &board);
        assert_eq!(a.files_remaining, 1);
    }

    #[test]
    fn downloads_count_as_files_left() {
        use crate::transfer_progress::TransferBoard;
        let board = TransferBoard::new();
        let a = compute_activity(&backlog(3, 30, 2, 20), &board);
        assert_eq!(a.files_remaining, 5);
        assert_eq!(a.bytes_total, 50);
        assert_eq!(a.bytes_done, 0);
    }

    // ── nothing a user sees today changes ───────────────────────────────────

    /// What the pre-slice-2 `emit_status` built, for the same situations.
    fn old_payload(state: &str, error: Option<&str>) -> Value {
        json!({ "state": state, "sync_root": ROOT, "error": error, "files_remaining": null })
    }

    fn assert_same_to_a_user(new: &Value, old: &Value) {
        assert_eq!(tray_tooltip(new), tray_tooltip(old), "tooltip: new {new} vs old {old}");
        assert_eq!(
            sync_status_engine(legacy_state(new)),
            sync_status_engine(legacy_state(old)),
            "sync_status engine: new {new} vs old {old}"
        );
    }

    #[test]
    fn every_new_state_looks_to_old_consumers_like_the_old_emitter_said() {
        let t = tracker();
        // A good tick: old said idle.
        let good = t.finish_tick(&Outcome::Synced, 5, Activity::default()).to_json();
        assert_same_to_a_user(&good, &old_payload("idle", None));
        // Work in flight: old said idle (it never said syncing).
        let syncing = t.pulse(false, true, || busy(14, 100, 10)).unwrap().to_json();
        assert_eq!(syncing["state"], "syncing");
        assert_same_to_a_user(&syncing, &old_payload("idle", None));
        // A tick that returned Ok while the link was down: old said idle.
        let quiet_offline = tick_outcome(Ok(()), &link_with(Some((offline_failure(), 10))), 0);
        let p = t.finish_tick(&quiet_offline, 6, Activity::default()).to_json();
        assert_eq!(p["state"], "offline");
        assert_same_to_a_user(&p, &old_payload("idle", None));
        // A tick that returned Err: old said error with the text, whatever the class.
        for err in [
            "connection refused",
            "HTTP 503 Service Unavailable",
            "decrypt failed",
            "HTTP 401 Unauthorized",
        ] {
            let e = anyhow::anyhow!(err);
            let p = t
                .finish_tick(&tick_outcome(Err(&e), &link_with(None), 0), 7, Activity::default())
                .to_json();
            assert_same_to_a_user(&p, &old_payload("error", Some(err)));
        }
        // Paused, stopped, running: unchanged.
        assert_same_to_a_user(&t.paused().to_json(), &old_payload("paused", None));
        assert_same_to_a_user(&t.stopped().to_json(), &old_payload("stopped", None));
        assert_same_to_a_user(&t.running().to_json(), &old_payload("running", None));
        // The pause toggle in lib.rs: a bare `{state}` with no legacy_state.
        assert_eq!(tray_tooltip(&json!({ "state": "paused" })), "Beebeeb · Paused");
        assert_eq!(legacy_state(&json!({ "state": "idle" })), "idle");
        assert_eq!(legacy_state(&json!({})), "");
    }

    #[test]
    fn a_file_moving_before_the_first_tick_ends_keeps_the_old_running_tooltip() {
        // Before this slice the tray said `Beebeeb · running` until the first tick ended, while
        // the initial upload was still going. The pulse must not turn that into `Synced`.
        let t = tracker();
        let started = t.running().to_json();
        assert_eq!(tray_tooltip(&started), "Beebeeb · running");

        let moving = t.pulse(false, true, || busy(3, 30, 1)).unwrap().to_json();
        assert_eq!(moving["state"], "syncing", "the popover still sees the new state");
        assert_eq!(moving["legacy_state"], "running");
        assert_eq!(tray_tooltip(&moving), "Beebeeb · running");
        assert_same_to_a_user(&moving, &old_payload("running", None));

        // The last file lands before the tick ends: still not `Synced`, the tick has not said so.
        let landed = t.pulse(false, true, Activity::default).unwrap().to_json();
        assert_eq!(landed["state"], "idle");
        assert_eq!(tray_tooltip(&landed), "Beebeeb · running");

        // The tick ends: now it is `Synced`, and later pulses keep saying so.
        let done = t.finish_tick(&Outcome::Synced, 9, Activity::default()).to_json();
        assert_eq!(tray_tooltip(&done), "Beebeeb · Synced");
        let again = t.pulse(false, true, || busy(2, 20, 1)).unwrap().to_json();
        assert_eq!(tray_tooltip(&again), "Beebeeb · Synced");
    }

    #[test]
    fn a_pulse_after_a_resume_says_idle_like_the_pause_toggle_did() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 1, Activity::default());
        assert_eq!(t.pulse(true, false, Activity::default).unwrap().legacy_state, "paused");
        let resumed = t.pulse(false, true, || busy(2, 20, 1)).unwrap().to_json();
        assert_eq!(resumed["state"], "syncing");
        assert_same_to_a_user(&resumed, &old_payload("idle", None));
        // Resumed before the first tick ever ended (paused straight after start).
        let t = tracker();
        t.running();
        assert_eq!(t.pulse(true, false, Activity::default).unwrap().legacy_state, "paused");
        let resumed = t.pulse(false, true, || busy(2, 20, 1)).unwrap().to_json();
        assert_same_to_a_user(&resumed, &old_payload("idle", None));
    }

    #[test]
    fn the_new_states_would_change_the_tooltip_if_the_listener_read_them() {
        // Why `legacy_state` exists: read `state` and two tooltips change today.
        assert_eq!(
            tray_tooltip(&json!({ "state": "syncing", "files_remaining": 14 })),
            "Beebeeb · Syncing 14 files…"
        );
        assert_eq!(tray_tooltip(&json!({ "state": "offline" })), "Beebeeb · Offline");
    }

    // ── the JSON the TypeScript side validates ──────────────────────────────

    #[test]
    fn the_syncing_and_offline_events_are_exactly_the_shared_fixtures() {
        let t = tracker();
        t.finish_tick(&Outcome::Synced, 100, Activity::default());
        let syncing = t
            .pulse(false, true, || busy(14, 1_288_490_189, 489_626_271))
            .unwrap()
            .to_json();
        let expected: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/engine-status.syncing.json")).unwrap();
        assert_eq!(syncing, expected);

        let t = tracker();
        let err = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)).context("sync_ops");
        let offline = t
            .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 500, Activity::default())
            .to_json();
        let expected: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/engine-status.offline.json")).unwrap();
        assert_eq!(offline, expected);
    }
}
