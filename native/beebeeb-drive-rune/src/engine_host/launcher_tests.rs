//! R-OW1 for the starts outside Files (the tray's and the menu's Open in File Explorer, a page, the viewer's Open in
//! another app): while the launch lock is held (Windows' app chooser is open) the window's thread is handed back at
//! once and its poll answers; a press from a session that ended starts nothing; and no caller starts on the window's
//! thread. Nothing is started for real: the stand-ins take the launch lock and ask `still` inside it, as the
//! system's own do.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::Launcher;
use crate::engine_host::launch;
use crate::engine_host::launch_tests::{code_only, non_test_files, without_inline_tests};
use crate::frames::files::source::ActionError;

/// The launch lock held on a thread of its own, as Windows' app chooser holds it while it is open.
fn chooser() -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (holding, held) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        launch::locked(|| {
            holding.send(()).unwrap();
            let _ = released.recv_timeout(Duration::from_secs(30));
        })
    });
    held.recv().unwrap();
    (release, thread)
}

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

/// A start that takes the launch lock, asks `still` inside it and records the path only when it held.
fn recording(seen: &'static Mutex<Vec<PathBuf>>) -> impl Fn(&Path, &dyn Fn() -> bool) -> Result<(), ActionError> {
    move |path, still| {
        launch::locked(|| still().then(|| seen.lock().unwrap().push(path.to_path_buf())))
            .ok_or(ActionError::Unavailable)
    }
}

// `Start` is a plain fn pointer, so each test has its own stand-in and its own record.
static HELD_SEEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
fn held_start(path: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    recording(&HELD_SEEN)(path, still)
}
static ENDED_SEEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
fn ended_start(path: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    recording(&ENDED_SEEN)(path, still)
}
static ORDER_SEEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
fn order_start(path: &Path, still: &dyn Fn() -> bool) -> Result<(), ActionError> {
    recording(&ORDER_SEEN)(path, still)
}

/// While the launch lock is held, the hand-over returns in well under 500 ms (the window's poll never waits behind
/// it); nothing starts meanwhile; once the lock is let go the starts run in the order asked, each saying its answer.
#[test]
fn a_start_outside_files_never_waits_on_the_launch_lock() {
    let launcher = Launcher::default();
    let answered = Arc::new(AtomicUsize::new(0));
    let (release, chooser) = chooser();
    let asked_at = Instant::now();
    for name in ["/an invented folder", "/an invented page", "/an invented file"] {
        let answered = answered.clone();
        launcher
            .open(
                held_start,
                PathBuf::from(name),
                || true,
                move |answer| {
                    assert_eq!(answer, Ok(()));
                    answered.fetch_add(1, SeqCst);
                },
            )
            .unwrap();
    }
    let took = asked_at.elapsed();
    assert!(took < Duration::from_millis(500), "the window's thread waited {took:?}");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        HELD_SEEN.lock().unwrap().is_empty(),
        "nothing starts while the lock is held"
    );
    release.send(()).unwrap();
    chooser.join().unwrap();
    assert!(within(|| answered.load(SeqCst) == 3), "the three starts ran");
    assert_eq!(
        *HELD_SEEN.lock().unwrap(),
        [
            PathBuf::from("/an invented folder"),
            PathBuf::from("/an invented page"),
            PathBuf::from("/an invented file")
        ],
        "in the order asked"
    );
}

/// A press whose session ended while the start waited for the lock starts 0, and says nothing.
#[test]
fn a_press_from_a_session_that_ended_starts_nothing() {
    let launcher = Launcher::default();
    let session_holds = Arc::new(AtomicBool::new(true));
    let said = Arc::new(AtomicUsize::new(0));
    let (release, chooser) = chooser();
    let (still, said_to) = (session_holds.clone(), said.clone());
    launcher
        .open(
            ended_start,
            PathBuf::from("/an invented folder"),
            move || still.load(SeqCst),
            move |_| {
                said_to.fetch_add(1, SeqCst);
            },
        )
        .unwrap();
    // The session ends (a lock, a sign-out, a sign-in) while the chooser still holds the lock.
    session_holds.store(false, SeqCst);
    // A second press of the NEW session behind it still runs: the first one's end does not stop the worker.
    let fresh = Arc::new(AtomicUsize::new(0));
    let fresh_to = fresh.clone();
    launcher
        .open(
            ended_start,
            PathBuf::from("/a later folder"),
            || true,
            move |_| {
                fresh_to.fetch_add(1, SeqCst);
            },
        )
        .unwrap();
    release.send(()).unwrap();
    chooser.join().unwrap();
    assert!(within(|| fresh.load(SeqCst) == 1), "the later press ran");
    assert_eq!(
        *ENDED_SEEN.lock().unwrap(),
        [PathBuf::from("/a later folder")],
        "0 starts for the ended session"
    );
    assert_eq!(said.load(SeqCst), 0, "nothing said to a session that ended");
}

/// A start the worker could not begin is refused at once, never queued.
#[test]
fn the_order_of_starts_is_the_order_asked_across_clones() {
    let launcher = Launcher::default();
    let other = launcher.clone();
    for (who, name) in [(&launcher, "/one"), (&other, "/two"), (&launcher, "/three")] {
        who.open(order_start, PathBuf::from(name), || true, |_| {}).unwrap();
    }
    assert!(within(|| ORDER_SEEN.lock().unwrap().len() == 3));
    assert_eq!(
        *ORDER_SEEN.lock().unwrap(),
        [PathBuf::from("/one"), PathBuf::from("/two"), PathBuf::from("/three")]
    );
    assert!(within(|| launcher.waiting() == 0));
}

/// The two calls that start on a thread of their own already (the first-run step's folder, the bundle's reveal and
/// draft), by file and how many calls each has. A new `|| true` call anywhere else is a window-thread start.
const ON_THEIR_OWN_THREAD: [(&str, usize); 3] =
    [("engine_host.rs", 1), ("engine_host/drive_settings_tools.rs", 2), ("engine_host/found_while_paused.rs", 1)];

/// The lines of the calls in `src` that start the system's open or reveal with a session that always holds
/// (`&|| true`) rather than the press's own.
fn unbound_starts(src: &str) -> Vec<usize> {
    let code = without_inline_tests(&code_only(src));
    let mut lines = Vec::new();
    for call in ["open_path(", "reveal_in_file_explorer("] {
        let mut from = 0;
        while let Some(n) = code[from..].find(call) {
            let at = from + n;
            from = at + call.len();
            let before = code[..at].trim_end();
            if before.ends_with("fn") {
                continue;
            }
            let rest = &code[from..];
            let args = &rest[..rest.find(')').unwrap_or(rest.len())];
            // `&|| true)` closes the call: its argument list runs to the first `)` after the closure.
            if args.contains("|| true") || rest[..rest.len().min(80)].contains("&|| true") {
                lines.push(code[..at].matches('\n').count() + 1);
            }
        }
    }
    lines.sort_unstable();
    lines
}

/// Every caller of the system's open and reveal: none starts on the window's thread. The window-thread callers pass
/// no `&|| true` (they hand a start to the launch worker with the press's own session); the ones that do are the two
/// that already run on a thread of their own, counted exactly.
#[test]
fn no_caller_starts_the_system_on_the_windows_thread() {
    // The instrument first, on fixed text.
    assert_eq!(unbound_starts("fn x() { open_path(&p, &|| true); }"), [1]);
    assert_eq!(
        unbound_starts("fn x() {\n reveal_in_file_explorer(p, &|| true)\n}"),
        [2]
    );
    assert_eq!(
        unbound_starts("fn open_path(p: &Path, still: &dyn Fn() -> bool) {}"),
        Vec::<usize>::new()
    );
    assert_eq!(
        unbound_starts("fn x() { self.launcher.open(self.open, p, move || e.session_turn() == t, |a| {}); }"),
        Vec::<usize>::new(),
        "a start handed to the worker with the press's session"
    );
    assert_eq!(
        unbound_starts("// open_path(&p, &|| true)\n"),
        Vec::<usize>::new(),
        "a comment is not a start"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    non_test_files(&root, &mut files);
    assert!(files.len() > 100, "the scan reads the crate: {} files", files.len());
    let mut found: Vec<(String, usize)> = Vec::new();
    for file in &files {
        let name = file
            .strip_prefix(&root)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        let lines = unbound_starts(&std::fs::read_to_string(file).unwrap());
        if !lines.is_empty() {
            found.push((name, lines.len()));
        }
    }
    found.sort();
    let mut allowed: Vec<(String, usize)> = ON_THEIR_OWN_THREAD.iter().map(|(f, n)| (f.to_string(), *n)).collect();
    allowed.sort();
    assert_eq!(
        found, allowed,
        "a start with an always-holding session outside the two worker threads"
    );
}
