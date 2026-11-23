//! RC2-6b: the tick's two per-row placeholder passes (Windows: `refresh_placeholders` and
//! `reconcile_placeholder_state`) run only when something can have changed.
//!
//! On LargeAccount::REFERENCE (41,510 items, 2,741 folders) the two passes listed every folder of the sync root twice on
//! every 30 s tick, nothing changed or not: 1,133 file operations a second on an idle account, locked or not
//! (board/evidence/drive-complete/rc2-6). Both look for things the NTFS change journal names: a placeholder that left
//! its folder (a delete, a rename) and an attribute change made in Explorer (pin, unpin, "Free up space", a
//! hydration). So the tick reads its own journal feed (a second reader beside the scan's, with its own position) and
//! skips both passes when the feed names nothing, the tick applied no item and no queued operation finished.
//!
//! They still run in full: on the first tick of a run, when the feed cannot say ([`Changes::Unknown`], or no feed
//! at all off Windows / without a journal), after the machine slept, once an hour as the backstop, on the tick after
//! one that ran because something happened (a listing that showed an attribute change late is read again before the
//! journal record that named it is forgotten), and on the tick after a pass that left work to retry.

use std::time::{Duration, Instant, SystemTime};

use crate::watcher::{ChangeFeed, Changes};

/// The backstop: the passes run in full at least this often (the scan's `FULL_RECHECK_INTERVAL`, an hour).
pub const PASS_BACKSTOP: Duration = Duration::from_secs(60 * 60);

/// A gap between two ticks this long means the machine slept (the tick is 30 s; the scan uses 4 × its interval):
/// the journal may have wrapped meanwhile.
pub const SLEPT_GAP: Duration = Duration::from_secs(5 * 60);

/// What this tick did besides the passes, as far as the gate cares.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickWork {
    /// Items the pull applied (`tick.applied_item_ids`).
    pub applied_items: usize,
    /// Queued operations that finished (the queue step's count).
    pub finished_ops: usize,
}

/// What one gated run of the passes reported back: whether something is left to retry next tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassOutcome {
    /// A row was left for the next pass (a folder that could not be made or listed, a create that failed, a
    /// name check that did not finish, a listing that lagged).
    pub retry: bool,
}

/// Why the passes run this tick; `None` from [`PassGate::due`] when they do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunWhy {
    /// First tick of this run, or the last run asked for another.
    Owed,
    /// The journal feed could not say what changed.
    FeedUnknown,
    /// The feed named a path under the sync root.
    FeedNamed,
    /// The pull applied items, or queued operations finished.
    TickWork,
    /// The machine slept since the last tick.
    Slept,
    /// The hourly backstop.
    Backstop,
}

impl RunWhy {
    /// A run that something happening caused, which owes one follow-up run.
    fn was_a_change(self) -> bool {
        matches!(self, RunWhy::FeedNamed | RunWhy::TickWork)
    }
}

/// The gate's memory across ticks. One per engine run: a fresh gate runs the passes on its first tick.
#[derive(Debug)]
pub struct PassGate {
    last_full: Option<Instant>,
    last_tick: Option<SystemTime>,
    owed: bool,
}

impl Default for PassGate {
    fn default() -> Self {
        Self {
            last_full: None,
            last_tick: None,
            owed: true,
        }
    }
}

impl PassGate {
    /// Whether the passes run this tick, from what `feed` says and what the tick did. Takes the feed every call, so
    /// a change made while the passes run is named next tick.
    pub fn due(&mut self, feed: &mut dyn ChangeFeed, work: TickWork, now: Instant, wall: SystemTime) -> Option<RunWhy> {
        // Always taken: the feed's position moves on whether or not the passes run.
        let changes = feed.take();
        let slept = self
            .last_tick
            .is_some_and(|last| wall.duration_since(last).is_ok_and(|gap| gap >= SLEPT_GAP));
        self.last_tick = Some(wall);
        if self.owed {
            Some(RunWhy::Owed)
        } else if changes == Changes::Unknown {
            Some(RunWhy::FeedUnknown)
        } else if slept {
            Some(RunWhy::Slept)
        } else if self.last_full.is_none_or(|at| now.duration_since(at) >= PASS_BACKSTOP) {
            Some(RunWhy::Backstop)
        } else if work.applied_items > 0 || work.finished_ops > 0 {
            Some(RunWhy::TickWork)
        } else if matches!(changes, Changes::Paths(_)) {
            Some(RunWhy::FeedNamed)
        } else {
            None
        }
    }

    /// The passes ran (for `why`) and reported `outcome`. A run that did not finish (it panicked) passes
    /// `PassOutcome { retry: true }`.
    pub fn ran(&mut self, why: RunWhy, outcome: PassOutcome, now: Instant) {
        self.last_full = Some(now);
        self.owed = outcome.retry || why.was_a_change();
    }

    /// The whole gated step: decides, runs `passes` when due, records what it reported. `None`: skipped.
    pub fn step(
        &mut self,
        feed: &mut dyn ChangeFeed,
        work: TickWork,
        now: Instant,
        wall: SystemTime,
        passes: impl FnOnce() -> PassOutcome,
    ) -> Option<(RunWhy, PassOutcome)> {
        let why = self.due(feed, work, now, wall)?;
        let outcome = passes();
        self.ran(why, outcome, now);
        Some((why, outcome))
    }
}

#[cfg(test)]
#[path = "pass_gate_tests.rs"]
mod tests;
