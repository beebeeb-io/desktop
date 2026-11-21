//! When the engine makes the main window draw (dc-165).
//!
//! The engine rings its host after every report and every change of its own view: once per row while the first file
//! list comes in (41,510 rings for the large account). Each ring used to be an empty `AppHandle::update`, which Rune
//! takes as "the state changed": the view ran again and the whole window was drawn, once per ring, whether or not a
//! word on it had moved. On a PC without a GPU that was 11 cores for as long as the engine kept ringing.
//!
//! Two rules replace it:
//! - [`EngineWakes`]: rings are counted as one. The window is asked at once after a quiet spell, then at most every
//!   [`AT_MOST_EVERY`]; the last ring of a burst is always followed by one more ask.
//! - [`engine_woke`]: the ask runs the window's own poll (`host::poll`) and says what it said. A ring that moved
//!   nothing the window shows draws nothing (`AppHandle::update_if`: no view, no frame).
//!
//! What the views read from the engine while they draw (the status line's words, the banners) is not `State`, so
//! the poll could not see it move: [`follow_engine`] compares it with what the last ring left ([`Seen`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use rune::Tx;

use crate::State;
use crate::frames::global::source::GlobalStatus;

/// The window is asked at most this often while the engine keeps ringing: the rate a progress line moves at.
pub(crate) const AT_MOST_EVERY: Duration = Duration::from_millis(100);

/// The engine rang since the poll last compared what the views read from it ([`follow_engine`]). One for the
/// process: there is one engine and one main window. A ring another window's test left costs one comparison.
static RANG: AtomicBool = AtomicBool::new(false);

/// What the main window's views read from the engine while they draw, not from `State`: the window's state (the
/// banners, the loading count), the status line's words, what uploads and downloads, the note over the folder
/// shown, the conflicts under their dialog, and whether sync runs (the lock screen's line). Everything else a view
/// shows is in `State`, and a poll that changes it says so.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Seen {
    status: GlobalStatus,
    sync: Option<String>,
    sync_runs: bool,
    uploads: Option<crate::frames::files::uploads::Uploads>,
    downloads: Vec<crate::frames::files::keeping::Download>,
    notice: Option<crate::frames::files::protected::FolderNotice>,
    conflicts: Vec<crate::frames::files::actions_data::Conflict>,
}

impl Seen {
    fn of(s: &State) -> Seen {
        let source = &s.actions.source;
        // Read as the views read them: the conflicts only under their dialog (a read of this PC's state).
        let conflict_shown = matches!(s.shell.app.dialog(), Some(super::app::Dialog::Conflict { .. }));
        Seen {
            status: s.global.source.status(),
            sync: source.sync_state(),
            sync_runs: s.shell.sync_runs(),
            uploads: source.uploads(),
            downloads: source.downloads(),
            notice: source.folder_notice(s.listing.folder.id.as_deref()),
            conflicts: if conflict_shown { source.conflicts() } else { Vec::new() },
        }
    }
}

/// At the end of every poll: whether what the views read from the engine moved since it was last compared.
/// `drawing`: the poll already found something, so the view runs after it and reads the engine itself; what it
/// reads is kept as seen. Otherwise it is read only after a ring (the engine rings after every change of it), so a
/// frame without one costs a flag.
pub(crate) fn follow_engine(s: &mut State, drawing: bool) -> bool {
    let rang = RANG.swap(false, Ordering::AcqRel);
    if !drawing && !rang && s.shell.seen.is_some() {
        return false;
    }
    let now = Seen::of(s);
    // The first reading is the one the first frame draws.
    let moved = s.shell.seen.as_ref().is_some_and(|seen| *seen != now);
    s.shell.seen = Some(now);
    moved
}

/// The engine rang ([`EngineWakes`]), on the UI thread: the window's poll, and whether it found anything to draw.
/// For `AppHandle::update_if`: `false` leaves the window as it is drawn.
pub(crate) fn engine_woke(s: &mut State, tx: &mut Tx) -> bool {
    RANG.store(true, Ordering::Release);
    let now = tx.clock();
    super::host::poll(s, now, tx)
}

struct Rings {
    /// A ring the worker has not asked the window for yet.
    pending: AtomicBool,
    /// The engine's side is gone: the worker ends.
    closed: AtomicBool,
    lock: Mutex<()>,
    rang: Condvar,
}

/// The engine's rings, as one ask of the window at most every `every`. `ring` is for the engine's threads: it never
/// waits on the window.
pub(crate) struct EngineWakes {
    rings: Arc<Rings>,
}

impl EngineWakes {
    /// `ask` runs on this gate's own thread: post the window's update there, nothing else.
    pub(crate) fn start(every: Duration, ask: impl Fn() + Send + 'static) -> EngineWakes {
        let rings = Arc::new(Rings {
            pending: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            lock: Mutex::new(()),
            rang: Condvar::new(),
        });
        let worker = rings.clone();
        let started = std::thread::Builder::new().name("engine-wakes".into()).spawn(move || {
            loop {
                {
                    let mut guard = worker.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    while !worker.pending.load(Ordering::Acquire) && !worker.closed.load(Ordering::Acquire) {
                        guard = worker.rang.wait(guard).unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                }
                if worker.closed.load(Ordering::Acquire) {
                    return;
                }
                // Cleared before the ask: a ring that comes while the window is asked is asked for again.
                worker.pending.store(false, Ordering::Release);
                ask();
                std::thread::sleep(every);
            }
        });
        if started.is_err() {
            super::host::log("the engine's wakes have no thread: the window follows the engine at its own polls only");
        }
        EngineWakes { rings }
    }

    /// The engine has something new. Many rings before the window was asked are one.
    pub(crate) fn ring(&self) {
        RANG.store(true, Ordering::Release);
        if !self.rings.pending.swap(true, Ordering::AcqRel) {
            let _guard = self.rings.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            self.rings.rang.notify_one();
        }
    }
}

impl Drop for EngineWakes {
    fn drop(&mut self) {
        self.rings.closed.store(true, Ordering::Release);
        let _guard = self.rings.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.rings.rang.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc;
    use std::time::Instant;

    use rune::{Env, Runtime};

    use super::*;
    use crate::frames::global::source::{GlobalSource, Loading, Step};
    use crate::shell::host::{self, Shell};
    use crate::shell::{registry, state};

    /// A first file list on its way: `listed` is the engine's count, moved from another thread.
    struct Listing(Arc<AtomicU64>);

    impl GlobalSource for Listing {
        fn status(&self) -> GlobalStatus {
            GlobalStatus {
                loading: Some(Loading {
                    listed: self.0.load(Ordering::SeqCst),
                    total: 41_510,
                    left: None,
                    file_list: Step::InProgress,
                    thumbnails: Step::WaitingForFileList,
                }),
                ..GlobalStatus::default()
            }
        }
        fn check_connection(&mut self) -> bool {
            true
        }
    }

    fn listed(rt: &Runtime<State>) -> Option<u64> {
        let seen = rt.state.shell.seen.as_ref()?;
        Some(seen.status.loading.as_ref()?.listed)
    }

    /// "N of 41,510 listed": the engine counts 10,000 rows, ringing after each one, as the first pull does. The
    /// window is drawn at the progress rate, not once per row, and what it shows at the end is the last count.
    #[test]
    fn a_progress_burst_of_10k_updates_causes_at_most_20_redraws() {
        const ROWS: u64 = 10_000;
        let count = Arc::new(AtomicU64::new(0));
        let mut s = state::init_with(Shell::fixture());
        s.global.source = Box::new(Listing(count.clone()));
        let env = Env {
            system_dark: false,
            scale: 1.0,
        };
        let mut rt = Runtime::new(s, registry::view, (1280.0, 800.0), env);
        rt.poll = Some(host::poll);
        rt.frame(0.0);
        rt.frame(0.016);

        // The gate as the app wires it; the ask lands on this thread (the window's), as Rune's loop takes it.
        let (asked, asks) = mpsc::channel();
        let wakes = EngineWakes::start(AT_MOST_EVERY, move || {
            let _ = asked.send(());
        });
        let engine = std::thread::spawn(move || {
            for row in 1..=ROWS {
                count.store(row, Ordering::SeqCst);
                wakes.ring();
                // 20 ms per 1,000 rows: the burst lasts about a fifth of a second.
                if row % 1_000 == 0 {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            wakes
        });

        let began = Instant::now();
        let (mut asks_taken, mut redraws) = (0, 0);
        let mut wakes = None;
        // Until the engine is done and the gate has been quiet for three of its intervals.
        loop {
            match asks.recv_timeout(AT_MOST_EVERY * 3) {
                Ok(()) => {
                    asks_taken += 1;
                    if rt.update_if(engine_woke) {
                        rt.frame(0.016);
                        redraws += 1;
                    }
                }
                Err(_) if engine.is_finished() => break,
                Err(_) => {}
            }
            if engine.is_finished() && wakes.is_none() {
                // Kept alive past the burst: the last ring's ask still comes.
                wakes = Some(());
            }
            assert!(began.elapsed() < Duration::from_secs(20), "the burst never ended");
        }
        let _wakes = engine.join().expect("the engine's thread");
        assert!(asks_taken >= 1, "the window was never asked");
        assert!(asks_taken <= 20, "{asks_taken} asks for {ROWS} rings");
        assert!((1..=20).contains(&redraws), "{redraws} redraws for {ROWS} updates");
        assert_eq!(listed(&rt), Some(ROWS), "the last count is the one the window read");

        // And at rest: a ring with nothing new draws nothing.
        assert!(!rt.update_if(engine_woke), "a ring with the same count is no redraw");
    }

    /// A gate nobody rings asks nothing, and its thread ends with it.
    #[test]
    fn a_quiet_gate_asks_for_no_redraw() {
        let (asked, asks) = mpsc::channel();
        let wakes = EngineWakes::start(Duration::from_millis(5), move || {
            let _ = asked.send(());
        });
        assert!(asks.recv_timeout(Duration::from_millis(100)).is_err());
        wakes.ring();
        assert!(asks.recv_timeout(Duration::from_secs(5)).is_ok(), "a ring is asked for");
        drop(wakes);
        // The sender went with the thread.
        assert!(matches!(
            asks.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }
}
