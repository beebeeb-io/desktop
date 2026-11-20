//! w35-typing: text typed as Unicode packets (a remote desktop, `SendInput`'s Unicode mode) reaches the field that
//! has the keyboard. Rune drops those characters (winit gives them as text on an unidentified key); the platform
//! takes them and the poll types them.

use super::*;

/// What a remote desktop typed since the last poll, as the platform hands it over.
fn packets(platform: &FakePlatform, text: &[&str]) {
    platform.packet_text.lock().unwrap().extend(text.iter().map(|t| t.to_string()));
}

#[test]
fn text_typed_as_unicode_packets_reaches_the_focused_field() {
    let (_, platform, mut rt) = fixture();
    press(&mut rt, Key::Command('f'), ctrl());
    assert_eq!(rt.state.search.text, "");
    packets(&platform, &["h", "é", "😀"]);
    settle(&mut rt);
    assert_eq!(rt.state.search.text, "hé😀", "the packets' text is typed into Search names, in order, once");
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
