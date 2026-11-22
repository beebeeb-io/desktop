//! Trash (`DW-0747`, an item's menu `DW-0748`, emptying `DW-0749b`, and its empty state `DW-0117`): what was deleted in the last 30 days and
//! where it came from. Nothing here opens (an item can be restored); Details and a read-only Preview show over the list
//! (`frames::item`).

pub mod actions;
pub mod scenes;
pub mod source;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_live;

use crate::State;
use crate::frames::shared::dialogs::{Dialog, PASSWORD_INPUT};
use crate::frames::shared::parts::chrome;
use crate::frames::shared::parts::controls::{ButtonKind, button};
use crate::frames::shared::parts::menu::{Entry, Open, Took, menu};
use crate::frames::shared::parts::table::{Columns, Row, header, rows, table_row};
use crate::frames::shared::parts::toolbar::{Bar, toolbar};
use crate::frames::shared::state_body;
use crate::frames::{self, Intent};
use crate::theme::{icons, tokens};
use crate::ui::file_row::ROW_H;
use crate::ui::kit;
use crate::ui::view_state::{ViewState, try_again};
use rune::{Cx, El, Key, KeyEvent, Tx, col, el};
use source::{BoardTrash, Loaded, Notice, PurgeEnd, TrashSource};
use std::borrow::Cow;
use std::sync::Arc;

const COLUMNS: Columns = Columns {
    names: ["Name", "Deleted", "Size", "Where it was", ""],
    extra_w: 150.0,
    pc_w: 0.0,
};

pub struct TrashUi {
    pub source: Box<dyn TrashSource>,
    /// The selected row's place in the list as drawn.
    pub selected: Option<usize>,
    /// The selected ITEM: what every action acts on. The list re-reads and re-sorts under it, so `selected` is
    /// worked out again from this id on each poll (`resync`).
    pub selected_item: Option<String>,
    /// The selected item's menu, while it is open (DW-0748).
    pub menu: Option<Open>,
    /// What the view says over the list (a refused restore, a control not available yet).
    pub notice: Option<Notice>,
    /// The gate and account the last poll saw: a change drops every name at once.
    seen: Option<(crate::shell::app::GateState, Option<String>)>,
    /// Whether the Trash section was open at the last poll.
    was_open: bool,
    /// A sync check finished and the list is to be read again: waits while a menu or a dialog is open.
    pub(crate) reread_due: bool,
    /// The list's order and what the toolbar's search filters it by (`shared::actions_sort`).
    pub sort: crate::frames::shared::actions_sort::SectionSort,
    /// The one sorted copy of the source's answer (`rows`).
    arranged: crate::frames::shared::actions_sort::Arranged,
}

/// How often the poll runs again while a read or a restore is on its way.
const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// A sync check finished (`shell::host::poll`): the list is read again, on the first poll with nothing open over it.
pub fn after_a_check(s: &mut State) {
    s.frames.trash.reread_due = true;
    reread_if_free(s);
}

/// The read a finished check asked for, when nothing is open over the list (`after_a_check`, and every poll).
fn reread_if_free(s: &mut State) {
    let ready = s.shell.app.gate() == crate::shell::app::GateState::Ready;
    let free = s.frames.trash.menu.is_none() && s.frames.shared.dialog.is_none();
    let ui = &mut s.frames.trash;
    if ui.reread_due && ready && free {
        ui.reread_due = false;
        ui.source.refresh();
    }
}

/// The window's poll (`shell::host::poll`): a gate or account change drops the source's names at once, on the
/// window's side; opening the Trash section asks for the list (the source decides whether it is old); what arrived
/// is taken in, and the selection follows its item. `true`: something drawn changed.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let gate = (s.shell.app.gate(), s.shell.app.account().map(str::to_owned));
    let open =
        s.sidebar_section == crate::ui::sidebar::SidebarSection::Trash && gate.0 == crate::shell::app::GateState::Ready;
    reread_if_free(s);
    let ui = &mut s.frames.trash;
    let mut changed = false;
    if ui.seen.as_ref() != Some(&gate) {
        if ui.seen.is_some() {
            ui.source.forget();
            ui.selected = None;
            ui.selected_item = None;
            ui.menu = None;
            ui.notice = None;
            // The section must re-open (`opened`) on the next poll so a new read starts: without this,
            // an account change while the gate stays Ready would show "Loading" forever.
            ui.was_open = false;
            ui.reread_due = false;
            changed = true;
        }
        ui.seen = Some(gate);
    }
    let ui = &mut s.frames.trash;
    if open && !ui.was_open {
        ui.notice = None;
        ui.source.opened();
        changed = true;
    }
    ui.was_open = open;
    // Words kept for later wait until Trash is on screen: handed to a window showing something else, the opening
    // of Trash would wipe them unseen.
    ui.source.on_screen(open);
    let pumped = ui.source.pump();
    if let Some(notice) = pumped.notice {
        ui.notice = Some(notice);
    }
    // A delete for good ended: its dialog closes (a wrong password keeps it, for another try).
    match pumped.purge {
        Some(PurgeEnd::Done | PurgeEnd::Stopped) => s.frames.shared.close_purge_dialog(),
        Some(PurgeEnd::WrongPassword) => {
            s.frames.shared.fields.password.clear();
            s.frames.shared.fields.problem = Some("That password is not right. Nothing was deleted.");
            tx.focus(PASSWORD_INPUT, true);
        }
        None => {}
    }
    let ui = &mut s.frames.trash;
    if pumped.busy {
        tx.wake_after(POLL_EVERY);
    } else if let Some(after) = pumped.wake_in {
        tx.wake_after(after);
    }
    changed |= pumped.changed;
    if pumped.changed {
        ui.resync();
        // The item a Delete permanently? dialog is about is not in the list any more (restored, or gone): it closes.
        let gone = match &s.frames.shared.dialog {
            Some(Dialog::DeleteForever(id)) => {
                let ui = &s.frames.trash;
                !ui.source.purging()
                    && ui.source.loaded() == Loaded::Ready
                    && !ui.source.items().iter().any(|r| &r.id == id)
            }
            _ => false,
        };
        if gone {
            s.frames.shared.close_purge_dialog();
        }
    }
    changed
}

use crate::frames::shared::actions_sort as sort;

/// The open menu's name, entries and width: Sort's under the toolbar, else the selected item's.
fn menu_of(open: &Open) -> (&'static str, &'static [Entry], f32) {
    if sort::is_sort_menu(open) {
        ("Sort by", sort::SORT_MENU, 180.0)
    } else {
        ("Trash item", MENU, 220.0)
    }
}

const MENU: &[Entry] = &[
    Entry::new("Restore"),
    Entry::new("Details").keys("Alt+Enter"),
    Entry::new("Delete permanently\u{2026}").danger().rule(),
];

/// Do what the menu's item `n` says, for the selected item; focus goes back to it (or to the
/// row that took its place).
fn pick(s: &mut State, n: usize, tx: &mut Tx) {
    if s.frames.trash.menu.as_ref().is_some_and(sort::is_sort_menu) {
        s.frames.trash.menu = None;
        s.frames.trash.sort.pick(n);
        s.frames.trash.selected = None;
        s.frames.trash.selected_item = None;
        return;
    }
    s.frames.trash.menu = None;
    let id = s.frames.trash.selected_id();
    match (n, id) {
        (0, _) => {
            s.frames.trash.restore_selected();
        }
        (1, Some(_)) if !s.frames.trash.source.actions_available() => {
            s.frames.trash.notice = Some(Notice::NotAvailable);
        }
        (2, Some(_)) if !s.frames.trash.source.purge_available() => {
            s.frames.trash.notice = Some(Notice::NotAvailable);
        }
        (1, Some(id)) => s.frames.intents.push(Intent::About("DW-0411", id)),
        // The warning for THIS item (DW-0748: "the same big warning as Empty Trash, for this item"), and the password.
        (2, Some(id)) => {
            open_purge(s, Dialog::DeleteForever(id), tx);
            return;
        }
        _ => {}
    }
    return_focus(s, tx);
}

/// Open the password dialog of a delete for good over Trash, its field in focus.
fn open_purge(s: &mut State, dialog: Dialog, tx: &mut Tx) {
    s.frames.trash.menu = None;
    s.frames.shared.open_dialog(dialog);
    tx.focus(PASSWORD_INPUT, true);
}

fn close_menu(s: &mut State, tx: &mut Tx) {
    s.frames.trash.menu = None;
    return_focus(s, tx);
}

fn return_focus(s: &State, tx: &mut Tx) {
    if let Some(i) = s.frames.trash.selected {
        tx.return_focus(format!("row-{i}"), tx.focus_visible());
    }
}

/// The pointer is on the menu's item `n`.
fn light(s: &mut State, n: usize) {
    if let Some(open) = s.frames.trash.menu.as_mut() {
        open.hot = n;
    }
}

impl Default for TrashUi {
    fn default() -> Self {
        TrashUi {
            source: Box::new(BoardTrash::new()),
            selected: None,
            selected_item: None,
            menu: None,
            notice: None,
            seen: None,
            was_open: false,
            reread_due: false,
            sort: Default::default(),
            arranged: Default::default(),
        }
    }
}

impl TrashUi {
    /// The rows as drawn: the source's, in the Sort and the filter of the toolbar's search.
    /// One copy is held per answer of the source and per sort, so a frame (and every key) shares it.
    pub fn rows(&self) -> Arc<Vec<Row>> {
        self.arranged.get(&self.sort, self.source.rev(), || self.source.items())
    }

    /// A Trash on `source`, nothing selected.
    pub fn on(source: Box<dyn TrashSource>) -> Self {
        TrashUi {
            source,
            ..TrashUi::default()
        }
    }

    /// The id of the selected item: the item chosen, else (a scene that set only the row) the row's.
    pub fn selected_id(&self) -> Option<String> {
        self.selected_item
            .clone()
            .or_else(|| self.rows().get(self.selected?).map(|item| item.id.clone()))
    }

    /// Move the selection to row `i` and its item; focus goes with it.
    fn select(&mut self, i: usize, tx: &mut Tx) {
        self.pick_row(i);
        // The list draws only the rows in view: the one the arrow went to is brought in first.
        tx.scroll_into_view(format!("row-{i}"));
        tx.focus(format!("row-{i}"), true);
    }

    /// Select row `i` and the item on it.
    pub fn pick_row(&mut self, i: usize) {
        self.selected = Some(i);
        self.selected_item = self.rows().get(i).map(|item| item.id.clone());
    }

    /// The list changed: the selection stays on its ITEM (its row may have moved), or ends when the item is gone.
    pub fn resync(&mut self) {
        let Some(id) = self.selected_item.as_deref() else {
            return;
        };
        match self.rows().iter().position(|item| item.id == id) {
            Some(i) => self.selected = Some(i),
            None => {
                self.selected = None;
                self.selected_item = None;
                self.menu = None;
            }
        }
    }

    /// Restore the selected ITEM; the selection moves to the row that takes its place.
    pub fn restore_selected(&mut self) -> bool {
        let Some(id) = self.selected_id() else {
            return false;
        };
        let at = self.rows().iter().position(|item| item.id == id);
        let done = self.source.restore(&id);
        let items = self.rows();
        let left = items.len();
        self.selected = at.or(self.selected).filter(|_| left > 0).map(|i| i.min(left - 1));
        self.selected_item = self.selected.and_then(|i| items.get(i)).map(|item| item.id.clone());
        done
    }
}

/// The words of a notice.
pub(crate) fn notice_words(notice: &Notice) -> String {
    match notice {
        Notice::RestoreRefused(name) => {
            format!("Couldn\u{2019}t restore \u{201C}{name}\u{201D}. It\u{2019}s still in Trash.")
        }
        Notice::RestoreNotDone(names) => match names.as_slice() {
            [name] => format!(
                "Couldn\u{2019}t restore \u{201C}{name}\u{201D} before Beebeeb was locked or signed out. \
                 It\u{2019}s still in Trash."
            ),
            [name, rest @ ..] => format!(
                "Couldn\u{2019}t restore \u{201C}{name}\u{201D} and {} before Beebeeb was locked or signed out. \
                 They\u{2019}re still in Trash.",
                plural(rest.len(), "other item", "other items")
            ),
            // A source never says it without a name; no item is claimed to be anywhere.
            [] => "A restore didn\u{2019}t go through before Beebeeb was locked or signed out.".into(),
        },
        Notice::RefreshFailed => "Couldn\u{2019}t refresh Trash. What\u{2019}s shown may be out of date.".into(),
        Notice::NotAvailable => "Not available yet in this version.".into(),
        Notice::FoldersKept { done, folders, inside } => folders_kept_words(*done, *folders, *inside),
        Notice::PurgeNotConfirmed { done: 0 } => {
            "Beebeeb didn\u{2019}t confirm the delete. Some items may be gone; check Trash.".into()
        }
        Notice::PurgeNotConfirmed { done } => format!(
            "{} deleted for good. Beebeeb didn\u{2019}t confirm the rest. Some of them may be gone; check Trash.",
            plural(*done, "item", "items")
        ),
        Notice::PurgeStopped { done: 0, why } => format!("Nothing was deleted. {why}"),
        Notice::PurgeStopped { done, why } => format!(
            "{} deleted for good. The rest are still in Trash. {why}",
            plural(*done, "item", "items")
        ),
    }
}

/// The words when a delete kept folders (and what it deleted): the reason is the server's, in our words.
fn folders_kept_words(done: usize, folders: usize, inside: u64) -> String {
    let one = folders == 1;
    let gone = plural(done, "item", "items");
    let kept = plural(folders, "folder", "folders");
    let stay = if one { "stays" } else { "stay" };
    let what = if inside == 0 {
        format!(
            "{} items that cannot be deleted from Trash",
            if one { "it holds" } else { "they hold" }
        )
    } else {
        format!(
            "{} {} that {} not in Trash",
            if one { "it holds" } else { "they hold" },
            plural(inside as usize, "item", "items"),
            if inside == 1 { "is" } else { "are" }
        )
    };
    let see = if one { "the folder" } else { "the folders" };
    format!("{gone} deleted for good. {kept} {stay} in Trash because {what}. Restore {see} to see inside.")
}

/// The notice over the list (`.mk-banner` without a bar). Keys: `notice`, `notice-words`.
fn notice_note(notice: &Notice, t: &tokens::Theme) -> El {
    const WORDS: rune::TextStyle = rune::TextStyle::new(12.5, 400.0).lh(17.0);
    el().key("notice-wrap")
        .w_full()
        .px(crate::theme::scale::space::S14)
        .pt(10.0)
        .child(
            col()
                .key("notice")
                .role("status")
                .live_region(rune::LiveRegion::Polite)
                .label(notice_words(notice))
                .w_full()
                .pt(9.0)
                .pb(9.0)
                .pl(12.0)
                .pr(10.0)
                .radius(crate::theme::tokens::radius::LG)
                .fill(t.text.alpha(0.06))
                .child(
                    rune::rich(notice_words(notice), Vec::new())
                        .key("notice-words")
                        .font(WORDS)
                        .color(t.text_secondary)
                        .w_full(),
                ),
        )
}

/// What stands for a list that is not here (`None`: it is, draw it): Loading while the first answer is out, Failed with
/// the reason (and Try again, where trying again can help) for the rest. Never a blank or an empty-looking list.
fn away(loaded: Loaded, t: &tokens::Theme) -> Option<El> {
    let failed = |title: &str, why: &str, retry: bool| {
        let retry = retry.then(|| {
            try_again("trash-failed", t).on_press(|s: &mut State| {
                s.frames.trash.notice = None;
                s.frames.trash.source.retry();
            })
        });
        state_body(
            ViewState::Failed {
                key: "trash-failed",
                title,
                why,
                retry,
            },
            t,
        )
    };
    match loaded {
        Loaded::Ready => None,
        Loaded::Loading => Some(state_body(
            ViewState::Loading {
                words: "Loading Trash\u{2026} Getting the list from Beebeeb.",
            },
            t,
        )),
        Loaded::Offline => Some(failed(
            "Can\u{2019}t reach Beebeeb",
            "Trash shows again once you\u{2019}re online.",
            true,
        )),
        Loaded::Failed => Some(failed(
            "Couldn\u{2019}t load Trash",
            "Beebeeb answered with an error.",
            true,
        )),
        Loaded::SessionEnded => Some(failed("Your session ended", "Sign in again to see Trash.", false)),
        Loaded::SignedOut => Some(failed("Signed out", "Sign in to see Trash.", false)),
    }
}

/// A refresh that failed while a list is shown: Failed with Try again, over the list that may be out of date.
fn refresh_failed(t: &tokens::Theme) -> El {
    state_body(
        ViewState::Failed {
            key: "trash-failed",
            title: "Couldn\u{2019}t refresh Trash",
            why: "What\u{2019}s shown may be out of date.",
            retry: Some(try_again("trash-failed", t).on_press(|s: &mut State| {
                s.frames.trash.notice = None;
                s.frames.trash.source.retry();
            })),
        },
        t,
    )
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
    el().key("banner-wrap")
        .w_full()
        .px(crate::theme::scale::space::S14)
        .pt(10.0)
        .child(
            col()
                .key("banner")
                .role("status")
                .label("Emptying Trash")
                .w_full()
                .pt(9.0)
                .pb(9.0)
                .pl(12.0)
                .pr(10.0)
                .gap(crate::theme::scale::space::S6)
                .radius(crate::theme::tokens::radius::LG)
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
    // Nothing answers for Trash yet (a build that ships before the engine lane wires it):
    // the shell's "not built yet" frame, never a list of sample rows.
    if !s.frames.trash.source.wired() {
        return crate::shell::placeholder::window(s, cx, crate::shell::registry::group_for(&s.shell.app));
    }
    let t = tokens::theme(cx.system_dark());
    let ui = &s.frames.trash;
    let items = ui.rows();
    let totals = ui.source.totals();
    let is_empty = totals.items == 0;
    // A list that is not here yet (loading, offline, signed out…) says so; never "Trash is empty" for it.
    let missing = if items.is_empty() {
        away(ui.source.loaded(), &t)
    } else {
        None
    };

    let emptying = ui.source.emptying();
    let sub = if missing.is_some() {
        String::new()
    } else if is_empty {
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
    let slot = if is_empty || missing.is_some() {
        Vec::new()
    } else if emptying.is_some() {
        // Already emptying (DW-0749b): the button waits.
        vec![button("Empty Trash\u{2026}", ButtonKind::GhostOff, None, &t)]
    } else {
        vec![
            button("Empty Trash\u{2026}", ButtonKind::GhostDanger, None, &t).on_press_tx(
                |s: &mut State, tx: &mut Tx| {
                    if s.frames.trash.source.purge_available() {
                        open_purge(s, Dialog::EmptyTrash, tx);
                    } else {
                        s.frames.trash.notice = Some(Notice::NotAvailable);
                    }
                },
            ),
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

    let body = if let Some(state) = missing {
        state
    } else if is_empty {
        state_body(
            ViewState::Empty {
                key: "trash-empty",
                icon: &icons::TRASH,
                title: "Trash is empty",
                body: "Files you delete stay here until you delete them for good.",
                action: None,
            },
            &t,
        )
    } else {
        // Only the rows in view are built (`virtual_rows`), from the one sorted copy of the answer.
        let list = rows("Trash", Vec::new()).child(
            col()
                .key("trash-rows")
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    items.len(),
                    ROW_H,
                    |s: &State, i| {
                        if i < s.frames.trash.rows().len() {
                            Cow::Owned(format!("row-{i}"))
                        } else {
                            Cow::Borrowed("row-oob")
                        }
                    },
                    move |s: &State, i| match s.frames.trash.rows().get(i) {
                        Some(r) => table_row(i, r, s.frames.trash.selected == Some(i), &COLUMNS, &t)
                            // The list is one Tab stop: the selected row, or the first.
                            .tab_index(if s.frames.trash.selected.unwrap_or(0) == i {
                                0
                            } else {
                                -1
                            })
                            // The place in the whole list, not in the rows built.
                            .position_in_set(i + 1, s.frames.trash.rows().len())
                            .on_press(move |s: &mut State| s.frames.trash.pick_row(i))
                            .on_secondary_press(move |s: &mut State, press, _| {
                                s.frames.trash.pick_row(i);
                                s.frames.trash.menu = Some(Open::at(press.x, press.y));
                            }),
                        None => el().key("empty"),
                    },
                ),
        );
        let mut body = col().key("list-body");
        if let Some(e) = emptying {
            body = body.child(emptying_note(e, &t));
        }
        body.child(header(&COLUMNS, &t)).child(list)
    };
    let body = match &ui.notice {
        // A refresh that failed with a list on screen: Failed with Try again over the list.
        Some(Notice::RefreshFailed) => col()
            .key("notice-body")
            .w_full()
            .grow(1.0)
            .child(refresh_failed(&t))
            .child(body),
        Some(notice) => col()
            .key("notice-body")
            .w_full()
            .grow(1.0)
            .child(notice_note(notice, &t))
            .child(body),
        None => body,
    };
    let window = cx.layout_size("window");
    chrome::window(
        s,
        cx,
        &t,
        bar,
        body,
        "Items in Trash are deleted for good after 30 days",
        ui.menu.as_ref().map(|open| {
            let (name, entries, w) = menu_of(open);
            menu(name, entries, open, w, window, &t, pick, close_menu, light, on_key)
        }),
    )
}

/// Trash's keys. A menu takes every key while it is open. Otherwise, in the list (or with
/// focus nowhere): the arrows, Home and End move the selection AND focus; Delete asks for the
/// permanent delete of the selected item; Alt+Enter asks for its details; Space previews it
/// ("nothing here opens (Preview works)", DW-0747); the Menu key or Shift+F10 opens the menu
/// of the SELECTED item under it. Nothing opens with Enter. In the search field, Enter asks
/// for the search.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if let Some(open) = s.frames.trash.menu.as_mut() {
        match open.on_key(ev, menu_of(open).1, tx) {
            Took::Pick(n) => pick(s, n, tx),
            Took::Close => close_menu(s, tx),
            Took::Key => {}
        }
        return true;
    }
    if ev.is_command('f') {
        tx.focus(crate::ui::keys::SEARCH_KEY, true);
        return true;
    }
    match tx.focused() {
        Some(key) if key == crate::ui::keys::SEARCH_KEY => {
            return crate::frames::shared::parts::toolbar::search_key(s, ev);
        }
        Some(key) if !key.starts_with("row-") => return false,
        _ => {}
    }
    let count = s.frames.trash.rows().len();
    if count == 0 {
        return false;
    }
    let ui = &mut s.frames.trash;
    if frames::is_menu_key(ev) {
        return match ui.selected {
            Some(i) => {
                ui.menu = Some(Open::under(format!("row-{i}")));
                true
            }
            None => false,
        };
    }
    if frames::is_alt_enter(ev) || frames::is_space(ev) {
        let Some(id) = ui.selected_id() else {
            return false;
        };
        if !ui.source.actions_available() {
            ui.notice = Some(Notice::NotAvailable);
            return true;
        }
        s.frames.intents.push(if frames::is_space(ev) {
            Intent::Do("preview", id)
        } else {
            Intent::About("DW-0411", id)
        });
        return true;
    }
    if !frames::plain(ev) {
        return false;
    }
    match ev.key {
        Key::Down => ui.select(ui.selected.map_or(0, |i| (i + 1).min(count - 1)), tx),
        Key::Up => ui.select(ui.selected.map_or(0, |i| i.saturating_sub(1)), tx),
        Key::Home => ui.select(0, tx),
        Key::End => ui.select(count - 1, tx),
        Key::PageDown | Key::PageUp => {
            let to = crate::frames::shared::parts::table::page_to(ui.selected, count, ev.key == Key::PageDown);
            ui.select(to, tx)
        }
        Key::Text(_) => {
            let rows = ui.rows();
            match crate::frames::shared::parts::table::jump_to(rows.iter().map(|r| r.name.as_str()), ui.selected, ev) {
                Some(i) => ui.select(i, tx),
                None => return false,
            }
        }
        Key::Delete => match ui.selected_id() {
            Some(_) if !ui.source.purge_available() => ui.notice = Some(Notice::NotAvailable),
            Some(id) => open_purge(s, Dialog::DeleteForever(id), tx),
            None => return false,
        },
        _ => return false,
    }
    true
}
