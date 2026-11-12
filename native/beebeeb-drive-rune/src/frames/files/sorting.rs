//! The order of the rows: the folder's sort, a click on a column header, and the order itself
//! (DW-0403, DW-0404). A protected folder keeps the order its owner gives it.

use super::{Sort, SortBy, long_list};
use crate::State;
use crate::source::DriveItem;

impl State {
    /// Change this folder's sort and put the rows in that order.
    pub fn set_sort(&mut self, sort: Sort) {
        // A protected folder keeps its owner's order: no choice of the Sort menu applies there.
        if self.in_protected_folder() {
            return;
        }
        self.actions.sort = sort;
        // The order of the rows is the window's; the source hears of it only with a session.
        if !self.session_ended() {
            self.actions.source.set_sort(self.listing.folder.id.as_deref(), sort);
        }
        self.forget_undo();
        self.apply_sort();
    }

    /// A click on a column header: sort by it; a second click reverses the order (DW-0404).
    pub fn sort_by_column(&mut self, by: SortBy) {
        let sort = self.actions.sort;
        self.set_sort(if sort.by == by {
            Sort {
                reversed: !sort.reversed,
                ..sort
            }
        } else {
            Sort {
                by,
                reversed: false,
                ..sort
            }
        });
    }

    /// Put the rows in the folder's order; what was selected stays selected. Backups stays the
    /// first row whatever the sort (DW-0404).
    pub fn apply_sort(&mut self) {
        // A protected folder stays in the order its owner gives it (devices, then services; the
        // years newest first: DW-0422, DW-0424).
        if self.in_protected_folder() {
            return;
        }
        let sort = self.actions.sort;
        let id_of = |s: &State, row: usize| s.listing.items.get(row).map(|i| i.id.clone());
        let lead = self.selected.and_then(|i| id_of(self, i));
        let anchor = self.actions.anchor.and_then(|i| id_of(self, i));
        let selected: Vec<String> = self.actions.selection.iter().filter_map(|&i| id_of(self, i)).collect();
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
            let name = long_list::sort_key(&a.name).cmp(&long_list::sort_key(&b.name));
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
        let row_of = |s: &State, id: &String| s.listing.items.iter().position(|i| &i.id == id);
        self.selected = lead.and_then(|id| row_of(self, &id));
        self.actions.anchor = anchor.and_then(|id| row_of(self, &id));
        self.actions.selection = selected.iter().filter_map(|id| row_of(self, id)).collect();
    }
}
