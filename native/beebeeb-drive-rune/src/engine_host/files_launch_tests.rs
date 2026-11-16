//! Review of #467 on the real engine: a start that waited for the launch lock compares the session of its press inside
//! the lock and starts nothing once it moved (P3-1); a session end empties the launch worker's queue (P3-1); Open With
//! asks the engine whether the kept copy is here (P3-2). The stand-ins record what starts: each takes the launch lock
//! and asks `still` inside it, as the system's own do. Every name is invented; nothing is started for real.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

use super::*;
use crate::engine_host::files_launch::{Start, Starts};
use crate::engine_host::{launch, open_path, reveal_in_file_explorer};
use crate::frames::files::source::AppChoice;

/// The launch lock held on a thread of its own, as Windows' app chooser holds it while it is open.
struct Chooser {
    release: std::sync::mpsc::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl Chooser {
    fn open() -> Chooser {
        let (holding, held) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::spawn(move || {
            launch::locked(|| {
                holding.send(()).unwrap();
                let _ = released.recv_timeout(Duration::from_secs(30));
            })
        });
        held.recv().unwrap();
        Chooser { release, thread }
    }

    /// The chooser closes: the lock is let go.
    fn close(self) {
        self.release.send(()).unwrap();
        self.thread.join().unwrap();
    }
}

/// Waits until `what` holds (true), at most 10 s (false).
fn within(what: impl Fn() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(10);
    while !what() {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

// The session test's stand-ins: each asks `still` inside the launch lock, counts that it asked, and starts only when
// it held. Only that test uses them.
static MOVED_ASKED: AtomicUsize = AtomicUsize::new(0);
static MOVED_STARTED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn record_moved(kind: &'static str, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    launch::locked(|| {
        let now = still();
        MOVED_ASKED.fetch_add(1, SeqCst);
        if !now {
            return Err(ActionError::Unavailable);
        }
        MOVED_STARTED.lock().unwrap().push(kind);
        Ok(())
    })
}

fn moved_open(_: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    record_moved("open", still)
}

fn moved_reveal(_: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    record_moved("reveal", still)
}

fn moved_app(_: &Path, _: &str, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    record_moved("app", still)
}

/// While the launch lock is held: Open, Show in File Explorer, Open in File Explorer and Open With ▸ an app on a kept
/// file. Then `between` runs (handed the engine and its sync runner), the lock is let go, and the four starts are
/// waited for. What started, sorted.
fn four_starts(tag: &str, between: impl FnOnce(&Set, tokio::runtime::Runtime)) -> Vec<&'static str> {
    let mut set = set(tag, true, true);
    let running = hold_running(&set.engine);
    put_kept(&set, NOTES, "Invented notes.txt", FileStatus::Local, true);
    set.source.starts = Starts {
        open: moved_open,
        reveal: moved_reveal,
        app: Some(moved_app),
    };
    MOVED_ASKED.store(0, SeqCst);
    MOVED_STARTED.lock().unwrap().clear();
    let chooser = Chooser::open();
    let notes = row(NOTES, "Invented notes.txt", false);
    let app = AppChoice {
        id: "invented.exe".into(),
        label: "Invented".into(),
    };
    assert_eq!(set.source.open(std::slice::from_ref(&notes)), Ok(()));
    assert_eq!(set.source.show_in_file_explorer(std::slice::from_ref(&notes)), Ok(()));
    assert_eq!(set.source.open_in_file_explorer(None), Ok(()));
    assert_eq!(set.source.open_in_app(&notes, &app), Ok(()));
    between(&set, running);
    chooser.close();
    assert!(
        within(|| MOVED_ASKED.load(SeqCst) == 4),
        "each start asked for its session: {} of 4",
        MOVED_ASKED.load(SeqCst)
    );
    assert!(
        set.source.changes().is_empty(),
        "no failure is said, also not for an ended session"
    );
    let mut started = MOVED_STARTED.lock().unwrap().clone();
    started.sort();
    started
}

/// P3-1: the app locks while Windows' app chooser holds the launch lock. Each of the four starts asked before the lock
/// compares the session of its press once it has the launch lock, and starts nothing.
#[test]
fn a_start_that_waited_for_the_lock_starts_nothing_once_the_session_moved() {
    // The instrument first: the same presses with the session unchanged start all four once the lock is let go.
    assert_eq!(four_starts("b1-same", |_, _| {}), ["app", "open", "open", "reveal"]);
    let started = four_starts("b1-moved", |set, running| {
        let before = set.engine.session_turn();
        // The sync runner's runtime goes first, so that the lock's stop of sync is confirmed at once.
        drop(running);
        let _ = set.engine.lock();
        assert_ne!(set.engine.session_turn(), before, "the lock moved the session on");
    });
    assert_eq!(started, Vec::<&str>::new(), "nothing starts for the session that ended");
}

/// P3-1 on this system's own starts (Linux; Windows' and macOS's have the same shape and are checked on those
/// systems): `still` is asked once, with the launch lock held, and `false` starts nothing. Nothing is started here.
#[cfg(target_os = "linux")]
#[test]
fn the_systems_starts_ask_for_the_session_inside_the_launch_lock() {
    // The instrument first: outside a start the lock is seen free (another test's start may hold it a moment).
    assert!(
        (0..20).any(|_| launch::free_within(Duration::from_millis(250))),
        "the lock is never seen free"
    );
    let file = Path::new("/an invented folder/Invented notes.txt");
    let starts: [(&str, Start); 2] = [("open", open_path), ("reveal", reveal_in_file_explorer)];
    for (name, start) in starts {
        let held = Mutex::new(Vec::new());
        let still = || {
            held.lock()
                .unwrap()
                .push(!launch::free_within(Duration::from_millis(100)));
            false
        };
        assert_eq!(
            start(file, &still),
            Err(ActionError::Unavailable),
            "{name}: nothing started"
        );
        assert_eq!(
            *held.lock().unwrap(),
            [true],
            "{name}: asked once, the launch lock held"
        );
    }
}

// The queue test's stand-ins: each counts that it was begun, then starts under the launch lock while `still` holds.
static QUEUE_BEGUN: AtomicUsize = AtomicUsize::new(0);
static QUEUE_STARTED: Mutex<Vec<(&'static str, PathBuf)>> = Mutex::new(Vec::new());

fn record_queued(kind: &'static str, path: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    QUEUE_BEGUN.fetch_add(1, SeqCst);
    launch::locked(|| still().then(|| QUEUE_STARTED.lock().unwrap().push((kind, path.to_path_buf()))))
        .ok_or(ActionError::Unavailable)
}

fn queued_open(path: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    record_queued("open", path, still)
}

fn queued_reveal(path: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    record_queued("reveal", path, still)
}

/// P3-1: a session end empties the launch worker's queue at once, while the lock is still held and before the
/// session's turn is compared by any start: what was asked and not begun never starts.
#[test]
fn a_session_end_empties_the_launch_queue() {
    let mut set = set("b3-queue", true, true);
    let _running = hold_running(&set.engine);
    let root = set.engine.sync_root().expect("a sync folder");
    set.source.starts = Starts {
        open: queued_open,
        reveal: queued_reveal,
        app: None,
    };
    let chooser = Chooser::open();
    let notes = row(NOTES, "Invented notes.txt", false);
    // Open: the worker begins it at once, and it waits inside its start for the lock.
    assert_eq!(set.source.open(std::slice::from_ref(&notes)), Ok(()));
    assert!(
        within(|| QUEUE_BEGUN.load(SeqCst) == 1),
        "the worker is in the first start"
    );
    // Show in File Explorer: it waits its turn in the queue.
    assert_eq!(set.source.show_in_file_explorer(std::slice::from_ref(&notes)), Ok(()));
    assert_eq!(set.source.pending.len(), 1, "one start waits");
    let turn = set.engine.session_turn();
    set.source.forget_starts();
    assert_eq!(set.source.pending.len(), 0, "emptied while the lock is still held");
    assert_eq!(
        set.engine.session_turn(),
        turn,
        "the turn did not move: only the queue dropped it"
    );
    chooser.close();
    // A press after it, through the same worker: once it started, everything handed over before it has run.
    assert_eq!(set.source.open_in_file_explorer(None), Ok(()));
    assert!(
        within(|| QUEUE_STARTED.lock().unwrap().len() == 2),
        "{:?}",
        QUEUE_STARTED.lock().unwrap()
    );
    let started = QUEUE_STARTED.lock().unwrap().clone();
    assert_eq!(started[0].0, "open");
    assert!(
        started[0].1.ends_with("Invented notes.txt"),
        "the Open begun before the end"
    );
    assert_eq!(started[1], ("open", root), "the press after it: Open in File Explorer");
    assert_eq!(QUEUE_BEGUN.load(SeqCst), 2, "Show in File Explorer was never begun");
}

/// P3-2: Open With hands a file to another app only when the engine says it is kept AND its bytes are here, asked at
/// the press. Right after Keep on this PC the row is marked kept while the copy still comes down: not here yet.
#[test]
fn open_with_asks_the_engine_whether_the_kept_copy_is_here() {
    let set = set("b4-here", true, true);
    let _running = hold_running(&set.engine);
    let mut marked = row(NOTES, "Invented notes.txt", false);
    marked.pc_status = Some("available_offline".into());
    put_kept(&set, NOTES, "Invented notes.txt", FileStatus::Downloading, true);
    assert!(
        !FilesSource::kept_here(&set.source, &marked),
        "kept, still coming down: not here, whatever the row says"
    );
    put_kept(&set, NOTES, "Invented notes.txt", FileStatus::Local, false);
    assert!(!FilesSource::kept_here(&set.source, &marked), "here, not kept");
    put_kept(&set, NOTES, "Invented notes.txt", FileStatus::Local, true);
    assert!(FilesSource::kept_here(&set.source, &marked), "kept and here");
    let unmarked = row(NOTES, "Invented notes.txt", false);
    assert!(
        FilesSource::kept_here(&set.source, &unmarked),
        "the engine's facts answer, not the row's mark"
    );
}
