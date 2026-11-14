//! This PC: Sync history and Available offline (the board's DW-1301 to DW-1306, DW-0428,
//! DW-0715, DW-0120). `FRAMES.md` beside this file lists every frame and which are built.
//!
//! [`Ui`] is what the two sections keep; every control changes it through one of its methods
//! and reads the engine through [`source::ThisPcSource`]. What leaves the group (show a file
//! in the window or in File Explorer, open a kept file, the ask before Remove all) is an
//! [`Intent`]; [`poll`] carries each one out through the window's own Files source and the
//! shell's routes ([`carry_out`]), and says a refusal in the window's toast.
//!
//! Nothing here is secret: names, folders, sizes and times of the person's own files. The
//! search field holds a file name. Comment mode is allowed on both sections.

mod chrome;
mod history;
pub mod icons;
mod offline;
pub mod scenes;
pub mod source;

#[cfg(test)]
mod tests;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx};

use crate::State;
pub use crate::shell::app::Section;
use crate::shell::registry::{FeedbackPolicy, Frame, FrameGroup};
use source::{Change, FixtureSource, Kept, Origin, Standing, ThisPcSource, What};

/// The registry's lines for the two sections (`shell/registry.rs`).
pub const SYNC_HISTORY: Frame = Frame {
    group: FrameGroup::Section(Section::SyncHistory),
    view: history::view,
    on_key: Some(on_key),
    feedback: FeedbackPolicy::Allowed,
};
pub const AVAILABLE_OFFLINE: Frame = Frame {
    group: FrameGroup::Section(Section::AvailableOffline),
    view: offline::view,
    on_key: Some(on_key),
    feedback: FeedbackPolicy::Allowed,
};

/// The history's filter (`.kit-seg`), in the board's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    ThisPc,
    OtherDevices,
    Problems,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::ThisPc, Filter::OtherDevices, Filter::Problems];

    pub fn label(self, problems: usize) -> String {
        match self {
            Filter::All => "All".into(),
            Filter::ThisPc => "This PC".into(),
            Filter::OtherDevices => "Other devices".into(),
            Filter::Problems => format!("Problems \u{00B7} {problems}"),
        }
    }
}

/// Where a control wants to go that this group does not draw. An intent about one item
/// carries that item's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Show the changed file in the window; a trashed file opens Trash (DW-1301 > DW-0401).
    ShowInBeebeeb(String),
    /// Show the file in File Explorer (DW-0428, DW-1306 > DW-1180).
    ShowInFileExplorer(String),
    /// Open a kept item, also offline (DW-0715 > DW-0408).
    Open(String),
    /// "Remove all from this PC" asks once before anything is removed; the answer comes back
    /// through [`Ui::remove_all_confirmed`].
    AskRemoveAll,
    /// The New menu (Files' DW-0415).
    New,
}

/// The menu on a kept row (DW-1306).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowMenu {
    pub row: usize,
    /// Where it opens, in the window.
    pub at: (f32, f32),
    /// The item the keyboard is on.
    pub hot: usize,
}

/// The row menu's items, in the board's order.
pub const MENU_ITEMS: [&str; 3] = ["Open", "Show in File Explorer", "Remove from this PC"];

pub struct Ui {
    source: Box<dyn ThisPcSource>,
    pub filter: Filter,
    /// Search history. Its own field: never Files' search.
    pub search: TextEdit,
    /// The selected kept row.
    pub selected: Option<usize>,
    pub menu: Option<RowMenu>,
    /// The window opened on this section (the flyout's "View full history"): Back has nowhere
    /// to go even when the route machine remembers a place.
    pub first_place: bool,
    intents: Vec<Intent>,
    /// "Remove all from this PC" was pressed: the window asks once before anything is removed.
    pub confirm_remove_all: bool,
    /// The board's sample PC (the frame gate's scenes): the status lines hold the board's sample words. Never in
    /// the running app.
    sample: bool,
    /// A change the source refused, and when: the window's toast says it for [`REFUSAL_FOR`].
    pub refusal: Option<(&'static str, std::time::Instant)>,
    /// A section of this group was on screen at the last poll: the next arrival is an opening.
    was_open: bool,
}

/// How long a refusal's toast stays.
pub const REFUSAL_FOR: std::time::Duration = std::time::Duration::from_secs(5);
/// How soon the window looks again while an answer is out or a toast shows.
pub const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

impl Ui {
    pub fn new(source: Box<dyn ThisPcSource>) -> Ui {
        Ui {
            source,
            filter: Filter::All,
            search: TextEdit::new(""),
            selected: None,
            menu: None,
            first_place: false,
            intents: Vec::new(),
            confirm_remove_all: false,
            sample: false,
            refusal: None,
            was_open: false,
        }
    }

    /// The board's sample PC.
    pub fn fixture() -> Ui {
        Ui::sample(FixtureSource::board())
    }

    /// A sample PC of the board, for the frame gate's scenes.
    pub fn sample(source: FixtureSource) -> Ui {
        Ui {
            sample: true,
            ..Ui::new(Box::new(source))
        }
    }

    /// The board's sample words stand in the status lines (a scene), never a real PC's.
    pub fn is_sample(&self) -> bool {
        self.sample
    }

    /// Whether the source has lists to show, or why not.
    pub fn standing(&self) -> Standing {
        self.source.standing()
    }

    /// `false`: nothing answers yet (the engine lane has not wired This PC).
    pub fn wired(&self) -> bool {
        self.source.wired()
    }

    pub fn problem_count(&self) -> usize {
        self.source.problems().len()
    }

    pub fn kept_count(&self) -> usize {
        self.source.kept_count()
    }

    pub fn kept(&self) -> Vec<Kept> {
        self.source.kept()
    }

    pub fn today(&self) -> chrono::NaiveDate {
        self.source.today()
    }

    /// The changes the list shows: the filter, then the search (names and folders).
    pub fn changes(&self) -> Vec<Change> {
        let all = match self.filter {
            Filter::Problems => self.source.problems(),
            _ => self.source.history(),
        };
        let query = self.search.text.trim().to_lowercase();
        all.into_iter()
            .filter(|c| match self.filter {
                Filter::All | Filter::Problems => true,
                Filter::ThisPc => c.origin == Origin::ThisPc,
                Filter::OtherDevices => c.origin != Origin::ThisPc,
            })
            .filter(|c| {
                query.is_empty()
                    || c.name.to_lowercase().contains(&query)
                    || c.what.place().is_some_and(|p| p.to_lowercase().contains(&query))
            })
            .collect()
    }

    /// The history has nothing at all (DW-1305), whatever the filter and the search say.
    pub fn no_history(&self) -> bool {
        self.source.history().is_empty() && self.source.problems().is_empty()
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
    }

    pub fn retry(&mut self, problem: &str) {
        self.source.retry(problem);
    }

    pub fn retry_all(&mut self) {
        self.source.retry_all();
    }

    pub fn clear_search(&mut self) -> bool {
        if self.search.text.is_empty() {
            return false;
        }
        self.search.clear();
        true
    }

    pub fn ask(&mut self, intent: Intent) {
        self.intents.push(intent);
    }

    /// What the controls asked for since the last call, oldest first ([`carry_out`]).
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// A double-click or Enter on a change.
    pub fn show_change(&mut self, change: &Change) {
        self.ask(match self.filter {
            Filter::Problems => Intent::ShowInFileExplorer(change.id.clone()),
            _ => Intent::ShowInBeebeeb(change.id.clone()),
        });
    }

    // ---- Available offline ----

    pub fn select(&mut self, row: usize) {
        if row < self.kept_count() {
            self.selected = Some(row);
        }
    }

    pub fn move_selection(&mut self, delta: isize) -> bool {
        let count = self.kept_count();
        if count == 0 {
            return false;
        }
        self.selected = Some(match self.selected {
            Some(i) => i.saturating_add_signed(delta).min(count - 1),
            None => 0,
        });
        true
    }

    pub fn open_menu(&mut self, row: usize, at: (f32, f32)) {
        if row < self.kept_count() {
            self.selected = Some(row);
            self.menu = Some(RowMenu { row, at, hot: 0 });
        }
    }

    pub fn close_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    /// Pick an item of the open row menu.
    pub fn pick(&mut self, item: usize) {
        let Some(menu) = self.menu.take() else { return };
        let Some(kept) = self.source.kept().into_iter().nth(menu.row) else {
            return;
        };
        match item {
            0 => self.ask(Intent::Open(kept.id)),
            1 => self.ask(Intent::ShowInFileExplorer(kept.id)),
            2 => self.remove_from_pc(&kept.id),
            _ => {}
        }
    }

    pub fn open_selected(&mut self) -> bool {
        let Some(kept) = self.selected.and_then(|i| self.source.kept().into_iter().nth(i)) else {
            return false;
        };
        self.ask(Intent::Open(kept.id));
        true
    }

    /// Stop keeping one item here. Nothing is deleted from Beebeeb.
    pub fn remove_from_pc(&mut self, id: &str) {
        self.source.remove_from_pc(id);
        self.selected = None;
    }

    /// The person said yes to "Remove all from this PC".
    pub fn remove_all_confirmed(&mut self) {
        self.confirm_remove_all = false;
        self.source.remove_all_from_pc();
        self.selected = None;
        self.menu = None;
    }

    /// The person said no, or pressed Esc: nothing is removed.
    pub fn remove_all_cancelled(&mut self) -> bool {
        std::mem::take(&mut self.confirm_remove_all)
    }
}

/// What the window says when something a row asked for did not happen. Fixed sentences: never a name.
pub const NOT_SHOWN: &str = "Couldn't show this in File Explorer.";
pub const NOT_OPENED: &str = "Couldn't open this.";

/// Carries out what the two sections asked for and cannot do themselves, in the order asked: `true` when something
/// was asked. A file is shown and opened through the window's own Files source (its guards: the session, sync
/// running, a connection), the window moves by the shell's routes. What is refused is said in the window's toast,
/// never only in a log line.
pub fn carry_out(s: &mut State, tx: &mut Tx) -> bool {
    use crate::shell::app::{Event, Route};
    use crate::shell::host;

    let intents = s.this_pc.take_intents();
    let asked = !intents.is_empty();
    for intent in intents {
        let refused = match intent {
            // The window on Trash for a file that went there, else on Files.
            Intent::ShowInBeebeeb(id) => {
                let trashed = s
                    .this_pc
                    .source
                    .history()
                    .iter()
                    .any(|change| change.id == id && matches!(change.what, What::MovedToTrash { .. }));
                let section = if trashed { Section::Trash } else { Section::Files };
                host::dispatch(s, Event::Navigate(Route::section(section)));
                None
            }
            Intent::ShowInFileExplorer(id) => s
                .actions
                .source
                .show_in_file_explorer(&[offline::drive_item(&id, "", false)])
                .err()
                .map(|_| NOT_SHOWN),
            Intent::Open(id) => open_kept(s, &id),
            Intent::AskRemoveAll => {
                s.this_pc.confirm_remove_all = s.this_pc.kept_count() > 0;
                None
            }
            // New things are made in Files: the window goes there with its New menu open (DW-0415).
            Intent::New => {
                host::dispatch(s, Event::Navigate(Route::section(Section::Files)));
                s.actions.menu = None;
                s.toggle_menu(crate::frames::files::MenuKind::New);
                None
            }
        };
        if let Some(words) = refused {
            s.this_pc.refusal = Some((words, std::time::Instant::now()));
            tx.wake_after(POLL_EVERY);
        }
    }
    asked
}

/// Opens a kept item: a folder in Files, a file in the window's viewer when it has one for the type, else in the
/// file's own app. Only an item the list holds; the words of a refusal otherwise.
fn open_kept(s: &mut State, id: &str) -> Option<&'static str> {
    use crate::shell::app::{Event, Route};

    let kept = s.this_pc.source.kept().into_iter().find(|kept| kept.id == id)?;
    let item = offline::drive_item(&kept.id, &kept.name, kept.kind == source::Kind::Folder);
    if item.is_folder {
        let route = Route {
            section: Section::Files,
            folder: Some(item.id),
            item: None,
        };
        crate::shell::host::dispatch(s, Event::Navigate(route));
        return None;
    }
    if crate::frames::viewers::open_item(s, &item) {
        return None;
    }
    s.actions.source.open(&[item]).err().map(|_| NOT_OPENED)
}

/// What the source's workers answered since the last frame, for the shell's poll: `true` when the window must draw
/// again. A refusal becomes the window's toast. Never calls the engine, never waits.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let asked = carry_out(s, tx);
    let open = matches!(
        s.shell.app.route().section,
        Section::SyncHistory | Section::AvailableOffline
    );
    let ui = &mut s.this_pc;
    // The ask before Remove all does not outlive its section, or the items it was about.
    let offline = s.shell.app.route().section == Section::AvailableOffline;
    if ui.confirm_remove_all && (!offline || ui.kept_count() == 0) {
        ui.confirm_remove_all = false;
    }
    // A section that just opened: the source reads again when what it holds is old.
    if open && !ui.was_open {
        ui.source.opened();
    }
    ui.was_open = open;
    let pumped = ui.source.pump();
    let mut changed = pumped.changed || asked;
    if let Some(words) = pumped.refusal {
        ui.refusal = Some((words, std::time::Instant::now()));
        changed = true;
    }
    if ui.refusal.is_some_and(|(_, at)| at.elapsed() >= REFUSAL_FOR) {
        ui.refusal = None;
        changed = true;
    }
    if pumped.busy || ui.refusal.is_some() {
        tx.wake_after(POLL_EVERY);
    }
    changed
}

/// The two sections' keys. The shell's own shortcuts ran first (`host::on_key`); Esc arrives
/// here when the shell had nothing to close.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let section = s.shell.app.route().section;
    let ui = &mut s.this_pc;
    // The ask before Remove all takes every key while it is up: Esc says no, nothing else reaches the list.
    if ui.confirm_remove_all {
        if ev.key == Key::Escape {
            ui.remove_all_cancelled();
        }
        return true;
    }
    if ev.is_command('f') {
        tx.focus(crate::ui::keys::SEARCH_KEY, true);
        return true;
    }
    if let Some(menu) = ui.menu.as_mut() {
        return match ev.key {
            Key::Escape => ui.close_menu(),
            Key::Down => {
                menu.hot = (menu.hot + 1).min(MENU_ITEMS.len() - 1);
                true
            }
            Key::Up => {
                menu.hot = menu.hot.saturating_sub(1);
                true
            }
            Key::Enter => {
                let hot = menu.hot;
                ui.pick(hot);
                true
            }
            _ => false,
        };
    }
    if ev.key == Key::Escape {
        return ui.clear_search();
    }
    // Enter on a change shows its file, as a double-click does.
    if section == Section::SyncHistory {
        let row = tx
            .focused()
            .and_then(|key| key.strip_prefix("change-")?.parse::<usize>().ok());
        return match (ev.key.clone(), row.and_then(|i| ui.changes().into_iter().nth(i))) {
            (Key::Enter, Some(change)) => {
                ui.show_change(&change);
                true
            }
            _ => false,
        };
    }
    // The list's keys are the list's: with focus on a row or nowhere.
    if section != Section::AvailableOffline || tx.focused().is_some_and(|key| !key.starts_with("row-")) {
        return false;
    }
    match ev.key {
        Key::Down => ui.move_selection(1),
        Key::Up => ui.move_selection(-1),
        Key::Enter => ui.open_selected(),
        _ => false,
    }
}

/// The window around a section's content: caption, sidebar, and the column the section fills.
fn window(s: &State, cx: &mut Cx<State>, here: Section, main: El) -> El {
    let t = crate::theme::tokens::theme(cx.system_dark());
    rune::col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(crate::ui::caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(
            rune::row()
                .key("body")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(chrome::sidebar(s, &s.this_pc, here, &t))
                .child(match s.this_pc.refusal {
                    // A refused change: said over the section, never only in a log line.
                    Some((words, _)) => main.child(crate::ui::toast::layer(crate::ui::toast::toast(
                        &crate::theme::icons_files::ALERT,
                        crate::ui::toast::Tone::Failed,
                        words,
                        None,
                        &t,
                    ))),
                    None => main,
                }),
        )
}
