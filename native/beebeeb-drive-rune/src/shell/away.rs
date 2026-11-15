//! The away lock's host half (decision 2519; the owner's ruling of 2026-11-14: app lock off never locks). Core holds
//! no clock: the host says when, core says what. This part only MEASURES and hands what it saw to the engine
//! ([`ShellActions::away_job`]), which compares it with the delay the person chose ("Locks again after", stored as
//! the engine's `lock.after`) and locks through core's `lock_after_away`. No rule lives here.
//!
//! - **Idle.** While the engine's lock is open ([`ShellActions::screen_open`]: whatever the window shows, the first-run
//!   steps and the no-plan screen too; review of #452), the main window's poll asks the engine with how long this PC
//!   has had no input ([`Platform::idle_for`]; a platform that cannot say gives this app's own windows' last input),
//!   counted from no earlier than the lock last opened (an unlock by face is no input). The engine answers when an idle
//!   time could lock next; the poll arms ONE wake for then (`Tx::wake_after`, which polls a hidden window too), never
//!   a timer thread, and never more often than [`LOOK_AT_MOST_EVERY`] apart, so a delay changed in Settings (or app
//!   lock turned on) is followed within that, without a restart.
//! - **Events.** Windows' session lock (the tray's [`TrayEvent::SessionLocked`]) and the main window closing to the
//!   tray are said to the engine at once; it decides whether they lock. So is a wake from sleep
//!   ([`TrayEvent::Resumed`]), as idle time: the PC's time without input before it slept plus the sleep, measured by
//!   the tray at the suspend and the resume, so the wake's own key press cannot hide it.
//! - The engine's look reads the credential store and waits for the lock's write hold: it runs on a worker, one idle
//!   look at a time. The job is made on the window's thread, so it holds the session it was measured for.
//!
//! Must not: lock by itself (only the engine does); keep a look for a session that closed; arm a wake per poll.
//!
//! [`Platform::idle_for`]: super::platform::Platform::idle_for
//! [`TrayEvent::SessionLocked`]: super::platform::TrayEvent::SessionLocked
//! [`TrayEvent::Resumed`]: super::platform::TrayEvent::Resumed
//! [`ShellActions::screen_open`]: super::actions::ShellActions::screen_open

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use rune::{AppHandle, Tx};

use super::actions::{Away, AwayAnswer, ShellActions};
use crate::State;

/// The longest the poll waits between two looks while the session is open: a shorter delay, or app lock turned on,
/// is followed within this.
pub const LOOK_AT_MOST_EVERY: Duration = Duration::from_secs(60);

/// A wake counts from the frame's clock, read before this poll measures: it can come this much before the time it was
/// armed for. A poll within it takes the wake as spent and arms again (never a look left waiting for no poll).
const WAKE_SLACK: Duration = Duration::from_millis(250);

/// One look's answer, from its worker.
struct Looked {
    /// An idle look (an event's answer schedules nothing).
    idle: bool,
    answer: AwayAnswer,
}

pub struct AwayLock {
    /// When the session last opened (seen by the poll): the idle count starts no earlier.
    opened_at: Option<Instant>,
    /// When the next idle look is due. `None`: at the next poll that sees the session open.
    due: Option<Instant>,
    /// The wake asked for and not yet passed: no second one is armed for a later time.
    armed: Option<Instant>,
    /// An idle look on its way: no second one starts.
    looking: bool,
    /// This engine has no automatic lock (the fixture): nothing more is asked.
    without: bool,
    /// Looks made, answers taken in (idle and events) and wakes armed, for a test.
    pub(crate) looks: usize,
    pub(crate) answered: usize,
    pub(crate) arms: usize,
    tx: mpsc::Sender<Looked>,
    rx: mpsc::Receiver<Looked>,
}

impl Default for AwayLock {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            opened_at: None,
            due: None,
            armed: None,
            looking: false,
            without: false,
            looks: 0,
            answered: 0,
            arms: 0,
            tx,
            rx,
        }
    }
}

impl AwayLock {
    /// The main window's poll. `open`: the engine's lock is open ([`ShellActions::screen_open`]).
    /// `pc_idle`: [`Platform::idle_for`](super::platform::Platform::idle_for). Returns whether a look's answer came.
    pub fn poll(
        &mut self,
        open: bool,
        pc_idle: Option<Duration>,
        actions: &Arc<dyn ShellActions>,
        wake: Option<&AppHandle<State>>,
        tx: &mut Tx,
    ) -> bool {
        let now = Instant::now();
        let mut came = false;
        while let Ok(looked) = self.rx.try_recv() {
            came = true;
            self.answered += 1;
            if looked.idle {
                self.looking = false;
                let next = looked.answer.check_again_in.unwrap_or(LOOK_AT_MOST_EVERY);
                self.due = Some(now + next.min(LOOK_AT_MOST_EVERY));
            }
        }
        if self.armed.is_some_and(|armed| armed <= now + WAKE_SLACK) {
            self.armed = None;
        }
        if !open {
            // Locked, signed out, or not yet in: no count runs, and the next opening starts a new one.
            self.opened_at = None;
            self.due = None;
            return came;
        }
        let opened_at = *self.opened_at.get_or_insert(now);
        if self.without || self.looking {
            return came;
        }
        if let Some(due) = self.due.filter(|due| *due > now) {
            self.arm(due, now, tx);
            return came;
        }
        let pc_idle = pc_idle.or_else(|| rune::last_input().map(|at| now.saturating_duration_since(at)));
        let idle = pc_idle
            .unwrap_or(Duration::MAX)
            .min(now.saturating_duration_since(opened_at));
        if self.ask(Away::Idle(idle), actions, wake) {
            self.looking = true;
        } else if !self.without {
            // The worker could not start: try again after the longest wait.
            let due = now + LOOK_AT_MOST_EVERY;
            self.due = Some(due);
            self.arm(due, now, tx);
        }
        came
    }

    /// An event the engine is told at once: Windows' session lock, the main window closing to the tray, a wake from
    /// sleep (as idle time).
    pub fn event(&mut self, seen: Away, actions: &Arc<dyn ShellActions>, wake: Option<&AppHandle<State>>) {
        self.ask(seen, actions, wake);
    }

    /// One look on a worker; its answer comes down the channel and wakes the window. `false`: nothing was asked.
    fn ask(&mut self, seen: Away, actions: &Arc<dyn ShellActions>, wake: Option<&AppHandle<State>>) -> bool {
        let Some(job) = actions.away_job(seen) else {
            self.without = true;
            return false;
        };
        let (answers, wake) = (self.tx.clone(), wake.cloned());
        let idle = matches!(seen, Away::Idle(_));
        let started = std::thread::Builder::new()
            .name("away-lock".into())
            .spawn(move || {
                // A look that panicked answers as one that locked nothing, with the longest wait.
                let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or_default();
                let _ = answers.send(Looked { idle, answer });
                if let Some(main) = wake {
                    main.update(|_, _| {});
                }
            })
            .is_ok();
        if started {
            self.looks += 1;
        }
        started
    }

    /// Ask for a poll at `due`, unless one is armed for then or sooner.
    fn arm(&mut self, due: Instant, now: Instant, tx: &mut Tx) {
        if self.armed.is_some_and(|armed| armed <= due) {
            return;
        }
        tx.wake_after(due.saturating_duration_since(now));
        self.armed = Some(due);
        self.arms += 1;
    }
}

#[cfg(test)]
#[path = "away_tests.rs"]
mod tests;
