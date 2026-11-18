//! Sort (DW-0404) and the toolbar's search for Shared and Trash: the menu, the order, the filter.
//!
//! The order is of the source's raw facts (`Row::bytes`, `Row::at`, the name), never of the drawn words. The menu is
//! the sections' own `parts::menu`, under the toolbar's Sort button.

use rune::Tx;

use super::parts::menu::{Entry, Open, Place};
use super::parts::table::Row;
use crate::State;
use crate::frames::Intent;

/// The key of the toolbar's Sort button (`ui::toolbar`): the sort menu opens under it, which is also how an open menu
/// is known for the sort menu.
pub const SORT_BUTTON: &str = "sort-btn";

/// What the rows are ordered by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum By {
    Name,
    /// Newest first.
    #[default]
    Date,
    /// Largest first.
    Size,
}

/// A section's sort and what its search field filters by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionSort {
    pub by: By,
    /// The other way round: Z to A, oldest first, smallest first.
    pub reversed: bool,
    /// What the toolbar's search asked for: only rows whose name holds it (any case) are listed.
    pub filter: String,
}

pub const SORT_MENU: &[Entry] = &[
    Entry::new("Name"),
    Entry::new("Date"),
    Entry::new("Size"),
    Entry::new("Reverse order").rule(),
];

/// The open menu is the sort menu.
pub fn is_sort_menu(open: &Open) -> bool {
    matches!(&open.place, Place::Under(key) if key == SORT_BUTTON)
}

impl SectionSort {
    /// The sort menu's item `n`: a key sorts by it (the same key again reverses), the last item reverses.
    pub fn pick(&mut self, n: usize) {
        let by = match n {
            0 => By::Name,
            1 => By::Date,
            2 => By::Size,
            _ => {
                self.reversed = !self.reversed;
                return;
            }
        };
        self.reversed = if by == self.by { !self.reversed } else { false };
        self.by = by;
    }

    /// `rows` as this sort and filter list them. The order is stable: rows that tie keep the source's order (the
    /// default, newest first, leaves a source that gives no times as it is).
    pub fn arrange(&self, rows: Vec<Row>) -> Vec<Row> {
        let needle = self.filter.trim().to_lowercase();
        let mut rows: Vec<Row> = rows
            .into_iter()
            .filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle))
            .collect();
        match self.by {
            By::Name => rows.sort_by_key(|r| r.name.to_lowercase()),
            By::Date => rows.sort_by_key(|r| std::cmp::Reverse(r.at.unwrap_or(i64::MIN))),
            By::Size => rows.sort_by_key(|r| std::cmp::Reverse(r.bytes.unwrap_or(0))),
        }
        // Name runs A to Z; the others run biggest or newest first: reversing is the same flip for all.
        if self.reversed {
            rows.reverse();
        }
        rows
    }
}

/// Takes `intent` when it is one of these; `false`: not taken.
pub fn take(s: &mut State, _tx: &mut Tx, intent: &Intent) -> bool {
    let ui = &mut s.frames.shared;
    match intent {
        Intent::Frame("DW-0404") => {
            ui.menu = Some(Open::under(SORT_BUTTON));
            true
        }
        Intent::Do("search", typed) => {
            ui.sort.filter = typed.clone();
            ui.selected = None;
            true
        }
        _ => false,
    }
}
