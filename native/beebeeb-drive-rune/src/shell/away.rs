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
//! - **A gap.** Windows may not send the suspend broadcast (a Modern Standby PC sends it only to a window that
//!   registered for it: the tray registers, `RegisterSuspendResumeNotification`; review of #452, P2-A). So the poll also
//!   notes when it last ran, on two clocks, and the idle it measured then. A gap between two polls far past the look
//!   interval ([`AWAY_GAP`]) means the app did not run: the idle seen last plus the gap is held as idle time and said
//!   to the engine without waiting for the next look (the wake's own key press cannot hide it), as the tray's resume
//!   counts the idle before a sleep and the sleep (decision 2539). No broadcast is needed. A resume for the same wake
//!   measures it better (the poll's measure also counts input made between its last run and the sleep as idle), so it
//!   replaces the gap: one said before the poll finds the gap means the gap is not held, one said after it drops the
//!   held gap, and a found gap waits at most [`RESUME_GRACE`] for it before it is said.
//! - **The cover** (the owner's ruling D-B2-3, privacy). The poll that finds a gap covers the window's content at once:
//!   [`AwayLock::covers`], which the window draws as the lock screen (`shell::registry::view`). The grace only decides
//!   which look says the wake (the resume's, or the gap's after it); the content stays covered until that look's
//!   answer says it locked nothing. A look that locked leaves the cover until a later poll sees the session open
//!   again, so no frame shows the content between the engine's lock and the window's next read of the gate. Where
//!   nothing an idle time says could lock (app lock off: the engine's last idle answer; an engine without the
//!   automatic lock), nothing is covered: app lock off never locks, so it never shows the lock screen either.
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
use std::time::{Duration, Instant, SystemTime};

use rune::{AppHandle, Tx};

use super::actions::{Away, AwayAnswer, ShellActions};
use crate::State;

/// The longest the poll waits between two looks while the session is open: a shorter delay, or app lock turned on,
/// is followed within this.
pub const LOOK_AT_MOST_EVERY: Duration = Duration::from_secs(60);

/// The longest gap between two polls that is not time away: twice the longest wait between two looks (while the
/// engine is open a poll comes at least that often). A longer one means the app did not run: the PC slept without the
/// broadcast, or the process was stopped (also a blocked window or a wall clock set forward reads as one: it locks
/// sooner, never later).
pub const AWAY_GAP: Duration = Duration::from_secs(120);

/// How long a gap the poll found waits for the tray's resume of the same wake ([`TrayEvent::Resumed`]) before it is
/// said to the engine. The resume and the first poll after the wake race, and the poll's measure is the cruder one: it
/// counts input made between its last run and the sleep as idle, so said at once it could lock a person who typed and
/// then closed the lid. A resume within this replaces the gap; with none (Windows sent no broadcast), the gap is said
/// after it. Kept short: the window's content is covered from the poll that found the gap until the engine answered
/// (D-B2-3), so a wake that locks nothing shows the lock screen for this long.
///
/// [`TrayEvent::Resumed`]: super::platform::TrayEvent::Resumed
pub const RESUME_GRACE: Duration = Duration::from_secs(2);

/// When the poll last ran while the engine was open, on two clocks, and the idle it measured then.
#[derive(Clone, Copy, Debug)]
struct Seen {
    at: Instant,
    wall: SystemTime,
    /// This PC's time without input that poll measured, counted from no earlier than the lock last opened.
    idle: Duration,
}

impl Seen {
    /// The time since, by whichever of the two clocks saw more of it: the monotonic clock may not count a sleep (on
    /// Linux it does not), the wall clock does and can be set back (the tray's resume measures the same way).
    fn gap(&self, now: Instant, wall: SystemTime) -> Duration {
        now.saturating_duration_since(self.at)
            .max(wall.duration_since(self.wall).unwrap_or_default())
    }
}

/// A wake counts from the frame's clock, read before this poll measures: it can come this much before the time it was
/// armed for. A poll within it takes the wake as spent and arms again (never a look left waiting for no poll).
const WAKE_SLACK: Duration = Duration::from_millis(250);

/// One look's answer, from its worker.
struct Looked {
    /// An idle look (an event's answer schedules nothing).
    idle: bool,
    /// Which look it was ([`AwayLock::looks`] once it started).
    number: usize,
    /// `None`: the look panicked (it answers as one that locked nothing, with the longest wait).
    answer: Option<AwayAnswer>,
}

/// The window's content under the lock screen's look while a wake may lock (D-B2-3; the module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cover {
    /// Nothing is covered.
    Off,
    /// A gap was found and no look carries it yet: the grace, or a look that was out before it.
    Held,
    /// The look with this number says the wake: its answer lifts the cover, unless it locked.
    Asked(usize),
    /// A look locked, or the poll saw the session closed, under the cover: it lifts at a LATER poll that sees the
    /// session open (the window read the gate again before it, in the same frame).
    Locked,
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
    /// The poll's last run while the engine was open (`None`: closed since).
    seen: Option<Seen>,
    /// Time away found in a gap (the idle seen last plus the gap) and not yet said: the next look carries it, whenever
    /// it is due, once [`RESUME_GRACE`] passed.
    away: Option<Duration>,
    /// When the poll found the gap held in `away`: it is not said before [`RESUME_GRACE`] after it.
    gap_found_at: Option<Instant>,
    /// A resume (the tray's, as idle time) was said since the poll last ran: a gap that poll finds is that wake,
    /// already said, and is not held.
    resumed: bool,
    /// The window's content is covered (D-B2-3).
    cover: Cover,
    /// The engine's last idle answer said an idle time could lock (app lock on). `true` until it answered: a gap
    /// found before is covered.
    could_lock: bool,
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
            seen: None,
            away: None,
            gap_found_at: None,
            resumed: false,
            cover: Cover::Off,
            could_lock: true,
            looks: 0,
            answered: 0,
            arms: 0,
            tx,
            rx,
        }
    }
}

impl AwayLock {
    /// The window's content is covered: drawn as the lock screen while a wake may lock (D-B2-3; the module doc).
    pub fn covers(&self) -> bool {
        self.cover != Cover::Off
    }

    /// The main window's poll. `open`: the engine's lock is open ([`ShellActions::screen_open`]).
    /// `pc_idle`: [`Platform::idle_for`](super::platform::Platform::idle_for). Returns whether a look's answer came or
    /// the cover went on or off (either way the window draws again).
    pub fn poll(
        &mut self,
        open: bool,
        pc_idle: Option<Duration>,
        actions: &Arc<dyn ShellActions>,
        wake: Option<&AppHandle<State>>,
        tx: &mut Tx,
    ) -> bool {
        let covered = self.covers();
        let came = self.measure(open, pc_idle, actions, wake, tx);
        let covers = self.covers();
        if covers && !covered {
            // The content leaves in this frame's commit: every removed element otherwise fades out over 0.15 s, and
            // the files would still show under the cover while they fade.
            tx.instant();
        }
        came || covers != covered
    }

    fn measure(
        &mut self,
        open: bool,
        pc_idle: Option<Duration>,
        actions: &Arc<dyn ShellActions>,
        wake: Option<&AppHandle<State>>,
        tx: &mut Tx,
    ) -> bool {
        let (now, wall) = (Instant::now(), SystemTime::now());
        // A lock under the cover that an earlier poll took in: the window has read the gate since.
        let mut lift_if_open = self.cover == Cover::Locked;
        let mut came = false;
        while let Ok(looked) = self.rx.try_recv() {
            came = true;
            self.answered += 1;
            let answer = looked.answer.unwrap_or_default();
            if looked.idle {
                self.looking = false;
                let next = answer.check_again_in.unwrap_or(LOOK_AT_MOST_EVERY);
                self.due = Some(now + next.min(LOOK_AT_MOST_EVERY));
                // No time to wait and no lock: nothing an idle time says locks (app lock off). A look that panicked
                // says nothing of it.
                if looked.answer.is_some() {
                    self.could_lock = answer.locked || answer.check_again_in.is_some();
                }
            }
            if self.cover != Cover::Off && answer.locked {
                // The engine locked: this frame's gate may have been read before it.
                self.cover = Cover::Locked;
                lift_if_open = false;
            } else if self.cover != Cover::Locked {
                // The look that says the wake locked nothing.
                self.cover = Cover::Off;
            }
        }
        if self.armed.is_some_and(|armed| armed <= now + WAKE_SLACK) {
            self.armed = None;
        }
        if !open {
            // Locked, signed out, or not yet in: no count runs, and the next opening starts a new one (a gap too).
            self.opened_at = None;
            self.due = None;
            self.seen = None;
            self.away = None;
            self.gap_found_at = None;
            self.resumed = false;
            self.could_lock = true;
            if self.cover != Cover::Off {
                self.cover = Cover::Locked;
            }
            return came;
        }
        if lift_if_open {
            // Open again after the lock (an unlock, or another session): the lock screen was the window's.
            self.cover = Cover::Off;
        }
        let opened_at = *self.opened_at.get_or_insert(now);
        let pc_idle = pc_idle.or_else(|| rune::last_input().map(|at| now.saturating_duration_since(at)));
        let idle = pc_idle
            .unwrap_or(Duration::MAX)
            .min(now.saturating_duration_since(opened_at));
        // A gap far past the look interval: the app did not run, and the idle seen last plus the gap is time away. It
        // is held until a look carries it (one may be out now), and it does not wait for the next due. A resume said
        // since the last poll is this wake, measured better: the gap is not held.
        if let Some(seen) = self.seen.replace(Seen { at: now, wall, idle }) {
            let gap = seen.gap(now, wall);
            if gap > AWAY_GAP && !self.resumed {
                let away = seen.idle.saturating_add(gap);
                self.away = Some(self.away.map_or(away, |held| held.max(away)));
                self.gap_found_at = Some(now);
                // D-B2-3: the content is covered from this frame, until a look said this wake.
                if self.could_lock && !self.without {
                    self.cover = Cover::Held;
                }
            }
        }
        self.resumed = false;
        if self.without || self.looking {
            return came;
        }
        // A gap just found waits for a resume of the same wake, which would replace it.
        if let Some(grace_ends) = self
            .gap_found_at
            .map(|found| found + RESUME_GRACE)
            .filter(|grace_ends| *grace_ends > now)
        {
            self.arm(grace_ends, now, tx);
            return came;
        }
        // A cover with no look for its wake (the resume's did not start) does not wait for the due look either.
        if self.away.is_none()
            && self.cover != Cover::Held
            && let Some(due) = self.due.filter(|due| *due > now)
        {
            self.arm(due, now, tx);
            return came;
        }
        let idle = idle.max(self.away.unwrap_or_default());
        if let Some(number) = self.ask(Away::Idle(idle), true, actions, wake) {
            self.looking = true;
            self.away = None;
            self.gap_found_at = None;
            if self.cover == Cover::Held {
                self.cover = Cover::Asked(number);
            }
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
        let resume = matches!(seen, Away::Idle(_));
        if resume {
            // A wake from sleep (the tray's resume, the only idle time said as an event) measured this wake better
            // than a gap the poll found or is yet to find: a held gap is dropped, and the next poll holds none.
            self.resumed = true;
            self.away = None;
            self.gap_found_at = None;
        }
        // An event's answer schedules nothing, also a wake's (idle time, but not the poll's look).
        let number = self.ask(seen, false, actions, wake);
        // The resume says the wake the cover is for: its answer lifts it, unless it locked. One that did not start
        // leaves the cover held, and the next poll looks for it.
        if resume
            && let Some(number) = number
            && matches!(self.cover, Cover::Held | Cover::Asked(_))
        {
            self.cover = Cover::Asked(number);
        }
    }

    /// One look on a worker; its answer comes down the channel and wakes the window. `idle`: the poll's own look,
    /// whose answer says when the next one is due. The look's number, or `None`: nothing was asked.
    fn ask(
        &mut self,
        seen: Away,
        idle: bool,
        actions: &Arc<dyn ShellActions>,
        wake: Option<&AppHandle<State>>,
    ) -> Option<usize> {
        let Some(job) = actions.away_job(seen) else {
            // Nothing here locks: nothing is covered either.
            self.without = true;
            self.cover = Cover::Off;
            return None;
        };
        let number = self.looks + 1;
        let (answers, wake) = (self.tx.clone(), wake.cloned());
        let started = std::thread::Builder::new()
            .name("away-lock".into())
            .spawn(move || {
                // A look that panicked answers as one that locked nothing, with the longest wait.
                let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).ok();
                let _ = answers.send(Looked { idle, number, answer });
                if let Some(main) = wake {
                    main.update(|_, _| {});
                }
            })
            .is_ok();
        if !started {
            return None;
        }
        self.looks = number;
        Some(number)
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
