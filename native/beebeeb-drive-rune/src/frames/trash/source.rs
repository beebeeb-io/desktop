//! What Trash shows, and what it can do: a small trait with the board's sample as its fixture.

use crate::frames::shared::parts::table::{Extra, Kind, Row, Thumb};

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

/// How far a source that answers late has got with the list (the view draws each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Loaded {
    /// The list is here (an empty one too).
    #[default]
    Ready,
    /// The first answer is on its way.
    Loading,
    /// Beebeeb could not be reached, and no list was read.
    Offline,
    /// Beebeeb answered with an error, and no list was read.
    Failed,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    /// No session in memory: signed out, or locked.
    SignedOut,
}

/// Something the view says once, over the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Beebeeb refused to put this item back; it is still in Trash (the item's name).
    RestoreRefused(String),
    /// A restore pressed before Beebeeb was locked or signed out was not made; the item is still in Trash. No name:
    /// none is held across a lock.
    RestoreNotDone,
    /// A newer list could not be read; the one shown may be out of date.
    RefreshFailed,
    /// The control needs something this version does not have yet.
    NotAvailable,
    /// Deleting for good stopped before it was done (what is still in Trash stays there). `done`: how many are gone;
    /// `why`: the reason in words, never a server's.
    PurgeStopped { done: usize, why: String },
}

/// How a delete for good (Empty Trash, Delete permanently) ended, for the dialog that asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PurgeEnd {
    /// Everything asked for is gone.
    Done,
    /// The password was not the account's: nothing was deleted, and the dialog stays for another try.
    WrongPassword,
    /// It stopped early (the notice says why); the dialog closes.
    Stopped,
}

/// What [`TrashSource::pump`] took in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Something the view draws changed.
    pub changed: bool,
    /// A read or a restore is still on its way: poll again.
    pub busy: bool,
    pub notice: Option<Notice>,
    /// A delete for good that was asked for ended.
    pub purge: Option<PurgeEnd>,
}

pub trait TrashSource {
    /// `false`: nothing answers yet (`Unwired`), and the view draws the shell's "not built yet"
    /// frame instead of sample rows.
    fn wired(&self) -> bool {
        true
    }
    /// The deleted items of the last 30 days, newest deleted first.
    fn items(&self) -> Vec<Row>;
    /// Everything in Trash, also what the list has not loaded.
    fn totals(&self) -> TrashTotals;
    fn emptying(&self) -> Option<Emptying>;
    /// Put an item back where it came from. `false`: no such item.
    fn restore(&mut self, id: &str) -> bool;
    /// How far the list has got. A source that answers at once is always `Ready`.
    fn loaded(&self) -> Loaded {
        Loaded::Ready
    }
    /// The Trash section opened: read the list if it is missing or old.
    fn opened(&mut self) {}
    /// Take in what arrived since the last call (the window's poll).
    fn pump(&mut self) -> Pumped {
        Pumped::default()
    }
    /// The session or the account changed: drop every name and count at once.
    fn forget(&mut self) {}
    /// The sidebar's count. `None`: no badge (a source that cannot count without reading the whole list).
    fn count(&self) -> Option<usize> {
        Some(self.totals().items)
    }
    /// Whether Empty Trash, Details, Preview and Delete permanently can do anything here. `false`: pressing them
    /// says "not available yet" instead.
    fn actions_available(&self) -> bool {
        true
    }
    /// Whether Empty Trash and Delete permanently can be done here (a password is asked first). `false`: pressing
    /// them says "not available yet" instead.
    fn purge_available(&self) -> bool {
        self.actions_available()
    }
    /// Delete for good after proving the account `password`: the item `id`, or `None` for everything in Trash.
    /// `false`: nothing was started (no such item, nothing to delete, another delete is on its way, or this source
    /// cannot). The answer comes through [`TrashSource::pump`] ([`Pumped::purge`]).
    fn purge(&mut self, _id: Option<&str>, _password: &str) -> bool {
        false
    }
    /// A delete for good is on its way (the dialog's button waits).
    fn purging(&self) -> bool {
        false
    }
}

/// DW-0747's sample: six rows of the twelve items.
pub struct BoardTrash {
    rows: Vec<Row>,
    /// Items in Trash that are not among the listed rows.
    unlisted: usize,
    emptying: Option<Emptying>,
    /// How the last delete for good ended (told once, by the next pump).
    ended: Option<PurgeEnd>,
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
                    Kind::Photo(Some(Thumb::BoardLeaves)),
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
            ended: None,
        }
    }

    /// DW-0749b: emptying, five of the twelve deleted for good; three of the seven left are listed.
    pub fn emptying() -> Self {
        let mut trash = BoardTrash::new();
        trash.rows.drain(..3);
        trash.unlisted = 4;
        trash.emptying = Some(Emptying { done: 5, of: 12 });
        trash
    }

    /// These rows and nothing else (a test's own Trash).
    #[cfg(test)]
    pub fn of(rows: Vec<Row>) -> Self {
        BoardTrash {
            rows,
            unlisted: 0,
            emptying: None,
            ended: None,
        }
    }

    /// DW-0117: nothing in Trash.
    pub fn empty() -> Self {
        BoardTrash {
            rows: Vec::new(),
            unlisted: 0,
            emptying: None,
            ended: None,
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

    /// The board's sample takes any non-empty password (it proves nothing: no account is behind it).
    fn purge(&mut self, id: Option<&str>, password: &str) -> bool {
        if password.is_empty() {
            return false;
        }
        let started = match id {
            Some(id) => self.restore(id),
            None => {
                let any = !self.rows.is_empty();
                self.rows.clear();
                self.unlisted = 0;
                any
            }
        };
        if started {
            self.ended = Some(PurgeEnd::Done);
        }
        started
    }

    fn pump(&mut self) -> Pumped {
        let purge = self.ended.take();
        Pumped {
            changed: purge.is_some(),
            purge,
            ..Pumped::default()
        }
    }
}

/// The source of a build that ships before the engine lane has wired Trash: it answers
/// nothing, and the view draws the shell's "not built yet" frame instead of sample rows.
// Not used by the running app since Trash reads the engine; kept for a build or a test that turns Trash off.
#[allow(dead_code)]
pub struct Unwired;

impl TrashSource for Unwired {
    fn wired(&self) -> bool {
        false
    }
    fn items(&self) -> Vec<Row> {
        Vec::new()
    }
    fn totals(&self) -> TrashTotals {
        TrashTotals {
            items: 0,
            size: String::new(),
        }
    }
    fn emptying(&self) -> Option<Emptying> {
        None
    }
    fn restore(&mut self, _id: &str) -> bool {
        false
    }
}
