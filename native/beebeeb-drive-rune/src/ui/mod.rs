//! UI components for the Beebeeb Drive window: the design system every frame builds on.

pub mod caption;
pub mod detail_pane;
pub mod file_list;
pub mod gate;
pub mod kit;
pub mod overlays;
pub mod sidebar;
pub mod status_bar;
pub mod toolbar;
pub mod window;

#[cfg(test)]
mod tests;

use crate::State;
use rune::TextEdit;

/// What the Files window shows over or instead of its plain list: an open menu, further
/// selected rows, a name being edited, a list that is empty or could not load.
#[derive(Default)]
pub struct FilesUi {
    pub menu: Option<overlays::Menu>,
    /// Rows selected beside `State::selected` (Ctrl+click, Shift+click).
    pub also_selected: Vec<usize>,
    /// The row whose name is being edited, and the field.
    pub rename: Option<(usize, TextEdit)>,
}

impl FilesUi {
    pub fn renaming(&self, row: usize) -> Option<&TextEdit> {
        self.rename.as_ref().filter(|(i, _)| *i == row).map(|(_, e)| e)
    }
}

impl State {
    pub fn menu_open(&self, name: &str) -> bool {
        self.files.menu.as_ref().is_some_and(|m| m.name() == name)
    }

    pub fn toggle_menu(&mut self, name: &str) {
        self.files.menu = if self.menu_open(name) {
            None
        } else {
            overlays::Menu::named(name, self.selected)
        };
    }

    pub fn is_selected(&self, row: usize) -> bool {
        self.selected == Some(row) || self.files.also_selected.contains(&row)
    }

    /// Every selected row, in list order.
    pub fn selection(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .selected
            .into_iter()
            .chain(self.files.also_selected.iter().copied())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    pub fn select_only(&mut self, row: usize) {
        self.selected = Some(row);
        self.files.also_selected.clear();
        self.files.menu = None;
    }
}
