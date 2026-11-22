//! Text that arrives as Unicode packets (w35-typing).
//!
//! A remote desktop (RustDesk, an RDP client's Unicode mode), an on-screen keyboard and `SendInput` with
//! `KEYEVENTF_UNICODE` do not press keys: Windows posts a key-down of `VK_PACKET` and, after `TranslateMessage`,
//! one `WM_CHAR` with the character. winit reports that press as an unidentified key with the character only in
//! the event's text, and Rune builds its keys from the logical key alone, so the character was lost: typing
//! through such a tool did nothing in every field. [`Packets`] is the rule that picks those characters out of
//! the window's messages, and nothing else: a character that follows a real key stays winit's, so no key types
//! twice. The Windows hook that feeds it is `windows_packet.rs`; `shell::host::poll` hands what it took to the
//! field that has the keyboard.
//!
//! The key after a dead key is the second case of the same loss (US-International, the usual Dutch layout): `'`
//! then Space is the Space key with the text `'`, and `'` then `t` is the T key with the text `'t`; Rune typed a
//! space and a `t`, so an apostrophe or a quote could not be typed. The characters of the key that follows a dead
//! key are taken here too, all of them (`é` as well), so that key types what Windows composed and nothing else.

#[cfg(any(windows, test))]
/// `VK_PACKET`: the virtual key of a Unicode packet.
pub const VK_PACKET: u16 = 0xE7;

#[cfg(any(windows, test))]
/// A keyboard message of the window, as far as the rule needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyMessage {
    /// `WM_KEYDOWN` / `WM_SYSKEYDOWN` with its virtual key.
    Down(u16),
    /// `WM_CHAR` with its UTF-16 code unit.
    Char(u16),
    /// `WM_DEADCHAR` / `WM_SYSDEADCHAR`: the key that went down is a dead key, waiting for the next one.
    Dead,
}

#[cfg(any(windows, test))]
/// What to do with the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not a packet's: winit gets it as it is.
    Pass,
    /// A packet's code unit, taken from winit; `Some`: the text it completed.
    Take(Option<String>),
}

#[cfg(any(windows, test))]
/// The messages of one window thread, in the order they are taken off the queue.
#[derive(Debug, Default)]
pub struct Packets {
    /// The last key that went down was a packet, or the key after a dead key: the characters that follow are
    /// taken.
    after_packet: bool,
    /// The key that went down is the one after a dead key (not a packet).
    after_dead: bool,
    /// A dead key went down and the key it waits for has not.
    dead: bool,
    /// The first half of a character outside the BMP (an emoji arrives as two packets).
    high: Option<u16>,
}

#[cfg(any(windows, test))]
impl Packets {
    pub fn on(&mut self, message: KeyMessage) -> Verdict {
        match message {
            KeyMessage::Dead => {
                self.dead = true;
                Verdict::Pass
            }
            // Shift between the dead key and its letter (`'` then Shift+E): the dead key still waits.
            KeyMessage::Down(vk) if self.dead && is_modifier(vk) => Verdict::Pass,
            KeyMessage::Down(vk) => {
                self.after_dead = self.dead && vk != VK_PACKET;
                self.after_packet = vk == VK_PACKET || self.dead;
                self.dead = false;
                if !self.after_packet {
                    self.high = None;
                }
                Verdict::Pass
            }
            KeyMessage::Char(_) if !self.after_packet => Verdict::Pass,
            // A control character after a dead key (Backspace, Enter, Escape) is that key's own: winit's. A
            // packet's is taken: no key comes with it (`key_of` makes it one).
            KeyMessage::Char(unit) if self.after_dead && (unit < 0x20 || unit == 0x7F) => {
                self.after_packet = false;
                Verdict::Pass
            }
            KeyMessage::Char(unit @ 0xD800..=0xDBFF) => {
                self.high = Some(unit);
                Verdict::Take(None)
            }
            KeyMessage::Char(unit @ 0xDC00..=0xDFFF) => {
                let text = self.high.take().and_then(|high| String::from_utf16(&[high, unit]).ok());
                Verdict::Take(text)
            }
            KeyMessage::Char(unit) => {
                self.high = None;
                Verdict::Take(char::from_u32(u32::from(unit)).map(String::from))
            }
        }
    }
}

#[cfg(any(windows, test))]
/// Shift, Ctrl, Alt, Caps Lock and the Windows keys (`VK_SHIFT`.., `VK_CAPITAL`, `VK_LWIN`/`VK_RWIN`, the
/// left/right variants).
fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// The key a packet's text is, as the keyboard would have sent it: Enter, Tab and Backspace are keys, every other
/// control character is nothing, the rest is text.
pub fn key_of(text: &str) -> Option<rune::Key> {
    match text {
        "\r" | "\n" => Some(rune::Key::Enter),
        "\t" => Some(rune::Key::Tab),
        "\u{8}" => Some(rune::Key::Backspace),
        _ if text.is_empty() || text.chars().any(char::is_control) => None,
        _ => Some(rune::Key::Text(text.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(messages: &[KeyMessage]) -> (Vec<String>, Vec<KeyMessage>) {
        let (mut packets, mut taken, mut passed) = (Packets::default(), Vec::new(), Vec::new());
        for message in messages {
            match packets.on(*message) {
                Verdict::Pass => passed.push(*message),
                Verdict::Take(text) => taken.extend(text),
            }
        }
        (taken, passed)
    }

    #[test]
    fn a_packets_character_is_taken_and_its_key_stays() {
        let (taken, passed) = run(&[KeyMessage::Down(VK_PACKET), KeyMessage::Char(0xE9)]);
        assert_eq!(taken, ["é"]);
        assert_eq!(passed, [KeyMessage::Down(VK_PACKET)], "winit never sees the character");
    }

    #[test]
    fn a_real_keys_character_is_winits_so_no_key_types_twice() {
        let typed = [KeyMessage::Down(0x41), KeyMessage::Char(u16::from(b'a'))];
        let (taken, passed) = run(&typed);
        assert!(taken.is_empty(), "taken: {taken:?}");
        assert_eq!(passed, typed);
        // Also straight after a packet: the next real key ends the packet.
        let (taken, passed) = run(&[
            KeyMessage::Down(VK_PACKET),
            KeyMessage::Char(u16::from(b'x')),
            KeyMessage::Down(0x41),
            KeyMessage::Char(u16::from(b'a')),
        ]);
        assert_eq!(taken, ["x"]);
        assert_eq!(passed.last(), Some(&KeyMessage::Char(u16::from(b'a'))));
    }

    #[test]
    fn a_character_with_no_key_before_it_is_not_a_packets() {
        // An input method's committed text arrives without a key-down: winit's.
        let (taken, passed) = run(&[KeyMessage::Char(0x3042)]);
        assert!(taken.is_empty());
        assert_eq!(passed.len(), 1);
    }

    #[test]
    fn an_emoji_arrives_as_two_packets_and_is_one_text() {
        let (taken, passed) = run(&[
            KeyMessage::Down(VK_PACKET),
            KeyMessage::Char(0xD83D),
            KeyMessage::Down(VK_PACKET),
            KeyMessage::Char(0xDE00),
        ]);
        assert_eq!(taken, ["😀"]);
        assert_eq!(passed, [KeyMessage::Down(VK_PACKET); 2]);
    }

    #[test]
    fn half_an_emoji_is_dropped_not_typed() {
        let (taken, _) = run(&[
            KeyMessage::Down(VK_PACKET),
            KeyMessage::Char(0xDE00),
            KeyMessage::Down(VK_PACKET),
            KeyMessage::Char(0xD83D),
            KeyMessage::Down(VK_PACKET),
            KeyMessage::Char(u16::from(b'a')),
        ]);
        assert_eq!(taken, ["a"]);
    }

    const QUOTE: u16 = 0xDE; // VK_OEM_7
    const SHIFT: u16 = 0x10;

    #[test]
    fn the_key_after_a_dead_key_types_what_windows_composed() {
        // ' then e: é.
        let (taken, passed) = run(&[KeyMessage::Down(QUOTE), KeyMessage::Dead, KeyMessage::Down(0x45), KeyMessage::Char(0xE9)]);
        assert_eq!(taken, ["é"]);
        assert!(!passed.contains(&KeyMessage::Char(0xE9)));
        // ' then Space: the apostrophe, not a space.
        let (taken, _) = run(&[KeyMessage::Down(QUOTE), KeyMessage::Dead, KeyMessage::Down(0x20), KeyMessage::Char(0x27)]);
        assert_eq!(taken, ["'"]);
        // ' then t: both characters.
        let (taken, _) = run(&[
            KeyMessage::Down(QUOTE),
            KeyMessage::Dead,
            KeyMessage::Down(0x54),
            KeyMessage::Char(0x27),
            KeyMessage::Char(u16::from(b't')),
        ]);
        assert_eq!(taken, ["'", "t"]);
        // ' then Shift+E: É; the Shift in between does not end the wait.
        let (taken, _) = run(&[
            KeyMessage::Down(QUOTE),
            KeyMessage::Dead,
            KeyMessage::Down(SHIFT),
            KeyMessage::Down(0x45),
            KeyMessage::Char(0xC9),
        ]);
        assert_eq!(taken, ["É"]);
    }

    #[test]
    fn the_key_after_that_is_winits_again_and_a_control_key_after_a_dead_key_stays_a_key() {
        let (taken, passed) = run(&[
            KeyMessage::Down(QUOTE),
            KeyMessage::Dead,
            KeyMessage::Down(0x45),
            KeyMessage::Char(0xE9),
            KeyMessage::Down(0x41),
            KeyMessage::Char(u16::from(b'a')),
        ]);
        assert_eq!(taken, ["é"]);
        assert_eq!(passed.last(), Some(&KeyMessage::Char(u16::from(b'a'))));
        // ' then Backspace: the Backspace key, its character left to winit.
        let (taken, passed) = run(&[KeyMessage::Down(QUOTE), KeyMessage::Dead, KeyMessage::Down(0x08), KeyMessage::Char(0x08)]);
        assert!(taken.is_empty(), "taken: {taken:?}");
        assert_eq!(passed.last(), Some(&KeyMessage::Char(0x08)));
    }

    #[test]
    fn a_packets_control_character_is_taken() {
        let (taken, _) = run(&[KeyMessage::Down(VK_PACKET), KeyMessage::Char(0x0D)]);
        assert_eq!(taken, ["\r"]);
    }

    #[test]
    fn enter_tab_and_backspace_are_keys_and_other_control_characters_nothing() {
        assert_eq!(key_of("\r"), Some(rune::Key::Enter));
        assert_eq!(key_of("\t"), Some(rune::Key::Tab));
        assert_eq!(key_of("\u{8}"), Some(rune::Key::Backspace));
        assert_eq!(key_of("\u{1b}"), None);
        assert_eq!(key_of("€"), Some(rune::Key::Text("€".into())));
    }
}
