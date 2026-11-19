//! A helper that dies mid-edit comes back from the last snapshot (contract §5.4), and the snapshot timing that makes
//! there be one.
//!
//! - **Snapshots.** While there are edits since the last snapshot (or save), the worker asks `SaveSize{Snapshot}` itself
//!   after [`Cadence::idle`] without input and at least every [`Cadence::every`]. The result goes to the window as
//!   [`EditEvent::Saved`] (the source stores it) and into [`Held`](super::held::Held). Nothing changed: nothing asked.
//! - **Restart.** The helper exits (`Stop::Exited`) while the document is open for editing: its in-flight edit requests
//!   are answered `Failed`, a new helper is started and told `OpenEdit` with the snapshot (the opened bytes when there is
//!   none; `NewDoc` again for a new document with no snapshot), and once it has opened the window gets
//!   [`EditEvent::Restored`]. The undo history is gone ([`RESTORED_WORDS`]).
//! - A helper that exits while the document is still opening (not yet `Pages`) is replaced the same way, from the opened
//!   bytes and without `Restored`.
//! - **Second crash within [`Cadence::crash_window`]**: the document is opened read-only (the viewer's `Open`), the window
//!   gets [`EditEvent::ReadOnly`] with the snapshot as a copy, and `edit` calls are refused.

use std::time::{Duration, Instant, SystemTime};

use super::edit::{EditAsked, EditCommand, EditEvent, EditFailure, Mode};
use super::super::{OfficeState, Out, Purpose, RequestId, Starter, lock};
use super::held::Held;
use super::{Asked, Command, Doc, Phase, Worker};

/// What the window says after a restart.
pub const RESTORED_WORDS: &str = "The editor stopped and was restarted from your last saved copy. Undo history was lost.";
/// What the window says after a second crash.
pub const READ_ONLY_WORDS: &str = "The editor stopped again. This document is open to read only; your last saved copy is offered as a copy.";

/// The snapshot and crash timing. Production: 2 s idle, 10 s at most between snapshots, 60 s crash window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cadence {
    /// A snapshot after this long without input.
    pub(crate) idle: Duration,
    /// ...and at least this often while there are edits since the last one.
    pub(crate) every: Duration,
    /// A second crash within this long of the first opens the document read-only.
    pub(crate) crash_window: Duration,
}

impl Cadence {
    pub(crate) const PRODUCTION: Cadence = Cadence {
        idle: Duration::from_secs(2),
        every: Duration::from_secs(10),
        crash_window: Duration::from_secs(60),
    };
}

/// Request ids the worker makes itself (snapshots) count down from here, far from the window's (which count up from 0).
const OWN_BASE: u32 = 0xF000_0000;

fn is_own(id: RequestId) -> bool {
    id.0 >= OWN_BASE
}

/// The worker's snapshot and crash bookkeeping.
pub(super) struct State {
    cadence: Cadence,
    crashes: Vec<Instant>,
    /// A helper started by a restart that has not opened yet.
    restarting: bool,
    /// Said once the restarted helper has opened: (when the state it opened is from, whether that is a snapshot).
    restored: Option<(SystemTime, bool)>,
    /// Edits since the last snapshot or save.
    dirty: bool,
    /// Edits since the last save landed (a snapshot does not clear it): what a lock must not lose.
    pub(super) unsaved: bool,
    /// The snapshot the worker asked for a lock, until it lands or fails.
    pub(super) flush_id: Option<RequestId>,
    /// Edits sent so far, and the count when the save that is out was asked.
    pub(super) edit_seq: u64,
    pub(super) save_started: u64,
    last_input: Instant,
    dirty_since: Instant,
    own: u32,
}

impl State {
    pub(super) fn new(cadence: Cadence) -> State {
        let now = Instant::now();
        State {
            cadence,
            crashes: Vec::new(),
            restarting: false,
            restored: None,
            dirty: false,
            unsaved: false,
            flush_id: None,
            edit_seq: 0,
            save_started: 0,
            last_input: now,
            dirty_since: now,
            own: 0,
        }
    }
}

fn asked_id<T>(asked: &EditAsked<T>) -> RequestId {
    match asked {
        EditAsked::Op { id }
        | EditAsked::Copy { id }
        | EditAsked::Layouts { id }
        | EditAsked::Image { id, .. }
        | EditAsked::SaveSize { id, .. }
        | EditAsked::SaveInto { id, .. } => *id,
    }
}

impl<S: Starter> Worker<S> {
    /// An edit (not a click) or an image went out or waits: there is something to snapshot.
    pub(super) fn note_edit(&mut self) {
        let now = Instant::now();
        self.rs.edit_seq += 1;
        self.rs.unsaved = true;
        if !self.rs.dirty {
            self.rs.dirty = true;
            self.rs.dirty_since = now;
        }
        self.rs.last_input = now;
    }

    /// A save was asked: what it will hold is every edit counted so far.
    pub(super) fn note_save_asked(&mut self) {
        self.rs.save_started = self.rs.edit_seq;
    }

    /// A snapshot or save landed: keep it, and say whether edits came after it was asked.
    pub(super) fn keep(&mut self, bytes: &[u8], revision: u64) {
        if let Some(held) = self.held.as_mut() {
            held.store_snapshot(bytes, revision);
        }
        if self.rs.save_started == self.rs.edit_seq {
            self.rs.dirty = false;
        } else {
            self.rs.dirty_since = Instant::now();
        }
    }

    /// A lock is coming ([`super::super::Shared::flush_begin`]): seal the work typed since the last save, now. The
    /// helper is asked for a fresh snapshot; when none can be asked (it is restarting, or gone) the last one held is
    /// sealed. Every way out says done, so the lock never waits for more than its bound.
    pub(super) fn on_flush(&mut self) {
        if !self.rs.unsaved {
            return self.flush_complete();
        }
        if self.editable() {
            self.rs.own = (self.rs.own + 1) % (u32::MAX - OWN_BASE);
            let id = RequestId(OWN_BASE + self.rs.own);
            self.rs.flush_id = Some(id);
            self.edit_command(EditCommand::Save {
                id,
                purpose: Purpose::Snapshot,
            });
            return;
        }
        if let (Some(sink), Some(copy)) = (self.shared.flush_sink(), self.held.as_ref().and_then(Held::snapshot_copy)) {
            sink(&copy);
        }
        self.flush_complete();
    }

    pub(super) fn flush_complete(&mut self) {
        self.rs.flush_id = None;
        self.shared.flush_complete();
    }

    /// A snapshot the helper refused: not again at once.
    pub(super) fn snapshot_backoff(&mut self) {
        let now = Instant::now();
        self.rs.last_input = now;
        self.rs.dirty_since = now;
    }

    pub(super) fn is_own_id(&self, id: RequestId) -> bool {
        is_own(id)
    }

    /// When the next snapshot is due: never while nothing changed, a save is out or the session is not open.
    fn snapshot_at(&self) -> Option<Instant> {
        let run = self.run.as_ref()?;
        if !self.rs.dirty || run.save_busy || !self.editable() {
            return None;
        }
        let idle = self.rs.last_input + self.rs.cadence.idle;
        let every = self.rs.dirty_since + self.rs.cadence.every;
        Some(idle.min(every))
    }

    /// The next thing the worker must act on without input: a request timer, a budget, or a snapshot.
    pub(super) fn next_due(&self) -> Option<(Instant, Due)> {
        let stop = self.deadline().map(|(at, why)| (at, Due::Stop(why)));
        let snapshot = self.snapshot_at().map(|at| (at, Due::Snapshot));
        match (stop, snapshot) {
            (Some(stop), Some(snapshot)) => Some(if snapshot.0 < stop.0 { snapshot } else { stop }),
            (stop, snapshot) => stop.or(snapshot),
        }
    }

    pub(super) fn fire(&mut self, due: Due) {
        match due {
            Due::Stop(why) => self.stopped(why),
            Due::Snapshot => {
                self.rs.own = (self.rs.own + 1) % (u32::MAX - OWN_BASE);
                let id = RequestId(OWN_BASE + self.rs.own);
                self.edit_command(EditCommand::Save {
                    id,
                    purpose: Purpose::Snapshot,
                });
            }
        }
    }

    fn outstanding_edit_ids(&self) -> Vec<RequestId> {
        let Some(run) = self.run.as_ref() else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        for sent in &run.sent {
            if let Asked::Edit(asked) = &sent.asked {
                ids.push(asked_id(asked));
            }
        }
        for command in &run.deferred {
            if let Command::Edit(EditCommand::Send { id, .. } | EditCommand::Image { id, .. } | EditCommand::Save { id, .. }) =
                command
            {
                ids.push(*id);
            }
        }
        ids.retain(|id| !is_own(*id));
        ids
    }

    /// The helper exited. `true`: the session goes on with a new helper (or read-only); `false`: not an editing session
    /// that can be restarted, the caller stops as before.
    pub(super) fn try_restart(&mut self) -> bool {
        // A helper that dies before its document is open (a slow first start, a kill while it loads) is replaced as
        // well: there is nothing to restore then, only the opened bytes to open again.
        let before_open = !self.rs.restarting && matches!(self.phase, Phase::Opening);
        let open = self.rs.restarting || before_open || matches!(self.phase, Phase::Pages { .. } | Phase::Sheets);
        if matches!(self.mode, Mode::View) || !open || self.run.is_none() || self.shared.is_closed() {
            return false;
        }
        let Some(held) = self.held.as_ref() else {
            return false;
        };
        let now = Instant::now();
        let window = self.rs.cadence.crash_window;
        self.rs.crashes.retain(|at| now.duration_since(*at) < window);
        let read_only = !self.rs.crashes.is_empty();
        self.rs.crashes.push(now);
        let bytes = held.restart_bytes();
        if read_only && bytes.is_none() {
            return false;
        }
        let from_snapshot = held.has_snapshot();
        let at = held.at();
        let copy = if read_only { held.snapshot_copy() } else { None };

        let ids = self.outstanding_edit_ids();
        self.kill_run();
        for id in ids {
            self.shared.push(Out::Edit(EditEvent::Failed {
                id,
                why: EditFailure::Failed,
            }));
        }
        let fresh = State::new(self.rs.cadence);
        let crashes = std::mem::take(&mut self.rs.crashes);
        self.rs = State { crashes, ..fresh };

        if let (Mode::New(kind, _), true) = (self.mode, bytes.is_some()) {
            self.mode = Mode::Edit(kind);
        }
        self.doc = bytes.map_or(Doc::Blank, Doc::Bytes);
        if read_only {
            self.mode = Mode::View;
            self.retried = true;
            lock(&self.shared.inner).read_only = true;
            self.shared.push(Out::Edit(EditEvent::ReadOnly { at, copy }));
        } else {
            self.rs.restarting = true;
            // No `Restored` for a helper that never opened: the window has no edits to say are back.
            self.rs.restored = (!before_open).then_some((at, from_snapshot));
        }
        self.phase = Phase::Opening;
        self.shared.push(Out::State(OfficeState::Opening));
        self.start_helper();
        true
    }

    /// The restarted helper has opened: said once.
    pub(super) fn opened_after_restart(&mut self) {
        self.rs.restarting = false;
        if let Some((at, from_snapshot)) = self.rs.restored.take() {
            self.shared.push(Out::Edit(EditEvent::Restored { at, from_snapshot }));
        }
    }
}

/// What the worker acts on without input.
pub(super) enum Due {
    Stop(super::Stop),
    Snapshot,
}
