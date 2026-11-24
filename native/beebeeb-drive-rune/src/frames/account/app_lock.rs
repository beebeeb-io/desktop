//! App lock's delay (DW-0716, DW-0716b): the menu the pop-up button of Sign-in and security's
//! Lock Beebeeb row drops, and its keyboard. The row itself is `security_overview`'s.
//!
//! Nothing here is a secret: the menu holds four lengths of time. Turning the lock off or
//! choosing a longer delay asks for Windows Hello first; that question is the source's
//! (`AccountSource::set_app_lock`, `set_lock_after`), asked by the engine, never drawn here.
//! When Windows Hello does not confirm it, the row offers the account password instead
//! (`app_lock_password`, no board frame: D-2).

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::menu::{self, Entry};
use rune::{Align, El, Key, KeyEvent, Side, Tx};

use super::{AccountUi, Dialog};

/// How soon Beebeeb locks again, as the menu says it (the iPhone's choices, DW-0716).
pub const DELAYS: [&str; 4] = ["Immediately", "5 min", "30 min", "60 min"];

/// The key of the point in the Lock Beebeeb row where the menu's top-left corner goes: the
/// menu follows its row when the pane scrolls or the window is another size.
pub const ANCHOR: &str = "item-0-0-menu-at";

/// The index of `lock_after` in the menu. `None`: the stored delay is not one of the menu's
/// (another build's value, another unit): no row is checked then, so the menu never claims a
/// delay, least of all the strictest one, that the pop-up button beside it contradicts.
pub fn chosen(lock_after: &str) -> Option<usize> {
    DELAYS.iter().position(|d| *d == lock_after)
}

/// The menu just opened (`go`): it opens on the current choice. With the lock off, or with
/// nothing answering for the account, there is no delay to choose and nothing opens.
pub fn arrive(ui: &mut AccountUi) {
    if !matches!(ui.dialog, Some(Dialog::LockDelay(_))) {
        return;
    }
    // From what the page itself draws: a source that cannot set the lock has no row for the menu to hang on.
    let lock = ui.source.as_deref().and_then(|s| s.security_facts().app_lock);
    ui.dialog = match lock {
        // No delay (a source that sets none): no menu to open.
        Some((true, after)) if !after.is_empty() => Some(Dialog::LockDelay(chosen(&after))),
        _ => None,
    };
}

/// Take delay `index`, and close the menu. An index the menu does not have chooses nothing.
pub fn choose(s: &mut State, index: Option<usize>) {
    if let (Some(delay), Some(source)) = (index.and_then(|i| DELAYS.get(i)), s.account.source.as_mut()) {
        source.set_lock_after(delay);
    }
    s.account.dialog = None;
}

/// The open menu (`ui::menu`): the delays, the current one checked, `hot` under the pointer or the keys.
pub fn menu(lock_after: &str, hot: Option<usize>, t: &Theme) -> El {
    let now = chosen(lock_after);
    let entries: Vec<Entry> = DELAYS
        .iter()
        .enumerate()
        .map(|(i, label)| Entry::check(label, label, Some(i) == now))
        .collect();
    menu::menu(
        "menu-0",
        "Locks again after",
        130.0,
        &entries,
        hot,
        t,
        |s: &mut State, i: usize, _: &mut Tx| choose(s, Some(i)),
        |s: &mut State, i: usize| s.account.dialog = Some(Dialog::LockDelay(Some(i))),
        |s: &mut State, _: &mut Tx| s.account.dialog = None,
    )
    .anchored(ANCHOR, Side::Bottom, Align::Start, 0.0)
}

/// The menu's keys: the arrows, Home and End move, Enter takes, Esc closes, Tab closes and
/// moves on. With no row under the keys yet (an unknown stored delay), Down starts at the
/// first and Up at the last, and Enter takes nothing. `None`: not a key of the menu's.
pub fn on_key(s: &mut State, hot: Option<usize>, ev: &KeyEvent) -> Option<bool> {
    let last = DELAYS.len() - 1;
    let hot = match ev.key {
        Key::Down => hot.map_or(0, |h| (h + 1).min(last)),
        Key::Up => hot.map_or(last, |h| h.saturating_sub(1)),
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
    s.account.dialog = Some(Dialog::LockDelay(Some(hot)));
    Some(true)
}
