//! What this section's frames asked of the shell (w17-search-actions, audit SE1).
//!
//! - `Frame("DW-0404")`: Sort opens its menu (Files' entries, `files::menus::sort_menu`); a choice orders the results in
//!   Search's own state (`SearchUi::sort`), never Files' folder sort. `Frame("DW-1102")`: the results as a grid.
//! - `Frame("DW-0414")`: New opens Files' New menu, as This PC's does (Files shows while its menu is open).
//! - `About("DW-0407" | "DW-0413", id)`: Share… and Version history for the result, through Files' dialogs
//!   (`begin_share_for`, `begin_versions_for`); the dialog stands over the results.
//! - `Do("keep-offline", id)`: Available offline, through `FilesSource::keep_offline`.
//! - `About("DW-0402", id)`: the result's menu (the Menu key, a right-click): Open, Show in Files, Share…, Version
//!   history…, Details.
//! - Open (`OpenItem`) and Details (`DW-0411`) are the shell's and `frames::item`'s.

use std::time::{Duration, Instant};

use rune::{Key, KeyEvent, Tx};

use super::source::Hit;
use std::sync::Arc;

use super::{Shows, close, hits, open_result};
use crate::State;
use crate::frames::Intent;
use crate::frames::files::menus;
use crate::frames::files::source::{Action, ActionError};
use crate::frames::files::{MenuKind, Sort, SortBy, Toast};
use crate::frames::shared::parts::table::Kind;
use crate::source::DriveItem;
use crate::theme::tokens::Theme;
use crate::ui::menu::{self, Entry};

/// How long a notice stays.
const NOTICE_FOR: Duration = Duration::from_secs(4);

/// What a menu of this window is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOf {
    Sort,
    Result(String),
}

/// The open menu: what it is for, the highlighted entry, and (a result's) where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub of: MenuOf,
    pub hot: Option<usize>,
    pub at: (f32, f32),
}

/// Takes `intent` when it is this section's; `false`: not taken (the router logs it as `NotWired`).
pub fn take(s: &mut State, tx: &mut Tx, intent: &Intent) -> bool {
    match intent {
        Intent::Frame("DW-0404") => toggle_sort(s),
        Intent::Frame("DW-1102") => s.frames.find.grid = true,
        Intent::Frame("DW-0414") => {
            s.frames.find.menu = None;
            s.toggle_menu(MenuKind::New);
        }
        Intent::About("DW-0407", id) => return share(s, tx, id),
        Intent::About("DW-0413", id) => return versions(s, tx, id),
        Intent::Do("keep-offline", id) => return keep_offline(s, tx, id),
        Intent::About("DW-0402", id) => return open_result_menu(s, id),
        _ => return false,
    }
    true
}

/// The results as the window shows them: the source's, in the chosen order.
pub fn shows(s: &State) -> Shows {
    match s.frames.find.shows(&s.search.text) {
        Shows::Results { hits, loading } => Shows::Results {
            hits: sorted(s, hits),
            loading,
        },
        other => other,
    }
}

/// `hits` in the chosen order (the source's own when none was chosen).
pub fn sorted(s: &State, hits: Arc<[Hit]>) -> Arc<[Hit]> {
    let Some(sort) = s.frames.find.sort else {
        return hits;
    };
    let facts = |hit: &Hit| {
        let known = s.actions.source.item_of(&hit.id);
        let item = known
            .clone()
            .unwrap_or_else(|| crate::frames::item::display_item(&hit.id, &hit.name, hit.kind == Kind::Folder, hit.pc));
        let changed = known
            .map(|k| k.modified_at)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| hit.changed.clone());
        (
            hit.name.to_lowercase(),
            crate::frames::files::source::kind_of(&item),
            item.size.unwrap_or(0),
            changed,
        )
    };
    let mut keyed: Vec<_> = hits.iter().map(|hit| (facts(hit), hit.clone())).collect();
    keyed.sort_by(|(a, ha), (b, hb)| {
        let (fa, fb) = (ha.kind == Kind::Folder, hb.kind == Kind::Folder);
        if sort.folders_first && fa != fb {
            return fb.cmp(&fa);
        }
        let order = match sort.by {
            SortBy::Name => a.0.cmp(&b.0),
            SortBy::Type => a.1.cmp(b.1).then_with(|| a.0.cmp(&b.0)),
            SortBy::Size => a.2.cmp(&b.2).then_with(|| a.0.cmp(&b.0)),
            SortBy::DateChanged => a.3.cmp(&b.3).then_with(|| a.0.cmp(&b.0)),
        };
        if sort.reversed { order.reverse() } else { order }
    });
    keyed.into_iter().map(|(_, hit)| hit).collect::<Vec<_>>().into()
}

/// The result as Files' actions take it: Files' own item when it knows it, else one made from the row.
fn item_of(s: &State, id: &str) -> Option<DriveItem> {
    let hit = super::hit(s, id)?;
    Some(
        s.actions
            .source
            .item_of(id)
            .unwrap_or_else(|| crate::frames::item::display_item(&hit.id, &hit.name, hit.kind == Kind::Folder, hit.pc)),
    )
}

fn say(s: &mut State, tx: &mut Tx, words: impl Into<String>) {
    s.frames.find.notice = Some((words.into(), Instant::now()));
    tx.wake_after(NOTICE_FOR);
}

/// The notice, while it is fresh.
pub fn notice(s: &State) -> Option<&str> {
    s.frames
        .find
        .notice
        .as_ref()
        .filter(|(_, at)| at.elapsed() < NOTICE_FOR)
        .map(|(words, _)| words.as_str())
}

fn share(s: &mut State, tx: &mut Tx, id: &str) -> bool {
    let Some(item) = item_of(s, id) else { return false };
    s.frames.find.menu = None;
    if s.begin_share_for(vec![item]) {
        s.frames.find.dialog_over = true;
    } else {
        say(s, tx, Toast::failure(Action::Share, ActionError::Protected));
    }
    true
}

fn versions(s: &mut State, tx: &mut Tx, id: &str) -> bool {
    let Some(item) = item_of(s, id) else { return false };
    s.frames.find.menu = None;
    if s.begin_versions_for(item) {
        s.frames.find.dialog_over = true;
    } else {
        say(s, tx, "A folder has no version history.");
    }
    true
}

/// Available offline: the other way round from what the result is now.
fn keep_offline(s: &mut State, tx: &mut Tx, id: &str) -> bool {
    let Some(item) = item_of(s, id) else { return false };
    let kept = s
        .frames
        .find
        .kept
        .get(id)
        .copied()
        .or_else(|| s.frames.find.source.details(id).map(|d| d.kept_offline))
        .unwrap_or(false);
    let keep = !kept;
    match s.actions.source.keep_offline(std::slice::from_ref(&item), keep) {
        Ok(()) => {
            s.frames.find.kept.insert(id.to_string(), keep);
        }
        Err(why) => {
            let action = if keep {
                Action::KeepOffline
            } else {
                Action::RemoveOffline
            };
            say(s, tx, Toast::failure(action, why));
        }
    }
    true
}

fn toggle_sort(s: &mut State) {
    let open = matches!(s.frames.find.menu, Some(Menu { of: MenuOf::Sort, .. }));
    s.frames.find.menu = (!open).then_some(Menu {
        of: MenuOf::Sort,
        hot: None,
        at: (0.0, 0.0),
    });
}

/// The result's menu, at the pointer when it asked for it, else beside the result.
fn open_result_menu(s: &mut State, id: &str) -> bool {
    let all = hits(s);
    let Some(i) = all.iter().position(|h| h.id == id) else {
        return false;
    };
    s.frames.find.selected = Some(id.to_string());
    let list = result_entries(s, id);
    let (w, h) = (menus::FILE_MENU_W, menu::height(&list));
    let (win_w, win_h) = match s.actions.window {
        (w, h) if w > 0.0 && h > 0.0 => (w, h),
        _ => (1278.0, 798.0),
    };
    let at = s.frames.find.menu_at.take().unwrap_or((320.0, 150.0 + 30.0 * i as f32));
    let at = (at.0.min(win_w - w - 8.0).max(8.0), at.1.min(win_h - h - 8.0).max(8.0));
    s.frames.find.menu = Some(Menu {
        of: MenuOf::Result(id.to_string()),
        hot: None,
        at,
    });
    true
}

fn result_entries(s: &State, id: &str) -> Vec<Entry> {
    let folder = item_of(s, id).is_some_and(|i| i.is_folder);
    vec![
        Entry::keyed("open", "Open", "Enter"),
        Entry::item("show-in-files", "Show in Files"),
        Entry::Sep,
        Entry::item("share", "Share\u{2026}"),
        Entry::item("versions", "Version history\u{2026}").dimmed(folder),
        Entry::Sep,
        Entry::keyed("details", "Details", "Alt+Enter"),
    ]
}

fn entries(s: &State, of: &MenuOf) -> Vec<Entry> {
    match of {
        MenuOf::Sort => menus::sort_menu(s.frames.find.sort.unwrap_or_default()),
        MenuOf::Result(id) => result_entries(s, id),
    }
}

/// Run entry `index` of the open menu (a dimmed one does nothing).
pub fn pick(s: &mut State, index: usize, tx: &mut Tx) {
    let Some(open) = s.frames.find.menu.clone() else { return };
    let list = entries(s, &open.of);
    let Some(id) = list.get(index).filter(|e| !e.is_off()).and_then(|e| e.id()) else {
        return;
    };
    let MenuOf::Result(result) = open.of else {
        // Sort's choices keep the menu open, as a menu of ticks does.
        return pick_sort(s, id);
    };
    s.frames.find.menu = None;
    match id {
        "open" => {
            let all = hits(s);
            if let Some(at) = all.iter().position(|h| h.id == result) {
                open_result(s, &all, at);
            }
        }
        "show-in-files" => show_in_files(s, tx, &result),
        "share" => {
            share(s, tx, &result);
        }
        "versions" => {
            versions(s, tx, &result);
        }
        "details" => {
            crate::frames::item::open_details(s, &result, tx);
        }
        _ => {}
    }
}

/// Files on the result's folder with it selected: search ends first, so Files shows.
fn show_in_files(s: &mut State, tx: &mut Tx, id: &str) {
    if s.actions.source.item_of(id).is_none() {
        return say(s, tx, "Couldn't find it in Files.");
    }
    close(s, tx);
    crate::frames::files::reveal::reveal(s, tx, id);
}

fn pick_sort(s: &mut State, id: &str) {
    let sort = s.frames.find.sort.unwrap_or_default();
    let next = match id {
        "sort-name" => Sort {
            by: SortBy::Name,
            ..sort
        },
        "sort-date" => Sort {
            by: SortBy::DateChanged,
            ..sort
        },
        "sort-size" => Sort {
            by: SortBy::Size,
            ..sort
        },
        "sort-type" => Sort {
            by: SortBy::Type,
            ..sort
        },
        "order-forward" => Sort {
            reversed: false,
            ..sort
        },
        "order-reversed" => Sort { reversed: true, ..sort },
        "folders-first" => Sort {
            folders_first: !sort.folders_first,
            ..sort
        },
        _ => return,
    };
    s.frames.find.sort = Some(next);
}

fn hover(s: &mut State, index: usize) {
    if let Some(open) = s.frames.find.menu.as_mut() {
        open.hot = Some(index);
    }
}

fn close_menu(s: &mut State, tx: &mut Tx) {
    s.frames.find.menu = None;
    match super::selected_row_key(s) {
        Some(key) => tx.focus(key, false),
        None => tx.focus(crate::ui::keys::SEARCH_KEY, false),
    }
}

/// The arrows, Home, End, Enter and Esc of the open menu (heard on the menu itself: it is modal).
fn menu_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(open) = s.frames.find.menu.clone() else {
        return false;
    };
    let list = entries(s, &open.of);
    let to = |s: &mut State, hot: Option<usize>| {
        if let Some(m) = s.frames.find.menu.as_mut() {
            m.hot = hot;
        }
    };
    match ev.key {
        Key::Down => to(s, menu::step(&list, open.hot, 1)),
        Key::Up => to(s, menu::step(&list, open.hot, -1)),
        Key::Home => to(s, menu::step(&list, None, 1)),
        Key::End => to(s, menu::step(&list, None, -1)),
        Key::Enter => match open.hot {
            Some(at) => pick(s, at, tx),
            None => return false,
        },
        Key::Escape => close_menu(s, tx),
        _ => return false,
    }
    true
}

/// The open menu as a layer over the window: Sort under its button (as Files', 262 pt from the right edge), a
/// result's at its point.
pub fn menu_layer(s: &State, t: &Theme) -> Option<rune::El> {
    let open = s.frames.find.menu.as_ref()?;
    let list = entries(s, &open.of);
    let (name, width) = match open.of {
        MenuOf::Sort => ("Sort", menus::SORT_MENU_W),
        MenuOf::Result(_) => ("File", menus::FILE_MENU_W),
    };
    let surface = menu::menu("menu", name, width, &list, open.hot, t, pick, hover, close_menu).on_key(menu_key);
    Some(match open.of {
        MenuOf::Sort => rune::row()
            .key("menu-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .pt(112.0)
            .pr(226.0 + 36.0)
            .justify_end()
            .items_start()
            .child(surface),
        MenuOf::Result(_) => surface.absolute(open.at.0, open.at.1),
    })
}
