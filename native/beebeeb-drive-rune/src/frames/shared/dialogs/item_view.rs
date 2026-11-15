//! Drawing an item's Details (DW-0411's card) and its read-only Preview (DW-0408's panel in `Mode::ReadOnly`) over
//! Trash, Shared or Search (`frames::item`). Both are drawn from the origin's CURRENT row, each frame: a row that
//! is not listed draws nothing.

use super::Dialog;
use crate::State;
use crate::frames::files::actions_data::Previewed;
use crate::frames::files::preview::{BLANK_CARD, Mode, PAGE_CARD, card};
use crate::frames::files::share::file_head;
use crate::frames::item::{Fact, Origin, found};
use crate::frames::{self};
use crate::theme::tokens::Theme;
use crate::ui::settings_list::{self as lists, Item, Trailing};
use crate::ui::{button, dialog};
use rune::{El, Key, KeyEvent, Tx, row};

/// The first `list-<n>` the card's list takes: after the lists of the window behind it (Search's results draw a
/// `list-0` of their own).
fn list_number(s: &State, origin: Origin) -> usize {
    match origin {
        Origin::Shared => s.frames.shared.lists_behind(),
        Origin::Trash => 0,
        Origin::Search => 1,
    }
}

/// Details (DW-0411's card, `ui::dialog::card` as Files draws it): the item's tile, name and kind, then its facts.
/// `None`: its row is not listed now.
pub(super) fn details(s: &State, origin: Origin, id: &str, t: &Theme) -> Option<El> {
    let found = found(s, origin, id)?;
    let n = list_number(s, origin);
    let mut rows: Vec<El> = Vec::new();
    for (title, fact) in &found.facts {
        let index = rows.len();
        let trailing = match fact {
            Fact::Figure(words) => Trailing::Mono(words.clone()),
            Fact::Words(words) => Trailing::Value(words.clone()),
        };
        let mut it = Item::new(None, *title).trailing(trailing);
        if index > 0 {
            it = it.ruled();
        }
        rows.push(lists::item(n, index, &it, t));
    }
    let body = vec![
        file_head(&found.item, &found.item.name, &found.kind, false, 40.0, t),
        lists::list(n, None, rows, None, t),
    ];
    let foot = row().center_y().justify_end().child(
        button::outline_button("btn-0", "Done", None, t)
            .default_button()
            .on_press_tx(close),
    );
    Some(dialog::card("Details", 420.0, "Details", None, body, foot, t))
}

/// The read-only Preview: DW-0408's card with no Share, no Open and no Make available offline. `None`: its row is
/// not listed now.
pub(super) fn preview(s: &State, origin: Origin, id: &str, t: &Theme) -> Option<El> {
    let found = found(s, origin, id)?;
    // Borrowed, never copied: the page's only copy outside the drawn labels is the one the fields hold and wipe.
    let loading = Previewed::Loading;
    let state = s.frames.shared.fields.preview.state().unwrap_or(&loading);
    let (w, h, _) = if matches!(state, Previewed::Shown(_)) {
        PAGE_CARD
    } else {
        BLANK_CARD
    };
    // A window smaller than the board's: the card stays inside it, under the caption.
    let win_h = s.actions.window.1;
    let h = if win_h > 0.0 {
        h.min((win_h - crate::TITLEBAR_H - 32.0).max(120.0))
    } else {
        h
    };
    let mode = Mode::ReadOnly {
        tail: origin.tail(),
        close,
        size: found.size.clone(),
    };
    Some(card(&found.item, state, &mode, (w, h), t))
}

/// The panel's keys: Space closes it (when focus is not on one of its controls, which Space presses); Left and Right
/// stop here (nothing steps through the list behind); every other key is the dialog's.
pub(super) fn preview_keys(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match &ev.key {
        _ if frames::is_space(ev) && tx.focused().is_none_or(|key| !key.starts_with("pv-")) => {
            close(s, tx);
            true
        }
        Key::Left | Key::Right => true,
        _ => super::on_key(s, ev, tx),
    }
}

/// Close the item dialog (its fields, the page among them, are wiped), and give the keyboard back to its row.
pub(super) fn close(s: &mut State, tx: &mut Tx) {
    let origin = match &s.frames.shared.dialog {
        Some(Dialog::Details { origin, .. } | Dialog::Preview { origin, .. }) => Some(*origin),
        _ => None,
    };
    super::close(s);
    if let Some(origin) = origin {
        crate::frames::item::return_focus(s, origin, tx);
    }
}
