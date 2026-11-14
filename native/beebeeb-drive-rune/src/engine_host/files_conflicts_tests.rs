//! Two versions on the engine's rows (`ConflictCalls`), against a double of the engine: what is drawn and when, the
//! words for each side, what an answer sends, and the session rule. Every name is invented.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use beebeeb_engine::app::conflicts_api::{Conflicts, WaitingConflict};

use super::*;

const WAIT: Duration = Duration::from_secs(10);

struct Double {
    session: AtomicU64,
    waiting: Mutex<Vec<WaitingConflict>>,
    reads: AtomicUsize,
    asked: Mutex<Vec<(u64, String, ConflictChoice)>>,
    answer: Mutex<Result<(), ResolveRefused>>,
    /// The first answer waits for this before it returns.
    gate: Mutex<Option<mpsc::Receiver<()>>>,
}

impl Double {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            session: AtomicU64::new(1),
            waiting: Mutex::new(vec![waiting("file-1", "Invented draft.txt")]),
            reads: AtomicUsize::new(0),
            asked: Mutex::default(),
            answer: Mutex::new(Ok(())),
            gate: Mutex::default(),
        })
    }
}

fn waiting(id: &str, name: &str) -> WaitingConflict {
    WaitingConflict {
        file_id: id.into(),
        name: name.into(),
        here_modified: "2026-11-05T09:12:00Z".into(),
        here_size: 1_258_291,
        there_created: "2026-11-05T08:50:00Z".into(),
        there_size: 1_363_148,
    }
}

impl ConflictEngine for Double {
    fn unlocked(&self) -> bool {
        true
    }
    fn session(&self) -> SessionGeneration {
        SessionGeneration::for_test(self.session.load(Ordering::SeqCst))
    }
    fn account(&self) -> Option<String> {
        Some("invented@example.eu".into())
    }
    fn waiting(&self, session: SessionGeneration) -> Result<Conflicts, ConflictsRefused> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(Conflicts {
            session,
            files: self.waiting.lock().unwrap().clone(),
            unread: 0,
        })
    }
    fn resolve(&self, session: SessionGeneration, file: &str, choice: ConflictChoice) -> Result<(), ResolveRefused> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.recv_timeout(WAIT);
        }
        let number = self.session.load(Ordering::SeqCst);
        self.asked.lock().unwrap().push((number, file.into(), choice));
        // The engine's own rule: a session that moved on is refused with nothing done.
        if session != SessionGeneration::for_test(number) {
            return Err(ResolveRefused::NotSignedIn);
        }
        let answer = *self.answer.lock().unwrap();
        if answer.is_ok() {
            self.waiting.lock().unwrap().retain(|w| w.file_id != file);
        }
        answer
    }
    fn wake(&self) {}
}

/// Pump until `done` holds (the workers answer on their own threads).
fn until(calls: &mut ConflictCalls<Double>, mut done: impl FnMut(&ConflictCalls<Double>) -> bool) -> Vec<Change> {
    let end = Instant::now() + WAIT;
    let mut all = Vec::new();
    while Instant::now() < end {
        all.extend(calls.take());
        if done(calls) {
            return all;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no answer in time");
}

#[test]
fn nothing_is_drawn_before_the_engine_answered_then_both_sides_in_the_dialogs_words() {
    let engine = Double::new();
    let mut calls = ConflictCalls::new(engine.clone());
    assert!(calls.conflicts().is_empty(), "nothing before the first look");
    calls.take();
    assert!(calls.conflicts().is_empty(), "the read is on its way");
    until(&mut calls, |c| !c.conflicts().is_empty());
    let shown = calls.conflicts();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].file, "file-1");
    assert_eq!(shown[0].here.device, "This PC");
    assert_eq!(shown[0].there.device, "Another device");
    assert_eq!(
        (shown[0].here.kind, shown[0].there.kind),
        (Device::Computer, Device::Computer)
    );
    assert_eq!(shown[0].here.size, crate::source::format_size(1_258_291));
    assert_eq!(shown[0].there.size, crate::source::format_size(1_363_148));
    assert_eq!(
        shown[0].there.when,
        crate::ui::file_list::format_date("2026-11-05T08:50:00Z")
    );
}

#[test]
fn an_answer_is_sent_once_hides_the_file_and_the_list_is_read_again() {
    let engine = Double::new();
    let mut calls = ConflictCalls::new(engine.clone());
    until(&mut calls, |c| !c.conflicts().is_empty());
    let (hold, gate) = mpsc::channel();
    *engine.gate.lock().unwrap() = Some(gate);
    calls.keep("file-1", Keep::There).expect("accepted");
    assert!(calls.conflicts().is_empty(), "not offered again while it is on its way");
    assert_eq!(
        calls.keep("file-1", Keep::Here),
        Err(ActionError::InFlight),
        "never a second answer"
    );
    let reads = engine.reads.load(Ordering::SeqCst);
    hold.send(()).unwrap();
    let changes = until(&mut calls, |c| {
        engine.reads.load(Ordering::SeqCst) > reads && !c.reading
    });
    assert!(changes.is_empty(), "it worked: nothing to say");
    assert!(calls.conflicts().is_empty());
    assert_eq!(
        *engine.asked.lock().unwrap(),
        [(1, "file-1".to_string(), ConflictChoice::KeepThere)]
    );
}

#[test]
fn an_answer_that_did_not_work_says_so_and_the_file_waits_again() {
    let engine = Double::new();
    *engine.answer.lock().unwrap() = Err(ResolveRefused::Call(AccountCallError::Offline));
    let mut calls = ConflictCalls::new(engine.clone());
    until(&mut calls, |c| !c.conflicts().is_empty());
    calls.keep("file-1", Keep::Both).expect("accepted");
    let changes = until(&mut calls, |c| c.resolving.is_empty());
    assert_eq!(changes, [Change::Failed(Action::KeepVersion, ActionError::Offline)]);
    until(&mut calls, |c| !c.conflicts().is_empty());
}

#[test]
fn an_answer_that_lands_under_another_session_is_dropped() {
    let engine = Double::new();
    let mut calls = ConflictCalls::new(engine.clone());
    until(&mut calls, |c| !c.conflicts().is_empty());
    let (hold, gate) = mpsc::channel();
    *engine.gate.lock().unwrap() = Some(gate);
    calls.keep("file-1", Keep::Here).expect("accepted");
    // The vault locks and unlocks while the call runs.
    engine.session.store(2, Ordering::SeqCst);
    hold.send(()).unwrap();
    let changes = until(&mut calls, |c| {
        c.resolving.is_empty() && c.seen.as_ref().is_some_and(|s| s.0 == SessionGeneration::for_test(2))
    });
    assert!(
        changes.is_empty(),
        "the earlier session's answer says nothing to this one"
    );
    assert_eq!(
        engine.waiting.lock().unwrap().len(),
        1,
        "the engine refused it with nothing done, and the file still waits"
    );
}

#[test]
fn a_file_the_engine_did_not_list_is_refused_with_nothing_sent() {
    let engine = Double::new();
    let mut calls = ConflictCalls::new(engine.clone());
    until(&mut calls, |c| !c.conflicts().is_empty());
    assert_eq!(calls.keep("other", Keep::Here), Err(ActionError::Failed));
    assert!(engine.asked.lock().unwrap().is_empty());
}
