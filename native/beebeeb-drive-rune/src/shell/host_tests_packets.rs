//! w35-typing: text typed as Unicode packets (a remote desktop, `SendInput`'s Unicode mode) reaches the field that
//! has the keyboard. Rune drops those characters (winit gives them as text on an unidentified key); the platform
//! takes them and the poll types them.

use super::*;

/// What a remote desktop typed since the last poll, as the platform hands it over.
fn packets(platform: &FakePlatform, text: &[&str]) {
    platform
        .packet_text
        .lock()
        .unwrap()
        .extend(text.iter().map(|t| t.to_string()));
}

#[test]
fn text_typed_as_unicode_packets_reaches_the_focused_field() {
    let (_, platform, mut rt) = fixture();
    press(&mut rt, Key::Command('f'), ctrl());
    assert_eq!(rt.state.search.text, "");
    packets(&platform, &["h", "é", "😀"]);
    settle(&mut rt);
    assert_eq!(
        rt.state.search.text, "hé😀",
        "the packets' text is typed into Search names, in order, once"
    );
    // Nothing is typed again by the polls that follow.
    settle(&mut rt);
    assert_eq!(rt.state.search.text, "hé😀");
}

#[test]
fn a_packets_backspace_is_the_key_and_a_control_character_types_nothing() {
    let (_, platform, mut rt) = fixture();
    press(&mut rt, Key::Command('f'), ctrl());
    packets(&platform, &["a", "b", "\u{8}", "\u{1b}", "c"]);
    settle(&mut rt);
    assert_eq!(rt.state.search.text, "ac");
}

/// Where the owner met it: a text file in edit mode. The packets' text lands in the editor and marks it unsaved.
#[test]
fn text_typed_as_unicode_packets_reaches_the_text_editor() {
    use crate::frames::viewers::source::{FixtureViewerSource, board};
    let (_, platform, mut rt) = fixture();
    rt.update(|s, _| {
        assert!(crate::frames::viewers::open_from(
            s,
            Box::new(FixtureViewerSource::new()),
            board::NOTES
        ));
    });
    settle(&mut rt);
    press(&mut rt, Key::Command('e'), ctrl());
    let text = |rt: &Runtime<State>| {
        let ui = rt.state.viewer.ui.as_ref().expect("the note is open");
        ui.editor.as_ref().expect("in edit mode").edit.text.clone()
    };
    let before = text(&rt);
    packets(&platform, &["h", "o", "i", " ", "'", "é"]);
    settle(&mut rt);
    assert_eq!(
        text(&rt),
        format!("{before}hoi 'é"),
        "typed at the caret, at the end of the note"
    );
    assert!(rt.state.viewer.ui.as_ref().is_some_and(|ui| ui.dirty()));
}

/// The note open in edit mode, and its text.
fn editing() -> (std::sync::Arc<FakePlatform>, Runtime<State>) {
    use crate::frames::viewers::source::{FixtureViewerSource, board};
    let (_, platform, mut rt) = fixture();
    rt.update(|s, _| {
        assert!(crate::frames::viewers::open_from(
            s,
            Box::new(FixtureViewerSource::new()),
            board::NOTES
        ));
    });
    settle(&mut rt);
    press(&mut rt, Key::Command('e'), ctrl());
    (platform, rt)
}

fn edited(rt: &Runtime<State>) -> String {
    let ui = rt.state.viewer.ui.as_ref().expect("the note is open");
    ui.editor.as_ref().expect("in edit mode").edit.text.clone()
}

/// w36-txt-save-enter: Enter, pressed as the key (a keyboard; RustDesk sends Enter as `VK_RETURN`), starts a new
/// line in the text editor.
#[test]
fn enter_as_a_key_starts_a_new_line_in_the_text_editor() {
    let (_, mut rt) = editing();
    let before = edited(&rt);
    press(&mut rt, Key::Text("a".into()), Mods::default());
    press(&mut rt, Key::Enter, Mods::default());
    press(&mut rt, Key::Text("b".into()), Mods::default());
    settle(&mut rt);
    assert_eq!(edited(&rt), format!("{before}a\nb"));
}

/// The same from Unicode packets (`SendInput`'s Unicode mode types a line break as a packet of `\r`).
#[test]
fn enter_as_a_unicode_packet_starts_a_new_line_in_the_text_editor() {
    let (platform, mut rt) = editing();
    let before = edited(&rt);
    packets(&platform, &["a", "\r", "b", "\n", "c"]);
    settle(&mut rt);
    assert_eq!(edited(&rt), format!("{before}a\nb\nc"));
}
