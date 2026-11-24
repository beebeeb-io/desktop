//! One preview run at a time, on a thread of its own, stoppable; where it stands, for a window to show.
//!
//! A run is a closure that makes the previews ([`crate::app::Engine::make_previews_for`]) for ONE session: it asks
//! the session before every upload and ends by itself when the session moved on. This type only orders runs: a
//! start while one runs stops it and starts the new one after it ended (the latest ask wins); [`PreviewRun::stop`]
//! ends the run and drops a start that waits. Nothing here is timed: a run starts only when asked.
//!
//! Where it stands is kept with the session it is for: [`PreviewRun::state_for`] answers `Idle` for any other session,
//! so what one account's run came to is never shown to another.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::account::SessionGeneration;

/// Why a run ended before it made every preview it planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// The setting's format is HEIC: this PC has no HEIC encoder, so it makes none.
    HeicNotMadeHere,
    /// Beebeeb answered that the account's storage is full: the run stopped once.
    StorageFull,
    /// Beebeeb could not be reached.
    Offline,
    /// The run could not read what it needs (the setting, the account's files, this PC's state).
    Failed,
    /// Sync is paused on this PC: nothing is made, nothing is sent.
    Paused,
}

/// Where the run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Making { done: u32, total: u32 },
    Stopped(Stopped),
}

/// What a run came to: the counts, and why it ended early (`None`: it went through, was asked to stop, or its session
/// moved on).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunEnd {
    pub made: u32,
    /// Skipped without opening: not all its bytes on this PC, or its size is not the local state's.
    pub not_here: u32,
    /// Opened and not made (does not decode, too large), or refused by the server for this file alone.
    pub failed: u32,
    pub stopped: Option<Stopped>,
    /// The session moved on during the run: nothing more was sent.
    pub session_moved: bool,
}

/// A run's work: given the stop flag and a progress report (done, total), it makes the previews and says how it ended.
pub type Work = Box<dyn FnOnce(&AtomicBool, &dyn Fn(u32, u32)) -> RunEnd + Send>;

struct Inner {
    /// Where the run stands, and the session it is for.
    state: Mutex<(RunState, Option<SessionGeneration>)>,
    /// A start asked while a run was on its way: run after it.
    waiting: Mutex<Option<(SessionGeneration, Work)>>,
    running: AtomicBool,
    stop: AtomicBool,
    /// Told whenever the state moves (a window wakes to draw it).
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// A test holds the thread here: it found no work and still says it runs.
    #[cfg(test)]
    before_idle: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// One preview run at a time. Cheap to clone: every clone is the same run.
#[derive(Clone)]
pub struct PreviewRun {
    inner: Arc<Inner>,
}

impl Default for PreviewRun {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewRun {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new((RunState::Idle, None)),
                waiting: Mutex::new(None),
                running: AtomicBool::new(false),
                stop: AtomicBool::new(false),
                on_change: Mutex::new(None),
                #[cfg(test)]
                before_idle: Mutex::new(None),
            }),
        }
    }

    /// Told whenever the state moves (the window's wake). Replaces an earlier one.
    pub fn on_change(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.inner.on_change.lock().unwrap_or_else(PoisonError::into_inner) = Some(wake);
    }

    /// Test only: `hook` runs on the thread each time it found no work, before it says it no longer runs.
    #[cfg(test)]
    pub(crate) fn before_idle(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.inner.before_idle.lock().unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }

    /// Where the run of `session` stands; `Idle` for a run of any other session.
    pub fn state_for(&self, session: SessionGeneration) -> RunState {
        match *self.inner.state.lock().unwrap_or_else(PoisonError::into_inner) {
            (state, Some(of)) if of == session => state,
            _ => RunState::Idle,
        }
    }

    /// Whether a run is on its way (of any session).
    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    /// Start `work` for `session`. While a run is on its way it is stopped, and `work` runs after it ended (a later
    /// start replaces a waiting one). `false`: no thread could be started (nothing runs).
    pub fn start(&self, session: SessionGeneration, work: Work) -> bool {
        let inner = &self.inner;
        {
            let mut waiting = inner.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            *waiting = Some((session, work));
            // Under the queue's lock: the thread takes a work and clears the flag under it too, so this stops the
            // run on its way and never the work just queued.
            inner.stop.store(true, Ordering::SeqCst);
        }
        if inner.running.swap(true, Ordering::SeqCst) {
            // A thread is on its way: it takes the work. It looks for one again after it said it no longer runs, so
            // a work queued as it ends is not left behind.
            return true;
        }
        let run = self.clone();
        let started = std::thread::Builder::new()
            .name("photo-previews".into())
            .spawn(move || run.go());
        if started.is_err() {
            inner.waiting.lock().unwrap_or_else(PoisonError::into_inner).take();
            inner.running.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// End the run on its way (after the photo in hand; nothing more is uploaded) and drop a start that waits.
    pub fn stop(&self) {
        let mut waiting = self.inner.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.take();
        self.inner.stop.store(true, Ordering::SeqCst);
    }

    fn set(&self, state: RunState, session: SessionGeneration) {
        *self.inner.state.lock().unwrap_or_else(PoisonError::into_inner) = (state, Some(session));
        let wake = self
            .inner
            .on_change
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(wake) = wake {
            wake();
        }
    }

    /// The thread: every work that waits, one after the other, until none does.
    fn go(&self) {
        loop {
            let next = {
                let mut waiting = self.inner.waiting.lock().unwrap_or_else(PoisonError::into_inner);
                let next = waiting.take();
                if next.is_some() {
                    self.inner.stop.store(false, Ordering::SeqCst);
                }
                next
            };
            let Some((session, work)) = next else {
                #[cfg(test)]
                {
                    let hook = self
                        .inner
                        .before_idle
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    if let Some(hook) = hook {
                        hook();
                    }
                }
                self.inner.running.store(false, Ordering::SeqCst);
                // A start that queued after the take saw this thread still running and left the work to it: take
                // it, unless a new thread already did.
                let queued = self
                    .inner
                    .waiting
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .is_some();
                if queued && !self.inner.running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                return;
            };
            self.set(RunState::Making { done: 0, total: 0 }, session);
            let progress = |done: u32, total: u32| self.set(RunState::Making { done, total }, session);
            let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&self.inner.stop, &progress)))
                .unwrap_or(RunEnd {
                    stopped: Some(Stopped::Failed),
                    ..RunEnd::default()
                });
            self.set(ended.stopped.map_or(RunState::Idle, RunState::Stopped), session);
        }
    }
}
