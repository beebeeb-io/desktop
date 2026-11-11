//! Trash (`DW-0747`, an item's menu `DW-0748`, emptying `DW-0749b`, and its empty state `DW-0117`): what was deleted in the last 30 days and
//! where it came from. Nothing here opens; an item can be restored.

pub mod scenes;
pub mod source;
#[cfg(test)]
mod tests;

use crate::State;
use crate::frames::Intent;
use crate::frames::shared::parts::controls::{ButtonKind, button};
use crate::frames::shared::parts::menu::{Entry, Open, menu};
use crate::frames::shared::parts::table::{Columns, header, rows, table_row};
use crate::frames::shared::parts::toolbar::{Bar, toolbar};
use crate::frames::shared::parts::{chrome, empty};
use crate::theme::{icons, tokens};
use crate::ui::kit;
use rune::{Cx, El, Key, KeyEvent, Tx, col, el};
use source::{BoardTrash, TrashSource};

const COLUMNS: Columns = Columns {
    names: ["Name", "Deleted", "Size", "Where it was", ""],
    extra_w: 150.0,
    pc_w: 0.0,
};

pub struct TrashUi {
    pub source: Box<dyn TrashSource>,
    pub selected: Option<usize>,
    /// The selected item's menu, while it is open (DW-0748).
    pub menu: Option<Open>,
}

const MENU: &[Entry] = &[
    Entry::new("Restore"),
    Entry::new("Details").keys("Alt+Enter"),
    Entry::new("Delete permanently\u{2026}").danger().rule(),
];

fn pick(s: &mut State, n: usize) {
    s.frames.trash.menu = None;
    match n {
        0 => {
            s.frames.trash.restore_selected();
        }
        1 => s.frames.intents.push(Intent::Frame("DW-0411")),
        2 => s.frames.intents.push(Intent::Frame("DW-0749")),
        _ => {}
    }
}

fn close_menu(s: &mut State) {
    s.frames.trash.menu = None;
}

impl Default for TrashUi {
    fn default() -> Self {
        TrashUi {
            source: Box::new(BoardTrash::new()),
            selected: None,
            menu: None,
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

/// The note while Trash empties (`.mk-banner`): how far it is, and that the view can be left.
/// Keys: `banner`, `banner-text`, `banner-bar`, `banner-bar-fill`.
fn emptying_note(e: source::Emptying, t: &tokens::Theme) -> El {
    const WORDS: rune::TextStyle = rune::TextStyle::new(12.5, 400.0).lh(17.0);
    let count = format!("{} of {}", e.done, plural(e.of, "item", "items"));
    let words = format!("Emptying Trash. {count} deleted for good. You can leave this view; it carries on.");
    let start = "Emptying Trash. ".len();
    // The board's bar is in whole per cents (5 of 12: 42 %).
    let per_cent = if e.of == 0 {
        0.0
    } else {
        (e.done as f32 * 100.0 / e.of as f32).round()
    };
    kit::note_text("banner-text", "banner-words", &words, WORDS, t.text_secondary);
    let text = rune::rich(words, vec![rune::Span::new(start, start + count.len()).weight(700.0)])
        .key("banner-words")
        .font(WORDS)
        .color(t.text_secondary)
        .w_full();
    el().key("banner-wrap").w_full().px(14.0).pt(10.0).child(
        col()
            .key("banner")
            .role("status")
            .label("Emptying Trash")
            .w_full()
            .pt(9.0)
            .pb(9.0)
            .pl(12.0)
            .pr(10.0)
            .gap(6.0)
            .radius(10.0)
            .fill(t.text.alpha(0.06))
            .child(text)
            .child(
                el().key("banner-bar")
                    .role("progressbar")
                    .label(format!("{per_cent} %"))
                    .w_full()
                    .h(4.0)
                    .radius(2.0)
                    .fill(t.selection)
                    .child(
                        el().key("banner-bar-fill")
                            .w_frac(per_cent / 100.0)
                            .h(4.0)
                            .radius(2.0)
                            .fill(t.text_secondary),
                    ),
            ),
    )
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

    let emptying = ui.source.emptying();
    let sub = if is_empty {
        plural(0, "item", "items")
    } else if emptying.is_some() {
        format!("{} left", plural(totals.items, "item", "items"))
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
    } else if emptying.is_some() {
        // Already emptying (DW-0749b): the button waits.
        vec![button("Empty Trash\u{2026}", ButtonKind::GhostOff, None, &t)]
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
                    .on_secondary_press(move |s: &mut State, press, _| {
                        s.frames.trash.selected = Some(i);
                        s.frames.trash.menu = Some(Open::at(press.x, press.y));
                    })
            })
            .collect();
        let mut body = col().key("list-body");
        if let Some(e) = emptying {
            body = body.child(emptying_note(e, &t));
        }
        body.child(header(&COLUMNS, &t)).child(rows("Trash", list))
    };
    chrome::window(
        s,
        cx,
        &t,
        bar,
        body,
        "Items in Trash are deleted for good after 30 days",
        ui.menu
            .as_ref()
            .map(|open| menu("Trash item", MENU, open, 220.0, &t, pick, close_menu)),
    )
}

/// Trash's keys: the arrows select, Delete asks for the permanent delete (DW-0748's last
/// item), Ctrl+Z is not here (restore is the menu's). Nothing opens with Enter.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if let Some(open) = s.frames.trash.menu.as_mut() {
        let hot = open.hot;
        return match ev.key {
            Key::Enter => {
                pick(s, hot);
                true
            }
            Key::Escape => {
                close_menu(s);
                true
            }
            _ => open.on_key(ev, MENU.len()),
        };
    }
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
