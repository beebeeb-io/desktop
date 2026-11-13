//! What a window may read of sync from any thread without waiting (contract
//! `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`, §4.2): the gate as the handle last
//! decided it, where the last start is, the Explorer entry, the pause switch, the connection and the session.
//!
//! The handle writes it at the end of every transition (`crate::app::Engine`); the engine task's own reports reach
//! it through [`ViewHost`], the host the handle gives every engine it starts. `Engine::view` copies it and takes no
//! lock that a transition, the engine slot or the credential store holds: [`ViewCell`]'s lock is held for one copy
//! or one assignment, never across another lock or a call.
//!
//! Must not: carry a path, a file's name, an email, an id, a key or a token.

use std::sync::{Arc, Mutex, PoisonError};

use crate::app::SyncStartRefused;
use crate::engine_status::State;
use crate::host::{ConflictNotice, EngineEvent, EngineHost, Notice};
use crate::signin::gate::GateState;
use crate::transfer_progress::TransferBoard;

/// Where the last start is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// No start since the handle opened, or the engine was stopped.
    NotAsked,
    /// A start is past the keys and on its way to an engine.
    Starting,
    /// An engine task runs, on a folder that is registered (Windows) and bound to the account.
    Running,
    /// The start, or the engine task right after it, refused: nothing syncs.
    Refused(SyncStartRefused),
}

/// The sync folder's entry in File Explorer (Windows' shell registration, best effort: `windows_cf::connect_root`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Explorer {
    /// Not asked: every OS but Windows, before the first start, and after a stop or a refusal.
    Unknown,
    Adding,
    Added,
    /// Windows refused it, with its code when it gave one. Sync runs without it.
    Failed {
        code: Option<i32>,
    },
}

/// One copy of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncView {
    pub gate: GateState,
    pub start: Start,
    /// Which start `start` is about: one more for every start that refused or made an engine, so a window says a
    /// refusal once per attempt.
    pub attempt: u64,
    pub explorer: Explorer,
    pub paused: bool,
    /// The engine's last report said there is no connection.
    pub offline: bool,
    /// The server refused this session's token again and again (`AuthHealth`).
    pub session_ended: bool,
    /// What the running engine last reported of itself (`engine-status`); `None` before its first report.
    pub reported: Option<State>,
    /// When the running engine's last finished check reached the server (Unix seconds), as its last report said;
    /// `None` before its first finished check.
    pub last_check_at: Option<i64>,
}

impl Default for SyncView {
    /// What the handle says before it read anything: signed out (as `Engine::gate` answers then), nothing asked.
    fn default() -> Self {
        SyncView {
            gate: GateState::SignedOut,
            start: Start::NotAsked,
            attempt: 0,
            explorer: Explorer::Unknown,
            paused: false,
            offline: false,
            session_ended: false,
            reported: None,
            last_check_at: None,
        }
    }
}

/// The engine task's report of its own start: it connected the root (and how the Explorer entry went), it refused
/// (and why), or it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartReport {
    Connected { explorer: Explorer },
    Refused(SyncStartRefused),
    Ended,
}

/// The one place the view is kept.
#[derive(Default)]
pub(crate) struct ViewCell(Mutex<SyncView>);

impl ViewCell {
    pub(crate) fn read(&self) -> SyncView {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn write(&self, change: impl FnOnce(&mut SyncView)) {
        change(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner));
    }
}

/// What one report of the engine task changes. A report counts only while the view says an engine runs: a stop
/// writes `NotAsked` before it tells the task to end, so the stopped task's own end changes nothing. An engine that
/// ended by itself, with no reason of its own, is a refusal (`Other`): it no longer syncs, and the view must not say
/// it does (invariant I12).
pub(crate) fn apply(view: &mut SyncView, report: StartReport) {
    if view.start != Start::Running {
        return;
    }
    match report {
        StartReport::Connected { explorer } => view.explorer = explorer,
        StartReport::Refused(kind) => {
            view.start = Start::Refused(kind);
            view.explorer = Explorer::Unknown;
        }
        StartReport::Ended => {
            view.start = Start::Refused(SyncStartRefused::Other);
            view.explorer = Explorer::Unknown;
        }
    }
}

/// The host the handle gives the engine: every call goes on to the host the handle was opened with, and on the way
/// the engine's reports of itself are written into the view.
pub(crate) struct ViewHost {
    inner: Arc<dyn EngineHost>,
    cell: Arc<ViewCell>,
}

impl ViewHost {
    pub(crate) fn new(inner: Arc<dyn EngineHost>, cell: Arc<ViewCell>) -> Self {
        ViewHost { inner, cell }
    }
}

impl EngineHost for ViewHost {
    fn event(&self, event: EngineEvent) {
        if let EngineEvent::Status(status) = &event {
            let (state, checked_at) = (status.status.state, status.status.last_tick_ok_at);
            self.cell.write(|view| {
                view.offline = state == State::Offline;
                view.reported = Some(state);
                view.last_check_at = checked_at;
            });
        }
        self.inner.event(event);
    }

    fn notify(&self, notice: Notice) -> Result<(), String> {
        self.inner.notify(notice)
    }

    fn conflict_opens_window(&self) -> bool {
        self.inner.conflict_opens_window()
    }

    fn open_conflict(&self, conflict: &ConflictNotice) -> Result<(), String> {
        self.inner.open_conflict(conflict)
    }

    fn transfer_board(&self) -> Option<Arc<TransferBoard>> {
        self.inner.transfer_board()
    }

    fn file_provider(&self) -> Option<Arc<dyn crate::file_provider_port::FileProviderPort>> {
        self.inner.file_provider()
    }

    fn now(&self) -> std::time::SystemTime {
        self.inner.now()
    }

    fn start_report(&self, report: StartReport) {
        self.cell.write(|view| apply(view, report));
        self.inner.start_report(report);
        self.inner.wake();
    }

    fn wake(&self) {
        self.inner.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> SyncView {
        SyncView {
            start: Start::Running,
            attempt: 1,
            ..SyncView::default()
        }
    }

    /// The engine task's reports move the view only while it says an engine runs; an engine that ended by itself is
    /// never left saying it syncs.
    #[test]
    fn a_report_counts_only_while_the_view_says_an_engine_runs() {
        let mut view = running();
        apply(
            &mut view,
            StartReport::Connected {
                explorer: Explorer::Added,
            },
        );
        assert_eq!((view.start, view.explorer), (Start::Running, Explorer::Added));
        apply(&mut view, StartReport::Ended);
        assert_eq!(view.start, Start::Refused(SyncStartRefused::Other));
        assert_eq!(view.attempt, 1, "an end is the same attempt");

        let mut connect_failed = running();
        let kind = SyncStartRefused::RegistrationFailed {
            code: Some(0x8007_017C_u32 as i32),
        };
        apply(&mut connect_failed, StartReport::Refused(kind));
        apply(&mut connect_failed, StartReport::Ended);
        assert_eq!(
            connect_failed.start,
            Start::Refused(kind),
            "the end keeps the reason it ended for"
        );

        // A stop wrote `NotAsked` before the task ended: its end report changes nothing.
        let mut stopped = SyncView::default();
        apply(&mut stopped, StartReport::Ended);
        assert_eq!(stopped.start, Start::NotAsked);
    }
}
