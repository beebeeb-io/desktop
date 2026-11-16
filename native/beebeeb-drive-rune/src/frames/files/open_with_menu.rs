//! DW-1562: Open With, for a Word, Excel or PowerPoint file the Office viewer shows (decision D-OV-14). The file
//! menu's first line reads "Open (Beebeeb)" (the viewer, as Open), and "Open With ›" follows it, with beside it:
//! Beebeeb, the apps Windows reports for the type, and Other… (Windows' own chooser).
//!
//! - Only for ONE selected file the viewer shows (`kinds::office_route` → `Shows`). Several files, a folder, another
//!   type, or the Office switches off: the file menu is as it was.
//! - An app, or Other…, opens only the copy KEPT on this PC: nothing is fetched, decrypted or copied for another app
//!   (D-OV-13, 2482 Q8). A file that is not kept says "Keep this file on this PC first." and opens nothing.
//! - Beebeeb is never made the default app for a type: no line here registers anything.

use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use rune::{El, Key, KeyEvent, Tx};

use super::source::Action;
use super::{MenuKind, Toast};
use crate::State;
use crate::frames::viewers::kinds::{self, OfficeShips};
use crate::source::DriveItem;
use crate::theme::tokens::Theme;
use crate::ui::menu::{self, Entry, SUBMENU};

/// The submenu's width (the board's second `.wn-menu` of DW-1562).
pub const SUB_W: f32 = 230.0;
/// Where the submenu stands from the file menu's corner: 232 pt in (over the menu's right edge), 24 pt down.
const SUB_AT: (f32, f32) = (232.0, 24.0);
/// "Open With" in the file menu: right after "Open (Beebeeb)".
pub const OPEN_WITH: usize = 1;
/// The file menu's entry that shows the submenu.
pub const OPEN_WITH_ID: &str = "open-with";
const BEEBEEB_ID: &str = "open-with-beebeeb";
const OTHER_ID: &str = "open-with-other";
/// The apps' lines, by their place in the list Windows reported (at most 8, `FilesSource::apps_for`).
const APP_IDS: [&str; 8] = [
    "open-with-app-0",
    "open-with-app-1",
    "open-with-app-2",
    "open-with-app-3",
    "open-with-app-4",
    "open-with-app-5",
    "open-with-app-6",
    "open-with-app-7",
];

/// An app Windows reports for a type, as Open With lists it (`FilesSource::apps_for`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppChoice {
    /// The handler's own name, as Windows gives it: opaque, only ever handed back to find the same handler.
    pub id: String,
    /// What the menu says: "Microsoft Word".
    pub label: String,
}

/// The apps the board's PC reports for a type (DW-1562: Microsoft Word for "Service agreement.docx"), the fixture's
/// answer (`FilesFixture`). The board draws no other type's list, so no other type has one.
pub(super) fn board_apps(name: &str) -> Vec<AppChoice> {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("docx" | "docm" | "dotx" | "dotm" | "doc" | "dot") => vec![AppChoice {
            id: "WINWORD.EXE".into(),
            label: "Microsoft Word".into(),
        }],
        _ => Vec::new(),
    }
}

/// What Open With keeps between frames (`Actions::open_with`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ui {
    /// The Office switches a board scene (DW-1562) or a test turns on: the viewer on, these types shipped. `None`: the
    /// switches as they ship (`kinds::OFFICE_VIEWER_ON`, `kinds::OFFICE_SHIPS`). Nothing in the app sets it.
    pub board_office: Option<OfficeShips>,
    /// The submenu's line the keyboard is on (Right-arrow went into it); `None`: the keyboard is in the file menu.
    pub line: Option<usize>,
}

/// Does the Office viewer show a file of this name (`kinds::office_route(name)` is `Some(OfficeRoute::Shows(_))`)?
/// Under a board scene's switches when it set them.
pub fn viewer_shows(s: &State, name: &str) -> bool {
    let route = match s.actions.open_with.board_office {
        Some(ships) => kinds::office_route_with(name, true, ships),
        None => kinds::office_route(name),
    };
    matches!(route, Some(kinds::OfficeRoute::Shows(_)))
}

/// The one file Open With is for: a single selected file the viewer shows, outside Backups and the service folders
/// (their menu is Open, Preview, Show in File Explorer and Details only: `menus::file_menu`). `None`: the menu is as
/// before, and no submenu is drawn or picked.
pub fn offered_for(s: &State) -> Option<DriveItem> {
    match &s.selected_items()[..] {
        [only] if !only.is_folder && !only.is_protected && !s.protects_selection() && viewer_shows(s, &only.name) => {
            Some(only.clone())
        }
        _ => None,
    }
}

/// The file menu with Open With (DW-1562): Open reads "Open (Beebeeb)", and "Open With ›" follows it. Unchanged for
/// any other selection. The apps are asked here, when the menu opens, so they are there when Open With is pointed at.
pub fn add_to(s: &State, entries: &mut Vec<Entry>) {
    let Some(item) = offered_for(s) else { return };
    let _ = s.actions.source.apps_for(&item);
    let Some(open) = entries.iter_mut().find(|entry| entry.id() == Some("open")) else {
        return;
    };
    *open = Entry::keyed("open", open_label(s.identity.product_name()), "Enter");
    entries.insert(OPEN_WITH, Entry::keyed(OPEN_WITH_ID, "Open With", SUBMENU));
}

/// "Open (<the app's name>)". The name is the identity's the window was handed (a test build says its own), never a
/// literal here. Made once per name, and there are two.
fn open_label(name: &'static str) -> &'static str {
    static MADE: Mutex<Vec<(&'static str, &'static str)>> = Mutex::new(Vec::new());
    let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, label)) = made.iter().find(|(made_for, _)| *made_for == name) {
        return label;
    }
    let label: &'static str = Box::leak(format!("Open ({name})").into_boxed_str());
    made.push((name, label));
    label
}

/// The submenu: Beebeeb (the viewer), the apps Windows reports, Other… (the board's lines and separators).
pub fn entries(s: &State) -> Vec<Entry> {
    let apps = offered_for(s)
        .map(|item| s.actions.source.apps_for(&item))
        .unwrap_or_default();
    let mut out = vec![Entry::item(BEEBEEB_ID, s.identity.product_name()), Entry::Sep];
    for (id, app) in APP_IDS.into_iter().zip(&apps) {
        out.push(Entry::item(id, words(&app.label)));
    }
    if !apps.is_empty() {
        out.push(Entry::Sep);
    }
    out.push(Entry::item(OTHER_ID, "Other\u{2026}"));
    out
}

/// An app's words as a menu holds words (`&'static str`): each distinct label is kept once for the run of the app.
/// Bounded by what Windows reports: at most 8 apps per type, asked once per type.
fn words(label: &str) -> &'static str {
    static HELD: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(&kept) = held.get(label) {
        return kept;
    }
    let kept: &'static str = Box::leak(label.to_owned().into_boxed_str());
    held.insert(kept);
    kept
}

/// Where the submenu stands (window points), while "Open With" is the file menu's highlighted entry.
pub fn at(s: &State) -> Option<(f32, f32)> {
    let open = s.actions.menu?;
    if open.kind != MenuKind::File || open.hot != Some(OPEN_WITH) || offered_for(s).is_none() {
        return None;
    }
    // Only beside the menu's own "Open With" line (never over another menu's line at that place).
    if super::menus::file_menu(s).get(OPEN_WITH).and_then(Entry::id) != Some(OPEN_WITH_ID) {
        return None;
    }
    let (win_w, _) = s.actions.window;
    let x = open.at.0 + SUB_AT.0;
    // Where there is no room at the right, it stands left of the menu.
    let x = if x + SUB_W + 8.0 > win_w {
        (open.at.0 - SUB_W + 4.0).max(8.0)
    } else {
        x
    };
    Some((x, open.at.1 + SUB_AT.1))
}

/// The submenu beside the file menu, inside it (a press on it is a press inside the menu's modal). Keyed `menu-1`, as
/// the board names the second menu.
pub fn flyout(s: &State, t: &Theme) -> Option<El> {
    let (x, y) = at(s)?;
    let corner = s.actions.menu?.at;
    Some(
        menu::flyout_hot(
            "menu-1",
            "Open With",
            SUB_W,
            &entries(s),
            line(s),
            t,
            pick,
            hover,
            close,
        )
        .absolute(x - corner.0, y - corner.1),
    )
}

fn pick(s: &mut State, index: usize, tx: &mut Tx) {
    s.pick_open_with(index, tx);
}

fn close(s: &mut State, tx: &mut Tx) {
    s.close_menu_tx(tx);
}

/// The pointer over a line of the submenu: it is the highlighted one (the keyboard goes on from it).
fn hover(s: &mut State, index: usize) {
    s.actions.open_with.line = Some(index);
}

/// The submenu's line the keyboard (or the pointer) is on, while the submenu is drawn.
fn line(s: &State) -> Option<usize> {
    at(s).and(s.actions.open_with.line)
}

/// Put the keyboard on the submenu's line `index`.
fn focus_line(s: &mut State, lines: &[Entry], index: usize, tx: &mut Tx) {
    s.actions.open_with.line = Some(index);
    if let Some(key) = menu::item_key("menu-1", lines, index) {
        tx.focus(key, false);
    }
}

/// Open With's keys in the file menu (`keys.rs`, before the menu's own):
/// - on "Open With": Right-arrow opens the submenu with its first line;
/// - in the submenu: Up, Down, Home, End move; Enter picks; Left-arrow and Escape go back to "Open With";
/// - Tab: from "Open With" into the submenu, then from line to line (wrapping); Shift+Tab back, and from the submenu's
///   first line to "Open With".
///
/// `None`: not Open With's key (the file menu's own keys go on as before).
pub fn key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> Option<bool> {
    let open = s.actions.menu?;
    if open.kind != MenuKind::File {
        return None;
    }
    let Some(here) = line(s) else {
        let on_open_with = open.hot == Some(OPEN_WITH) && at(s).is_some();
        let goes_in = ev.key == Key::Right || (ev.key == Key::Tab && !ev.mods.shift);
        return (goes_in && on_open_with).then(|| {
            s.enter_open_with(tx);
            true
        });
    };
    let lines = entries(s);
    let first = menu::step(&lines, None, 1);
    let next = match ev.key {
        Key::Tab if ev.mods.shift && Some(here) == first => return Some(s.leave_open_with(tx)),
        Key::Tab if ev.mods.shift => menu::step(&lines, Some(here), -1),
        Key::Tab => menu::step(&lines, Some(here), 1),
        Key::Down => menu::step(&lines, Some(here), 1),
        Key::Up => menu::step(&lines, Some(here), -1),
        Key::Home => menu::step(&lines, None, 1),
        Key::End => menu::step(&lines, None, -1),
        Key::Enter => {
            s.pick_open_with(here, tx);
            return Some(true);
        }
        Key::Left | Key::Escape => return Some(s.leave_open_with(tx)),
        Key::Right => return Some(true),
        _ => return None,
    };
    if let Some(next) = next {
        focus_line(s, &lines, next, tx);
    }
    Some(true)
}

impl State {
    /// "Open With" picked in the file menu: the menu stays, its apps beside it. Picked with Enter, the keyboard goes
    /// into the submenu, on its first line (as Right-arrow does).
    pub fn show_open_with(&mut self, tx: &mut Tx) {
        if let Some(open) = self.actions.menu.as_mut() {
            open.hot = Some(OPEN_WITH);
        }
        if tx.by_keys() {
            self.enter_open_with(tx);
        }
    }

    /// The keyboard out of the submenu, back on "Open With" (the submenu stays drawn beside it). Always handled.
    fn leave_open_with(&mut self, tx: &mut Tx) -> bool {
        self.actions.open_with.line = None;
        let menu_lines = self.menu_entries(MenuKind::File);
        if let Some(key) = menu::item_key("menu", &menu_lines, OPEN_WITH) {
            tx.focus(key, false);
        }
        true
    }

    /// The keyboard into the submenu, on its first line.
    pub fn enter_open_with(&mut self, tx: &mut Tx) {
        if let Some(open) = self.actions.menu.as_mut() {
            open.hot = Some(OPEN_WITH);
        }
        let lines = entries(self);
        if let Some(first) = menu::step(&lines, None, 1) {
            focus_line(self, &lines, first, tx);
        }
    }

    /// Run the line `index` of Open With's submenu; the menu closes.
    pub fn pick_open_with(&mut self, index: usize, tx: &mut Tx) {
        let lines = entries(self);
        let Some(id) = lines
            .get(index)
            .filter(|line| !line.is_off())
            .and_then(|line| line.id())
        else {
            return;
        };
        let Some(item) = offered_for(self) else { return };
        self.close_menu_tx(tx);
        if id == BEEBEEB_ID {
            // The viewer: what Open (Beebeeb) does.
            self.open_selection(tx);
            return;
        }
        if self.needs_session(tx) {
            return;
        }
        // Only the copy kept on this PC goes to another app: nothing is fetched or decrypted for it.
        if !self.selection_is_kept() {
            self.show_toast(Toast::KeepFirst, tx);
            return;
        }
        let answer = if id == OTHER_ID {
            self.actions.source.open_with(&item)
        } else {
            let Some(place) = APP_IDS.iter().position(|app| *app == id) else {
                return;
            };
            // The app at that place of the list the submenu showed (held per type for the run: the same list).
            let Some(app) = self.actions.source.apps_for(&item).into_iter().nth(place) else {
                return;
            };
            self.actions.source.open_in_app(&item, &app)
        };
        if let Err(why) = answer {
            self.failed(Action::OpenWith, why, tx);
        }
    }
}
