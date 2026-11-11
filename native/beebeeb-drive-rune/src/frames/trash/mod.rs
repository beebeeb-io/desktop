//! Trash (`DW-0747`, and its empty state `DW-0117`): what was deleted in the last 30 days and
//! where it came from. Nothing here opens; an item can be restored.

pub mod scenes;
pub mod source;
#[cfg(test)]
mod tests;

use crate::State;
use crate::frames::Intent;
use crate::frames::shared::parts::controls::{ButtonKind, button};
use crate::frames::shared::parts::table::{Columns, header, rows, table_row};
use crate::frames::shared::parts::toolbar::{Bar, toolbar};
use crate::frames::shared::parts::{chrome, empty};
use crate::theme::{icons, tokens};
use rune::{Cx, El, Key, KeyEvent, Tx, col};
use source::{BoardTrash, TrashSource};

const COLUMNS: Columns = Columns {
    names: ["Name", "Deleted", "Size", "Where it was", ""],
    extra_w: 150.0,
    pc_w: 0.0,
};

pub struct TrashUi {
    pub source: Box<dyn TrashSource>,
    pub selected: Option<usize>,
}

impl Default for TrashUi {
    fn default() -> Self {
        TrashUi {
            source: Box::new(BoardTrash::new()),
            selected: None,
        }
    }
}

impl TrashUi {
    /// Restore the selected item; the selection stays on the row that takes its place.
    pub fn restore_selected(&mut self) -> bool {
        let items = self.source.items();
        let Some(item) = self.selected.and_then(|i| items.get(i)) else {
            return false;
        };
        let done = self.source.restore(&item.id);
        let left = self.source.items().len();
        self.selected = self
            .selected
            .filter(|_| left > 0)
            .map(|i| i.min(left.saturating_sub(1)));
        done
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    match n {
        0 => format!("No {many}"),
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    let ui = &s.frames.trash;
    let items = ui.source.items();
    let totals = ui.source.totals();
    let is_empty = totals.items == 0;

    let sub = if is_empty {
        plural(0, "item", "items")
    } else {
        format!(
            "{} \u{00B7} {}, counts toward your storage",
            plural(totals.items, "item", "items"),
            totals.size
        )
    };
    // Empty Trash… is hidden when there is nothing to empty (DW-0117).
    let slot = if is_empty {
        Vec::new()
    } else {
        vec![
            button("Empty Trash\u{2026}", ButtonKind::GhostDanger, None, &t)
                .on_press(|s: &mut State| s.frames.intents.push(Intent::Frame("DW-0749"))),
        ]
    };
    let bar = toolbar(
        s,
        Bar {
            title: "Trash",
            sub,
            slot,
            slot_sep: false,
            new_off: true,
        },
        &t,
    );

    let body = if is_empty {
        empty::empty(
            &icons::TRASH,
            "Trash is empty",
            "Files you delete stay here until you delete them for good.",
            None,
            &t,
        )
    } else {
        let list = items
            .iter()
            .enumerate()
            .map(|(i, r)| {
                table_row(i, r, ui.selected == Some(i), &COLUMNS, &t)
                    .on_press(move |s: &mut State| s.frames.trash.selected = Some(i))
                    .on_secondary_press(move |s: &mut State, _, _| {
                        s.frames.trash.selected = Some(i);
                        s.frames.intents.push(Intent::Frame("DW-0748"));
                    })
            })
            .collect();
        col()
            .key("list-body")
            .child(header(&COLUMNS, &t))
            .child(rows("Trash", list))
    };
    chrome::window(
        s,
        cx,
        &t,
        bar,
        body,
        "Items in Trash are deleted for good after 30 days",
        None,
    )
}

/// Trash's keys: the arrows select, Delete asks for the permanent delete (DW-0748's last
/// item), Ctrl+Z is not here (restore is the menu's). Nothing opens with Enter.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.is_command('f') {
        tx.focus(crate::ui::keys::SEARCH_KEY, true);
        return true;
    }
    if tx.focused().is_some_and(|key| !key.starts_with("row-")) {
        return false;
    }
    let count = s.frames.trash.source.items().len();
    if count == 0 {
        return false;
    }
    let ui = &mut s.frames.trash;
    match ev.key {
        Key::Down => {
            ui.selected = Some(ui.selected.map_or(0, |i| (i + 1).min(count - 1)));
            true
        }
        Key::Up => {
            ui.selected = Some(ui.selected.map_or(0, |i| i.saturating_sub(1)));
            true
        }
        Key::Delete if ui.selected.is_some() => {
            s.frames.intents.push(Intent::Frame("DW-0749"));
            true
        }
        _ => false,
    }
}
