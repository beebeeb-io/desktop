//! The Files window's keyboard: selection, open and back, search.
//!
//! The window root carries it (`El::on_key`), and so does the app (`AppSpec::on_key`), for
//! the keys that arrive while nothing has focus. A row's menu key (Menu, Shift+F10) is
//! Rune's: it calls the row's `on_secondary_press`.

use crate::State;
use rune::{Key, KeyEvent, Tx};

/// The search field's key (`Tx::focus`).
pub const SEARCH_KEY: &str = "search-input";

pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // Ctrl+F (⌘F on a Mac): the search field, wherever focus is.
    if ev.is_command('f') {
        tx.focus(SEARCH_KEY, true);
        return true;
    }
    // The list's keys are the list's: with focus on a row or nowhere. A focused field, button or
    // switch keeps Enter, Backspace and the arrows for itself.
    if tx.focused().is_some_and(|key| !key.starts_with("row-")) {
        return false;
    }
    match ev.key {
        Key::Down => {
            s.move_selection(1);
            true
        }
        Key::Up => {
            s.move_selection(-1);
            true
        }
        Key::Enter => s.open_selected(),
        Key::Left if ev.mods.alt => s.go_back(),
        Key::Backspace => s.go_back(),
        _ => false,
    }
}
