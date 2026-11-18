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
    // Wait until the restarted helper is open for editing again.
    assert!(wait_for(|| {
        let _ = host.poll();
        host.edit(die()).is_ok()
    }));
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
    assert!(wait_for(|| {
        let _ = host.poll();
        host.edit(die()).is_ok()
    }));
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
