//! One keyboard model for lists that own their keys: the Windows file-manager keys for a list
//! whose rows are virtual and whose selection the app holds (Files, Search, Shared, the archive
//! viewer). Pure: no clock, no Rune state. The caller passes the key, the list's shape, its
//! cursor and the time, and gets back one [`ListAction`] to apply.
//!
//! - Up/Down one row, Home/End the ends, PageUp/PageDown `ListKeys::page` rows (stopping at the ends).
//! - Shift+move extends from the anchor, Ctrl+move moves focus only; both only when `multi`
//!   (in a single-select list they act as a plain move). Shift+Ctrl+move also extends.
//! - Space selects the focused row, Ctrl+Space toggles it (multi), Shift+Space extends to it (multi).
//!   While a type-ahead buffer is live (under 1000 ms old) Space types a space instead, as in Explorer.
//! - Enter is Open, F2 Rename, Delete Delete (on the caller's selection), the Menu key or Shift+F10
//!   Menu at the focused row, Alt+Up Parent.
//! - Printable text is type-ahead: a case-insensitive prefix match on `label`; the buffer resets
//!   after 1000 ms without a typed key; a repeated single letter cycles through the rows starting
//!   with it and wraps; no match leaves focus where it is.
//!
//! Focus and anchor are indexes; the caller maps them to item ids and owns the selection. "Ctrl"
//! here is `mods.ctrl || mods.cmd`, so ⌘ works the same on macOS.
//!
//! Lists whose rows take real focus (built rows in a role list/listbox `focus_group`) do not need
//! this: Rune's own roving focus and type-select already give them these keys.

use std::ops::RangeInclusive;

use rune::{Key, KeyEvent};

/// How long the type-ahead buffer lives after the last typed key.
pub const TYPE_AHEAD_MS: u64 = 1000;

/// The shape of the list: how many rows, how many fit a page, whether several may be selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListKeys {
    pub len: usize,
    pub page: usize,
    pub multi: bool,
}

/// Where the keyboard is in the list. Keep one per list in the app state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListCursor {
    /// The focused row.
    pub focus: Option<usize>,
    /// Where a Shift range starts.
    pub anchor: Option<usize>,
    /// The type-ahead buffer, lowercased.
    buf: String,
    /// When the last type-ahead key came.
    last_ms: u64,
}

/// What a move does to the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Select {
    /// Select only the row moved to (it becomes the anchor).
    Only,
    /// Select [`extend_range`] from the anchor to the row moved to.
    Extend,
    /// Leave the selection as it is: focus only.
    Keep,
}

/// What the caller should do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAction {
    /// Not a list key, or nothing to do.
    None,
    /// Focus moved to `to` (already set on the cursor); apply `select`.
    Move {
        to: usize,
        select: Select,
    },
    /// Toggle this row in the selection.
    Toggle(usize),
    Open(usize),
    Rename(usize),
    /// Delete the selection.
    Delete,
    /// Open the context menu at this row.
    Menu(usize),
    /// Go to the parent folder.
    Parent,
}

/// The rows a Shift range covers: from the anchor (else the focus, else `to`) to `to`, in order.
pub fn extend_range(cursor: &ListCursor, to: usize) -> RangeInclusive<usize> {
    let from = cursor.anchor.or(cursor.focus).unwrap_or(to);
    from.min(to)..=from.max(to)
}

/// Handle one key for a list that owns its keyboard. Updates `cursor` (focus, anchor, type-ahead)
/// and says what to do with the selection. `now_ms` is any monotonic millisecond clock.
pub fn on_list_key(
    ev: &KeyEvent,
    keys: &ListKeys,
    cursor: &mut ListCursor,
    now_ms: u64,
    label: impl Fn(usize) -> String,
) -> ListAction {
    let m = ev.mods;
    let ctrl = m.ctrl || m.cmd;
    // A list that shrank: keep the cursor inside it.
    if keys.len == 0 {
        cursor.focus = None;
        cursor.anchor = None;
    } else {
        cursor.focus = cursor.focus.map(|f| f.min(keys.len - 1));
        cursor.anchor = cursor.anchor.map(|a| a.min(keys.len - 1));
    }
    let typing = !cursor.buf.is_empty() && now_ms.saturating_sub(cursor.last_ms) < TYPE_AHEAD_MS;

    // Type-ahead first: printable text without Ctrl (AltGr, which is Ctrl+Alt, still types).
    if let Key::Text(s) = &ev.key {
        let space = s == " ";
        if !space || (typing && !ctrl && !m.shift) {
            if ctrl && !m.alt {
                return ListAction::None;
            }
            return type_ahead(s, keys, cursor, now_ms, typing, &label);
        }
    }
    cursor.buf.clear();

    if keys.len == 0 {
        return match ev.key {
            Key::Up if m.alt => ListAction::Parent,
            Key::Delete if !m.alt && !ctrl => ListAction::Delete,
            _ => ListAction::None,
        };
    }
    let last = keys.len - 1;
    let page = keys.page.max(1);
    let focus = cursor.focus;

    let to = match ev.key {
        Key::Up if m.alt => return ListAction::Parent,
        _ if m.alt => return ListAction::None,
        Key::Up => focus.map_or(0, |f| f.saturating_sub(1)),
        Key::Down => focus.map_or(0, |f| (f + 1).min(last)),
        Key::Home => 0,
        Key::End => last,
        Key::PageUp => focus.map_or(0, |f| f.saturating_sub(page)),
        Key::PageDown => focus.map_or(0, |f| (f + page).min(last)),
        Key::Text(_) => {
            // Space (the type-ahead case returned above).
            let Some(f) = focus else { return ListAction::None };
            return if keys.multi && ctrl && !m.shift {
                cursor.anchor = Some(f);
                ListAction::Toggle(f)
            } else if keys.multi && m.shift {
                ListAction::Move {
                    to: f,
                    select: Select::Extend,
                }
            } else {
                cursor.anchor = Some(f);
                ListAction::Move {
                    to: f,
                    select: Select::Only,
                }
            };
        }
        Key::Enter if !ctrl && !m.shift => return focus.map_or(ListAction::None, ListAction::Open),
        Key::Function(2) if !ctrl && !m.shift => return focus.map_or(ListAction::None, ListAction::Rename),
        Key::Delete if !ctrl => return ListAction::Delete,
        Key::ContextMenu => return focus.map_or(ListAction::None, ListAction::Menu),
        Key::Function(10) if m.shift && !ctrl => return focus.map_or(ListAction::None, ListAction::Menu),
        _ => return ListAction::None,
    };

    let select = if keys.multi && m.shift {
        if cursor.anchor.is_none() {
            cursor.anchor = Some(focus.unwrap_or(to));
        }
        Select::Extend
    } else if keys.multi && ctrl {
        Select::Keep
    } else {
        cursor.anchor = Some(to);
        Select::Only
    };
    cursor.focus = Some(to);
    ListAction::Move { to, select }
}

fn type_ahead(
    s: &str,
    keys: &ListKeys,
    cursor: &mut ListCursor,
    now_ms: u64,
    typing: bool,
    label: &impl Fn(usize) -> String,
) -> ListAction {
    let typed: String = s
        .chars()
        .filter(|c| !c.is_control())
        .flat_map(char::to_lowercase)
        .collect();
    if typed.is_empty() {
        return ListAction::None;
    }
    if !typing {
        cursor.buf.clear();
    }
    cursor.buf.push_str(&typed);
    cursor.last_ms = now_ms;
    if keys.len == 0 {
        return ListAction::None;
    }

    let mut chars = cursor.buf.chars();
    let first = chars.next().unwrap_or(' ');
    let cycling = chars.all(|c| c == first);
    // Cycling (one letter, maybe repeated) starts after the focus; a longer prefix may stay put.
    let (needle, start) = if cycling {
        (first.to_string(), cursor.focus.map_or(0, |f| f + 1))
    } else {
        (cursor.buf.clone(), cursor.focus.unwrap_or(0))
    };
    let hit = (0..keys.len)
        .map(|i| (start + i) % keys.len)
        .find(|&i| label(i).to_lowercase().starts_with(&needle));
    match hit {
        Some(to) => {
            cursor.focus = Some(to);
            cursor.anchor = Some(to);
            ListAction::Move {
                to,
                select: Select::Only,
            }
        }
        None => ListAction::None,
    }
}

#[cfg(test)]
#[path = "list_keys_tests.rs"]
mod tests;
