//! What Trash shows, and what it can do: a small trait with the board's sample as its fixture.

use crate::frames::shared::parts::table::{Extra, Kind, Row};

/// The totals in the toolbar and the confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashTotals {
    pub items: usize,
    /// As the account shows a size ("240 MB").
    pub size: String,
}

/// Emptying in progress (DW-0749b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Emptying {
    pub done: usize,
    pub of: usize,
}

pub trait TrashSource {
    /// The deleted items of the last 30 days, newest deleted first.
    fn items(&self) -> Vec<Row>;
    /// Everything in Trash, also what the list has not loaded.
    fn totals(&self) -> TrashTotals;
    fn emptying(&self) -> Option<Emptying>;
    /// Put an item back where it came from. `false`: no such item.
    fn restore(&mut self, id: &str) -> bool;
}

/// DW-0747's sample: six rows of the twelve items.
pub struct BoardTrash {
    rows: Vec<Row>,
    /// Items in Trash that are not among the listed rows.
    unlisted: usize,
    emptying: Option<Emptying>,
}

fn row(id: &str, name: &str, kind: Kind, date: &str, size: &str, from: &str) -> Row {
    Row {
        id: id.into(),
        name: name.into(),
        kind,
        date: date.into(),
        size: size.into(),
        extra: Extra::Words(format!("From {from}")),
        pc: None,
    }
}

impl BoardTrash {
    pub fn new() -> Self {
        BoardTrash {
            rows: vec![
                row(
                    "t-old-invoices",
                    "Old invoices",
                    Kind::Folder,
                    "2 Nov 2026",
                    "6 items",
                    "Documents",
                ),
                row(
                    "t-contract-old",
                    "Contract old.pdf",
                    Kind::Document,
                    "30 Oct 2026",
                    "1.1 MB",
                    "Documents",
                ),
                row(
                    "t-budget-2025",
                    "Budget 2025.xlsx",
                    Kind::Sheet,
                    "28 Oct 2026",
                    "92 KB",
                    "Files",
                ),
                row(
                    "t-img-2044",
                    "IMG_2044.HEIC",
                    Kind::Photo,
                    "24 Oct 2026",
                    "3.8 MB",
                    "Backups",
                ),
                row(
                    "t-meeting-notes",
                    "Meeting notes.pdf",
                    Kind::Document,
                    "12 Oct 2026",
                    "410 KB",
                    "Files",
                ),
                row("t-drafts", "Drafts", Kind::Folder, "3 Oct 2026", "2 items", "Documents"),
            ],
            unlisted: 6,
            emptying: None,
        }
    }

    /// DW-0117: nothing in Trash.
    pub fn empty() -> Self {
        BoardTrash {
            rows: Vec::new(),
            unlisted: 0,
            emptying: None,
        }
    }
}

impl TrashSource for BoardTrash {
    fn items(&self) -> Vec<Row> {
        self.rows.clone()
    }

    fn totals(&self) -> TrashTotals {
        TrashTotals {
            items: self.rows.len() + self.unlisted,
            size: "240 MB".into(),
        }
    }

    fn emptying(&self) -> Option<Emptying> {
        self.emptying
    }

    fn restore(&mut self, id: &str) -> bool {
        let before = self.rows.len();
        self.rows.retain(|r| r.id != id);
        self.rows.len() != before
    }
}
