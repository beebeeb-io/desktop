//! Search (`DW-0801` Recent, `DW-0802` results, `DW-0802b` results during a first sign-in,
//! `DW-0803` no results): a state of the Files window. The board: "Ctrl+F puts the cursor in the
//! toolbar's search field; results replace the list until Esc." The checklist is `FRAMES.md`.
//!
//! - **Where it shows.** The registry's Files line (`shell/files_frame.rs`) asks [`over_files`]
//!   first: while search is open (Ctrl+F, or words in the window's search field) this group draws
//!   the window, and [`files_key`] takes its keys, so no key reaches the Files list behind it.
//! - **The query** is the window's search field (`State::search`), the one the Files toolbar edits:
//!   a word typed there opens search, and the field keeps the keyboard across the switch (both
//!   windows key it on the same path: `window/body/content-area/toolbar/search-box/search-input`).
//! - **Data**: [`source::SearchSource`]. A window starts on `source::Unwired` and says search is
//!   not available yet; only a board frame's scene sets the board's sample (`source::BoardSearch`).
//! - **What it asks of the rest of the app** goes to `frames::Ui::intents`: open a result
//!   (`OpenItem`), its file menu (`About("DW-0402")`), Details, Share…, Version history, Available
//!   offline, and the toolbar's Sort, New and grid view.

pub mod source;
#[cfg(test)]
mod tests;
mod view;

use rune::{El, Key, KeyEvent, TextEdit, Tx};

use crate::State;
use crate::frames::{self, Intent};
use crate::shell::app::Section;
use crate::ui::gate::Scene;
use crate::ui::keys::SEARCH_KEY;
use source::{Answer, BoardSearch, Hit, Loading, Narrow, SearchSource, Unwired};

/// What the Search group keeps on `State` (`frames::Ui::find`).
pub struct SearchUi {
    pub source: Box<dyn SearchSource>,
    /// Search was opened (Ctrl+F) and shows until Esc or ×, also with an empty field (DW-0801).
    pub open: bool,
    pub narrow: Narrow,
    /// The selected result, by its id: it stays selected while it is among the results. With
    /// none, the first result is the selected one (DW-0802).
    pub selected: Option<String>,
    /// A board frame's focused field: the board draws the field focused, with its caret, and a
    /// headless render has no focus to draw either. Only a scene sets it; the first key clears it.
    pub pictured: bool,
}

impl Default for SearchUi {
    /// What a window on the engine starts on: nothing to search with until the engine lane wires
    /// Search.
    fn default() -> Self {
        SearchUi {
            source: Box::new(Unwired),
            open: false,
            narrow: Narrow::All,
            selected: None,
            pictured: false,
        }
    }
}

impl SearchUi {
    /// What a window without an engine starts on (`source::first`).
    pub fn first(dev_build: bool) -> Self {
        SearchUi {
            source: source::first(dev_build),
            ..SearchUi::default()
        }
    }
}

/// What the content area shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Shows {
    /// Nothing can search here yet (`SearchSource::wired` is false).
    Unavailable,
    /// The field is empty: the recent searches (DW-0801).
    Recent(Vec<String>),
    /// The matches (DW-0802), and how far the list has loaded while it still loads (DW-0802b).
    Results { hits: Vec<Hit>, loading: Option<Loading> },
    /// No match (DW-0803).
    Nothing { loading: Option<Loading> },
    /// The query is being answered.
    Searching,
}

impl Shows {
    /// The toolbar's line under "Search".
    pub fn count_words(&self) -> String {
        let so_far = |loading: &Option<Loading>| if loading.is_some() { " so far" } else { "" };
        match self {
            Shows::Unavailable => "Not available yet".into(),
            Shows::Searching => "Searching\u{2026}".into(),
            Shows::Recent(_) => "Files".into(),
            Shows::Results { hits, loading } if hits.len() == 1 => format!("1 result{}", so_far(loading)),
            Shows::Results { hits, loading } => format!("{} results{}", hits.len(), so_far(loading)),
            Shows::Nothing { loading } => format!("No results{}", so_far(loading)),
        }
    }
}

impl SearchUi {
    /// What the content area shows for the words in the field.
    pub fn shows(&self, typed: &str) -> Shows {
        if !self.source.wired() {
            return Shows::Unavailable;
        }
        let query = typed.trim();
        if query.is_empty() {
            return Shows::Recent(self.source.recent());
        }
        match self.source.search(query, self.narrow) {
            Answer::Unavailable => Shows::Unavailable,
            Answer::Searching => Shows::Searching,
            Answer::Found { hits, loading } if hits.is_empty() => Shows::Nothing { loading },
            Answer::Found { hits, loading } => Shows::Results { hits, loading },
        }
    }

    /// The selected result's place among `hits`: the one chosen, else the first.
    pub fn selected_index(&self, hits: &[Hit]) -> Option<usize> {
        self.selected
            .as_ref()
            .and_then(|id| hits.iter().position(|h| &h.id == id))
            .or(if hits.is_empty() { None } else { Some(0) })
    }

    /// Narrow to `narrow` (a pill): the selection starts again at the first result.
    pub fn narrow_to(&mut self, narrow: Narrow) {
        self.narrow = narrow;
        self.selected = None;
    }
}

/// Is search open: opened with Ctrl+F, or words in the window's search field.
pub fn is_open(s: &State) -> bool {
    s.frames.find.open || !s.search.text.trim().is_empty()
}

/// Something of the Files window that takes the keyboard on its own (a menu, a dialog, a name
/// being typed in a row) keeps it: search does not open over it.
fn files_busy(s: &State) -> bool {
    s.actions.menu.is_some() || s.actions.dialog.is_some() || s.actions.rename.is_some()
}

/// While search shows, Alt+Left and Alt+Right are not the window's Back and Forward: the board
/// draws both off ("Esc: ends search"), and a route change would leave search open over the
/// place it arrives at. The shell asks before it takes the two keys (`host::shell_keys`).
pub fn holds_back(s: &State) -> bool {
    crate::shell::registry::group_for(&s.shell.app) == crate::shell::registry::FrameGroup::Section(Section::Files)
        && s.shell.app.dialog().is_none()
        && is_open(s)
        && !files_busy(s)
}

/// The Files window's view while search is open (the registry's Files line asks first).
pub fn over_files(s: &State, cx: &mut rune::Cx<State>) -> Option<El> {
    (is_open(s) && !files_busy(s)).then(|| view::window(s, cx))
}

/// The Files window's keys while search is open: `Some` takes the key (no key reaches the list
/// behind), `None` leaves it to Files. With search closed, Ctrl+F opens it (DW-0801).
pub fn files_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> Option<bool> {
    if files_busy(s) {
        return None;
    }
    if !is_open(s) {
        if ev.is_command('f') {
            open(s, tx);
            return Some(true);
        }
        return None;
    }
    // The window's section shortcuts stay the window's (Ctrl+1 to Ctrl+7).
    if matches!(ev.key, Key::Command('1'..='7')) {
        return None;
    }
    Some(on_key(s, ev, tx))
}

/// Search reads the account's file names: not while the shell's session gate is closed. `true`:
/// refused; the keyboard goes to the banner's Sign in, as the Files window's refusals do.
fn refused(s: &mut State, tx: &mut Tx) -> bool {
    if !crate::shell::session_gate::refuse(s) {
        return false;
    }
    tx.focus("banner-action", true);
    true
}

/// Open search with the field focused (DW-0801).
pub fn open(s: &mut State, tx: &mut Tx) {
    if refused(s, tx) {
        return;
    }
    s.frames.find.open = true;
    tx.focus(SEARCH_KEY, true);
}

/// End search (Esc, ×): the field is emptied and the window shows the view it came from.
pub fn close(s: &mut State, tx: &mut Tx) {
    let ui = &mut s.frames.find;
    ui.open = false;
    ui.selected = None;
    ui.narrow = Narrow::All;
    ui.pictured = false;
    s.search.clear();
    tx.focus(SEARCH_KEY, false);
}

/// Run a recent search: its words in the field, the results under it (DW-0801 to DW-0802).
pub fn run(s: &mut State, query: &str, tx: &mut Tx) {
    if refused(s, tx) {
        return;
    }
    s.search = TextEdit::new(query);
    s.frames.find.selected = None;
    s.frames.find.source.remember(query);
    tx.focus(SEARCH_KEY, true);
}

/// The results for the field as it is now (empty while there is none).
fn hits(s: &State) -> Vec<Hit> {
    match s.frames.find.shows(&s.search.text) {
        Shows::Results { hits, .. } => hits,
        _ => Vec::new(),
    }
}

/// The key of result `i` in the list: focus goes where the selection goes.
fn row_key(i: usize) -> String {
    format!("row-{i}")
}

/// Select result `i` and give it the keyboard.
fn select(s: &mut State, hits: &[Hit], i: usize, tx: &mut Tx) {
    if let Some(hit) = hits.get(i) {
        s.frames.find.selected = Some(hit.id.clone());
        tx.focus(row_key(i), true);
    }
}

/// Open result `i` (Enter, a double-click): the search is kept as a recent one.
fn open_result(s: &mut State, hits: &[Hit], i: usize) -> bool {
    let Some(hit) = hits.get(i) else { return false };
    if crate::shell::session_gate::refuse(s) {
        return true;
    }
    s.frames.find.selected = Some(hit.id.clone());
    let query = s.search.text.trim().to_string();
    s.frames.find.source.remember(&query);
    // "Double-click a result: opens it" (DW-0408). The search stays as it is behind the open
    // item, so Back shows the same results again.
    // THE ONE OPENING PATH: this is Search's single call site. When the viewers lane's router
    // (`viewers::open_item(state, &DriveItem) -> bool`, by file type) is on the branch, this
    // line becomes that call, with today's dispatch as what it falls back to.
    crate::shell::host::dispatch(s, crate::shell::app::Event::OpenItem(hit.id.clone()));
    true
}

/// Search's keys, wherever focus is in its window (the window's `on_key`, and [`files_key`]
/// while nothing has focus). Esc ends search; Ctrl+F goes to the field; Alt+Shift+P shows or
/// hides the details panel. In the field: Down goes into what is under it, Enter keeps the
/// search as a recent one. On the pills: Left and Right narrow. In the results (or with focus
/// nowhere): the arrows, Home and End select, Enter opens, the Menu key asks for the file menu,
/// Alt+Enter for Details. In the recent searches: the arrows move, Enter runs one.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    s.frames.find.pictured = false;
    if ev.key == Key::Escape && frames::plain(ev) {
        close(s, tx);
        return true;
    }
    // Every other key of Search reads or opens something of the account.
    if refused(s, tx) {
        return true;
    }
    if ev.is_command('f') {
        // Asked for by name: search now stays open over an emptied field too (DW-0801).
        s.frames.find.open = true;
        tx.focus(SEARCH_KEY, true);
        return true;
    }
    let m = ev.mods;
    if m.alt && m.shift && !m.ctrl && !m.cmd && matches!(&ev.key, Key::Text(p) if p.eq_ignore_ascii_case("p")) {
        s.show_detail_pane = !s.show_detail_pane;
        return true;
    }
    let focused = tx.focused().map(str::to_owned);
    match focused.as_deref() {
        Some(SEARCH_KEY) => field_key(s, ev, tx),
        Some(key) if key.starts_with("seg-0-") => narrow_key(s, ev, tx),
        Some(key) if key.starts_with(view::RECENT_ITEM) => recent_key(s, key, ev, tx),
        Some(key) if !key.starts_with("row-") => false,
        _ => results_key(s, ev, tx),
    }
}

/// The search field's own keys (`on_input_key`): what the field does not use itself.
fn field_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    s.frames.find.pictured = false;
    if !frames::plain(ev) {
        return false;
    }
    match ev.key {
        Key::Escape => {
            close(s, tx);
            true
        }
        // "↓ from the field: moves into the results" (DW-0802); into the recent searches (DW-0801).
        Key::Down => match s.frames.find.shows(&s.search.text) {
            Shows::Results { hits, .. } => {
                let i = s.frames.find.selected_index(&hits).unwrap_or(0);
                select(s, &hits, i, tx);
                true
            }
            Shows::Recent(recent) if !recent.is_empty() => {
                tx.focus(view::recent_key(0), true);
                true
            }
            _ => false,
        },
        Key::Enter => {
            let query = s.search.text.trim().to_string();
            if query.is_empty() {
                return false;
            }
            s.frames.find.source.remember(&query);
            true
        }
        _ => false,
    }
}

/// The pills' keys while one of them has focus.
fn narrow_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if !frames::plain(ev) {
        return false;
    }
    let at = Narrow::ALL.iter().position(|n| *n == s.frames.find.narrow).unwrap_or(0);
    let to = match ev.key {
        Key::Left => at.saturating_sub(1),
        Key::Right => (at + 1).min(Narrow::ALL.len() - 1),
        Key::Home => 0,
        Key::End => Narrow::ALL.len() - 1,
        Key::Down => {
            let hits = hits(s);
            if hits.is_empty() {
                return false;
            }
            let i = s.frames.find.selected_index(&hits).unwrap_or(0);
            select(s, &hits, i, tx);
            return true;
        }
        _ => return false,
    };
    let narrow = Narrow::ALL[to];
    s.frames.find.narrow_to(narrow);
    tx.focus(
        crate::frames::shared::parts::controls::segment_key(0, narrow.words()),
        true,
    );
    true
}

/// The recent searches' keys while one of them has focus (`item-0-<m>`, the board's grouped list).
fn recent_key(s: &mut State, key: &str, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if !frames::plain(ev) {
        return false;
    }
    let Shows::Recent(recent) = s.frames.find.shows(&s.search.text) else {
        return false;
    };
    let Some(m) = key
        .strip_prefix(view::RECENT_ITEM)
        .and_then(|n| n.parse::<usize>().ok())
    else {
        return false;
    };
    let runs = ev.key == Key::Enter || frames::is_space(ev);
    match ev.key {
        Key::Down if m + 1 < recent.len() => tx.focus(view::recent_key(m + 1), true),
        Key::Up if m > 0 => tx.focus(view::recent_key(m - 1), true),
        Key::Up => tx.focus(SEARCH_KEY, true),
        _ if runs => match recent.get(m) {
            Some(query) => run(s, &query.clone(), tx),
            None => return false,
        },
        _ => return false,
    }
    true
}

/// The results' keys, with focus on a result or nowhere.
fn results_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let hits = hits(s);
    let Some(at) = s.frames.find.selected_index(&hits) else {
        return false;
    };
    let id = hits[at].id.clone();
    if frames::is_menu_key(ev) {
        // The usual file menu, plus Show in enclosing folder (DW-0402).
        s.frames.intents.push(Intent::About("DW-0402", id));
        return true;
    }
    if frames::is_alt_enter(ev) {
        s.frames.intents.push(Intent::About("DW-0411", id));
        return true;
    }
    if !frames::plain(ev) {
        return false;
    }
    let last = hits.len() - 1;
    match ev.key {
        Key::Down => select(s, &hits, (at + 1).min(last), tx),
        Key::Up if at == 0 => tx.focus(SEARCH_KEY, true),
        Key::Up => select(s, &hits, at - 1, tx),
        Key::Home => select(s, &hits, 0, tx),
        Key::End => select(s, &hits, last, tx),
        Key::Enter => return open_result(s, &hits, at),
        _ => return false,
    }
    true
}

/// A board frame: the Files window searching, the field focused (as the board draws it).
fn scene(s: &mut State, typed: &str, source: BoardSearch) {
    crate::frames::go(s, Section::Files);
    s.search = TextEdit::new(typed);
    s.frames.find = SearchUi {
        source: Box::new(source),
        open: true,
        narrow: Narrow::All,
        selected: None,
        pictured: true,
    };
}

/// Search opened: Recent, Clear, what search covers.
pub fn dw_0801(s: &mut State) {
    scene(s, "", BoardSearch::new());
}
/// Results for "Contr": the pills, three rows with Where, the first one's details.
pub fn dw_0802(s: &mut State) {
    scene(s, "Contr", BoardSearch::new());
}
/// The same during a first sign-in: the note with the live count.
pub fn dw_0802b(s: &mut State) {
    scene(s, "Contr", BoardSearch::loading());
}
/// No results for "harbr".
pub fn dw_0803(s: &mut State) {
    scene(s, "harbr", BoardSearch::new());
}

/// The board frames of Search (`--scene DW-0801`, the frame gate).
pub const FRAMES: &[Scene<State>] = &[
    ("DW-0801", dw_0801),
    ("DW-0802", dw_0802),
    ("DW-0802b", dw_0802b),
    ("DW-0803", dw_0803),
];
