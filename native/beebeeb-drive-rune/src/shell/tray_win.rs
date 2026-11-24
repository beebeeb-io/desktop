//! The pure half of the Windows notification-area icon: everything `platform/windows_tray.rs`
//! decides without calling Windows, so it is tested on every platform.

use super::tray::{MenuAction, MenuEntry};

/// `NIN_SELECT` and `NIN_KEYSELECT` (shellapi.h): a click, and Enter or Space, on the icon.
pub const NIN_SELECT: u32 = 0x0400;
pub const NIN_KEYSELECT: u32 = 0x0401;
/// `WM_CONTEXTMENU`: a right-click, or Shift+F10 on the focused icon.
pub const WM_CONTEXTMENU: u32 = 0x007B;
/// Enter sends `NIN_KEYSELECT` twice; a second one within this many milliseconds is the same
/// keypress.
pub const KEY_REPEAT_MS: u128 = 250;
/// `NOTIFYICONDATAW::szTip` holds 128 UTF-16 units, the last one the terminator.
pub const TIP_UNITS: usize = 128;

/// What the icon reported, decoded from a `NOTIFYICON_VERSION_4` callback message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Callback {
    /// `NIN_SELECT`, `NIN_KEYSELECT`, `WM_CONTEXTMENU`, or a mouse message.
    pub event: u32,
    /// The anchor point in screen pixels; negative on a display left of or above the primary.
    pub x: i32,
    pub y: i32,
}

/// Version 4: the event is the LOW word of `lParam` (its high word is the icon's id), and the
/// anchor is in `wParam`: x in the low word, y in the next, each a SIGNED 16-bit value.
pub fn decode_callback(wparam: usize, lparam: isize) -> Callback {
    Callback {
        event: (lparam as u32) & 0xFFFF,
        x: i32::from((wparam & 0xFFFF) as u16 as i16),
        y: i32::from(((wparam >> 16) & 0xFFFF) as u16 as i16),
    }
}

/// Whether a `NIN_KEYSELECT` is the second half of one Enter press.
pub fn is_key_repeat(since_last_ms: Option<u128>) -> bool {
    since_last_ms.is_some_and(|ms| ms < KEY_REPEAT_MS)
}

/// The tooltip as `szTip` takes it: at most 127 units and never half of a surrogate pair. The
/// caller writes it into a zeroed buffer, which terminates it.
pub fn tip_units(tooltip: &str) -> Vec<u16> {
    let mut units: Vec<u16> = Vec::with_capacity(TIP_UNITS);
    for c in tooltip.chars() {
        let mut pair = [0u16; 2];
        let encoded = c.encode_utf16(&mut pair);
        if units.len() + encoded.len() > TIP_UNITS - 1 {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units
}

/// The size in bytes of a 1-bit AND mask: one bit per pixel, each row padded to 16 bits.
pub fn and_mask_len(width: u32, height: u32) -> usize {
    (width.div_ceil(16) * 2 * height) as usize
}

/// The command id of the menu entry at `index` (`TrackPopupMenu` returns 0 for "nothing").
pub fn menu_command(index: usize) -> usize {
    index + 1
}

/// What `TrackPopupMenu` returned, as an action: only an ENABLED item of this menu. Zero
/// (dismissed), a separator, a greyed item and an id outside the menu are nothing.
pub fn menu_pick(menu: &[MenuEntry], command: u32) -> Option<MenuAction> {
    match menu.get((command as usize).checked_sub(1)?) {
        Some(MenuEntry::Item {
            action, enabled: true, ..
        }) => Some(*action),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tray::{TrayInputs, tray_model};
    use super::*;

    #[test]
    fn the_v4_callback_decodes_the_event_and_a_signed_anchor() {
        // A click at (1700, 1050) on the primary; the high word of lParam is the icon id.
        let w = (1050usize << 16) | 1700;
        let l = ((1isize) << 16) | NIN_SELECT as isize;
        assert_eq!(
            decode_callback(w, l),
            Callback {
                event: NIN_SELECT,
                x: 1700,
                y: 1050
            }
        );
        // A display to the left of and above the primary: both coordinates negative.
        let (x, y) = (-1234i16, -20i16);
        let w = ((y as u16 as usize) << 16) | (x as u16 as usize);
        let l = ((7isize) << 16) | WM_CONTEXTMENU as isize;
        assert_eq!(
            decode_callback(w, l),
            Callback {
                event: WM_CONTEXTMENU,
                x: -1234,
                y: -20
            }
        );
        // The icon id in the high word never leaks into the event.
        assert_eq!(
            decode_callback(0, (0xFFFFisize << 16) | NIN_KEYSELECT as isize).event,
            NIN_KEYSELECT
        );
        // On 64-bit, bits above 32 of wParam are not coordinates.
        #[cfg(target_pointer_width = "64")]
        assert_eq!(decode_callback((1usize << 40) | 5, 0).x, 5);
    }

    #[test]
    fn enter_on_the_b_is_one_toggle() {
        assert!(!is_key_repeat(None));
        assert!(is_key_repeat(Some(0)));
        assert!(is_key_repeat(Some(249)));
        assert!(!is_key_repeat(Some(250)));
        assert!(!is_key_repeat(Some(5_000)));
    }

    #[test]
    fn the_tooltip_is_cut_at_127_units_and_never_inside_a_pair() {
        assert_eq!(
            tip_units("Beebeeb: paused"),
            "Beebeeb: paused".encode_utf16().collect::<Vec<_>>()
        );
        assert_eq!(tip_units(&"a".repeat(127)).len(), 127);
        assert_eq!(tip_units(&"a".repeat(128)).len(), 127);
        assert_eq!(tip_units(&"a".repeat(500)).len(), 127);
        // 126 units, then a character that needs two: it does not fit, and is not halved.
        let text = format!("{}\u{1F41D}", "a".repeat(126));
        let units = tip_units(&text);
        assert_eq!(units.len(), 126);
        assert!(String::from_utf16(&units).is_ok());
        // 125 units and the pair: exactly 127.
        let text = format!("{}\u{1F41D}b", "a".repeat(125));
        let units = tip_units(&text);
        assert_eq!(units.len(), 127);
        assert!(String::from_utf16(&units).is_ok());
        assert!(tip_units("").is_empty());
    }

    #[test]
    fn the_and_mask_rows_are_padded_to_sixteen_bits() {
        assert_eq!(and_mask_len(16, 16), 32);
        assert_eq!(and_mask_len(20, 20), 80);
        assert_eq!(and_mask_len(24, 24), 96);
        assert_eq!(and_mask_len(32, 32), 128);
        assert_eq!(and_mask_len(40, 40), 240);
        assert_eq!(and_mask_len(1, 3), 6);
    }

    #[test]
    fn a_menu_pick_is_an_enabled_item_of_this_menu_and_nothing_else() {
        let signed_out = tray_model(&TrayInputs::default()).menu;
        // Open Beebeeb, Open in File Explorer (greyed), separator, Pause (greyed), Settings
        // (greyed), separator, Quit.
        assert_eq!(signed_out.len(), 7);
        assert_eq!(menu_pick(&signed_out, 0), None, "dismissed");
        assert_eq!(
            menu_pick(&signed_out, menu_command(0) as u32),
            Some(MenuAction::OpenBeebeeb)
        );
        assert_eq!(menu_pick(&signed_out, menu_command(1) as u32), None, "greyed");
        assert_eq!(menu_pick(&signed_out, menu_command(2) as u32), None, "a separator");
        assert_eq!(menu_pick(&signed_out, menu_command(3) as u32), None, "greyed");
        assert_eq!(menu_pick(&signed_out, menu_command(4) as u32), None, "greyed");
        assert_eq!(menu_pick(&signed_out, menu_command(6) as u32), Some(MenuAction::Quit));
        assert_eq!(menu_pick(&signed_out, 8), None, "past the end");
        assert_eq!(menu_pick(&signed_out, u32::MAX), None);
        let ready = tray_model(&TrayInputs {
            signed_in: true,
            explorer_ready: true,
            ..TrayInputs::default()
        })
        .menu;
        let picks: Vec<_> = (0..=8).map(|command| menu_pick(&ready, command)).collect();
        assert_eq!(
            picks,
            vec![
                None,
                Some(MenuAction::OpenBeebeeb),
                Some(MenuAction::OpenInFileExplorer),
                None,
                Some(MenuAction::PauseSyncing),
                Some(MenuAction::Settings),
                None,
                Some(MenuAction::Quit),
                None
            ]
        );
    }
}
