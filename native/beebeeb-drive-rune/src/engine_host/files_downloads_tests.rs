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
}

fn session(n: u64) -> SessionGeneration {
    SessionGeneration::for_test(n)
}

fn one() -> Vec<DownloadRow> {
    vec![DownloadRow {
        file_id: "f-down".into(),
        done: 4_000,
        total: 9_000,
    }]
}

impl Double {
    fn new(read: Result<Vec<DownloadRow>, LocalStateRefused>) -> Arc<Self> {
        Arc::new(Self {
            stamp: Mutex::new((session(1), Some("one@example.eu".into()))),
            read: Mutex::new(read),
            reads: AtomicUsize::new(0),
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
    fn wake(&self) {}
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
            total: 9_000
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
