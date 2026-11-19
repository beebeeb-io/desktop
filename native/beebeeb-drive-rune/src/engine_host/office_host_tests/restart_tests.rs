//! A helper that dies mid-edit comes back from the last snapshot (`office_host/restart.rs`, `held.rs`), against the fake
//! helper: the snapshot timing, one `Restored`, a second crash within the window opening the document read-only.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

use beebeeb_office_protocol::office::{self, Message};
use zeroize::Zeroizing;

use super::fake_helper_tests::*;
use crate::engine_host::office_host::*;

const SNAPSHOT_LEN: u64 = 10;

fn marks() -> office::Marks {
    office::Marks {
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        style: office::ParaStyle::Normal,
        align: office::TextAlign::Left,
        list: office::ListKind::None,
    }
}

fn edited(revision: u64) -> Message {
    Message::Edited(office::Edited {
        revision,
        caret: None,
        selection: Vec::new(),
        can_undo: true,
        can_redo: false,
        dirty: office::Dirty::All,
        pages: None,
        cells: Vec::new(),
        slides: None,
        marks: marks(),
    })
}

fn die() -> EditOp {
    EditOp::InsertText("die".to_owned())
}

/// A helper that serves an editing session (and the viewer's `Open`); an `InsertText("die")` makes it exit.
fn mortal() -> Fake {
    mortal_slow(0)
}

/// [`mortal`] whose answer to the size of a save takes `delay_ms`.
fn mortal_slow(delay_ms: u64) -> Fake {
    Fake::new(move |link| {
        let mut revision = 0;
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) | Message::OpenEdit(_) | Message::NewDoc(_) => opened(&[LETTER]),
                Message::Edit(office::Edit {
                    op: EditOp::InsertText(text),
                }) if text == "die" => return,
                Message::Edit(_) => {
                    revision += 1;
                    edited(revision)
                }
                Message::SaveSize(_) => {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    Message::SaveReady(office::SaveReady {
                    token: 7,
                    len: SNAPSHOT_LEN,
                    revision,
                    })
                }
                Message::SaveInto(into) => {
                    link.paint(into.out, 0x5a);
                    Message::SavedInto(office::SavedInto { len: into.out.len })
                }
                _ => Message::Error(office::Error {
                    code: office::ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    })
}

fn cadence(idle_ms: u64, every_ms: u64, window_ms: u64) -> Cadence {
    Cadence {
        idle: Duration::from_millis(idle_ms),
        every: Duration::from_millis(every_ms),
        crash_window: Duration::from_millis(window_ms),
    }
}

fn start(fake: Fake, cadence: Cadence, registry: &Registry) -> OfficeHost {
    let mut host = open_mode_cadence(
        fake,
        registry,
        TEST_TIMEOUTS,
        cadence,
        Mode::Edit(EditKind::Docx),
        DocKind::Docx,
        Some(Zeroizing::new(doc())),
        noop_wake(),
    );
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    host
}

fn wait_events(host: &mut OfficeHost, got: &mut Vec<EditEvent>, want: impl Fn(&[EditEvent]) -> bool) -> bool {
    let until = std::time::Instant::now() + WITHIN;
    loop {
        let _ = host.poll();
        got.extend(host.poll_edit());
        if want(got) {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn snapshots(got: &[EditEvent]) -> usize {
    got.iter()
        .filter(|event| matches!(event, EditEvent::Saved { purpose: Purpose::Snapshot, .. }))
        .count()
}

fn restored(got: &[EditEvent]) -> usize {
    got.iter().filter(|event| matches!(event, EditEvent::Restored { .. })).count()
}

fn save_sizes_asked(log: &Log) -> usize {
    lock(&log.sent)
        .iter()
        .filter(|(kind, _)| *kind == office::KIND_SAVE_SIZE)
        .count()
}

/// The sections made so far: the document's first, then each save, then each restart's document.
fn sections(log: &Log) -> Vec<usize> {
    lock(&log.section_sizes).clone()
}

#[test]
fn a_killed_helper_restarts_from_the_snapshot_and_says_restored_once() {
    let registry = Registry::new();
    let fake = mortal();
    let log = fake.log.clone();
    let mut host = start(fake, cadence(40, 400, 60_000), &registry);
    let mut got = Vec::new();

    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| snapshots(got) == 1), "a snapshot after idle: {got:?}");
    let doc_len = doc().len();
    assert_eq!(sections(&log), vec![doc_len, SNAPSHOT_LEN as usize]);

    let dying = host.edit(die()).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| restored(got) == 1), "{got:?}");
    assert!(
        got.iter().any(|event| matches!(event, EditEvent::Failed { id, .. } if *id == dying)),
        "the request that was out is answered: {got:?}"
    );
    assert_eq!(log.starts(), 2);
    assert_eq!(
        sections(&log).last(),
        Some(&(SNAPSHOT_LEN as usize)),
        "the new helper opened the snapshot, not the opened bytes: {:?}",
        sections(&log)
    );
    assert!(matches!(
        got.iter().find(|event| matches!(event, EditEvent::Restored { .. })),
        Some(EditEvent::Restored { from_snapshot: true, .. })
    ));
    assert!(!host.read_only());
    // The session goes on, and nothing says Restored a second time.
    assert!(matches!(host.state(), OfficeState::Pages { pages: 1, .. }), "{:?}", host.state());
    host.edit(EditOp::SelectAll).expect("editing works again");
    std::thread::sleep(Duration::from_millis(100));
    let _ = host.poll();
    got.extend(host.poll_edit());
    assert_eq!(restored(&got), 1);
}

#[test]
fn without_a_snapshot_the_restart_opens_the_opened_bytes() {
    let registry = Registry::new();
    let fake = mortal();
    let log = fake.log.clone();
    // Nothing idle for 30 s: no snapshot exists when the helper dies.
    let mut host = start(fake, cadence(30_000, 30_000, 60_000), &registry);
    let mut got = Vec::new();
    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    host.edit(die()).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| restored(got) == 1), "{got:?}");
    assert_eq!(sections(&log), vec![doc().len(), doc().len()]);
    assert!(matches!(
        got.iter().find(|event| matches!(event, EditEvent::Restored { .. })),
        Some(EditEvent::Restored { from_snapshot: false, .. })
    ));
}

#[test]
fn two_kills_within_the_window_open_it_read_only_and_offer_the_snapshot() {
    let registry = Registry::new();
    let fake = mortal();
    let log = fake.log.clone();
    let mut host = start(fake, cadence(40, 400, 60_000), &registry);
    let mut got = Vec::new();

    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| snapshots(got) == 1));
    host.edit(die()).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| restored(got) == 1), "{got:?}");
    // `Restored` comes after the new helper opened: editing works again.
    host.edit(die()).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| {
        got.iter().any(|event| matches!(event, EditEvent::ReadOnly { .. }))
    }), "{got:?}");

    assert_eq!(restored(&got), 1, "Restored once, then read-only: {got:?}");
    match got.iter().find(|event| matches!(event, EditEvent::ReadOnly { .. })) {
        Some(EditEvent::ReadOnly { copy: Some(copy), .. }) => assert_eq!(&copy[..], &[0x5a; SNAPSHOT_LEN as usize]),
        other => panic!("the snapshot is offered as a copy: {other:?}"),
    }
    assert!(host.read_only());
    assert_eq!(log.starts(), 3, "a viewer opens the snapshot");
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert_eq!(host.edit(EditOp::SelectAll), Err(EditRefused::NotEditing));
    assert_eq!(host.save_bytes(Purpose::Save), Err(EditRefused::NotEditing));
}

#[test]
fn a_second_kill_after_the_window_restarts_again() {
    let registry = Registry::new();
    let fake = mortal();
    let log = fake.log.clone();
    let mut host = start(fake, cadence(30_000, 30_000, 150), &registry);
    let mut got = Vec::new();
    host.edit(die()).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| restored(got) == 1));
    std::thread::sleep(Duration::from_millis(300));
    host.edit(die()).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| restored(got) == 2), "{got:?}");
    assert!(!host.read_only());
    assert_eq!(log.starts(), 3);
}

#[test]
fn no_snapshot_is_asked_while_nothing_changed() {
    let registry = Registry::new();
    let fake = mortal();
    let log = fake.log.clone();
    let mut host = start(fake, cadence(30, 100, 60_000), &registry);
    let mut got = Vec::new();
    // Opened and untouched, and a click: not an edit.
    let _ = host.hit(0, 10.0, 10.0, HitMode::Place);
    std::thread::sleep(Duration::from_millis(400));
    let _ = host.poll();
    got.extend(host.poll_edit());
    assert_eq!(save_sizes_asked(&log), 0, "{got:?}");

    // One edit: one snapshot, then quiet again.
    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| snapshots(got) == 1), "{got:?}");
    std::thread::sleep(Duration::from_millis(400));
    let _ = host.poll();
    got.extend(host.poll_edit());
    assert_eq!(save_sizes_asked(&log), 1);
    assert_eq!(snapshots(&got), 1);
}

#[test]
fn steady_typing_still_snapshots_every_so_often() {
    let registry = Registry::new();
    let fake = mortal();
    // Input every 20 ms never leaves 200 ms idle, but `every` (150 ms) is reached.
    let mut host = start(fake, cadence(200, 150, 60_000), &registry);
    let mut got = Vec::new();
    let typing = Arc::new(AtomicUsize::new(0));
    let until = std::time::Instant::now() + Duration::from_millis(600);
    while std::time::Instant::now() < until {
        host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
        typing.fetch_add(1, SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        let _ = host.poll();
        got.extend(host.poll_edit());
    }
    assert!(snapshots(&got) >= 2, "{} edits, snapshots: {}", typing.load(SeqCst), snapshots(&got));
}

// ---- the lock's flush (`flush_all_in`): unsaved work is sealed on the worker's thread before the session ends ----

fn collecting(host: &mut OfficeHost) -> Arc<std::sync::Mutex<Vec<Vec<u8>>>> {
    let kept: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::default();
    let sink = kept.clone();
    host.set_flush_sink(Arc::new(move |bytes| sink.lock().unwrap().push(bytes.to_vec())));
    kept
}

#[test]
fn a_lock_seals_what_was_typed_before_any_timer_fired() {
    let registry = Registry::new();
    // No timer for 30 s: only the flush can have taken a snapshot.
    let mut host = start(mortal(), cadence(30_000, 30_000, 60_000), &registry);
    let kept = collecting(&mut host);
    host.edit(EditOp::InsertText("last words".to_owned())).expect("sent");

    let began = std::time::Instant::now();
    flush_all_in(&registry, Duration::from_secs(2));
    assert!(began.elapsed() < Duration::from_secs(2), "the flush answered, it did not run to its bound");
    assert_eq!(&*kept.lock().unwrap(), &vec![vec![0x5a; SNAPSHOT_LEN as usize]], "one sealed snapshot of the latest text");
}

#[test]
fn a_lock_seals_nothing_when_nothing_is_unsaved() {
    let registry = Registry::new();
    let mut host = start(mortal(), cadence(30_000, 30_000, 60_000), &registry);
    let kept = collecting(&mut host);
    flush_all_in(&registry, Duration::from_secs(1));
    assert!(kept.lock().unwrap().is_empty(), "no edits: no blob");

    // Typed, then saved: the save holds it all, so the lock has nothing of it to keep.
    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    let id = host.save_bytes(Purpose::Save).expect("sent");
    let mut got = Vec::new();
    assert!(
        wait_events(&mut host, &mut got, |got| got
            .iter()
            .any(|e| matches!(e, EditEvent::Saved { id: got_id, purpose: Purpose::Save, .. } if *got_id == id))),
        "{got:?}"
    );
    flush_all_in(&registry, Duration::from_secs(1));
    assert!(kept.lock().unwrap().is_empty(), "saved: no blob");
}

#[test]
fn a_helper_that_does_not_answer_never_holds_the_lock_past_its_bound() {
    let registry = Registry::new();
    let fake = Fake::new(move |link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) | Message::OpenEdit(_) | Message::NewDoc(_) => opened(&[LETTER]),
                Message::Edit(_) => edited(1),
                Message::SaveSize(_) => {
                    std::thread::sleep(Duration::from_secs(5));
                    return;
                }
                _ => return,
            };
            link.answer(id, reply);
        }
    });
    let mut host = start(fake, cadence(30_000, 30_000, 60_000), &registry);
    let kept = collecting(&mut host);
    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    let began = std::time::Instant::now();
    flush_all_in(&registry, Duration::from_millis(300));
    let took = began.elapsed();
    assert!(took >= Duration::from_millis(300) && took < Duration::from_millis(1_500), "bounded: {took:?}");
    assert!(kept.lock().unwrap().is_empty());
}

// ---- a screen lock's flush (`screen_lock_begin_in`): the lock is immediate, the seal and the close come after ----

fn leaked_registry() -> &'static Registry {
    Box::leak(Box::new(Registry::new()))
}

#[test]
fn a_screen_lock_returns_at_once_then_seals_the_last_text_and_closes_the_helper() {
    let registry = leaked_registry();
    // The helper takes 150 ms to answer the snapshot: the call that follows the lock frame must not wait for it.
    let fake = mortal_slow(150);
    let mut host = start(fake, cadence(30_000, 30_000, 60_000), registry);
    let kept = collecting(&mut host);
    host.edit(EditOp::InsertText("last words".to_owned())).expect("sent");

    let began = std::time::Instant::now();
    let run = screen_lock_begin_in(registry, Duration::from_secs(3));
    assert!(began.elapsed() < Duration::from_millis(100), "the lock does not wait for the seal: {:?}", began.elapsed());
    assert!(kept.lock().unwrap().is_empty(), "not sealed yet when the lock call returned");
    assert_ne!(host.state(), OfficeState::Closed(CloseWhy::Locked), "the helper is still there for the seal");

    run.finish();
    assert_eq!(kept.lock().unwrap().len(), 1, "one sealed snapshot of the latest text");
    assert_eq!(host.state(), OfficeState::Closed(CloseWhy::Locked), "then the helper is closed");
}

#[test]
fn the_shells_close_during_a_screen_lock_flush_waits_for_the_seal() {
    let registry = leaked_registry();
    let mut host = start(mortal(), cadence(30_000, 30_000, 60_000), registry);
    let kept = collecting(&mut host);
    host.edit(EditOp::InsertText("last words".to_owned())).expect("sent");

    let run = screen_lock_begin_in(registry, Duration::from_secs(2));
    // The shell's poll sees the gate leave Ready and ends the session: it must not kill the helper mid-seal.
    end_session_in(registry, CloseWhy::SignedOut);
    assert_ne!(host.state(), OfficeState::Closed(CloseWhy::SignedOut), "deferred, not killed");
    run.finish();
    assert_eq!(kept.lock().unwrap().len(), 1, "the seal landed first");
    assert_eq!(host.state(), OfficeState::Closed(CloseWhy::SignedOut), "the shell's reason stands");
    // With no flush running, the shell's close is immediate again.
    let mut again = start(mortal(), cadence(30_000, 30_000, 60_000), registry);
    end_session_in(registry, CloseWhy::Locked);
    assert_eq!(again.state(), OfficeState::Closed(CloseWhy::Locked));
    let _ = again.poll();
}

fn saved_for(got: &[EditEvent], purpose: Purpose) -> usize {
    got.iter()
        .filter(|event| matches!(event, EditEvent::Saved { purpose: p, .. } if *p == purpose))
        .count()
}

/// w23: Ctrl+S after a restart gives the document's bytes (the window's save asks `Purpose::Save`).
#[test]
fn a_save_after_a_restart_answers_with_the_documents_bytes() {
    let registry = Registry::new();
    let mut host = start(mortal(), cadence(40, 400, 60_000), &registry);
    let mut got = Vec::new();
    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| snapshots(got) == 1));
    host.edit(die()).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| restored(got) == 1), "{got:?}");
    let id = host.save_bytes(Purpose::Save).expect("the restarted session takes a save");
    assert!(
        wait_events(&mut host, &mut got, |got| saved_for(got, Purpose::Save) == 1),
        "the save is answered: {got:?}"
    );
    assert!(got.iter().any(|event| matches!(event, EditEvent::Saved { id: i, purpose: Purpose::Save, .. } if *i == id)));
}

/// A helper that serves an editing session; the first helper started exits at its `exit_at`'th message (0: never),
/// every later one lives.
fn dies_once_at(exit_at: usize) -> Fake {
    let links = Arc::new(AtomicUsize::new(0));
    Fake::new(move |link| {
        let first = links.fetch_add(1, SeqCst) == 0;
        let mut seen = 0;
        let mut revision = 0;
        while let Some((id, message)) = link.next() {
            seen += 1;
            if first && seen == exit_at {
                return;
            }
            let reply = match message {
                Message::Open(_) | Message::OpenEdit(_) | Message::NewDoc(_) => opened(&[LETTER]),
                Message::Edit(_) => {
                    revision += 1;
                    edited(revision)
                }
                Message::SaveSize(_) => Message::SaveReady(office::SaveReady {
                    token: 7,
                    len: SNAPSHOT_LEN,
                    revision,
                }),
                Message::SaveInto(into) => {
                    link.paint(into.out, 0x5a);
                    Message::SavedInto(office::SavedInto { len: into.out.len })
                }
                _ => Message::Error(office::Error {
                    code: office::ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    })
}

/// w23: the helper is killed while the document is still opening (a slow first start): the session starts again from
/// the opened bytes instead of ending as "stopped".
#[test]
fn a_helper_killed_while_the_document_opens_is_started_again_from_the_opened_bytes() {
    let registry = Registry::new();
    let fake = dies_once_at(1);
    let log = fake.log.clone();
    let mut host = open_mode_cadence(
        fake,
        &registry,
        TEST_TIMEOUTS,
        cadence(40, 400, 60_000),
        Mode::Edit(EditKind::Docx),
        DocKind::Docx,
        Some(Zeroizing::new(doc())),
        noop_wake(),
    );
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(matches!(host.state(), OfficeState::Pages { .. }), "opened on the second helper: {:?}", host.state());
    assert_eq!(log.starts(), 2);
    host.edit(EditOp::InsertText("a".to_owned())).expect("the session edits");
    host.save_bytes(Purpose::Save).expect("and saves");
    let mut got = Vec::new();
    assert!(wait_events(&mut host, &mut got, |got| saved_for(got, Purpose::Save) == 1), "{got:?}");
    assert_eq!(restored(&got), 0, "nothing was open to restore: {got:?}");
}

/// w23: the helper is killed while a save's size is asked: that save is answered failed, the session restarts, and the
/// next Ctrl+S saves.
#[test]
fn a_helper_killed_during_a_save_restarts_and_the_next_save_answers() {
    let registry = Registry::new();
    // Messages of the first helper: OpenEdit, Edit, SaveSize (the third) -> it exits.
    let mut host = start(dies_once_at(3), cadence(30_000, 30_000, 60_000), &registry);
    let mut got = Vec::new();
    host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    let first = host.save_bytes(Purpose::Save).expect("sent");
    assert!(wait_events(&mut host, &mut got, |got| restored(got) == 1), "{got:?}");
    assert!(
        got.iter().any(|event| matches!(event, EditEvent::Failed { id, .. } if *id == first)),
        "the save that was out is answered: {got:?}"
    );
    let second = host.save_bytes(Purpose::Save).expect("the restarted session takes a save");
    assert!(wait_events(&mut host, &mut got, |got| saved_for(got, Purpose::Save) == 1), "{got:?}");
    assert!(got.iter().any(|event| matches!(event, EditEvent::Saved { id, .. } if *id == second)));
}
