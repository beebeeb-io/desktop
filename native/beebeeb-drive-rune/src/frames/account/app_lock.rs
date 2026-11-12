//! App lock's delay (DW-0716, DW-0716b): the menu the pop-up button of Sign-in and security's
//! Lock Beebeeb row drops, and its keyboard. The row itself is `security::pane`'s.
//!
//! Nothing here is a secret: the menu holds four lengths of time. Turning the lock off or
//! changing the delay asks for Windows Hello first; that question is the source's
//! (`AccountSource::set_app_lock`, `set_lock_after`), asked by the engine, never drawn here.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::settings_controls as controls;
use rune::{Align, El, Key, KeyEvent, Side};

use super::{AccountUi, Dialog};

/// How soon Beebeeb locks again, as the menu says it (the iPhone's choices, DW-0716).
pub const DELAYS: [&str; 4] = ["Immediately", "5 min", "30 min", "60 min"];

/// The key of the point in the Lock Beebeeb row where the menu's top-left corner goes: the
/// menu follows its row when the pane scrolls or the window is another size.
pub const ANCHOR: &str = "item-0-0-menu-at";

/// The index of `lock_after` in the menu; the first when the source says something else.
pub fn chosen(lock_after: &str) -> usize {
    DELAYS.iter().position(|d| *d == lock_after).unwrap_or(0)
}

/// The menu just opened (`go`): it opens on the current choice. With the lock off, or with
/// nothing answering for the account, there is no delay to choose and nothing opens.
pub fn arrive(ui: &mut AccountUi) {
    if !matches!(ui.dialog, Some(Dialog::LockDelay(_))) {
        return;
    }
    let sec = ui.source.as_deref().map(|s| s.security());
    ui.dialog = match sec {
        Some(sec) if sec.app_lock => Some(Dialog::LockDelay(chosen(&sec.lock_after))),
        _ => None,
    };
}

/// Take delay `index`, and close the menu. An index the menu does not have chooses nothing.
pub fn choose(s: &mut State, index: usize) {
    if let (Some(delay), Some(source)) = (DELAYS.get(index), s.account.source.as_mut()) {
        source.set_lock_after(delay);
    }
    s.account.dialog = None;
}

/// The open menu: the delays, the current one checked, `hot` under the pointer or the keys.
pub fn menu(lock_after: &str, hot: usize, t: &Theme) -> El {
    let now = chosen(lock_after);
    let items = DELAYS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            controls::menu_item(0, i, label, i == now, i == hot, t).on_press(move |s: &mut State| choose(s, i))
        })
        .collect();
    controls::menu(0, "Locks again after", 130.0, items, t)
        .anchored(ANCHOR, Side::Bottom, Align::Start, 0.0)
        .modal(true, |s: &mut State| s.account.dialog = None)
}

/// The menu's keys: the arrows, Home and End move, Enter takes, Esc closes, Tab closes and
/// moves on. `None`: not a key of the menu's.
pub fn on_key(s: &mut State, hot: usize, ev: &KeyEvent) -> Option<bool> {
    let last = DELAYS.len() - 1;
    let hot = match ev.key {
        Key::Down => (hot + 1).min(last),
        Key::Up => hot.saturating_sub(1),
        Key::Home => 0,
        Key::End => last,
        Key::Enter => {
            choose(s, hot);
            return Some(true);
        }
        Key::Escape => {
            s.account.dialog = None;
            return Some(true);
        }
        Key::Tab => {
            s.account.dialog = None;
            return Some(false);
        }
        _ => return None,
    };
    s.account.dialog = Some(Dialog::LockDelay(hot));
    Some(true)
}
