//! The guards a review asked for: default-deny frames, hostile inbox content, secure containers,
//! caps, and the element a note was opened on. Same harness as `tests.rs`.

use std::path::Path;

use rune::{Cx, El, Env, Key, Mods, Runtime, col, text};
use serde_json::Value;

use super::model::{self, BLANK_RGBA, NOTE_SCHEMA, Rgba, StatusState};
use super::tests::*;
use super::{Mode, capture};
use crate::State;

fn focused_key(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

#[test]
fn comment_mode_is_denied_unless_the_frame_opted_in() {
    assert_eq!(
        super::FrameInfo::unknown().feedback,
        super::Policy::Denied("this screen has not opted in"),
        "a frame nobody registered is denied"
    );
    let inbox = temp_inbox("denied");
    let mut rt = runtime(&inbox);
    rt.state.feedback.frame_override = Some(super::FrameInfo::denied("DW-0101", "sign-in", "it shows your password"));
    plain(&mut rt, Key::Function(8));
    assert_eq!(rt.state.feedback.mode, Mode::Off);
    assert!(!has_key(&rt, "fb-overlay"));
    let shown = shown_text(&rt);
    assert!(
        shown.contains(super::DENIED_LINE) && shown.contains("it shows your password"),
        "the refusal is visible: {shown}"
    );
    // The notice goes with the next key, which still reaches the app.
    let selected = rt.state.selected;
    plain(&mut rt, Key::Down);
    assert!(!shown_text(&rt).contains(super::DENIED_LINE));
    assert_ne!(rt.state.selected, selected);

    // Allowed again: it starts. A frame that turns denied under an open comment mode ends it,
    // and a capture already asked for is refused.
    rt.state.feedback.frame_override = None;
    plain(&mut rt, Key::Function(8));
    assert_eq!(rt.state.feedback.mode, Mode::Picking);
    let at = hover(&mut rt, "row-2-name");
    click(&mut rt, at);
    plain(&mut rt, Key::Text("typed on an allowed frame".into()));
    rt.state.feedback.frame_override = Some(super::FrameInfo::denied(
        "DW-0102",
        "recovery",
        "it shows your recovery phrase",
    ));
    plain(&mut rt, Key::Enter);
    assert_eq!(rt.state.feedback.mode, Mode::Off);
    assert!(model::scan(&inbox, &[]).is_empty(), "nothing was written");
    assert!(shown_text(&rt).contains("recovery phrase"));
}

#[test]
fn the_note_names_the_frame_the_app_reports() {
    let inbox = temp_inbox("frame");
    let mut rt = runtime(&inbox);
    rt.state.feedback.frame_override = Some(super::FrameInfo::allowed("DW-0777", "shared/with-me"));
    let dir = save_note_on_row(&mut rt, "row-4", "which frame");
    let note: Value = serde_json::from_slice(&std::fs::read(dir.join("note.json")).unwrap()).unwrap();
    assert_eq!(note["frame"]["id"], "DW-0777");
    assert_eq!(note["frame"]["route"], "shared/with-me");
}

#[test]
fn a_note_is_refused_when_its_row_became_another_item_and_the_keyboard_returns_to_the_field() {
    let inbox = temp_inbox("moved");
    let mut rt = runtime(&inbox);
    plain(&mut rt, Key::Function(8));
    let at = hover(&mut rt, "row-6-name");
    while rt.state.feedback.target().unwrap().key.as_deref() != Some("row-6") {
        key(
            &mut rt,
            Key::Up,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
    }
    click(&mut rt, at);
    plain(&mut rt, Key::Text("about Contract draft".into()));
    // The listing changes under the popover: row-6 is now another file.
    rt.state.listing.items[6].name = "Somebody else.pdf".into();
    plain(&mut rt, Key::Enter);
    step(&mut rt);
    assert_eq!(rt.state.feedback.mode, Mode::Writing, "not saved, the popover is back");
    assert!(
        rt.state
            .feedback
            .message
            .as_deref()
            .unwrap()
            .contains("changed while you were writing")
    );
    assert!(model::scan(&inbox, &[]).is_empty());
    assert_eq!(rt.state.feedback.note.text, "about Contract draft", "the text is kept");
    assert_eq!(
        focused_key(&rt).as_deref(),
        Some(super::ui::NOTE_INPUT),
        "the keyboard is back in the field"
    );
    plain(&mut rt, Key::Text("!".into()));
    assert_eq!(rt.state.feedback.note.text, "about Contract draft!");
}

#[test]
fn esc_in_the_popover_goes_back_one_step_not_two() {
    let inbox = temp_inbox("esc");
    let mut rt = runtime(&inbox);
    plain(&mut rt, Key::Function(8));
    let at = hover(&mut rt, "row-1-name");
    click(&mut rt, at);
    assert_eq!(focused_key(&rt).as_deref(), Some(super::ui::NOTE_INPUT));
    plain(&mut rt, Key::Escape);
    assert_eq!(
        rt.state.feedback.mode,
        Mode::Picking,
        "Esc with the field focused closes the popover only"
    );
    click(&mut rt, at);
    let tag = rt.find("fb-tag-1").unwrap();
    click(&mut rt, tag);
    plain(&mut rt, Key::Escape);
    assert_eq!(
        rt.state.feedback.mode,
        Mode::Picking,
        "Esc with a tag focused closes the popover only"
    );
    plain(&mut rt, Key::Escape);
    assert_eq!(rt.state.feedback.mode, Mode::Off);
}

struct Phrase {
    marked: bool,
}

fn phrase_view(s: &Phrase, _: &mut Cx<Phrase>) -> El {
    let words = col()
        .key(if s.marked { "secure-phrase" } else { "phrase" })
        .padding(8.0)
        .gap(4.0)
        .child(text("alpha-word").key("w1").color(rune::Color::BLACK))
        .child(text("bravo-word").key("w2").color(rune::Color::BLACK));
    col()
        .key("window")
        .w_full()
        .h_full()
        .fill(rune::Color::WHITE)
        .padding(20.0)
        .gap(12.0)
        .child(text("Your recovery phrase").key("title"))
        .child(words)
}

#[test]
fn a_secure_container_redacts_its_whole_subtree_and_blanks_its_whole_box() {
    let inbox = temp_inbox("container");
    for marked in [false, true] {
        let mut rt = Runtime::with_text(
            Phrase { marked },
            phrase_view,
            (400.0, 200.0),
            Env {
                system_dark: false,
                scale: SCALE,
            },
            text_system(),
        );
        rt.frame(0.0);
        let (tree, rigid) = capture::snapshot(&rt);
        let group = tree
            .iter()
            .position(|e| e.key.as_deref() == Some(if marked { "secure-phrase" } else { "phrase" }))
            .unwrap();
        let mut img = capture::render_headless(&mut rt, SCALE);
        let area = img.px_box(tree[group].visible, SCALE);
        let ink = |img: &Rgba| img.crop(area).px.chunks_exact(4).filter(|p| p[0] < 100).count();
        assert!(ink(&img) > 50, "the words are drawn before redaction");
        let blanked = model::redact_pixels(&mut img, &tree, SCALE, rigid).unwrap();
        let root = tree.iter().position(|e| e.key.as_deref() == Some("window")).unwrap();
        let body = serde_json::to_string(&model::tree_json(&tree, root)).unwrap();
        if marked {
            assert_eq!(blanked, 3, "the group and both words");
            assert!(tree.iter().filter(|e| e.secure).all(|e| e.text.is_none()));
            assert!(
                !body.contains("alpha-word") && !body.contains("bravo-word"),
                "no word of the phrase is in the tree"
            );
            assert!(img.is_filled(area, BLANK_RGBA), "the whole box is flat");
            let meta = model::NoteMeta {
                comment: "c".into(),
                tags: vec![],
                scale: SCALE,
                dark: false,
                window_size: [400.0, 200.0],
                frame: serde_json::json!({}),
            };
            let id = model::write_note(&inbox, &meta, &tree, root, &img).unwrap();
            for file in ["note.json", "tree.json"] {
                assert!(
                    !std::fs::read_to_string(inbox.join(&id).join(file))
                        .unwrap()
                        .contains("-word")
                );
            }
        } else {
            // Without the marker the same view leaks: the marker is what protects.
            assert_eq!(blanked, 0);
            assert!(body.contains("alpha-word") && ink(&img) > 50);
        }
    }
}

fn snap(key: &str) -> model::ElementSnap {
    model::ElementSnap {
        path: key.into(),
        key: Some(key.into()),
        role: "group".into(),
        label: None,
        text: None,
        rect: [0.0, 0.0, 8.0, 8.0],
        visible: [0.0, 0.0, 8.0, 8.0],
        fill: [1.0; 4],
        text_color: None,
        radius: 0.0,
        opacity: 1.0,
        secure: false,
        focused: false,
        moving: false,
        exiting: false,
        parent: None,
    }
}

#[test]
fn a_capture_is_refused_while_anything_moves_or_leaves() {
    let rest = vec![snap("a"), snap("b")];
    assert_eq!(model::still(&rest), Ok(()));
    for (moving, exiting) in [(true, false), (false, true)] {
        let mut tree = rest.clone();
        tree[1].moving = moving;
        tree[1].exiting = exiting;
        assert!(model::still(&tree).unwrap_err().contains("still moving"));
    }
}

#[test]
fn only_the_two_known_surface_formats_are_captured() {
    use wgpu::TextureFormat as F;
    assert_eq!(capture::blue_first(F::Rgba8UnormSrgb), Ok(false));
    assert_eq!(capture::blue_first(F::Rgba8Unorm), Ok(false));
    assert_eq!(capture::blue_first(F::Bgra8UnormSrgb), Ok(true));
    assert_eq!(capture::blue_first(F::Bgra8Unorm), Ok(true));
    for other in [F::Rgba16Float, F::Rgb10a2Unorm, F::R8Unorm] {
        assert!(
            capture::blue_first(other).is_err(),
            "{other:?} is refused, not written as garbage"
        );
    }
}

fn disk_note(inbox: &Path, comment: &str) -> String {
    let tree = vec![snap("window")];
    let img = Rgba {
        w: 16,
        h: 16,
        px: vec![200; 16 * 16 * 4],
    };
    let meta = model::NoteMeta {
        comment: comment.into(),
        tags: vec![],
        scale: 1.0,
        dark: false,
        window_size: [16.0, 16.0],
        frame: serde_json::json!({"id": "t"}),
    };
    model::write_note(inbox, &meta, &tree, 0, &img).unwrap()
}

#[test]
fn the_scan_trusts_nothing_in_the_folder_and_rereads_only_what_changed() {
    let inbox = temp_inbox("scan");
    let a = disk_note(&inbox, "first");
    let b = disk_note(&inbox, "second \u{202e}evil\u{0007}");
    let first = model::scan(&inbox, &[]);
    assert_eq!(first.len(), 2);
    assert!(
        first.iter().any(|n| n.comment == "second evil"),
        "bidi and control characters never reach the panel"
    );

    // A copied folder (same note.json, another name), an id that is not an id, a symlink, an
    // oversized note.json, a half-written folder: none becomes a row.
    let copy = inbox.join(format!("{a}-copy"));
    std::fs::create_dir(&copy).unwrap();
    std::fs::copy(inbox.join(&a).join("note.json"), copy.join("note.json")).unwrap();
    let odd = inbox.join("has space");
    std::fs::create_dir(&odd).unwrap();
    std::fs::copy(inbox.join(&a).join("note.json"), odd.join("note.json")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(inbox.join(&a), inbox.join("linked")).unwrap();
    let big = inbox.join("20990101T000000Z-bigbig");
    std::fs::create_dir(&big).unwrap();
    std::fs::write(
        big.join("note.json"),
        format!(
            r#"{{"schema":"{NOTE_SCHEMA}","id":"20990101T000000Z-bigbig","comment":"{}"}}"#,
            "x".repeat(300_000)
        ),
    )
    .unwrap();
    std::fs::create_dir(inbox.join(".tmp-half")).unwrap();
    let again = model::scan(&inbox, &first);
    assert_eq!(
        again.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
        first.iter().map(|n| n.id.as_str()).collect::<Vec<_>>()
    );

    // Unchanged files are not read again: an entry of the earlier scan is taken as it is, so a
    // marker planted in it survives; a note whose status.json appears is read afresh.
    let mut marked = first.clone();
    for n in &mut marked {
        n.comment = format!("cached:{}", n.comment);
    }
    std::fs::write(inbox.join(&b).join("status.json"), r#"{"state":"seen"}"#).unwrap();
    let third = model::scan(&inbox, &marked);
    let (na, nb) = (
        third.iter().find(|n| n.id == a).unwrap(),
        third.iter().find(|n| n.id == b).unwrap(),
    );
    assert_eq!(na.comment, "cached:first", "the unchanged note was not re-read");
    assert_eq!(
        (nb.comment.as_str(), nb.status.state),
        ("second evil", StatusState::Seen),
        "the changed one was"
    );

    // A picture that is not a picture, or claims absurd dimensions, is refused before decoding.
    std::fs::write(inbox.join(&a).join("window.png"), b"not a png").unwrap();
    assert!(Rgba::read_png(&inbox.join(&a).join("window.png")).is_err());
    let mut huge = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
    ];
    huge.extend_from_slice(&[0, 1, 0, 0, 0, 1, 0, 0, 8, 6, 0, 0, 0, 0, 0, 0, 0]);
    std::fs::write(inbox.join(&b).join("window.png"), &huge).unwrap();
    assert!(Rgba::read_png(&inbox.join(&b).join("window.png")).is_err());
}

#[test]
fn a_full_inbox_drops_only_the_oldest_fixed_notes_or_refuses() {
    let inbox = temp_inbox("cap");
    let ids: Vec<String> = (0..3).map(|i| disk_note(&inbox, &format!("note {i}"))).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    let notes = model::scan(&inbox, &[]);
    assert_eq!(notes.len(), 3);
    assert_eq!(
        model::make_room(&notes, 4, u64::MAX),
        Ok(vec![]),
        "under the cap nothing is touched"
    );
    let err = model::make_room(&notes, 3, u64::MAX).unwrap_err();
    assert!(err.contains("inbox is full (3 notes"), "{err}");
    assert_eq!(
        model::scan(&inbox, &[]).len(),
        3,
        "with no fixed note, nothing is deleted"
    );

    for id in &sorted[..2] {
        std::fs::write(
            inbox.join(id).join("status.json"),
            r#"{"state":"fixed","fixed_in_commit":"0123456"}"#,
        )
        .unwrap();
    }
    let notes = model::scan(&inbox, &notes);
    assert_eq!(
        model::make_room(&notes, 3, u64::MAX),
        Ok(vec![sorted[0].clone()]),
        "the oldest fixed note makes room"
    );
    let left = model::scan(&inbox, &[]);
    assert_eq!(left.len(), 2);
    assert!(left.iter().all(|n| n.id != sorted[0]));
    // The byte cap works the same way, and a cap that cannot be met deletes nothing at all.
    assert!(model::make_room(&left, 100, 1).is_err());
    assert_eq!(model::scan(&inbox, &[]).len(), 2);
}
