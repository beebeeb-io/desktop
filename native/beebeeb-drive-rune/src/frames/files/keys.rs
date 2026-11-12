//! The Files frames' keyboard. The list's keys work with focus on the list or nowhere; an open menu
//! takes the arrows, Enter and Escape; a focused field, button or switch keeps its own keys.
//!
//! | Key | Does |
//! | --- | --- |
//! | Up, Down (+Shift) | select the next row (extend the range), and scroll it into view |
//! | Home, End | the first, the last row |
//! | Ctrl+A | select every row |
//! | Enter | open: a folder in place, a file in its app |
//! | Space | Preview (DW-0408) of a file; nothing on a folder |
//! | Alt+Enter, F2 | Details (DW-0411); Rename in the row (DW-0410: Enter saves, Esc cancels) |
//! | Delete | Move to Trash, with Undo |
//! | Ctrl+Z, Ctrl+D | Undo the move to Trash; Duplicate |
//! | Menu, Shift+F10 | the file menu, at the selected row |
//! | Ctrl+Shift+N, Ctrl+N, Ctrl+O | New folder, New file, Upload files (nothing in a folder that takes no additions) |
//! | Ctrl+1..5 | Files, Shared, Photos, Backups, Trash; Ctrl+6, Ctrl+7 Sync history (DW-1301), Available offline (DW-0715); Ctrl+, Settings (DW-0701) |
//! | Alt+Shift+P | the details panel |
//! | Alt+Left, Backspace; Alt+Right | back; forward |
//! | Ctrl+F | Search names |
//! | Escape | close the menu, else clear the selection |

use super::Intent;
use crate::State;
use crate::ui::file_list::ROW_H;
use crate::ui::keys::SEARCH_KEY;
use crate::ui::menu;
use crate::ui::sidebar::SidebarSection;
use rune::{Key, KeyEvent, Tx};

fn menu_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(open) = s.actions.menu else { return false };
    let entries = s.menu_entries(open.kind);
    let hot = match ev.key {
        Key::Down => menu::step(&entries, open.hot, 1),
        Key::Up => menu::step(&entries, open.hot, -1),
        Key::Home => menu::step(&entries, None, 1),
        Key::End => menu::step(&entries, None, -1),
        Key::Enter => {
            if let Some(hot) = open.hot {
                s.pick_menu(hot, tx);
            }
            return true;
        }
        Key::Escape => {
            s.close_menu_tx(tx);
            return true;
        }
        // Type-to-select: the next item that starts with the letter.
        Key::Text(ref typed) if !ev.mods.cmd && !ev.mods.ctrl && !ev.mods.alt => {
            let typed = typed.to_lowercase();
            let from = open.hot.map_or(0, |h| h + 1);
            (0..entries.len()).map(|n| (from + n) % entries.len()).find(|&i| {
                matches!(entries[i], menu::Entry::Item { label, off: false, .. }
                    if !typed.trim().is_empty() && label.to_lowercase().starts_with(&typed))
            })
        }
        // A menu is modal: no other key reaches the list under it.
        _ => return true,
    };
    if let (Some(hot), Some(open)) = (hot, s.actions.menu.as_mut()) {
        open.hot = Some(hot);
        // Keyboard focus goes with the highlight, so a screen reader says the item it is on.
        if let Some(key) = menu::item_key("menu", &entries, hot) {
            tx.focus(key, false);
        }
    }
    true
}

/// Where the Menu key opens the file menu when no row has focus: on the selected row.
fn menu_point(s: &State) -> (f32, f32) {
    let row = s.selected.unwrap_or(0) as f32;
    (430.0, 110.0 + 4.0 + row * ROW_H + 22.0)
}

fn reveal(row: Option<usize>, tx: &mut Tx) -> bool {
    if let Some(row) = row {
        // Also a row outside the realised window of a 5,000-row folder.
        tx.scroll_into_view(format!("row-{row}"));
    }
    row.is_some()
}

/// After the folder changed: the keyboard stays in the list (the table, or the empty folder's
/// area), so the next key is still the list's.
pub(super) fn focus_list(s: &State, tx: &mut Tx) {
    tx.focus(
        if s.list_error.is_some() {
            "file-list-error"
        } else if s.listing.items.is_empty() && s.can_add() {
            "empty-area"
        } else {
            "rows"
        },
        false,
    );
}

pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // A dialog takes every key; so does a name being typed in a row (its field has the keyboard,
    // and what the field does not use is nobody's).
    if let Some(handled) = super::protected::key(s, ev, tx) {
        return handled;
    }
    if s.actions.dialog.is_some() {
        return s.dialog_key(ev, tx);
    }
    if s.actions.rename.is_some() {
        if ev.key == Key::Escape {
            s.cancel_rename(tx);
        }
        return true;
    }
    if s.actions.menu.is_some() {
        return menu_key(s, ev, tx);
    }
    let m = ev.mods;
    // The window's shortcuts, wherever focus is.
    if ev.is_command('f') {
        tx.focus(SEARCH_KEY, true);
        return true;
    }
    if let Key::Command(c) = ev.key {
        match c {
            // Nothing is added to Backups, a service folder, Trash or Shared with me (DW-0414).
            'n' | 'o' if !s.can_add() => return false,
            'n' if m.shift => {
                s.begin_new_folder();
            }
            'n' => s.intent(Intent::Frame("DW-1521")),
            'o' => s.intent(Intent::Frame("DW-0415")),
            '1'..='5' => match SidebarSection::all().get(c as usize - '1' as usize) {
                Some(&section) => s.sidebar_section = section,
                None => return false,
            },
            '6' => s.intent(Intent::Frame("DW-1301")),
            '7' => s.intent(Intent::Frame("DW-0715")),
            ',' => s.intent(Intent::Frame("DW-0701")),
            _ => return list_key(s, ev, tx),
        }
        return true;
    }
    // Back and Forward are the window's, wherever focus is (but a field keeps its own Alt+arrows).
    let in_field = tx.focused() == Some(SEARCH_KEY);
    if m.alt && !in_field && matches!(ev.key, Key::Left | Key::Right) {
        let moved = if ev.key == Key::Left { s.back() } else { s.forward() };
        if moved {
            focus_list(s, tx);
        }
        return moved;
    }
    if m.alt && m.shift && matches!(&ev.key, Key::Text(p) if p.eq_ignore_ascii_case("p")) {
        s.show_detail_pane = !s.show_detail_pane;
        return true;
    }
    list_key(s, ev, tx)
}

fn list_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // The list's keys are the list's: with focus on a row or nowhere.
    if tx
        .focused()
        .is_some_and(|key| !matches!(key, "rows" | "empty-area" | "file-list-error"))
    {
        return false;
    }
    let m = ev.mods;
    let count = s.listing.items.len();
    match ev.key {
        Key::Down => reveal(s.arrow(1, m.shift), tx),
        Key::Up => reveal(s.arrow(-1, m.shift), tx),
        Key::Home if count > 0 => {
            if m.shift {
                s.extend_to(0);
            } else {
                s.select_only(0);
            }
            reveal(Some(0), tx)
        }
        Key::End if count > 0 => {
            if m.shift {
                s.extend_to(count - 1);
            } else {
                s.select_only(count - 1);
            }
            reveal(Some(count - 1), tx)
        }
        Key::Command('a') => {
            s.select_all();
            true
        }
        Key::Command('z') => s.undo(tx),
        Key::Command('d') => s.duplicate_selected(tx),
        Key::Enter if m.alt && s.selected.is_some() => {
            s.intent(Intent::Frame("DW-0411"));
            true
        }
        Key::Enter if s.selected.is_some() => {
            if s.open_selection(tx) {
                focus_list(s, tx);
            }
            true
        }
        // Preview: of a file. On a folder Space does nothing (DW-0402: "Preview is dimmed").
        Key::Text(ref space) if space == " " && s.selected.is_some() => s.preview(),
        Key::Function(2) if s.selected.is_some() => s.begin_rename(tx),
        Key::Delete => s.trash_selection(tx),
        Key::ContextMenu if s.selected.is_some() => {
            s.open_file_menu(menu_point(s));
            true
        }
        Key::Function(10) if m.shift && s.selected.is_some() => {
            s.open_file_menu(menu_point(s));
            true
        }
        Key::Backspace => {
            let moved = s.back();
            if moved {
                focus_list(s, tx);
            }
            moved
        }
        Key::Escape if !s.selected_rows().is_empty() => {
            s.clear_selection();
            true
        }
        _ => false,
    }
}
