//! The Files frames: the list with its menus, selection, toasts and details (`FRAMES.md` is the checklist).
//!
//! `view` is the Files window with what these frames add; `on_key` its keyboard; `SCENES` the
//! board frames this group builds, by frame id (what each sets on a fresh `State`).
//! Data: the listing comes from `crate::source::DriveSource`, everything else from `source::FilesSource`.

pub mod details;
pub mod keys;
pub mod list;
pub mod menus;
pub mod sidebar;
pub mod source;
pub mod toolbar;
pub mod window;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use crate::State;
use crate::source::DriveItem;
use rune::Tx;
use source::{FilesFixture, FilesSource};

pub use window::view;

/// The design system's own Files window and keyboard, which these frames supersede (same pieces,
/// plus menus, selection and toasts). Named here so they stay compiled and checked until the
/// design-system lane folds the variants in (PR body: "Requests to the design-system lane").
const _: (
    fn(&State, &mut rune::Cx<State>) -> rune::El,
    fn(&mut State, &rune::KeyEvent, &mut Tx) -> bool,
) = (crate::ui::window::files_window, crate::ui::keys::on_key);

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

/// What an action asks of the rest of the app: another lane's frame, or the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Go to a board frame this group does not draw (a dialog, a viewer, a picker, another section).
    Frame(&'static str),
    /// Open the file with this id in its app.
    Open(String),
    ShowInFileExplorer(String),
    CopyLink(String),
    /// Upload these files from this PC into the folder with this id (`None`: the root).
    Upload(Option<String>, Vec<std::path::PathBuf>),
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
    pub sort: Sort,
    /// What Undo puts back: the rows as they were, with their places.
    pub undo: Vec<(usize, DriveItem)>,
    pub intents: Vec<Intent>,
    /// The window's size, for keeping a menu inside it.
    pub window: (f32, f32),
    /// A board frame that draws only the lower part of the window (DW-0107, DW-0108): how many
    /// points of the window's top are out of the picture. 0 in the app.
    pub board_crop_top: f32,
    /// The folders Back left, nearest last: where Forward goes. Opening a folder empties it.
    pub forward: Vec<Option<String>>,
}

impl Default for Actions {
    fn default() -> Self {
        Actions {
            source: Box::new(FilesFixture::default()),
            menu: None,
            toast: None,
            toast_until: None,
            selection: BTreeSet::new(),
            anchor: None,
            sort: Sort::default(),
            undo: Vec::new(),
            intents: Vec::new(),
            window: (1278.0, 798.0),
            board_crop_top: 0.0,
            forward: Vec::new(),
        }
    }
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
        let from = self.actions.anchor.or(self.selected).unwrap_or(row).min(count - 1);
        self.actions.selection = (from.min(row)..=from.max(row)).collect();
        self.actions.anchor = Some(from);
        self.selected = Some(row);
    }

    pub fn select_all(&mut self) {
        let count = self.listing.items.len();
        if count == 0 {
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

    /// Close the menu and give the keyboard back to the list: the row the menu was for.
    pub fn close_menu_tx(&mut self, tx: &mut Tx) {
        self.actions.menu = None;
        if let Some(row) = self.selected {
            tx.focus(format!("row-{row}"), false);
        }
    }

    /// The toast to draw now: none once its time is over.
    pub fn live_toast(&self) -> Option<Toast> {
        match self.actions.toast_until {
            Some(until) if std::time::Instant::now() >= until => None,
            _ => self.actions.toast,
        }
    }

    pub(super) fn intent(&mut self, intent: Intent) {
        self.actions.intents.push(intent);
    }

    /// Show a toast, and take it away again after its time (6 s with Undo, 3 s without: DW-0405).
    fn show_toast(&mut self, toast: Toast, tx: &mut Tx) {
        let stay = std::time::Duration::from_secs(if toast == Toast::Trashed { 6 } else { 3 });
        self.actions.toast = Some(toast);
        self.actions.toast_until = Some(std::time::Instant::now() + stay);
        tx.wake_after(stay);
    }

    /// What every change of folder resets: the selection, what Undo holds, an open menu.
    fn entered(&mut self) {
        self.actions.selection.clear();
        self.actions.anchor = None;
        self.actions.undo.clear();
        self.actions.menu = None;
        self.selected = None;
    }

    /// Show the folder with this id (`None`: the root), leaving the one shown to Back.
    pub fn open_folder(&mut self, id: Option<String>) {
        self.files.back.push(self.listing.folder.id.clone());
        self.actions.forward.clear();
        (self.listing, self.list_error) = crate::load(self.source.as_ref(), id.as_deref());
        self.entered();
    }

    /// Back, leaving the folder shown to Forward.
    pub fn back(&mut self) -> bool {
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

    /// Open: a folder opens in place; a file goes to its app.
    pub fn open_row(&mut self, row: usize) -> bool {
        let Some(item) = self.listing.items.get(row) else {
            return false;
        };
        let id = item.id.clone();
        if item.is_folder {
            self.open_folder(Some(id));
            return true;
        }
        self.intent(Intent::Open(id));
        false
    }

    /// Move the selection to Trash, without a question: Undo and Trash both bring it back.
    pub fn trash_selection(&mut self, tx: &mut Tx) -> bool {
        let rows = self.selected_rows();
        let items: Vec<DriveItem> = rows.iter().map(|&i| self.listing.items[i].clone()).collect();
        if items.is_empty() || self.actions.source.trash(&items).is_err() {
            // A protected folder says why in its own frame (DW-1110).
            if items.iter().any(|i| i.is_protected) {
                self.intent(Intent::Frame("DW-1110"));
            }
            return false;
        }
        for &i in rows.iter().rev() {
            self.listing.items.remove(i);
        }
        self.actions.undo = rows.into_iter().zip(items).collect();
        self.clear_selection();
        self.show_toast(Toast::Trashed, tx);
        true
    }

    /// Undo the last Move to Trash: every row back where it was.
    pub fn undo(&mut self) -> bool {
        let rows = std::mem::take(&mut self.actions.undo);
        if rows.is_empty() {
            return false;
        }
        let items: Vec<DriveItem> = rows.iter().map(|(_, item)| item.clone()).collect();
        if self.actions.source.restore(&items).is_err() {
            self.actions.undo = rows;
            return false;
        }
        let mut last = None;
        for (i, item) in rows {
            let at = i.min(self.listing.items.len());
            self.listing.items.insert(at, item);
            last = Some(at);
        }
        self.actions.toast = None;
        if let Some(row) = last {
            self.select_only(row);
        }
        true
    }

    /// Duplicate the row the keyboard is on: the copy stands next to it, selected.
    pub fn duplicate_selected(&mut self, tx: &mut Tx) -> bool {
        let Some(row) = self.selected.filter(|&i| i < self.listing.items.len()) else {
            return false;
        };
        let names: Vec<&str> = self.listing.items.iter().map(|i| i.name.as_str()).collect();
        let Ok(copy) = self.actions.source.duplicate(&self.listing.items[row], &names) else {
            return false;
        };
        self.listing.items.insert(row + 1, copy);
        self.select_only(row + 1);
        self.show_toast(Toast::Duplicated, tx);
        true
    }

    /// Make available offline, or Remove from this PC when it already is.
    pub fn toggle_keep_offline(&mut self, tx: &mut Tx) -> bool {
        let Some(row) = self.selected.filter(|&i| i < self.listing.items.len()) else {
            return false;
        };
        let keep = self.listing.items[row].pc_status.as_deref() != Some("available_offline");
        if self
            .actions
            .source
            .keep_offline(&self.listing.items[row], keep)
            .is_err()
        {
            return false;
        }
        self.listing.items[row].pc_status = Some(if keep { "available_offline" } else { "online_only" }.into());
        if keep {
            self.show_toast(Toast::Keeping, tx);
        }
        true
    }

    /// Change the sort and put the rows in that order. Backups stays the first row whatever the sort (DW-0404).
    pub fn set_sort(&mut self, sort: Sort) {
        let lead = self
            .selected
            .and_then(|i| self.listing.items.get(i))
            .map(|i| i.id.clone());
        self.actions.sort = sort;
        self.actions.source.sort(sort);
        let kind = |i: &DriveItem| {
            i.name
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_lowercase())
                .unwrap_or_default()
        };
        self.listing.items.sort_by(|a, b| {
            use std::cmp::Ordering;
            let pinned = b.is_protected.cmp(&a.is_protected);
            let folders = if sort.folders_first {
                b.is_folder.cmp(&a.is_folder)
            } else {
                Ordering::Equal
            };
            let name = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            let by = match sort.by {
                SortBy::Name => name,
                // Newest first, largest first: the board's first Order choice for these.
                SortBy::DateChanged => b.modified_at.cmp(&a.modified_at),
                SortBy::Size => b.size.unwrap_or(0).cmp(&a.size.unwrap_or(0)),
                SortBy::Type => kind(a).cmp(&kind(b)),
            };
            let by = if sort.reversed { by.reverse() } else { by };
            pinned.then(folders).then(by).then(name)
        });
        self.actions.selection.clear();
        self.actions.anchor = None;
        self.selected = lead.and_then(|id| self.listing.items.iter().position(|i| i.id == id));
        if let Some(row) = self.selected {
            self.actions.selection.insert(row);
        }
    }

    /// Run the entry `index` of the open menu.
    pub fn pick_menu(&mut self, index: usize, tx: &mut Tx) {
        let Some(open) = self.actions.menu else { return };
        let entries = match open.kind {
            MenuKind::File => menus::file_menu(self),
            MenuKind::Sort => menus::sort_menu(self.actions.sort),
            MenuKind::New => menus::new_menu(),
        };
        let Some(id) = entries.get(index).and_then(crate::ui::menu::Entry::id) else {
            return;
        };
        // Sort's choices keep the menu open, as a menu of ticks does; everything else closes it.
        if open.kind != MenuKind::Sort {
            self.close_menu_tx(tx);
        }
        let lead = self
            .selected
            .and_then(|i| self.listing.items.get(i))
            .map(|i| i.id.clone());
        let sort = self.actions.sort;
        match id {
            "open" => {
                if let Some(row) = self.selected {
                    self.open_row(row);
                }
            }
            "preview" => self.intent(Intent::Frame("DW-0408")),
            "share" => self.intent(Intent::Frame("DW-0407")),
            "copy-link" => {
                if let Some(id) = lead {
                    self.intent(Intent::CopyLink(id));
                    self.show_toast(Toast::LinkCopied, tx);
                }
            }
            "keep-offline" => {
                self.toggle_keep_offline(tx);
            }
            "show-in-explorer" => {
                if let Some(id) = lead {
                    self.intent(Intent::ShowInFileExplorer(id));
                }
            }
            "details" => self.intent(Intent::Frame("DW-0411")),
            "rename" => self.intent(Intent::Frame("DW-0410")),
            "move" => self.intent(Intent::Frame("DW-0409")),
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
            "new-folder" => self.intent(Intent::Frame("DW-0417")),
            "new-file" => self.intent(Intent::Frame("DW-1521")),
            "new-document" => self.intent(Intent::Frame("DW-1558")),
            "upload-files" => self.intent(Intent::Frame("DW-0415")),
            "upload-folder" => self.intent(Intent::Frame("DW-0480")),
            _ => {}
        }
    }
}

/// The board frames this group builds: frame id, and what it sets on a fresh `State`.
/// `main.rs` lists them in `SCENES`; the shell's registry can take the same table.
pub const SCENES: &[(&str, fn(&mut State))] = &[
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
        if let Ok(copy) = s.actions.source.duplicate(&s.listing.items[6], &names) {
            s.listing.items.insert(7, copy);
            s.select_only(7);
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
    ("DW-1106", |s| {
        s.open_folder(Some(source::NEW_FOLDER_ID.to_string()));
    }),
];
