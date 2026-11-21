//! The editing session's rules (`office_host/edit.rs`) against the fake helper: answers are checked, a refusal ends the
//! helper, the save handshake makes a section of exactly the granted length.

use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{self, Message};
use zeroize::Zeroizing;

use super::fake_helper_tests::*;
use crate::engine_host::office_host::*;

fn page_rect(page: u32, x: f32, y: f32, w: f32, h: f32) -> office::PageRect {
    office::PageRect {
        page,
        rect: office::RectPt { x, y, w, h },
    }
}

fn edited(revision: u64, caret: Option<office::PageRect>) -> office::Edited {
    office::Edited {
        revision,
        caret,
        selection: Vec::new(),
        can_undo: true,
        can_redo: false,
        dirty: office::Dirty::All,
        pages: None,
        cells: Vec::new(),
        slides: None,
        marks: office::Marks {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            style: office::ParaStyle::Normal,
            align: office::TextAlign::Left,
            list: office::ListKind::None,
        },
    }
}

/// A helper that opens a Letter page and answers `edit` for every `Edit`; `save` decides the save handshake.
fn helper(
    edit: impl Fn(u32) -> Message + Send + Sync + 'static,
    save_ready_len: u64,
    saved_len: Option<u64>,
) -> Fake {
    Fake::new(move |link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::Open(_) | Message::OpenEdit(_) | Message::NewDoc(_) => opened(&[LETTER]),
                Message::Edit(_) | Message::Hit(_) | Message::InsertImage(_) => edit(id),
                Message::SaveSize(_) => Message::SaveReady(office::SaveReady {
                    token: 7,
                    len: save_ready_len,
                    revision: 3,
                }),
                Message::SaveInto(into) => {
                    link.paint(into.out, 0x5a);
                    Message::SavedInto(office::SavedInto {
                        len: saved_len.unwrap_or(into.out.len),
                    })
                }
                Message::Copy(_) => Message::Clip(office::Clip {
                    text: "hi".to_owned(),
                    html: None,
                }),
                _ => Message::Error(office::Error {
                    code: office::ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    })
}

fn open_editing(fake: Fake, registry: &Registry) -> (OfficeHost, std::sync::Arc<Log>) {
    let log = fake.log.clone();
    let wake = noop_wake();
    let mut host = open_mode_for_test(fake, registry, wake);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    (host, log)
}

fn open_mode_for_test(fake: Fake, registry: &Registry, wake: Wake) -> OfficeHost {
    open_mode(
        fake,
        registry,
        TEST_TIMEOUTS,
        Mode::Edit(EditKind::Docx),
        DocKind::Docx,
        Some(Zeroizing::new(doc())),
        wake,
    )
}

/// Polls until `want` edit events arrived (or 2 s, or the helper stopped).
fn edit_events(host: &mut OfficeHost, want: usize) -> Vec<EditEvent> {
    let mut got = Vec::new();
    let until = std::time::Instant::now() + WITHIN;
    loop {
        let _ = host.poll();
        got.extend(host.poll_edit());
        if got.len() >= want || stopped_state(host) || std::time::Instant::now() >= until {
            return got;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Polls until the window says "Renderer stopped".
fn until_stopped(host: &mut OfficeHost) -> bool {
    let mut events = Vec::new();
    pump(host, &mut events, |host, _| stopped_state(host))
}

fn stopped_state(host: &OfficeHost) -> bool {
    matches!(host.state(), OfficeState::Stopped { .. })
}

#[test]
fn an_edit_answer_arrives_for_its_request_id() {
    let registry = Registry::new();
    let fake = helper(|_| Message::Edited(edited(1, Some(page_rect(0, 72.0, 72.0, 1.0, 14.0)))), 10, None);
    let (mut host, log) = open_editing(fake, &registry);
    let first = host.edit(EditOp::InsertText("a".to_owned())).expect("sent");
    let second = host.edit(EditOp::SelectAll).expect("sent");
    assert_ne!(first, second);
    let got = edit_events(&mut host, 2);
    let ids: Vec<RequestId> = got
        .iter()
        .filter_map(|event| match event {
            EditEvent::Edited { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, vec![first, second], "{got:?}");
    assert_eq!(log.kills(), 0);
}

#[test]
fn a_rect_outside_its_page_is_refused_and_the_helper_ended() {
    let registry = Registry::new();
    // Letter is 612 x 792 pt: this caret is past the right edge.
    let fake = helper(|_| Message::Edited(edited(1, Some(page_rect(0, 600.0, 10.0, 50.0, 14.0)))), 10, None);
    let (mut host, log) = open_editing(fake, &registry);
    host.edit(EditOp::SelectAll).expect("sent");
    let got = edit_events(&mut host, 1);
    assert!(got.is_empty(), "nothing of it is used: {got:?}");
    assert!(until_stopped(&mut host));
    assert_eq!(log.kills(), 1, "the helper is ended");
    assert!(matches!(
        host.state(),
        OfficeState::Stopped {
            why: Stop::Protocol,
            can_retry: false
        }
    ));
}

#[test]
fn a_save_ready_over_the_cap_is_refused_and_the_helper_ended() {
    let registry = Registry::new();
    let fake = helper(|_| Message::Edited(edited(1, None)), check::MAX_SAVE_BYTES + 1, None);
    let (mut host, log) = open_editing(fake, &registry);
    host.save_bytes(Purpose::Save).expect("sent");
    assert!(until_stopped(&mut host));
    assert_eq!(log.kills(), 1);
    assert_eq!(log.section_sizes.lock().unwrap().len(), 1, "no section was made for it: only the document's");
    assert!(edit_events(&mut host, 0).is_empty());
}

#[test]
fn a_saved_into_of_the_wrong_length_is_refused_and_the_helper_ended() {
    let registry = Registry::new();
    let fake = helper(|_| Message::Edited(edited(1, None)), 10, Some(9));
    let (mut host, log) = open_editing(fake, &registry);
    host.save_bytes(Purpose::Snapshot).expect("sent");
    assert!(until_stopped(&mut host));
    assert_eq!(log.kills(), 1);
    assert!(host.poll_edit().is_empty(), "the bytes are not handed out");
}

#[test]
fn a_save_makes_a_section_of_exactly_len_and_hands_out_its_bytes() {
    let registry = Registry::new();
    let fake = helper(|_| Message::Edited(edited(4, None)), 10, None);
    let (mut host, log) = open_editing(fake, &registry);
    let save = host.save_bytes(Purpose::Save).expect("sent");
    // An edit during the save waits and goes out after it.
    let edit = host.edit(EditOp::SelectAll).expect("sent");
    let got = edit_events(&mut host, 2);
    assert_eq!(log.section_sizes.lock().unwrap().last(), Some(&10), "exactly len");
    match &got[0] {
        EditEvent::Saved {
            id,
            purpose,
            revision,
            bytes,
        } => {
            assert_eq!((*id, *purpose, *revision), (save, Purpose::Save, 3));
            assert_eq!(&bytes[..], &[0x5a; 10]);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&got[1], EditEvent::Edited { id, .. } if *id == edit), "{got:?}");
    assert_eq!(log.live(), 1, "the save's section is dropped: the document's stays");
}

#[test]
fn a_paste_over_the_text_cap_is_refused_with_words() {
    let registry = Registry::new();
    let fake = helper(|_| Message::Edited(edited(1, None)), 10, None);
    let (mut host, _) = open_editing(fake, &registry);
    let text = "x".repeat(check::MAX_OP_TEXT_BYTES + 1);
    assert_eq!(host.edit(EditOp::Paste { text }), Err(EditRefused::PasteTooLarge));
    assert!(EditRefused::PasteTooLarge.words().contains("paste"));
    let text = "x".repeat(check::MAX_OP_TEXT_BYTES + 1);
    assert_eq!(host.edit(EditOp::InsertText(text)), Err(EditRefused::TooMuchText));
}

#[test]
fn a_viewer_has_no_edit_calls() {
    let registry = Registry::new();
    let sizes = [LETTER];
    let (mut host, _, _) = open_fake(Fake::new(move |link| serve(link, &sizes)), &registry, DocKind::Docx);
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert_eq!(host.edit(EditOp::SelectAll), Err(EditRefused::NotEditing));
    assert_eq!(host.save_bytes(Purpose::Save), Err(EditRefused::NotEditing));
}

#[test]
fn a_new_document_opens_with_no_bytes() {
    let registry = Registry::new();
    let fake = helper(|_| Message::Edited(edited(1, None)), 10, None);
    let log = fake.log.clone();
    let mut host = open_mode(
        fake,
        &registry,
        TEST_TIMEOUTS,
        Mode::New(EditKind::Docx, Template::Blank),
        DocKind::Docx,
        None,
        noop_wake(),
    );
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    assert!(matches!(host.state(), OfficeState::Pages { pages: 1, .. }));
    assert_eq!(log.section_sizes.lock().unwrap().len(), 0, "no document section");
}

/// A helper whose editor guard refuses the file (`EditGuarded` with `dropped`) and whose viewer opens it.
fn guarding(dropped: Vec<office::DroppedPart>) -> Fake {
    Fake::new(move |link| {
        while let Some((id, message)) = link.next() {
            let reply = match message {
                Message::OpenEdit(_) => Message::EditGuarded(office::EditGuarded { dropped: dropped.clone() }),
                Message::Open(_) => opened(&[LETTER]),
                _ => Message::Error(office::Error {
                    code: office::ErrorCode::Internal,
                }),
            };
            link.answer(id, reply);
        }
    })
}

fn open_guarded(
    registry: &Registry,
    dropped: Vec<office::DroppedPart>,
    kind: EditKind,
    doc_kind: DocKind,
) -> (OfficeHost, std::sync::Arc<Log>) {
    let fake = guarding(dropped);
    let log = fake.log.clone();
    let mut host = open_mode(fake, registry, TEST_TIMEOUTS, Mode::Edit(kind), doc_kind, Some(Zeroizing::new(doc())), noop_wake());
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events), "{:?}", host.state());
    (host, log)
}

#[test]
fn edit_guarded_pptx_with_speaker_notes_opens_read_only_in_the_viewer() {
    let registry = Registry::new();
    let (mut host, log) = open_guarded(&registry, vec![office::DroppedPart::SpeakerNotes], EditKind::Pptx, DocKind::Pptx);
    // Not "Renderer stopped" (Stop::Protocol): the viewer's pages, from a second helper told `Open`.
    assert!(matches!(host.state(), OfficeState::Pages { pages: 1, .. }), "{:?}", host.state());
    assert_eq!(log.starts(), 2, "the guarded editor ends; a viewer helper opens the same document");
    assert_eq!(host.guarded(), vec![office::DroppedPart::SpeakerNotes]);
    assert_eq!(host.edit(office::EditOp::Undo), Err(EditRefused::NotEditing), "no edit on a guarded file");
}

#[test]
fn edit_guarded_other_is_named_too() {
    let registry = Registry::new();
    let (host, _log) = open_guarded(&registry, vec![office::DroppedPart::Other], EditKind::Docx, DocKind::Docx);
    assert!(matches!(host.state(), OfficeState::Pages { .. }), "{:?}", host.state());
    assert_eq!(host.guarded(), vec![office::DroppedPart::Other]);
}

#[test]
fn edit_guarded_to_a_viewer_open_is_a_protocol_stop() {
    // Only `OpenEdit` may be guarded: a viewer told `EditGuarded` broke the protocol.
    let fake = Fake::new(|link| {
        while let Some((id, _)) = link.next() {
            link.answer(id, Message::EditGuarded(office::EditGuarded { dropped: vec![office::DroppedPart::Ink] }));
        }
    });
    let registry = Registry::new();
    let mut host = open_mode(fake, &registry, TEST_TIMEOUTS, Mode::View, DocKind::Pptx, Some(Zeroizing::new(doc())), noop_wake());
    let mut events = Vec::new();
    assert!(until_open(&mut host, &mut events));
    assert!(matches!(host.state(), OfficeState::Stopped { why: Stop::Protocol, .. }), "{:?}", host.state());
    assert!(host.guarded().is_empty());
}
