//! The sheet tabs under the grid. No "+": a viewer adds no sheet. Live, the tabs that do not fit the bar go behind a
//! "more" control at its end, with the sheets the renderer could not name (`More::unnamed`); its menu lists the hidden
//! tabs (a press opens that sheet) and says how many sheets are not listed. The open sheet always has its tab in the bar.
//! The board draws three tabs that fit, so it draws no "more" control: its look here (a plain tab's words, "N more") is
//! this file's, named in the branch notes.
//!
//! An open workbook (`sheet_tabs_edit`): a double press on a tab renames the sheet (a text field in its place; Enter
//! sends one `RenameSheet`, Escape keeps the name), and a dimmed "+" says adding sheets is not there yet.

use std::cell::UnsafeCell;

use beebeeb_office_protocol::office::EditOp;
use rune::{Color, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text_input};

use super::sheets_scroll::follow;
use super::{Cell, SheetView, TAB, TAB_ON, TABS_H, fitted};
use crate::State;
use crate::frames::viewers::{self, source::Body};
use crate::theme::tokens::Theme;
use crate::ui::kit;

/// A tab's padding either side, the gap between tabs and the bar's own padding.
const TAB_PAD: f32 = 12.0;
const GAP: f32 = 2.0;
const BAR_PAD: f32 = 8.0;
/// The menu's width.
const MENU_W: f32 = 220.0;

fn tab_w(name: &str, on: bool) -> f32 {
    rune::text::advance(name, if on { TAB_ON } else { TAB }) + 2.0 * TAB_PAD
}

fn more_words(n: u32) -> String {
    format!("{n} more")
}

/// Which tabs are drawn in `room` points (the bar less its padding), in order, and which go behind "more". The open
/// sheet's tab is always drawn: it takes the last place when it would be hidden.
pub fn shown_tabs(view: &SheetView, room: f32) -> (Vec<usize>, Vec<usize>) {
    let n = view.tabs.len();
    let widths: Vec<f32> = view
        .tabs
        .iter()
        .enumerate()
        .map(|(i, name)| tab_w(name, i == view.active))
        .collect();
    let all = widths.iter().sum::<f32>() + GAP * n.saturating_sub(1) as f32;
    if all <= room && view.more.unnamed == 0 {
        return ((0..n).collect(), Vec::new());
    }
    // Room kept for the control, at its widest words (every sheet behind it).
    let total = u32::try_from(n).unwrap_or(u32::MAX).saturating_add(view.more.unnamed);
    let keep = tab_w(&more_words(total), false) + GAP;
    let mut shown: Vec<usize> = Vec::new();
    let mut used = 0.0;
    for (i, w) in widths.iter().enumerate() {
        let w = w + if shown.is_empty() { 0.0 } else { GAP };
        if used + w + keep > room {
            break;
        }
        used += w;
        shown.push(i);
    }
    if view.active < n && !shown.contains(&view.active) {
        while let Some(&last) = shown.last() {
            if used + GAP + widths[view.active] + keep <= room {
                break;
            }
            used -= widths[last] + if shown.len() > 1 { GAP } else { 0.0 };
            shown.pop();
        }
        shown.push(view.active);
    }
    let hidden = (0..n).filter(|i| !shown.contains(i)).collect();
    (shown, hidden)
}

/// What the "+" says.
pub const ADD_WORDS: &str = "Adding sheets isn't available in this editor yet";
/// The longest name a sheet has, and the characters it cannot have.
const NAME_MAX: usize = 31;
const NAME_BAD: [char; 7] = [':', '\\', '/', '?', '*', '[', ']'];

struct Renaming {
    /// The tab being renamed.
    tab: Option<usize>,
    edit: TextEdit,
}

thread_local! {
    /// The tab being renamed and its text. Rune's text field reaches its `TextEdit` through a function of the app
    /// state, which has no place for one of this file's; it stays with the window's thread (as `edit/sheet.rs`'s grid).
    static RENAME: UnsafeCell<Renaming> = UnsafeCell::new(Renaming { tab: None, edit: TextEdit::new("") });
}

fn with_rename<R>(f: impl FnOnce(&mut Renaming) -> R) -> R {
    // SAFETY: thread-local, and every use is one short call of `f`, which never calls `with_rename` again.
    RENAME.with(|r| f(unsafe { &mut *r.get() }))
}

/// The tab being renamed, if one is.
pub fn renaming() -> Option<usize> {
    with_rename(|r| r.tab)
}

/// The words typed so far (a copy).
pub fn typed() -> String {
    with_rename(|r| r.edit.text.clone())
}

/// Drop a rename in progress (the file closed, or the bar is not editable).
pub fn reset() {
    with_rename(|r| {
        if r.tab.is_some() || !r.edit.text.is_empty() {
            r.edit.clear();
            r.edit.focused = false;
        }
        r.tab = None;
    });
}

fn rename_edit(_: &mut State) -> &mut TextEdit {
    // SAFETY: as `with_rename`: Rune takes this borrow for one edit and lets it go before it calls anything of ours.
    RENAME.with(|r| unsafe { &mut (*r.get()).edit })
}

fn say(s: &mut State, words: &str) {
    if let Some(ui) = viewers::ui(s) {
        ui.toast = Some(words.to_string());
    }
}

/// Why `name` cannot be a sheet's name, in words; `None`: it can. `others`: the other sheets' names.
pub fn name_problem(name: &str, others: &[&str]) -> Option<&'static str> {
    if name.is_empty() {
        return Some("A sheet needs a name");
    }
    if name.chars().count() > NAME_MAX {
        return Some("A sheet name has at most 31 characters");
    }
    if name.contains(NAME_BAD) {
        return Some("A sheet name can't have : \\ / ? * [ ]");
    }
    if others.iter().any(|o| o.eq_ignore_ascii_case(name) || o.to_lowercase() == name.to_lowercase()) {
        return Some("Another sheet has that name");
    }
    None
}

/// Start renaming tab `i`: its field holds the name.
fn begin_rename(s: &mut State, tx: &mut Tx, i: usize) {
    let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body) else {
        return;
    };
    let Some(name) = view.tabs.get(i).cloned() else { return };
    view.more.open = false;
    with_rename(|r| {
        r.edit = TextEdit::new(name);
        r.edit.focused = true;
        r.tab = Some(i);
    });
    tx.instant();
    tx.focus(RENAME_KEY, false);
}

/// Keep the typed name: one `RenameSheet`. A name that is not one stays in the field, with words.
fn commit_rename(s: &mut State, tx: &mut Tx) {
    let Some(i) = renaming() else { return };
    let typed = typed();
    let name = typed.trim().to_string();
    let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body) else {
        reset();
        return;
    };
    let was = view.tabs.get(i).cloned().unwrap_or_default();
    if name == was {
        reset();
        return;
    }
    let others: Vec<&str> = view
        .tabs
        .iter()
        .enumerate()
        .filter(|(n, _)| *n != i)
        .map(|(_, t)| t.as_str())
        .collect();
    if let Some(words) = name_problem(&name, &others) {
        say(s, words);
        return;
    }
    let sheet = u32::try_from(i).unwrap_or(u32::MAX);
    tx.instant();
    let taken = match viewers::ui(s) {
        Some(ui) => {
            let (source, id) = ui.edit_source();
            let id = id.to_string();
            source.edit(&id, EditOp::RenameSheet { sheet, name: name.clone() })
        }
        None => false,
    };
    if !taken {
        say(s, "Renaming a sheet isn't available in this editor yet");
        reset();
        return;
    }
    if let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body)
        && let Some(tab) = view.tabs.get_mut(i)
    {
        *tab = name;
    }
    reset();
}

fn rename_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match ev.key {
        Key::Escape => {
            reset();
            true
        }
        Key::Enter => {
            commit_rename(s, tx);
            true
        }
        _ => false,
    }
}

/// The text field a tab becomes while it is renamed.
const RENAME_KEY: &str = "edit-sheet-tab-name";

/// A press that opens sheet `i`: its tab, its first row and column in view, the menu closed, the viewports at the top.
fn open_sheet(i: usize) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        if let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body) {
            view.active = i;
            view.sheet = u32::try_from(i).unwrap_or(u32::MAX);
            view.top_left = Cell { row: 0, col: 0 };
            view.more.open = false;
            follow(view, tx);
        }
    }
}

fn set_more_open(s: &mut State, open: bool) {
    if let Some(Body::Sheet(view)) = viewers::ui(s).map(|ui| &mut ui.file.body) {
        view.more.open = open;
    }
}

fn tab(view: &SheetView, i: usize, live: bool, editing: bool, t: &Theme) -> El {
    let name = &view.tabs[i];
    let on = i == view.active;
    if editing && renaming() == Some(i) {
        return text_input(&with_rename(|r| r.edit.clone()), TAB_ON)
            .key(RENAME_KEY)
            .label("Sheet name")
            .single_line()
            .color(t.text)
            .caret(t.accent_deep, t.selection)
            .on_edit(rename_edit)
            .on_input_key(rename_key)
            .h(22.0)
            .px(TAB_PAD)
            .w(tab_w(&typed(), true).max(80.0))
            .radius(5.0)
            .fill(t.paper)
            .border(2.0, t.text)
            .border_layout();
    }
    // The open sheet is a pill: the content fill, a hairline ring, 22 high, 5 round. The others are plain words.
    let mut tab = el()
        .key(format!("sh-tab-{i}"))
        .role("tab")
        .label(name.clone())
        .checked(on)
        .h(22.0)
        .px(TAB_PAD)
        .center()
        .radius(5.0)
        .child(fitted(
            format!("sh-tab-{i}-t"),
            name,
            if on { TAB_ON } else { TAB },
            if on { t.text } else { t.text_tertiary },
        ));
    if on {
        tab = tab.fill(t.paper).border(kit::HAIR, t.separator).border_layout();
    }
    if live && !on {
        tab = tab
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(open_sheet(i));
    }
    if editing {
        tab = tab
            .focusable(true)
            .on_double_press_tx(move |s: &mut State, tx: &mut Tx| begin_rename(s, tx, i));
    }
    tab
}

/// The menu over the bar's right end: the hidden tabs, then the sheets not listed.
fn menu(view: &SheetView, hidden: &[usize], t: &Theme) -> El {
    let mut list = col()
        .key("sh-more-menu")
        .role("menu")
        .label("More sheets")
        .pin(None, None, Some(BAR_PAD), Some(TABS_H))
        .w(MENU_W)
        .padding(4.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.20)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .modal_tx(true, |s: &mut State, _: &mut Tx| set_more_open(s, false));
    for &i in hidden {
        let name = &view.tabs[i];
        list = list.child(
            row()
                .key(format!("sh-more-{i}"))
                .role("menuitem")
                .label(name.clone())
                .w_full()
                .h(30.0)
                .px(10.0)
                .center_y()
                .radius(5.0)
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press_tx(open_sheet(i))
                .child(fitted(format!("sh-more-{i}-t"), name, TAB, t.text)),
        );
    }
    let unnamed = view.more.unnamed;
    if unnamed > 0 {
        let words = format!(
            "{unnamed} more {} not listed here",
            if unnamed == 1 { "sheet" } else { "sheets" }
        );
        list = list.child(
            row()
                .key("sh-more-unnamed")
                .w_full()
                .h(30.0)
                .px(10.0)
                .center_y()
                .child(fitted("sh-more-unnamed-t", &words, TAB, t.text_tertiary)),
        );
    }
    list
}

/// The tabs' bar. `room`: the bar's width as the last frame laid it out (`None` on a first build: every tab is drawn).
pub(super) fn sheet_tabs(view: &SheetView, live: bool, room: Option<f32>, t: &Theme) -> El {
    sheet_tabs_edit(view, live, room, false, t)
}

/// [`sheet_tabs`] for a workbook that may be open for editing (`editing`): a double press renames a tab, and a dimmed "+"
/// ends the bar.
pub fn sheet_tabs_edit(view: &SheetView, live: bool, room: Option<f32>, editing: bool, t: &Theme) -> El {
    let editing = editing && live;
    if !editing {
        reset();
    }
    let mut tabs = row()
        .key("sh-tabs")
        .role("tablist")
        .label("Sheets")
        .w_full()
        .h(TABS_H)
        .px(BAR_PAD)
        .gap(GAP)
        .center_y()
        .fill(t.surface)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout();
    let (shown, hidden) = match room {
        Some(w) if live => shown_tabs(view, w - 2.0 * BAR_PAD),
        _ => ((0..view.tabs.len()).collect(), Vec::new()),
    };
    for &i in &shown {
        tabs = tabs.child(tab(view, i, live, editing, t));
    }
    let behind = u32::try_from(hidden.len())
        .unwrap_or(u32::MAX)
        .saturating_add(view.more.unnamed);
    if !live || behind == 0 {
        return if editing { tabs.child(add_sheet(t)) } else { tabs };
    }
    let words = more_words(behind);
    let open = view.more.open;
    tabs = tabs.child(
        el().key("sh-tab-more")
            .role("button")
            .label(format!(
                "{behind} more {}",
                if behind == 1 { "sheet" } else { "sheets" }
            ))
            .checked(open)
            .h(22.0)
            .px(TAB_PAD)
            .center()
            .radius(5.0)
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| set_more_open(s, !open))
            .child(fitted("sh-tab-more-t", &words, TAB, t.text_tertiary)),
    );
    if editing {
        tabs = tabs.child(add_sheet(t));
    }
    if open {
        tabs = tabs.child(menu(view, &hidden, t));
    }
    tabs
}

/// The "+": there to be seen, dimmed; a press says why nothing happens.
fn add_sheet(t: &Theme) -> El {
    el().key("sh-tab-add")
        .role("button")
        .label("Add sheet")
        .description(ADD_WORDS)
        .help(ADD_WORDS)
        .h(22.0)
        .px(TAB_PAD)
        .center()
        .radius(5.0)
        .focusable(true)
        .on_press(|s: &mut State| say(s, ADD_WORDS))
        .child(fitted("sh-tab-add-t", "+", TAB, t.text_quaternary))
}
