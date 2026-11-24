//! The menu of a right-click on the list's empty space (DW-1103): New folder, Upload files…, the
//! view, Sort by with its choices beside it, Folders first and the details panel. Kept apart from
//! `menus.rs` so the file menu stays as it is.

use super::{Intent, MenuKind, OpenMenu, Sort, SortBy, menus};
use crate::State;
use crate::ui::menu::Entry;
use rune::Tx;

/// `.wn-menu` of the list's background: as wide as its longest line and shortcut ask.
pub const MENU_W: f32 = 265.67;
/// The Sort by choices that open beside it.
pub const SUB_W: f32 = 170.0;
/// Where the choices stand from the menu's corner: 216 pt in, 94 pt down (the board's flyout overlaps
/// the menu's right end and starts above its own row).
const SUB_AT: (f32, f32) = (216.0, 94.0);
/// The entry "Sort by" in [`entries`].
pub const SORT_BY: usize = 6;

/// The menu's entries; the view and Folders first read from the folder shown.
pub fn entries(s: &State) -> Vec<Entry> {
    let grid = crate::frames::global::grid::on(s);
    // A protected folder takes no new folder, no upload and keeps its owner's order: the entries are shown, dimmed
    // (DW-0414), as the toolbar's Sort menu does.
    let fixed = s.in_protected_folder();
    vec![
        Entry::keyed("new-folder", "New folder", "Ctrl+Shift+N").dimmed(fixed),
        Entry::keyed("upload-files", "Upload files\u{2026}", "Ctrl+O").dimmed(fixed),
        Entry::Sep,
        Entry::check("view-list", "as List", !grid),
        Entry::check("view-grid", "as Grid", grid),
        Entry::Sep,
        Entry::keyed("sort-by", "Sort by", crate::ui::menu::SUBMENU).dimmed(fixed),
        Entry::check("folders-first", "Folders first", s.actions.sort.folders_first),
        Entry::Sep,
        Entry::keyed("details-panel", "Show details panel", "Alt+Shift+P"),
    ]
}

/// Sort by's choices: what by, then which way (the file list's own wording for the key).
pub fn sort_entries(s: &State) -> Vec<Entry> {
    // The Sort menu without its headings and Folders first: the same ids, so one pick serves both.
    menus::sort_menu(s.actions.sort)
        .into_iter()
        .filter(|e| !matches!(e, Entry::Head(_)))
        .filter(|e| e.id() != Some("folders-first"))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .skip_while(|e| matches!(e, Entry::Sep))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|entry| entry.dimmed(s.in_protected_folder()))
        .collect()
}

impl State {
    /// Open the menu of the list's background at `at` (window points), kept inside the window.
    pub fn open_background_menu(&mut self, at: (f32, f32)) {
        let height = crate::ui::menu::height(&entries(self));
        let (win_w, win_h) = self.actions.window;
        let at = (
            at.0.min(win_w - MENU_W - 8.0).max(8.0),
            at.1.min(win_h - height - 8.0).max(8.0),
        );
        self.actions.menu = Some(OpenMenu {
            kind: MenuKind::Background,
            at,
            hot: None,
        });
    }

    /// Where Sort by's choices stand: beside the menu while "Sort by" is the highlighted entry.
    pub fn background_sort_at(&self) -> Option<(f32, f32)> {
        let open = self.actions.menu?;
        if open.kind != MenuKind::Background || open.hot != Some(SORT_BY) {
            return None;
        }
        let (win_w, _) = self.actions.window;
        let x = open.at.0 + SUB_AT.0;
        // Where there is no room at the right, the choices stand left of the menu.
        let x = if x + SUB_W + 8.0 > win_w {
            (open.at.0 - SUB_W + 4.0).max(8.0)
        } else {
            x
        };
        Some((x, open.at.1 + SUB_AT.1))
    }

    /// Run the entry `index` of the background menu. "Sort by" keeps the menu open and shows its choices.
    pub fn pick_background(&mut self, index: usize, tx: &mut Tx) {
        let entries = entries(self);
        let Some(id) = entries.get(index).filter(|e| !e.is_off()).and_then(|e| e.id()) else {
            return;
        };
        if id == "sort-by" {
            if let Some(open) = self.actions.menu.as_mut() {
                open.hot = Some(SORT_BY);
            }
            return;
        }
        self.close_menu_tx(tx);
        match id {
            "new-folder" => {
                self.begin_new_folder();
            }
            "upload-files" => self.intent(Intent::Frame("DW-0415")),
            "view-list" => crate::frames::global::grid::switch(self, false, tx),
            "view-grid" => {
                self.intent(Intent::Frame("DW-1102"));
                crate::frames::global::grid::switch(self, true, tx);
            }
            "folders-first" => {
                let sort = self.actions.sort;
                self.set_sort(Sort {
                    folders_first: !sort.folders_first,
                    ..sort
                });
            }
            "details-panel" => self.show_detail_pane = !self.show_detail_pane,
            _ => {}
        }
    }

    /// Run the entry `index` of Sort by's choices; the menu closes.
    pub fn pick_background_sort(&mut self, index: usize, tx: &mut Tx) {
        let entries = sort_entries(self);
        let Some(id) = entries.get(index).and_then(|e| e.id()) else {
            return;
        };
        self.close_menu_tx(tx);
        let sort = self.actions.sort;
        let by = |by| Sort { by, ..sort };
        match id {
            "sort-name" => self.set_sort(by(SortBy::Name)),
            "sort-date" => self.set_sort(by(SortBy::DateChanged)),
            "sort-size" => self.set_sort(by(SortBy::Size)),
            "sort-type" => self.set_sort(by(SortBy::Type)),
            "order-forward" => self.set_sort(Sort {
                reversed: false,
                ..sort
            }),
            "order-reversed" => self.set_sort(Sort { reversed: true, ..sort }),
            _ => {}
        }
    }
}
