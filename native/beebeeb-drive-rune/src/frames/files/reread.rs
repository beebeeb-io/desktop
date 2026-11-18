//! The folder the Files window shows, read again after the engine finished a check (`shell::host`,
//! `read_again_after_a_check`), and merged into the view. What arrived (synced from this PC's folder, or from another
//! device), was renamed or left shows without a click, as in a file manager; nothing else of the view moves:
//! - the selection, the focused row (the keyboard's, which the details panel shows) and where a Shift range starts
//!   stay with their items, by id; an item that left goes from them;
//! - a row that is still there keeps its place, with what the read says of it now (a new name, size or date does not
//!   move it until the folder is sorted again, as in File Explorer); a new row stands where the folder's sort puts it
//!   (`sorting::compare`); a protected folder shows its owner's order, as read. The Files list has no filter to keep;
//! - scrolled down the list, the first row in view stays where it is on screen when rows land or leave above it.
//!
//! While something is open over the rows (a name being typed in a row, a menu, a drag, a dialog) the read waits
//! ([`State::reread_waits`]) and runs once it closed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::State;
use crate::source::{Listing, SourceError};
use crate::ui::file_list::ROW_H;

/// The space above the list's first row inside its scroll viewport (`list::rows`: `.py(4.0)`).
pub(super) const LIST_TOP: f32 = 4.0;

/// How far down the Files list is scrolled, in points, as its viewport last said (`list::rows`, `on_scroll_offset`).
/// A viewport says it while a frame draws, which cannot change the state: it is kept beside it.
#[derive(Clone, Default)]
pub struct ListScroll(Arc<AtomicU32>);

impl ListScroll {
    pub fn set(&self, y: f32) {
        self.0.store(y.to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

impl State {
    /// Something is open over the rows that a read would move under it: a name typed in a row (its field stands on a
    /// row), a menu (opened for these rows), files dragged over the list (the row under them), a dialog of the Files
    /// window or of the shell, the message of a protected folder. The read of a finished check waits until it closed.
    pub(crate) fn reread_waits(&self) -> bool {
        let actions = &self.actions;
        actions.rename.is_some()
            || actions.menu.is_some()
            || actions.upload_menu.is_some()
            || actions.drop_hover.is_some()
            || actions.dialog.is_some()
            || actions.protected.alert.is_some()
            || self.shell.app.dialog().is_some()
    }

    /// Takes the folder shown, read again, into the view (the module's rules). Returns how far down the list is to be
    /// scrolled so the rows in view stay where they were on screen; `None`: as it is.
    ///
    /// A read that failed: a list with rows keeps them when the data could not be read for a moment (the next check
    /// reads again); a folder that is gone, or data that is not this account's, shows what navigation shows, and
    /// nothing of the rows stays.
    pub(crate) fn take_reread(&mut self, read: Listing, error: Option<SourceError>) -> Option<f32> {
        if let Some(error) = error {
            if error == SourceError::Unreadable && self.list_error.is_none() && !self.listing.items.is_empty() {
                return None;
            }
            self.listing = read;
            self.list_error = Some(error);
            self.selected = None;
            self.actions.selection.clear();
            self.actions.anchor = None;
            return None;
        }
        let id_of = |s: &State, row: usize| s.listing.items.get(row).map(|item| item.id.clone());
        let focus = self.selected.and_then(|row| id_of(self, row));
        let anchor = self.actions.anchor.and_then(|row| id_of(self, row));
        let chosen: Vec<String> = self
            .actions
            .selection
            .iter()
            .filter_map(|&row| id_of(self, row))
            .collect();
        // The first row in view (list view, scrolled off its top), or the first after it that is still there: its row
        // before the read, and its item.
        let scrolled = self.actions.list_scroll.get();
        let held = if scrolled > 0.0 && !crate::frames::global::grid::on(self) {
            let first = ((scrolled - LIST_TOP).max(0.0) / ROW_H) as usize;
            let kept: HashSet<&str> = read.items.iter().map(|item| item.id.as_str()).collect();
            (first..self.listing.items.len())
                .find(|&row| kept.contains(self.listing.items[row].id.as_str()))
                .map(|row| (row, self.listing.items[row].id.clone()))
        } else {
            None
        };
        // A protected folder keeps the order its owner gives it (`sorting.rs`): as read.
        let owners_order = self
            .actions
            .source
            .protection(read.folder.id.as_deref(), None)
            .is_some();
        let Listing { folder, path, items } = read;
        let items = if owners_order {
            items
        } else {
            let shown: HashSet<&str> = self.listing.items.iter().map(|item| item.id.as_str()).collect();
            let (still, new): (Vec<_>, Vec<_>) = items.into_iter().partition(|item| shown.contains(item.id.as_str()));
            let mut now: HashMap<String, _> = still.into_iter().map(|item| (item.id.clone(), item)).collect();
            let mut kept = self
                .listing
                .items
                .iter()
                .filter_map(|row| now.remove(&row.id))
                .collect::<Vec<_>>()
                .into_iter()
                .peekable();
            // The new rows in the folder's sort, each before the first row still there that comes after it: one pass,
            // however many arrived (the first sync brings every row at once).
            let sort = self.actions.sort;
            let mut new = new;
            new.sort_by(|a, b| super::sorting::compare(sort, a, b));
            let mut rows = Vec::with_capacity(kept.len() + new.len());
            for item in new {
                while let Some(row) =
                    kept.next_if(|row| super::sorting::compare(sort, row, &item) != std::cmp::Ordering::Greater)
                {
                    rows.push(row);
                }
                rows.push(item);
            }
            rows.extend(kept);
            rows
        };
        self.listing = Listing { folder, path, items };
        self.list_error = None;
        let row_of = |s: &State, id: &str| s.listing.items.iter().position(|item| item.id == id);
        self.selected = focus.and_then(|id| row_of(self, id.as_str()));
        self.actions.anchor = anchor.and_then(|id| row_of(self, id.as_str()));
        self.actions.selection = chosen.iter().filter_map(|id| row_of(self, id.as_str())).collect();
        if let Some(y) = self.apply_reveal() {
            return Some(y);
        }
        let (before, id) = held?;
        let now = row_of(self, id.as_str())?;
        (now != before).then(|| (scrolled + (now as f32 - before as f32) * ROW_H).max(0.0))
    }
}
