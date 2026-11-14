//! The Files frames: the list with its menus, selection, toasts and details (`FRAMES.md` is the checklist).
//!
//! `view` is the Files window with what these frames add; `on_key` its keyboard; `SCENES` the
//! board frames this group builds, by frame id (what each sets on a fresh `State`).
//! Data: the listing comes from `crate::source::DriveSource`, everything else from `source::FilesSource`
//! (counts, the account, the sync state; every action is a call on it, answered in a toast).

pub mod action_scenes;
pub mod actions_data;
pub mod browse_states;
pub mod changes;
pub mod conflict;
pub mod details;
pub mod dialogs;
pub mod facts;
pub mod keys;
pub mod link;
pub mod list;
pub mod local;
pub mod long_list;
pub mod menus;
pub mod names;
pub mod preview;
pub mod protected;
pub mod protected_fixture;
pub mod rename;
pub mod reset;
pub mod retry;
pub mod session;
pub mod share;
pub mod sidebar;
pub mod sorting;
pub mod source;
pub mod to_trash;
pub mod toolbar;
pub mod uploads;
pub mod window;

#[cfg(test)]
mod actions_tests;
#[cfg(test)]
mod browse_states_tests;
#[cfg(test)]
mod long_list_tests;
#[cfg(test)]
mod protected_tests;
#[cfg(test)]
mod rest_tests;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod uploads_tests;

use std::collections::BTreeSet;
use std::time::Duration;

use crate::State;
use crate::source::DriveItem;
use rune::Tx;
use source::{Action, ActionError, FilesFixture, FilesSource, NoEngine};

pub use window::{has_in_front, view};

/// The design system's own Files window and keyboard, which these frames supersede (same pieces,
/// plus menus, selection and toasts; the shell's Files line draws these frames). Named here so they
/// stay compiled and checked until the design-system lane folds the variants in (PR body:
/// "Requests to the design-system lane").
const _: crate::shell::registry::FrameFn = crate::ui::window::files_window;
const _: crate::shell::registry::KeyFn = crate::ui::keys::on_key;

/// Which menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    /// The file menu of the selection (DW-0402, DW-1104).
    File,
    /// The toolbar's Sort (DW-0404).
    Sort,
    /// The toolbar's New (DW-0414).
    New,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpenMenu {
    pub kind: MenuKind,
    /// Where the file menu's top-left corner is, in window points (the toolbar's menus hang from their button).
    pub at: (f32, f32),
    /// The highlighted entry.
    pub hot: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortBy {
    Name,
    DateChanged,
    Size,
    Type,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub by: SortBy,
    /// Z to A, oldest first, smallest first.
    pub reversed: bool,
    pub folders_first: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort {
            by: SortBy::Name,
            reversed: false,
            folders_first: true,
        }
    }
}

/// What an action asks of the rest of the app: a board frame this group does not draw (a dialog,
/// a viewer, a picker, another section). Where the shell has a route for it, it goes there too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Frame(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toast {
    /// "Moved to Trash", with Undo (DW-0405).
    Trashed,
    /// "Duplicated" (DW-0406).
    Duplicated,
    /// "Keeping on this PC" (DW-0412).
    Keeping,
    /// "Link copied" (DW-0107).
    LinkCopied,
    /// "Freed up space on this PC": the copies on this PC are gone, the items are online only.
    FreedUp,
    /// "Couldn't rename. Check your connection." with Try again (DW-0109).
    Failed(Action, ActionError),
}

impl Toast {
    /// How long it stays (DW-0107: 3 s; DW-0108: 6 s, for Undo; DW-0109: 8 s, or until Try again).
    pub fn stays(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Toast::Trashed => 6,
            Toast::Failed(..) => 8,
            _ => 3,
        })
    }

    /// The sentence of a failure: what did not happen and, where there is one, the way to fix it.
    /// Never a code (DW-0109).
    pub fn failure(action: Action, why: ActionError) -> String {
        let fix = match why {
            ActionError::Offline => " Check your connection.",
            ActionError::InFlight => " The last change is still on its way.",
            ActionError::NotAll => " Some items were left as they were.",
            ActionError::NotOnThisSystem => " It is not available on this computer yet.",
            ActionError::NotYet => " It is not available yet.",
            ActionError::HasLink => {
                " This file already has a link that can't be copied here yet. To get a new link, stop sharing the \
                 file in the web app first."
            }
            ActionError::NotSent => " It has changes that are not uploaded yet.",
            ActionError::HoldsKept => " Something in it is kept on this PC.",
            _ => "",
        };
        format!("Couldn't {}.{fix}", action.words())
    }
}

/// What the Files frames keep beside the listing.
pub struct Actions {
    pub source: Box<dyn FilesSource>,
    pub menu: Option<OpenMenu>,
    pub toast: Option<Toast>,
    /// When the toast goes away by itself (`None`: it stays, as in a board frame).
    pub toast_until: Option<std::time::Instant>,
    /// Every selected row. `State::selected` is the one the keyboard is on (and the details panel shows).
    pub selection: BTreeSet<usize>,
    /// Where a Shift range starts.
    pub anchor: Option<usize>,
    /// The sort of the folder shown (each folder has its own: `FilesSource::sort_of`).
    pub sort: Sort,
    /// What Undo puts back: the rows as they were, with their places. Emptied by whatever is done next.
    pub undo: Vec<(usize, DriveItem)>,
    /// Moves to and from Trash that are not answered yet, and what Try again of one asks for (`to_trash.rs`).
    pub trash: to_trash::Ui,
    /// Links that were asked for and are not answered yet, and what Try again of one asks for (`link.rs`).
    pub link: link::Ui,
    /// The last frames asked for, oldest first (at most [`INTENTS_KEPT`]).
    pub intents: Vec<Intent>,
    /// An upload that failed: what Try again uploads.
    pub retry_upload: Option<(Option<String>, Vec<std::path::PathBuf>)>,
    /// A link that could not be made: the dialog Try again opens again, with the same items
    /// and choices. Never the password: that is typed again.
    pub retry_share: Option<share::Share>,
    /// The window's size, for keeping a menu inside it.
    pub window: (f32, f32),
    /// A board frame that draws only the lower part of the window (DW-0107, DW-0108): how many
    /// points of the window's top are out of the picture. 0 in the app.
    pub board_crop_top: f32,
    /// The folders Back left, nearest last: where Forward goes. Opening a folder empties it.
    pub forward: Vec<Option<String>>,
    /// The open dialog: New folder (DW-0417) or Move to (DW-0409).
    pub dialog: Option<dialogs::Dialog>,
    /// The row whose name is a field (DW-0410).
    pub rename: Option<rename::Rename>,
    /// The name being typed: in the New folder dialog, or in a row.
    pub name_edit: rune::TextEdit,
    /// The name that was refused, and why (DW-0410b). It is shown while the field still holds that name.
    pub name_error: Option<(String, names::NameError)>,
    /// Folders the source made since the window last looked (`Change::Made`).
    pub made: Vec<DriveItem>,
    /// Whose disk a typed name has to fit on: this computer's (a test names the other one).
    pub host: names::Host,
    /// The password of a link being made (DW-0407): secure, wiped when the dialog closes.
    pub share_password: rune::TextEdit,
    /// Files answered since Two versions opened (DW-0429): the second reads "2 of 2".
    pub conflicts_done: usize,
    /// Windows' picker to open (DW-0415, DW-0480): the shell takes it (`State::take_picker`).
    pub picker: Option<uploads::PickerAsk>,
    /// Files dragged over the list (DW-0481).
    pub drop_hover: Option<uploads::DropHover>,
    /// Uploads is open over the status line (DW-0427), and the row whose menu is open in it.
    pub uploads_open: bool,
    pub upload_menu: Option<String>,
    /// Which session of this window this is: one more at each session's end (`reset.rs`). An
    /// answer that was asked for under an earlier one (a picker that comes back late) is dropped.
    pub session_no: u64,
    /// The protected folders: the open message, the section last seen (`protected.rs`).
    pub protected: protected::Ui,
    /// The letters typed in the list, which jump to a name (`long_list.rs`).
    pub typed: long_list::Typed,
    /// The session ended: what was refused (`session.rs`).
    pub session: session::Ui,
    /// A change of what this PC holds: what Try again repeats, what is still to be said (`local.rs`).
    pub local: local::Ui,
    /// During the first download: something was opened, so the card gave way to the strip (DW-1107).
    pub first_load_strip: bool,
    /// A board frame that draws a long list from this row on (DW-0420b: the gate draws one frame
    /// and cannot scroll). 0 in the app.
    pub board_first_row: usize,
}

pub const INTENTS_KEPT: usize = 32;

impl Actions {
    pub fn new(source: Box<dyn FilesSource>) -> Self {
        Actions {
            source,
            menu: None,
            toast: None,
            toast_until: None,
            selection: BTreeSet::new(),
            anchor: None,
            sort: Sort::default(),
            undo: Vec::new(),
            trash: to_trash::Ui::default(),
            link: link::Ui::default(),
            intents: Vec::new(),
            retry_upload: None,
            retry_share: None,
            window: (1278.0, 798.0),
            board_crop_top: 0.0,
            forward: Vec::new(),
            dialog: None,
            rename: None,
            name_edit: rune::TextEdit::default(),
            name_error: None,
            made: Vec::new(),
            host: names::Host::this(),
            share_password: rune::TextEdit::new("").secure(),
            conflicts_done: 0,
            picker: None,
            drop_hover: None,
            uploads_open: false,
            upload_menu: None,
            session_no: 0,
            protected: protected::Ui::default(),
            typed: long_list::Typed::default(),
            session: session::Ui::default(),
            local: local::Ui::default(),
            first_load_strip: false,
            board_first_row: 0,
        }
    }
}

/// A window with nothing behind it but its listing: no counts, no account, and every action refused
/// (`source::NoEngine`). The engine lane puts its own `FilesSource` here; the board's sample is
/// [`board_fixture`], and only that.
impl Default for Actions {
    fn default() -> Self {
        Actions::new(Box::new(NoEngine))
    }
}

/// The board's sample window: its folders (`source::BoardDrive`) and its account, counts and
/// actions (`source::FilesFixture`). What headless renders, the gate and the tests start from.
pub fn board_fixture(mut s: State) -> State {
    // One set of changes behind both: what New folder, Rename and Move do is still there when a
    // folder is listed again.
    let fixture = FilesFixture::default();
    s.source = Box::new(source::BoardDrive::sharing(&fixture.edits));
    (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
    s.actions = Actions::new(Box::new(fixture));
    s
}

impl State {
    /// Is this row drawn selected?
    pub fn is_selected(&self, row: usize) -> bool {
        self.selected == Some(row) || self.actions.selection.contains(&row)
    }

    /// The selected rows, lowest first.
    pub fn selected_rows(&self) -> Vec<usize> {
        let mut rows = self.actions.selection.clone();
        rows.extend(self.selected);
        rows.into_iter().filter(|&i| i < self.listing.items.len()).collect()
    }

    /// A plain click, or an arrow: this row alone.
    pub fn select_only(&mut self, row: usize) {
        if row >= self.listing.items.len() {
            return;
        }
        self.actions.selection = BTreeSet::from([row]);
        self.actions.anchor = Some(row);
        self.selected = Some(row);
    }

    /// Ctrl+click: add the row, or take it out.
    pub fn toggle_row(&mut self, row: usize) {
        if row >= self.listing.items.len() {
            return;
        }
        if self.selects_one_at_a_time() {
            return self.select_only(row);
        }
        let mut rows: BTreeSet<usize> = self.selected_rows().into_iter().collect();
        if !rows.remove(&row) {
            rows.insert(row);
            self.selected = Some(row);
        } else if self.selected == Some(row) {
            self.selected = rows.iter().next_back().copied();
        }
        self.actions.anchor = Some(row);
        self.actions.selection = rows;
    }

    /// Shift+click, Shift+arrow: everything from where the range started to this row.
    pub fn extend_to(&mut self, row: usize) {
        let count = self.listing.items.len();
        if row >= count {
            return;
        }
        if self.selects_one_at_a_time() {
            return self.select_only(row);
        }
        let from = self.actions.anchor.or(self.selected).unwrap_or(row).min(count - 1);
        self.actions.selection = (from.min(row)..=from.max(row)).collect();
        self.actions.anchor = Some(from);
        self.selected = Some(row);
    }

    pub fn select_all(&mut self) {
        let count = self.listing.items.len();
        if count == 0 || self.selects_one_at_a_time() {
            return;
        }
        self.actions.selection = (0..count).collect();
        self.actions.anchor = Some(0);
        self.selected = self.selected.or(Some(0));
    }

    pub fn clear_selection(&mut self) {
        self.actions.selection.clear();
        self.actions.anchor = None;
        self.selected = None;
    }

    /// The arrows: one row up or down, alone, or with Shift as a range.
    pub fn arrow(&mut self, delta: isize, extend: bool) -> Option<usize> {
        let count = self.listing.items.len();
        if count == 0 {
            return None;
        }
        let to = match self.selected {
            Some(i) => i.saturating_add_signed(delta).min(count - 1),
            None => 0,
        };
        if extend {
            self.extend_to(to);
        } else {
            self.select_only(to);
        }
        Some(to)
    }

    /// Open the file menu for the selection, its corner at `at` (window points), kept inside the window.
    pub fn open_file_menu(&mut self, at: (f32, f32)) {
        if self.selected_rows().is_empty() {
            return;
        }
        let entries = menus::file_menu(self);
        let (w, h) = (menus::FILE_MENU_W, crate::ui::menu::height(&entries));
        let (win_w, win_h) = self.actions.window;
        let at = (at.0.min(win_w - w - 8.0).max(8.0), at.1.min(win_h - h - 8.0).max(8.0));
        self.actions.menu = Some(OpenMenu {
            kind: MenuKind::File,
            at,
            hot: None,
        });
    }

    /// Open a toolbar menu, or close it when it is the one that is open.
    pub fn toggle_menu(&mut self, kind: MenuKind) {
        self.actions.menu = match self.actions.menu {
            Some(open) if open.kind == kind => None,
            _ => Some(OpenMenu {
                kind,
                at: (0.0, 0.0),
                hot: None,
            }),
        };
    }

    pub fn close_menu(&mut self) {
        self.actions.menu = None;
    }

    /// Close the menu and give the keyboard back to the list.
    pub fn close_menu_tx(&mut self, tx: &mut Tx) {
        self.actions.menu = None;
        tx.focus("rows", false);
    }

    /// The toast to draw now: none once its time is over.
    pub fn live_toast(&self) -> Option<Toast> {
        match self.actions.toast_until {
            Some(until) if std::time::Instant::now() >= until => None,
            _ => self.actions.toast,
        }
    }

    /// Ask for a frame this group does not draw. The shell's route machine takes the ones it has a
    /// place for (Settings, Sync history, Available offline); every one is kept for the lane that
    /// draws it.
    pub(super) fn intent(&mut self, intent: Intent) {
        use crate::shell::app::{Event, Route, Section, SettingsPane};
        let Intent::Frame(frame) = &intent;
        if self.refuses_frame(frame) {
            return;
        }
        let frame: &'static str = frame;
        // The status line shows the b's flyout (DW-0401 to DW-1201): a second surface, no route.
        let flyout = frame == "DW-1201";
        let event = match frame {
            "DW-0701" => Some(Event::OpenSettings(SettingsPane::General)),
            "DW-1301" => Some(Event::Navigate(Route::section(Section::SyncHistory))),
            "DW-0715" => Some(Event::Navigate(Route::section(Section::AvailableOffline))),
            _ => None,
        };
        if self.actions.intents.len() >= INTENTS_KEPT {
            self.actions.intents.remove(0);
        }
        self.actions.intents.push(intent);
        // The frames this group draws itself (lane files-actions): the dialog opens here.
        match frame {
            "DW-0407" => {
                self.begin_share();
            }
            "DW-0411" => {
                self.begin_details();
            }
            "DW-0413" => {
                self.begin_versions();
            }
            // Windows draws these two: the request for its picker, with our words on it.
            "DW-0415" => self.ask_picker(uploads::PickerKind::Files),
            "DW-0480" => self.ask_picker(uploads::PickerKind::Folder),
            _ => {}
        }
        if flyout {
            crate::frames::tray::show_from_window(self);
        }
        if let Some(event) = event {
            crate::shell::host::dispatch(self, event);
        }
    }

    /// May things be added to the folder shown? Not when it could not be read, and not to a
    /// folder the source protects (DW-0414: "New is dimmed in Backups, service folders, Trash…").
    pub fn can_add(&self) -> bool {
        // A protected folder takes nothing new, whatever the source would accept (`protected.rs`).
        self.list_error.is_none()
            && !self.in_protected_folder()
            && self.actions.source.can_add(self.listing.folder.id.as_deref())
    }

    pub(super) fn selected_items(&self) -> Vec<DriveItem> {
        self.selected_rows()
            .into_iter()
            .map(|i| self.listing.items[i].clone())
            .collect()
    }

    /// Something else was done: Ctrl+Z no longer brings the trashed rows back (DW-0108).
    pub(super) fn forget_undo(&mut self) {
        self.actions.undo.clear();
    }

    /// An action did not happen: say so (DW-0109). A protected folder says why in its own frame (DW-1110).
    pub(super) fn failed(&mut self, action: Action, why: ActionError, tx: &mut Tx) {
        let items = self.selected_items();
        if let Some(stay) = self.failed_of(action, why, &items) {
            tx.wake_after(stay);
        }
    }

    /// [`State::failed`] of these items (not always the selection: an answer that came later, Try again),
    /// without a wake: how long the message it set stays, for the caller to wake the window after.
    pub(super) fn failed_of(&mut self, action: Action, why: ActionError, items: &[DriveItem]) -> Option<Duration> {
        if why == ActionError::Protected {
            self.intent(Intent::Frame("DW-1110"));
            // Where the source has words for it, they are said here (`protected.rs`).
            self.say_protected(action, items)
        } else {
            Some(self.set_toast(Toast::Failed(action, why)))
        }
    }

    /// Show a toast, and take it away again after its time.
    pub(super) fn show_toast(&mut self, toast: Toast, tx: &mut Tx) {
        let stay = self.set_toast(toast);
        tx.wake_after(stay);
    }

    /// The toast, without a wake: how long it stays.
    pub(super) fn set_toast(&mut self, toast: Toast) -> Duration {
        let stay = toast.stays();
        self.actions.toast = Some(toast);
        self.actions.toast_until = Some(std::time::Instant::now() + stay);
        stay
    }

    /// [`State::take_changes`], with its messages: "Duplicated", or what failed. The engine lane
    /// calls this from its `handle.update` when the source has news.
    pub fn apply_changes(&mut self, tx: &mut Tx) {
        let (copied, failures) = self.take_changes();
        if copied {
            self.show_toast(Toast::Duplicated, tx);
        }
        if let Some(&(action, why)) = failures.last() {
            self.failed(action, why, tx);
        }
    }

    /// Select the rows with these ids; the keyboard is on the last of them.
    fn select_ids(&mut self, ids: &[String]) {
        let rows: BTreeSet<usize> = self
            .listing
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| ids.contains(&item.id))
            .map(|(i, _)| i)
            .collect();
        self.selected = rows.iter().next_back().copied();
        self.actions.anchor = self.selected;
        self.actions.selection = rows;
    }

    /// What every change of folder resets: the selection, what Undo holds, an open menu. The rows
    /// stand in the order this folder was left with (DW-0404: "Each folder remembers its own sort").
    fn entered(&mut self) {
        self.actions.selection.clear();
        self.actions.anchor = None;
        self.actions.undo.clear();
        // The rows an Undo waits for are another folder's now: its answer puts nothing in this one.
        self.actions.trash.restoring.clear();
        self.actions.menu = None;
        self.actions.rename = None;
        self.actions.protected.alert = None;
        self.selected = None;
        self.actions.sort = self
            .actions
            .source
            .sort_of(self.listing.folder.id.as_deref())
            .unwrap_or_default();
        self.apply_sort();
    }

    /// Show the folder with this id (`None`: the root), leaving the one shown to Back.
    pub fn open_folder(&mut self, id: Option<String>) {
        // A folder is read from the source: not while the session ended (`session.rs`).
        if self.refused_without_session() {
            return;
        }
        self.files.back.push(self.listing.folder.id.clone());
        self.actions.forward.clear();
        (self.listing, self.list_error) = crate::load(self.source.as_ref(), id.as_deref());
        self.entered();
    }

    /// Back, leaving the folder shown to Forward.
    pub fn back(&mut self) -> bool {
        if self.refused_without_session() {
            return false;
        }
        let here = self.listing.folder.id.clone();
        if !self.go_back() {
            return false;
        }
        self.actions.forward.push(here);
        self.entered();
        true
    }

    pub fn can_go_forward(&self) -> bool {
        !self.actions.forward.is_empty()
    }

    /// Forward: the folder Back left.
    pub fn forward(&mut self) -> bool {
        if self.refused_without_session() {
            return false;
        }
        let Some(id) = self.actions.forward.pop() else {
            return false;
        };
        self.files.back.push(self.listing.folder.id.clone());
        (self.listing, self.list_error) = crate::load(self.source.as_ref(), id.as_deref());
        self.entered();
        true
    }

    /// A click on a folder of the path: up to it. The folders passed stay behind for Back.
    pub fn open_crumb(&mut self, index: usize) {
        if let Some(crumb) = self.listing.path.get(index) {
            let id = crumb.id.clone();
            if id != self.listing.folder.id {
                self.open_folder(id);
            }
        }
    }

    /// Open the selection: one folder opens in place; files go to their apps, all of them.
    /// Returns whether the folder shown changed.
    pub fn open_selection(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return false;
        }
        // A file that changed in two places: the choice first (DW-0429).
        if self.review_selected_conflict() {
            return false;
        }
        let items = self.selected_items();
        // Opening something ends the first download's card (DW-1107: "shrinks to a strip").
        self.actions.first_load_strip |= !items.is_empty();
        // Without a connection a file that is not on this PC says so, and is not asked for (DW-1108).
        if let Some(item) = items.iter().find(|item| browse_states::needs_connection(self, item)) {
            let (id, name) = (item.id.clone(), item.name.clone());
            self.block_preview(id, name);
            return false;
        }
        let lead = self.selected.and_then(|i| self.listing.items.get(i)).cloned();
        let files: Vec<DriveItem> = items.iter().filter(|i| !i.is_folder).cloned().collect();
        let folder = match (&items[..], &lead) {
            ([only], _) if only.is_folder => Some(only.id.clone()),
            // Several folders and no file: the one the keyboard is on.
            (_, Some(lead)) if files.is_empty() && lead.is_folder => Some(lead.id.clone()),
            _ => None,
        };
        if let Some(id) = folder {
            self.open_folder(Some(id));
            return true;
        }
        if files.is_empty() {
            return false;
        }
        // One file of a type the window shows itself: its viewer (`frames::viewers`, the router).
        if matches!(&files[..], [only] if crate::frames::viewers::open_item(self, only)) {
            return true;
        }
        self.forget_undo();
        if let Err(why) = self.actions.source.open(&files) {
            self.failed(Action::Open, why, tx);
        }
        false
    }

    /// A double-click: this row alone, opened.
    pub fn open_row(&mut self, row: usize, tx: &mut Tx) -> bool {
        if row >= self.listing.items.len() {
            return false;
        }
        self.select_only(row);
        self.open_selection(tx)
    }

    /// Preview in the window (DW-0408): of a file. A folder has none (DW-0402: "Preview is dimmed").
    pub fn can_preview(&self) -> bool {
        self.selected
            .and_then(|i| self.listing.items.get(i))
            .is_some_and(|item| !item.is_folder)
    }

    pub fn preview(&mut self) -> bool {
        if !self.can_preview() {
            return false;
        }
        if self.refused_without_session() {
            return true;
        }
        // Without a connection a file that is not on this PC says so (`frames::global`, DW-0121).
        if crate::frames::global::preview_needs_connection(self) {
            return true;
        }
        self.intent(Intent::Frame("DW-0408"));
        if let Some(item) = self.selected.and_then(|i| self.listing.items.get(i)).cloned() {
            self.open_preview(item);
        }
        true
    }

    /// Duplicate the selection: each copy stands next to its original, selected (DW-0406).
    pub fn duplicate_selected(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let items = self.selected_items();
        if items.is_empty() {
            return false;
        }
        if self.protects(&items) {
            self.failed(Action::Duplicate, ActionError::Protected, tx);
            return true;
        }
        let names: Vec<&str> = self.listing.items.iter().map(|i| i.name.as_str()).collect();
        if let Err(why) = self.actions.source.duplicate(&items, &names) {
            self.failed(Action::Duplicate, why, tx);
            // Handled: there is a message to draw.
            return true;
        }
        self.forget_undo();
        self.apply_changes(tx);
        true
    }

    pub fn show_in_file_explorer(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let items = self.selected_items();
        if items.is_empty() {
            return false;
        }
        if let Err(why) = self.actions.source.show_in_file_explorer(&items) {
            self.failed(Action::ShowInFileExplorer, why, tx);
        }
        true
    }

    /// The toolbar's Open in File Explorer: the folder shown, in Explorer.
    pub fn open_in_file_explorer(&mut self, tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return true;
        }
        let folder = self.listing.folder.id.clone();
        if let Err(why) = self.actions.source.open_in_file_explorer(folder.as_deref()) {
            self.failed(Action::OpenInFileExplorer, why, tx);
        }
        true
    }

    /// After the folder, a dialog or a field changed: the keyboard is the list's again.
    pub(super) fn focus_list(&self, tx: &mut Tx) {
        keys::focus_list(self, tx);
    }

    /// Files dropped from File Explorer: into `folder` (`None`: the root). A folder that takes
    /// nothing refuses the drop, silently (DW-0481: "the row does not light up").
    pub fn upload(&mut self, folder: Option<String>, files: &[std::path::PathBuf], tx: &mut Tx) -> bool {
        if self.needs_session(tx) {
            return false;
        }
        if files.is_empty() || self.list_error.is_some() || !self.actions.source.can_add(folder.as_deref()) {
            return false;
        }
        // Nothing is dropped into a protected folder (the Backups row in Files, or inside it).
        if self.actions.source.protection(folder.as_deref(), None).is_some() {
            return false;
        }
        self.forget_undo();
        if let Err(why) = self.actions.source.upload(folder.as_deref(), files) {
            // Try again sends what was NOT copied in alone: a file already in the folder would get a second
            // copy under another name. Nothing left (every file is in): no Try again (`retry.rs`).
            let left = self.actions.source.upload_left().unwrap_or_else(|| files.to_vec());
            self.actions.retry_upload = Some((folder, left));
            self.failed(Action::Upload, why, tx);
            return false;
        }
        self.actions.drop_hover = None;
        true
    }

    /// The entries of the open menu.
    pub fn menu_entries(&self, kind: MenuKind) -> Vec<crate::ui::menu::Entry> {
        match kind {
            MenuKind::File => menus::file_menu(self),
            // A protected folder keeps its owner's order: the choices are shown, dimmed.
            MenuKind::Sort => {
                let fixed = self.in_protected_folder();
                menus::sort_menu(self.actions.sort)
                    .into_iter()
                    .map(|entry| entry.dimmed(fixed))
                    .collect()
            }
            MenuKind::New => menus::new_menu(),
        }
    }

    /// Run the entry `index` of the open menu. A dimmed entry does nothing and the menu stays.
    pub fn pick_menu(&mut self, index: usize, tx: &mut Tx) {
        let Some(open) = self.actions.menu else { return };
        let entries = self.menu_entries(open.kind);
        let Some(id) = entries.get(index).filter(|e| !e.is_off()).and_then(|e| e.id()) else {
            return;
        };
        // Sort's choices keep the menu open, as a menu of ticks does; everything else closes it.
        if open.kind != MenuKind::Sort {
            self.close_menu_tx(tx);
        }
        let sort = self.actions.sort;
        match id {
            "open" => {
                self.open_selection(tx);
            }
            "preview" => {
                self.preview();
            }
            "share" => self.ask_share(tx),
            "copy-link" => {
                self.copy_link(tx);
            }
            "keep-offline" => {
                self.toggle_keep_offline(tx);
            }
            "show-in-explorer" => {
                self.show_in_file_explorer(tx);
            }
            "details" => self.intent(Intent::Frame("DW-0411")),
            "rename" => {
                self.begin_rename(tx);
            }
            "move" => {
                self.begin_move(tx);
            }
            "free-up-space" => {
                self.free_up_space(tx);
            }
            "duplicate" => {
                self.duplicate_selected(tx);
            }
            "versions" => self.intent(Intent::Frame("DW-0413")),
            "trash" => {
                self.trash_selection(tx);
            }
            "sort-name" => self.set_sort(Sort {
                by: SortBy::Name,
                ..sort
            }),
            "sort-date" => self.set_sort(Sort {
                by: SortBy::DateChanged,
                ..sort
            }),
            "sort-size" => self.set_sort(Sort {
                by: SortBy::Size,
                ..sort
            }),
            "sort-type" => self.set_sort(Sort {
                by: SortBy::Type,
                ..sort
            }),
            "order-forward" => self.set_sort(Sort {
                reversed: false,
                ..sort
            }),
            "order-reversed" => self.set_sort(Sort { reversed: true, ..sort }),
            "folders-first" => self.set_sort(Sort {
                folders_first: !sort.folders_first,
                ..sort
            }),
            "new-folder" => {
                self.begin_new_folder();
            }
            "new-file" => self.intent(Intent::Frame("DW-1521")),
            "new-document" => self.intent(Intent::Frame("DW-1558")),
            "upload-files" => self.intent(Intent::Frame("DW-0415")),
            "upload-folder" => self.intent(Intent::Frame("DW-0480")),
            _ => {}
        }
    }
}

/// The board frames this group builds: frame id, and what it sets on a fresh `State`.
/// The shell joins this table to its own (`shell::scenes`).
pub const SCENES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1101", |s| {
        s.clear_selection();
        s.show_detail_pane = false;
    }),
    ("DW-0402", |s| {
        s.select_only(6);
        s.actions.menu = Some(OpenMenu {
            kind: MenuKind::File,
            at: (430.0, 316.0),
            hot: Some(3),
        });
    }),
    ("DW-1104", |s| {
        s.select_only(7);
        s.actions.menu = Some(OpenMenu {
            kind: MenuKind::File,
            at: (420.0, 346.0),
            hot: Some(6),
        });
    }),
    ("DW-0404", |s| {
        s.select_only(6);
        s.actions.menu = Some(OpenMenu {
            kind: MenuKind::Sort,
            at: (0.0, 0.0),
            hot: Some(10),
        });
    }),
    ("DW-0414", |s| {
        s.select_only(6);
        s.actions.menu = Some(OpenMenu {
            kind: MenuKind::New,
            at: (0.0, 0.0),
            hot: Some(0),
        });
    }),
    ("DW-0405", |s| {
        s.select_only(6);
        s.show_detail_pane = false;
        let item = s.listing.items[6].clone();
        if s.actions.source.trash(std::slice::from_ref(&item)).is_ok() {
            s.listing.items.remove(6);
            s.actions.undo = vec![(6, item)];
            s.clear_selection();
            s.actions.toast = Some(Toast::Trashed);
        }
    }),
    ("DW-0406", |s| {
        s.select_only(6);
        let names: Vec<&str> = s.listing.items.iter().map(|i| i.name.as_str()).collect();
        let original = s.listing.items[6].clone();
        if s.actions.source.duplicate(&[original], &names).is_ok() && s.take_changes().0 {
            s.actions.toast = Some(Toast::Duplicated);
        }
    }),
    // DW-0107, DW-0108: the two kinds of message, over the list as it is.
    ("DW-0107", |s| {
        s.show_detail_pane = false;
        s.clear_selection();
        s.actions.board_crop_top = 240.0;
        s.actions.toast = Some(Toast::LinkCopied);
    }),
    ("DW-0108", |s| {
        s.show_detail_pane = false;
        s.clear_selection();
        s.actions.board_crop_top = 240.0;
        s.actions.toast = Some(Toast::Trashed);
    }),
    // DW-0109: the message of a failure, the board's sample ("Couldn't rename. Check your connection.").
    ("DW-0109", |s| {
        s.show_detail_pane = false;
        s.clear_selection();
        s.actions.board_crop_top = 240.0;
        s.actions.toast = Some(Toast::Failed(Action::Rename, ActionError::Offline));
    }),
    ("DW-1106", |s| {
        s.open_folder(Some(source::NEW_FOLDER_ID.to_string()));
    }),
    // DW-0417: New folder, over the window as it is, "Untitled folder" selected in its field.
    ("DW-0417", |s| {
        s.select_only(6);
        s.begin_new_folder();
        s.actions.name_edit.focused = true;
    }),
    // DW-0409: Move to, for Contract draft.pdf, Documents chosen.
    ("DW-0409", |s| {
        s.select_only(6);
        let items = s.selected_items();
        s.open_move(items);
    }),
    // DW-0410: Contract draft.pdf's name as a field, "Contract draft" selected.
    ("DW-0410", |s| {
        s.select_only(6);
        s.open_rename(6);
        s.actions.name_edit.focused = true;
    }),
    // DW-0410b: the copy, being given the original's name: refused, and the tip says why.
    ("DW-0410b", |s| {
        s.select_only(6);
        s.show_detail_pane = false;
        let names: Vec<&str> = s.listing.items.iter().map(|i| i.name.as_str()).collect();
        let original = s.listing.items[6].clone();
        if s.actions.source.duplicate(&[original.clone()], &names).is_ok() && s.take_changes().0 {
            s.open_rename(7);
            let mut edit = rune::TextEdit::new(original.name.clone());
            edit.set_caret(names::stem_len(&original.name, false));
            edit.focused = true;
            s.actions.name_edit = edit;
            s.check_rename();
        }
    }),
    // The action dialogs and the preview (lane files-actions): `action_scenes.rs`.
    ("DW-0407", action_scenes::share),
    ("DW-0411", action_scenes::details),
    ("DW-0413", action_scenes::versions),
    ("DW-0408", action_scenes::preview),
    ("DW-0408b", action_scenes::no_preview),
    ("DW-0429", action_scenes::two_versions),
    // Uploads (lane files-actions): a drag over the list, uploads in their folder, the panel.
    ("DW-0481", action_scenes::drag_over_documents),
    ("DW-0426", action_scenes::uploading),
    ("DW-0427", action_scenes::uploads_panel),
];
