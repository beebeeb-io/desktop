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
    plain_status(state, sync_root, error).to_json()
}

/// [`plain_payload`] before it is turned into JSON.
pub fn plain_status(state: &str, sync_root: Option<String>, error: Option<&str>) -> StatusPayload {
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
    /// The engine's startup check of the files on this PC while it runs (0.8.15, see [`STARTUP_CHECK_KEY`]).
    pub startup_check: Option<StartupCheck>,
}

/// How far the engine's startup check of the files on this PC has got: `done` of `total` files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StartupCheck {
    pub done: u64,
    pub total: u64,
}

/// The `engine-status` key that carries the start-up guard's pass (0.8.15), which on Guus's PC walked 37,169 files with
/// no sign in any window: `{ "running": bool, "done": n, "total": n }`, as the engine lane writes it (branch
/// `fix/0815-guard-nonblocking` at 62f794d: `runner::with_startup_guard` on every engine payload, and the same object as
/// `sync_status.startup_guard`). The pause toggle's bare `{state}` payload does not carry it.
pub const STARTUP_CHECK_KEY: &str = "startup_guard";

/// The startup check a payload's [`STARTUP_CHECK_KEY`] value reports: `Some` only while a pass runs over at least one
/// file. A `done` past `total` is clamped.
pub fn startup_check_from_value(value: &Value) -> Option<StartupCheck> {
    let check = value.as_object()?;
    if check.get("running").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let total = check.get("total")?.as_u64()?;
    let done = check.get("done")?.as_u64()?;
    (total > 0).then_some(StartupCheck {
        done: done.min(total),
        total,
    })
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
        // Like `last_tick_ok_at`: every engine payload carries the key, so it is the truth; only the pause toggle's bare
        // `{state}` lacks it, and that says nothing about the pass.
        if let Some(check) = payload.get(STARTUP_CHECK_KEY) {
            self.startup_check = startup_check_from_value(check);
        }
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

    mod compat;
    mod payload;
}
