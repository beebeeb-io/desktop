//! UI components for the Beebeeb Drive window: the design system every frame builds on.

pub mod au;
pub mod button;
pub mod caption;
pub mod detail_pane;
pub mod empty;
pub mod file_list;
pub mod gate;
pub mod keys;
pub mod kit;
pub mod menu;
pub mod settings_card;
pub mod settings_controls;
pub mod settings_dialog;
pub mod settings_list;
pub mod settings_notice;
pub mod settings_shell;
pub mod sidebar;
pub mod status_bar;
pub mod status_line;
pub mod toast;
pub mod toolbar;
pub mod window;

#[cfg(test)]
mod tests;

use crate::State;

/// What the Files window keeps beside the listing: where Back goes.
#[derive(Default)]
pub struct FilesUi {
    /// The folders left behind, nearest last (`None` is the root).
    pub back: Vec<Option<String>>,
}

impl State {
    /// Select one row; the details panel follows.
    pub fn select(&mut self, row: usize) {
        if row < self.listing.items.len() {
            self.selected = Some(row);
        }
    }

    /// Move the selection by `delta` rows, stopping at the ends. With nothing selected, the first row.
    pub fn move_selection(&mut self, delta: isize) {
        let count = self.listing.items.len();
        if count == 0 {
            return;
        }
        self.selected = Some(match self.selected {
            Some(i) => i.saturating_add_signed(delta).min(count - 1),
            None => 0,
        });
    }

    /// Open the selected row if it is a folder: the list becomes its contents. A file does
    /// nothing here (opening it is the engine's, not the fixture's).
    pub fn open_selected(&mut self) -> bool {
        let Some(item) = self.selected.and_then(|i| self.listing.items.get(i)) else {
            return false;
        };
        if !item.is_folder {
            return false;
        }
        let id = item.id.clone();
        self.files.back.push(self.listing.folder.id.clone());
        (self.listing, self.list_error) = crate::load(self.source.as_ref(), Some(&id));
        self.selected = None;
        true
    }

    pub fn can_go_back(&self) -> bool {
        !self.files.back.is_empty()
    }

    /// Back to the folder this one was opened from.
    pub fn go_back(&mut self) -> bool {
        let Some(parent) = self.files.back.pop() else {
            return false;
        };
        (self.listing, self.list_error) = crate::load(self.source.as_ref(), parent.as_deref());
        self.selected = None;
        true
    }
}
