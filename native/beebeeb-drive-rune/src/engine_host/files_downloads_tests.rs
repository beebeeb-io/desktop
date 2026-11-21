//! The "Downloading" rows' adapter ([`DownloadCalls`]) against a double of the engine that names its own sessions and
//! accounts and counts what it was asked. Every name is invented.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

const WAIT: Duration = Duration::from_secs(10);

struct Double {
    stamp: Mutex<Stamp>,
    read: Mutex<Result<Vec<DownloadRow>, LocalStateRefused>>,
    reads: AtomicUsize,
    word: Mutex<Option<&'static str>>,
}

fn session(n: u64) -> SessionGeneration {
    SessionGeneration::for_test(n)
}

fn one() -> Vec<DownloadRow> {
    vec![DownloadRow {
        file_id: "f-down".into(),
        done: 4_000,
        total: 9_000,
        kept: true,
    }]
}

impl Double {
    fn new(read: Result<Vec<DownloadRow>, LocalStateRefused>) -> Arc<Self> {
        Arc::new(Self {
            stamp: Mutex::new((session(1), Some("one@example.eu".into()))),
            read: Mutex::new(read),
            reads: AtomicUsize::new(0),
            word: Mutex::new(Some(words::SYNCING)),
        })
    }
    fn move_to(&self, n: u64, account: &str) {
        *self.stamp.lock().unwrap() = (session(n), Some(account.into()));
    }
}

impl DownloadsEngine for Double {
    fn stamp(&self) -> Stamp {
        self.stamp.lock().unwrap().clone()
    }
    fn has_session(&self) -> bool {
        true
    }
    fn downloads_for(&self, _: SessionGeneration) -> Result<Vec<DownloadRow>, LocalStateRefused> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.read.lock().unwrap().clone()
    }
    fn word(&self) -> Option<&'static str> {
        *self.word.lock().unwrap()
    }
    fn wake(&self) {}
}

/// `take` for at most `within` (less than the age gate), until `done`.
fn take_within(calls: &mut DownloadCalls<Double>, within: Duration, done: impl Fn(&DownloadCalls<Double>) -> bool) {
    let start = Instant::now();
    while !done(calls) && start.elapsed() < within {
        calls.take();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn take_until(calls: &mut DownloadCalls<Double>, done: impl Fn(&DownloadCalls<Double>) -> bool) {
    let start = Instant::now();
    loop {
        calls.take();
        if done(calls) || start.elapsed() > WAIT {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn nothing_is_drawn_before_the_engine_answered() {
    let engine = Double::new(Ok(one()));
    let calls = DownloadCalls::new(engine.clone());
    assert!(calls.downloads().is_empty());
    assert_eq!(engine.reads.load(Ordering::SeqCst), 0, "no read at construction");
}

#[test]
fn what_the_engine_read_is_shown_as_the_rows_figures() {
    let engine = Double::new(Ok(one()));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| !c.downloads().is_empty());
    assert_eq!(
        calls.downloads(),
        vec![Download {
            id: "f-down".into(),
            done: 4_000,
            total: 9_000,
            kept: true
        }]
    );
}

#[test]
fn a_held_read_is_not_shown_under_another_session_or_account() {
    let engine = Double::new(Ok(one()));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| !c.downloads().is_empty());
    engine.move_to(2, "one@example.eu");
    assert!(calls.downloads().is_empty(), "another session");
    engine.move_to(1, "two@example.eu");
    assert!(calls.downloads().is_empty(), "another account");
}

#[test]
fn an_answer_that_lands_after_the_session_moved_is_dropped() {
    let engine = Double::new(Ok(one()));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    engine.move_to(2, "one@example.eu");
    // Let the worker answer under the old stamp, then take under the new one.
    let start = Instant::now();
    while engine.reads.load(Ordering::SeqCst) == 0 && start.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    calls.take();
    assert!(calls.downloads().is_empty());
}

#[test]
fn an_answer_that_lands_after_a_forget_is_dropped() {
    let engine = Double::new(Ok(one()));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    calls.forget();
    let start = Instant::now();
    while engine.reads.load(Ordering::SeqCst) == 0 && start.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    calls.take();
    assert!(calls.downloads().is_empty());
}

#[test]
fn an_unreadable_board_shows_no_bar() {
    let engine = Double::new(Err(LocalStateRefused::Unreadable));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |_| engine.reads.load(Ordering::SeqCst) > 0);
    std::thread::sleep(Duration::from_millis(50));
    calls.take();
    assert!(calls.downloads().is_empty());
}

#[test]
fn a_copy_coming_down_is_read_again_on_the_wake_once_the_read_is_old() {
    let engine = Double::new(Ok(one()));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| !c.downloads().is_empty());
    let first = engine.reads.load(Ordering::SeqCst);
    calls.take();
    assert_eq!(
        engine.reads.load(Ordering::SeqCst),
        first,
        "a fresh read is not asked again"
    );
    std::thread::sleep(STALE + Duration::from_millis(100));
    take_until(&mut calls, |_| engine.reads.load(Ordering::SeqCst) > first);
    assert!(engine.reads.load(Ordering::SeqCst) > first, "an old read is");
}

#[test]
fn the_status_word_is_never_up_to_date_while_a_copy_comes_down() {
    assert_eq!(with_downloads(Some(words::UP_TO_DATE), true), Some(words::SYNCING));
    assert_eq!(with_downloads(Some(words::UP_TO_DATE), false), Some(words::UP_TO_DATE));
    assert_eq!(
        with_downloads(Some(words::PAUSED), true),
        Some(words::PAUSED),
        "the engine's other words stand"
    );
    assert_eq!(with_downloads(None, true), None);
}

#[test]
fn a_copy_that_ended_leaves_the_row_at_the_engines_next_pulse_whatever_the_reads_age() {
    let engine = Double::new(Ok(one()));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| !c.downloads().is_empty());
    let first = engine.reads.load(Ordering::SeqCst);
    // The copy lands; the engine's own word moves to Idle in the same pulse. The held read is a moment old.
    *engine.read.lock().unwrap() = Ok(Vec::new());
    *engine.word.lock().unwrap() = Some(words::UP_TO_DATE);
    take_within(&mut calls, Duration::from_millis(600), |c| c.downloads().is_empty());
    assert!(calls.downloads().is_empty(), "the row left");
    assert!(
        engine.reads.load(Ordering::SeqCst) > first,
        "by one read, not by waiting out the age"
    );
    assert_eq!(
        with_downloads(Some(words::UP_TO_DATE), !calls.downloads().is_empty()),
        Some(words::UP_TO_DATE),
        "and the status word is the engine's again"
    );
}

#[test]
fn a_wake_while_the_engine_reports_syncing_reads_after_the_watch_ended() {
    let engine = Double::new(Ok(Vec::new()));
    *engine.word.lock().unwrap() = Some(words::UP_TO_DATE);
    let mut calls = DownloadCalls::new(engine.clone()).with_watch(Duration::ZERO);
    calls.ask();
    take_until(&mut calls, |_| engine.reads.load(Ordering::SeqCst) > 0);
    std::thread::sleep(Duration::from_millis(50));
    calls.take();
    assert!(calls.downloads().is_empty());
    // A download starts later: the engine's word says so.
    *engine.read.lock().unwrap() = Ok(one());
    *engine.word.lock().unwrap() = Some(words::SYNCING);
    std::thread::sleep(STALE + Duration::from_millis(100));
    take_until(&mut calls, |c| !c.downloads().is_empty());
    assert_eq!(calls.downloads().len(), 1, "it got its bar");
}

#[test]
fn the_watch_ends_and_an_idle_engine_is_not_read_again() {
    let engine = Double::new(Ok(Vec::new()));
    *engine.word.lock().unwrap() = Some(words::UP_TO_DATE);
    let mut calls = DownloadCalls::new(engine.clone()).with_watch(Duration::from_millis(100));
    calls.ask();
    take_until(&mut calls, |_| engine.reads.load(Ordering::SeqCst) > 0);
    std::thread::sleep(STALE + Duration::from_millis(200));
    let reads = engine.reads.load(Ordering::SeqCst);
    for _ in 0..5 {
        calls.take();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        engine.reads.load(Ordering::SeqCst),
        reads,
        "no read once the watch ended and nothing syncs"
    );
}

#[test]
fn a_copy_that_is_not_kept_is_shown_as_such() {
    let mut rows = one();
    rows[0].kept = false;
    let engine = Double::new(Ok(rows));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| !c.downloads().is_empty());
    assert!(!calls.downloads()[0].kept);
}

/// The wiring of `EngineFilesSource` (a real engine is not built here): the status line goes through
/// `with_downloads`, a kept copy that was accepted reads, and the account leaving forgets the held rows.
#[test]
fn the_files_source_wires_the_downloads_in() {
    let host = include_str!("mod.rs");
    let body = |head: &str| {
        let from = host.find(head).unwrap_or_else(|| panic!("{head} is gone"));
        host[from..].split("\n    fn ").next().unwrap().to_string()
    };
    assert!(body("fn sync_state(&self)").contains("files_downloads::with_downloads"));
    assert!(body("fn changes(&mut self)").contains("self.downloads.ask()"));
    assert!(body("fn changes(&mut self)").contains("self.downloads.take()"));
    assert!(body("fn forget_downloads(&mut self)").contains("self.downloads.forget()"));
    assert!(body("fn downloads(&self)").contains("self.downloads.downloads()"));
}

/// Paused (and Offline, Sync stopped) send no pulse: the engine wakes the host when a row leaves the board, and one
/// take after that wake clears the row.
#[test]
fn while_paused_a_row_that_ended_leaves_at_the_next_take_after_the_wake() {
    let engine = Double::new(Ok(one()));
    *engine.word.lock().unwrap() = Some(words::PAUSED);
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| !c.downloads().is_empty());
    // The rows end; the word stays Paused throughout. The held read is older than the age gate by the wake.
    *engine.read.lock().unwrap() = Ok(Vec::new());
    std::thread::sleep(STALE + Duration::from_millis(100));
    take_within(&mut calls, Duration::from_millis(600), |c| c.downloads().is_empty());
    assert!(calls.downloads().is_empty());
}

/// And a fetch that starts while Paused gets its bar at the wake of its row beginning.
#[test]
fn while_paused_a_fetch_that_starts_gets_its_bar_at_the_wake() {
    let engine = Double::new(Ok(Vec::new()));
    *engine.word.lock().unwrap() = Some(words::PAUSED);
    let mut calls = DownloadCalls::new(engine.clone()).with_watch(Duration::ZERO);
    calls.ask();
    take_until(&mut calls, |_| engine.reads.load(Ordering::SeqCst) > 0);
    std::thread::sleep(STALE + Duration::from_millis(100));
    *engine.read.lock().unwrap() = Ok(one());
    take_until(&mut calls, |c| !c.downloads().is_empty());
    assert_eq!(calls.downloads().len(), 1);
}

#[test]
fn a_steady_syncing_word_with_rows_held_does_not_read_again_inside_the_age_gate() {
    let engine = Double::new(Ok(one()));
    let mut calls = DownloadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| !c.downloads().is_empty());
    // Let any read the first sighting asked for land, then count.
    std::thread::sleep(Duration::from_millis(50));
    calls.take();
    std::thread::sleep(Duration::from_millis(50));
    let reads = engine.reads.load(Ordering::SeqCst);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        calls.take();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(engine.reads.load(Ordering::SeqCst), reads, "no read per wake");
}

/// The engine's word is Syncing from the very start: no move of the word is what starts the read, the wake is.
#[test]
fn a_wake_reads_for_a_download_that_starts_after_the_watch_with_the_word_at_syncing_throughout() {
    let engine = Double::new(Ok(Vec::new()));
    let mut calls = DownloadCalls::new(engine.clone()).with_watch(Duration::ZERO);
    calls.ask();
    take_until(&mut calls, |_| engine.reads.load(Ordering::SeqCst) > 0);
    std::thread::sleep(Duration::from_millis(50));
    calls.take();
    assert!(calls.downloads().is_empty());
    *engine.read.lock().unwrap() = Ok(one());
    std::thread::sleep(STALE + Duration::from_millis(100));
    take_until(&mut calls, |c| !c.downloads().is_empty());
    assert_eq!(calls.downloads().len(), 1);
}
