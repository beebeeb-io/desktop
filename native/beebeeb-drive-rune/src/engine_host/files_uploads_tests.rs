//! The Uploads panel's adapter ([`UploadCalls`]) against a double of the engine that names its own sessions and
//! accounts and counts what it was asked. Every name is invented.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::uploads_api::UploadRow;

use super::*;

const WAIT: Duration = Duration::from_secs(10);

struct Double {
    stamp: Mutex<Stamp>,
    read: Mutex<Result<UploadsRead, LocalStateRefused>>,
    reads: AtomicUsize,
    retries: AtomicUsize,
    cancels: AtomicUsize,
    pauses: AtomicUsize,
    retry_outcome: Mutex<RetryOutcome>,
    cancel_outcome: Mutex<CancelOutcome>,
    pause_result: Mutex<Result<(), String>>,
    /// The next retry waits on this before it answers.
    hold: Mutex<Option<mpsc::Receiver<()>>>,
}

fn session(n: u64) -> SessionGeneration {
    SessionGeneration::for_test(n)
}

fn row(file: &str, size: u64, new_file: bool, stands: UploadStands) -> UploadRow {
    UploadRow {
        file_id: file.into(),
        change_id: format!("op-{file}"),
        name: format!("Invented {file}.txt"),
        folder: "Trip".into(),
        size,
        new_file,
        stands,
    }
}

fn read_of(n: u64, rows: Vec<UploadRow>, paused: bool) -> UploadsRead {
    UploadsRead {
        session: session(n),
        rows,
        paused,
    }
}

fn three(n: u64) -> UploadsRead {
    read_of(
        n,
        vec![
            row(
                "stops",
                40,
                true,
                UploadStands::Stopped {
                    words: "Stopped after 5 tries.".into(),
                    retryable: true,
                },
            ),
            row(
                "moves",
                3 * 1024 * 1024,
                true,
                UploadStands::Moving {
                    sent: 1024 * 1024,
                    total: 3 * 1024 * 1024,
                },
            ),
            row("waits", 2 * 1024 * 1024, false, UploadStands::Waiting),
        ],
        false,
    )
}

impl Double {
    fn new(read: UploadsRead) -> Arc<Self> {
        Arc::new(Self {
            stamp: Mutex::new((session(1), Some("one@example.eu".into()))),
            read: Mutex::new(Ok(read)),
            reads: AtomicUsize::new(0),
            retries: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
            pauses: AtomicUsize::new(0),
            retry_outcome: Mutex::new(RetryOutcome::Retried),
            cancel_outcome: Mutex::new(CancelOutcome::Cancelled),
            pause_result: Mutex::new(Ok(())),
            hold: Mutex::new(None),
        })
    }
    fn move_to(&self, n: u64, account: &str) {
        *self.stamp.lock().unwrap() = (session(n), Some(account.into()));
    }
}

impl UploadsEngine for Double {
    fn stamp(&self) -> Stamp {
        self.stamp.lock().unwrap().clone()
    }
    fn has_session(&self) -> bool {
        true
    }
    fn uploads_for(&self, _: SessionGeneration) -> Result<UploadsRead, LocalStateRefused> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.read.lock().unwrap().clone()
    }
    fn retry_upload_for(&self, _: SessionGeneration, _: &str) -> Result<RetryOutcome, LocalStateRefused> {
        self.retries.fetch_add(1, Ordering::SeqCst);
        if let Some(hold) = self.hold.lock().unwrap().take() {
            let _ = hold.recv_timeout(WAIT);
        }
        Ok(*self.retry_outcome.lock().unwrap())
    }
    fn cancel_upload_for(&self, _: SessionGeneration, _: &str) -> Result<CancelOutcome, LocalStateRefused> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        Ok(*self.cancel_outcome.lock().unwrap())
    }
    fn set_paused(&self, _: bool) -> Result<(), String> {
        self.pauses.fetch_add(1, Ordering::SeqCst);
        self.pause_result.lock().unwrap().clone()
    }
    fn wake(&self) {}
}

/// `take` until `done` says so, or the wait ran out; everything handed over.
fn take_until(calls: &mut UploadCalls<Double>, done: impl Fn(&UploadCalls<Double>) -> bool) -> Vec<Change> {
    let start = Instant::now();
    let mut all = Vec::new();
    loop {
        all.extend(calls.take());
        if done(calls) || start.elapsed() > WAIT {
            return all;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn held(engine: &Arc<Double>) -> UploadCalls<Double> {
    let mut calls = UploadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |c| c.uploads().is_some());
    calls
}

#[test]
fn the_panel_lists_what_the_engine_holds() {
    let engine = Double::new(three(1));
    let mut calls = UploadCalls::new(engine.clone());
    assert!(calls.uploads().is_none(), "nothing before the engine answered");
    calls.ask();
    take_until(&mut calls, |c| c.uploads().is_some());
    let shown = calls.uploads().expect("rows");
    assert_eq!(shown.items.len(), 3);
    assert_eq!(
        shown.items[0].state,
        UploadState::Failed("Stopped after 5 tries.".into())
    );
    assert!(matches!(&shown.items[1].state, UploadState::Uploading { words, .. } if words == "1.0 MB of 3.0 MB"));
    assert!(matches!(&shown.items[2].state, UploadState::Waiting(w) if w == "2.0 MB \u{00B7} To Trip"));
    assert_eq!(shown.status, "Uploading 2 files \u{00B7} 4.0 MB left");
    assert_eq!(shown.figures, "2 files \u{00B7} 4.0 MB left");
}

#[test]
fn the_status_words_count_only_what_the_read_holds() {
    let stopped = |n: u64| {
        row(
            &format!("s{n}"),
            1,
            true,
            UploadStands::Stopped {
                words: "x".into(),
                retryable: false,
            },
        )
    };
    let said = |rows, paused| words(&read_of(1, rows, paused)).unwrap();
    assert_eq!(said(vec![stopped(1)], false).status, "1 upload needs attention");
    assert_eq!(
        said(vec![stopped(1), stopped(2)], false).figures,
        "2 uploads need attention"
    );
    let one = row("w", 1024, false, UploadStands::Waiting);
    assert_eq!(
        said(vec![one.clone(), stopped(1)], false).status,
        "Uploading 1 file \u{00B7} 1 KB left"
    );
    assert_eq!(
        said(vec![one], true).status,
        "Paused \u{00B7} 1 file \u{00B7} 1 KB left"
    );
    assert!(words(&read_of(1, vec![], false)).is_none());
}

#[test]
fn uploads_of_an_earlier_account_are_dropped() {
    let engine = Double::new(three(1));
    let calls = held(&engine);
    assert!(calls.uploads().is_some());
    engine.move_to(2, "two@example.eu");
    assert!(calls.uploads().is_none(), "another account sees nothing of the first");
    engine.move_to(1, "one@example.eu");
    assert!(calls.uploads().is_some());
}

#[test]
fn an_answer_that_lands_after_the_account_moved_is_dropped() {
    let engine = Double::new(three(1));
    let mut calls = UploadCalls::new(engine.clone());
    calls.ask();
    // The read is on its way (or in the inbox); the account moves before it is taken in.
    std::thread::sleep(Duration::from_millis(100));
    engine.move_to(2, "two@example.eu");
    calls.take();
    assert!(calls.uploads().is_none());
    assert!(
        calls
            .held
            .as_ref()
            .is_none_or(|h| h.0.1.as_deref() != Some("one@example.eu"))
    );
}

#[test]
fn uploads_never_reads_on_a_draw() {
    let engine = Double::new(three(1));
    let calls = held(&engine);
    let before = engine.reads.load(Ordering::SeqCst);
    for _ in 0..100 {
        let _ = calls.uploads();
    }
    assert_eq!(engine.reads.load(Ordering::SeqCst), before);
}

#[test]
fn try_again_asks_once_and_a_second_press_sends_nothing() {
    let engine = Double::new(three(1));
    let mut calls = held(&engine);
    let (release, hold) = mpsc::channel();
    *engine.hold.lock().unwrap() = Some(hold);
    assert_eq!(calls.retry("stops"), Ok(()));
    assert_eq!(calls.retry("stops"), Err(ActionError::InFlight));
    release.send(()).unwrap();
    let changes = take_until(&mut calls, |_| engine.retries.load(Ordering::SeqCst) == 1);
    assert_eq!(engine.retries.load(Ordering::SeqCst), 1);
    assert!(
        changes.is_empty(),
        "a retried change says nothing: the re-read shows it"
    );
}

#[test]
fn try_again_on_a_row_that_cannot_be_retried_asks_nothing() {
    let engine = Double::new(three(1));
    let mut calls = held(&engine);
    assert_eq!(calls.retry("moves"), Err(ActionError::Unavailable));
    assert_eq!(calls.retry("nothing"), Err(ActionError::Unavailable));
    assert_eq!(engine.retries.load(Ordering::SeqCst), 0);
    // A refused outcome (parked, superseded) is said.
    *engine.retry_outcome.lock().unwrap() = RetryOutcome::Parked;
    assert_eq!(calls.retry("stops"), Ok(()));
    let changes = take_until(&mut calls, |_| false);
    assert!(matches!(
        changes.as_slice(),
        [Change::Failed(Action::RetryUpload, ActionError::Failed)]
    ));
}

#[test]
fn pause_draws_after_the_engine_answered_and_says_a_failure() {
    let engine = Double::new(three(1));
    let mut calls = held(&engine);
    assert_eq!(calls.pause(true), Ok(()));
    assert!(!calls.uploads().unwrap().paused, "nothing drawn at the press");
    *engine.read.lock().unwrap() = Ok(read_of(1, three(1).rows, true));
    take_until(&mut calls, |c| c.uploads().is_some_and(|u| u.paused));
    assert!(calls.uploads().unwrap().paused);
    assert_eq!(engine.pauses.load(Ordering::SeqCst), 1);
    // A failed pause says so and the panel keeps the true state.
    *engine.pause_result.lock().unwrap() = Err("no".into());
    assert_eq!(calls.pause(false), Ok(()));
    let changes = take_until(&mut calls, |_| false);
    assert!(matches!(
        changes.as_slice(),
        [Change::Failed(Action::PauseUploads, ActionError::Failed)]
    ));
    assert!(calls.uploads().unwrap().paused);
}

#[test]
fn forget_drops_what_is_held_and_leaves_the_queue() {
    let engine = Double::new(three(1));
    let mut calls = held(&engine);
    calls.forget();
    assert!(calls.uploads().is_none());
    assert_eq!(
        engine.retries.load(Ordering::SeqCst) + engine.cancels.load(Ordering::SeqCst),
        0
    );
    assert_eq!(engine.pauses.load(Ordering::SeqCst), 0);
}

#[test]
fn cancel_is_for_a_new_file_only_and_says_what_was_done() {
    let engine = Double::new(three(1));
    let mut calls = held(&engine);
    assert!(calls.cancels("moves"));
    assert!(!calls.cancels("waits"), "an edit is never cancelled");
    assert_eq!(calls.cancel("waits"), Err(ActionError::Unavailable));
    assert_eq!(engine.cancels.load(Ordering::SeqCst), 0);
    assert_eq!(calls.cancel("moves"), Ok(()));
    let changes = take_until(&mut calls, |_| false);
    assert!(matches!(changes.as_slice(), [Change::Cancelled { id }] if id == "moves"));
    *engine.cancel_outcome.lock().unwrap() = CancelOutcome::RecycleFailed;
    assert_eq!(calls.cancel("moves"), Ok(()));
    let changes = take_until(&mut calls, |_| false);
    assert!(matches!(
        changes.as_slice(),
        [Change::Failed(Action::CancelUpload, ActionError::Failed)]
    ));
}

#[test]
fn a_read_that_failed_is_read_again_at_the_next_ask() {
    let engine = Double::new(three(1));
    *engine.read.lock().unwrap() = Err(LocalStateRefused::Unreadable);
    let mut calls = UploadCalls::new(engine.clone());
    calls.ask();
    take_until(&mut calls, |_| engine.reads.load(Ordering::SeqCst) == 1);
    take_until(&mut calls, |c| !c.reading);
    assert!(calls.uploads().is_none());
    *engine.read.lock().unwrap() = Ok(three(1));
    calls.ask();
    take_until(&mut calls, |c| c.uploads().is_some());
    assert_eq!(engine.reads.load(Ordering::SeqCst), 2);
}
