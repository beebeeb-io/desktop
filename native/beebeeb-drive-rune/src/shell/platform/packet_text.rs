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

/// `VK_PACKET`: the virtual key of a Unicode packet.
pub const VK_PACKET: u16 = 0xE7;

/// A keyboard message of the window, as far as the rule needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyMessage {
    /// `WM_KEYDOWN` / `WM_SYSKEYDOWN` with its virtual key.
    Down(u16),
    /// `WM_CHAR` with its UTF-16 code unit.
    Char(u16),
}

/// What to do with the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not a packet's: winit gets it as it is.
    Pass,
    /// A packet's code unit, taken from winit; `Some`: the text it completed.
    Take(Option<String>),
}

/// The messages of one window thread, in the order they are taken off the queue.
#[derive(Debug, Default)]
pub struct Packets {
    /// The last key that went down was a packet: the characters that follow are its.
    after_packet: bool,
    /// The first half of a character outside the BMP (an emoji arrives as two packets).
    high: Option<u16>,
}

impl Packets {
    pub fn on(&mut self, message: KeyMessage) -> Verdict {
        match message {
            KeyMessage::Down(vk) => {
                self.after_packet = vk == VK_PACKET;
                if !self.after_packet {
                    self.high = None;
                }
                Verdict::Pass
            }
            KeyMessage::Char(_) if !self.after_packet => Verdict::Pass,
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

    #[test]
    fn enter_tab_and_backspace_are_keys_and_other_control_characters_nothing() {
        assert_eq!(key_of("\r"), Some(rune::Key::Enter));
        assert_eq!(key_of("\t"), Some(rune::Key::Tab));
        assert_eq!(key_of("\u{8}"), Some(rune::Key::Backspace));
        assert_eq!(key_of("\u{1b}"), None);
        assert_eq!(key_of("€"), Some(rune::Key::Text("€".into())));
    }
}
