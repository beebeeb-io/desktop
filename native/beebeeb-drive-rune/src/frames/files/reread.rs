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

/// Reads a folder (`None`: the root) as the window holds it (`shell::state::load`). It may take long on a large
/// account: the running app calls it on a worker, never on the window's thread.
pub type FolderReader = Arc<dyn Fn(Option<&str>) -> (Listing, Option<SourceError>) + Send + Sync>;

/// Why a folder read was asked for, which says how its answer is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadFor {
    /// The folder shown, read again after a check: merged by id ([`State::take_reread`]).
    Reread,
    /// Another folder opened (Open, Forward, Back, a path's folder): it replaces the rows.
    Open,
    /// The root, read after the session gate opened.
    Gate,
}

/// A folder read on a worker, stamped with the folder and the account that asked for it.
pub struct PendingRead {
    pub(crate) folder: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) kind: ReadFor,
    answer: std::sync::mpsc::Receiver<(Listing, Option<SourceError>)>,
}

/// The Files window's folder reads on a worker (task 050): while one is out the rows shown stay (no flash); ONE read
/// is out at a time, and a newer open replaces the one before (its answer is dropped). `reader: None` (a scene, a
/// test, a `--db` window): every read is made at once on the caller's thread, as before.
#[derive(Default)]
pub struct FolderReads {
    pub(crate) reader: Option<FolderReader>,
    pub(crate) pending: Option<PendingRead>,
}

impl FolderReads {
    /// Asks the worker for `folder`. `false`: no reader is wired, nothing was asked.
    pub(crate) fn ask(&mut self, folder: Option<String>, account: Option<String>, kind: ReadFor) -> bool {
        let Some(reader) = self.reader.clone() else {
            return false;
        };
        let (send, answer) = std::sync::mpsc::channel();
        let (asked, worker, on_worker) = (folder.clone(), send.clone(), reader.clone());
        let spawned = std::thread::Builder::new().name("files-read".into()).spawn(move || {
            let _ = worker.send(on_worker(asked.as_deref()));
        });
        if spawned.is_err() {
            // No thread: the read is made here, as without a reader (never a folder that is never read).
            let _ = send.send(reader(folder.as_deref()));
        }
        self.pending = Some(PendingRead {
            folder,
            account,
            kind,
            answer,
        });
        true
    }

    /// The answer of the read out, once it came (the read is no longer out then). A worker that died answers nothing.
    pub(crate) fn take(&mut self) -> Option<(PendingRead, Listing, Option<SourceError>)> {
        let pending = self.pending.as_ref()?;
        match pending.answer.try_recv() {
            Ok((listing, error)) => self.pending.take().map(|p| (p, listing, error)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                None
            }
        }
    }

    /// Forgets the read out: its answer, when it comes, is dropped.
    pub(crate) fn drop_pending(&mut self) {
        self.pending = None;
    }

    pub(crate) fn opening(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.kind != ReadFor::Reread)
    }
}
