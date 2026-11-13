//! Trash on the real engine: [`EngineTrashSource`], the `TrashSource` of the running app.
//!
//! Where each field comes from: the SERVER only, for the session in memory ([`Engine::trash_listing`]); nothing is
//! read from this computer's local state, which may still belong to an earlier account.
//! - name: the server row's `name_encrypted`, decrypted in the engine with the session's vault key
//!   (`owner_keys::display_name`); a name that does not decrypt reads "Encrypted item <id>…" or the blocked label.
//! - kind: from the name's extension (`drive_v2::model::mime_for_name`) and the row's `is_folder` / `kind`.
//! - deleted: the row's `updated_at` (the listing has no separate trashed-at time).
//! - size: the row's `size_bytes` (a file's; a folder's cell is empty); totals add the files' sizes.
//! - "Where it was": left empty. The parent's name would come from the local state (`trash_entries`), which may
//!   belong to an earlier account, so that column is not bound.
//! - the sidebar's count: none ([`TrashSource::count`] is `None`): the engine has no count call, and counting means
//!   reading and decrypting the whole Trash, which the window must not do to draw a badge.
//! - `restore`: [`Engine::restore_items`], one item. Not bound: `emptying`, and the actions behind Empty Trash,
//!   Details, Preview and Delete permanently (no engine call; the view says "not available yet").
//!
//! When it reads: when the Trash section opens and the list is missing or older than [`FRESH_FOR`], and after a
//! restore was answered. Never from a draw call: `items`, `totals`, `loaded` and `count` only read what is here.
//!
//! Session rule: every answer carries the session it was made for (`TrashListing::session`, the engine's session
//! generation); the window drops one that is not the current session's, and drops the list it holds the moment the
//! session moves on (a lock, a sign-out, another sign-in): each accessor compares the list's session with
//! [`Engine::session_turn`] before it hands out a name, and `forget` (the window's poll, on a gate or account change)
//! clears everything.
//!
//! Thread rule: the read and each restore run on a worker of their own and send their answer down one channel, taken
//! in by [`TrashSource::pump`] (the window's poll); the worker wakes the window ([`Engine::wake_host`]).
//!
//! Must not: log a name, an id, a path or a server's words.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::account_dto::DesktopTrashItem;
use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::drive_v2::model::mime_for_name;
use chrono::{DateTime, Local, TimeZone};

use super::log;
use crate::frames::shared::parts::table::{Extra, Kind, Row};
use crate::frames::trash::source::{Emptying, Loaded, Notice, Pumped, TrashSource, TrashTotals};

#[cfg(test)]
mod tests;

/// A list this recent is not read again when the Trash section opens.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// Trash on the real engine. See the module's doc.
pub struct EngineTrashSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The last list read, and the session it belongs to.
    listing: Option<Listing>,
    /// What the view draws while there is no list.
    loaded: Loaded,
    /// When the last read was answered (a failure too).
    read_at: Option<Instant>,
    /// The read on its way, by number.
    reading: Option<u64>,
    /// Numbers the reads.
    reads: u64,
    /// A read was asked for while one was on its way: start it when that one lands.
    again: bool,
    /// Rows taken out of the list while their restore is on its way (put back if Beebeeb refuses).
    restoring: HashMap<String, Listed>,
    /// Items Beebeeb restored, with the number of the last read that began before it answered: a read up to that
    /// number may still list them and must not bring them back.
    restored: HashMap<String, u64>,
    /// A notice for the next pump.
    notice: Option<Notice>,
    /// Something changed outside an answer (a restore started, a forget): the next pump says so.
    dirty: bool,
}

struct Listing {
    session: SessionGeneration,
    /// Newest deleted first.
    rows: Vec<Listed>,
}

/// One row, with what it was sorted by and the bytes it adds to the total.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Listed {
    pub(crate) row: Row,
    /// Deleted, in Unix seconds (`i64::MIN`: not known, last).
    pub(crate) at: i64,
    /// A file's size; a folder's 0.
    pub(crate) bytes: i64,
}

enum Answer {
    /// The read with this number, and the list with its session, or why not.
    Listed(u64, Result<(SessionGeneration, Vec<Listed>), AccountCallError>),
    /// A restore of this item for this session, and whether Beebeeb took it.
    Restored(String, SessionGeneration, bool),
}

impl<C: CredentialStores> EngineTrashSource<C> {
    /// Nothing is asked until the Trash section opens.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            listing: None,
            loaded: Loaded::Loading,
            read_at: None,
            reading: None,
            reads: 0,
            again: false,
            restoring: HashMap::new(),
            restored: HashMap::new(),
            notice: None,
            dirty: false,
        }
    }

    /// The list, while it is the current session's. Asked on every accessor: a list of a session that ended is never
    /// handed out, even before the next poll clears it.
    fn current(&self) -> Option<&Listing> {
        self.listing
            .as_ref()
            .filter(|listing| listing.session == self.engine.session_turn() && self.engine.has_session())
    }

    /// Ask Beebeeb for the Trash on a worker, unless a read is on its way.
    fn read(&mut self) {
        if self.reading.is_some() {
            self.again = true;
            return;
        }
        self.again = false;
        self.reads += 1;
        let (engine, answers, number) = (self.engine.clone(), self.answers.clone(), self.reads);
        let started = std::thread::Builder::new().name("trash-list".into()).spawn(move || {
            // A panic in this worker must not leave `reading` set for good: send an error.
            let listed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine
                    .trash_listing()
                    .map(|listing| (listing.session, rows_of(listing.items, Local::now())))
            }))
            .unwrap_or(Err(AccountCallError::Offline));
            let _ = answers.send(Answer::Listed(number, listed));
            engine.wake_host();
        });
        match started {
            Ok(_) => {
                self.reading = Some(number);
                if self.current().is_none() {
                    self.loaded = Loaded::Loading;
                }
            }
            Err(_) => {
                log("trash: could not start a thread for the list");
                self.read_at = Some(Instant::now());
                self.loaded = Loaded::Failed;
            }
        }
    }

    fn take_in(&mut self, answer: Answer) {
        let now = self.engine.session_turn();
        match answer {
            Answer::Listed(number, result) => {
                if self.reading == Some(number) {
                    self.reading = None;
                }
                let again = std::mem::take(&mut self.again);
                self.read_at = Some(Instant::now());
                match result {
                    // Made for a session that is gone: dropped, never drawn.
                    Ok((session, _)) if session != now => {}
                    Ok((session, mut rows)) => {
                        // Rows whose restore is on its way, or that Beebeeb restored after this read began, stay out.
                        rows.retain(|listed| {
                            let id = &listed.row.id;
                            !self.restoring.contains_key(id)
                                && self.restored.get(id).is_none_or(|&before| number > before)
                        });
                        // A read that began after a restore answered is the truth about it.
                        self.restored.retain(|_, before| number <= *before);
                        self.listing = Some(Listing { session, rows });
                        self.loaded = Loaded::Ready;
                    }
                    Err(kind) => {
                        if kind != AccountCallError::NotSignedIn {
                            log(format!("trash: the list did not load ({kind})"));
                        }
                        let have = self.current().is_some();
                        match kind {
                            AccountCallError::NotSignedIn => {
                                self.listing = None;
                                self.loaded = Loaded::SignedOut;
                            }
                            AccountCallError::SessionEnded => {
                                self.listing = None;
                                self.loaded = Loaded::SessionEnded;
                            }
                            // A list is shown: it stays (no row comes back), and the view says it may be old.
                            _ if have => self.notice = Some(Notice::RefreshFailed),
                            AccountCallError::Offline => self.loaded = Loaded::Offline,
                            _ => self.loaded = Loaded::Failed,
                        }
                    }
                }
                // A change was answered while this read was on its way: read what Beebeeb has now.
                if again && self.engine.has_session() {
                    self.read();
                }
            }
            Answer::Restored(id, session, taken) => {
                let row = self.restoring.remove(&id);
                if session != now {
                    return;
                }
                if taken {
                    self.restored.insert(id, self.reads);
                } else {
                    log("trash: an item was not restored");
                    if let (Some(row), Some(listing)) = (row, self.listing.as_mut()) {
                        self.notice = Some(Notice::RestoreRefused(row.row.name.clone()));
                        listing.rows.push(row);
                        listing.rows.sort_by_key(|listed| std::cmp::Reverse(listed.at));
                    }
                }
                // Read again what Beebeeb has now.
                self.read();
            }
        }
    }
}

#[cfg(test)]
impl<C: CredentialStores> EngineTrashSource<C> {
    /// A read or a restore is on its way (the tests wait for it).
    fn busy(&self) -> bool {
        self.reading.is_some() || !self.restoring.is_empty()
    }

    /// The last answer is old: the next opening reads again (instead of waiting [`FRESH_FOR`]).
    fn expire(&mut self) {
        self.read_at = None;
    }
}

impl<C: CredentialStores> TrashSource for EngineTrashSource<C> {
    fn items(&self) -> Vec<Row> {
        self.current()
            .map(|listing| listing.rows.iter().map(|listed| listed.row.clone()).collect())
            .unwrap_or_default()
    }

    fn totals(&self) -> TrashTotals {
        match self.current() {
            Some(listing) => TrashTotals {
                items: listing.rows.len(),
                size: crate::source::format_size(listing.rows.iter().map(|listed| listed.bytes).sum()),
            },
            None => TrashTotals {
                items: 0,
                size: String::new(),
            },
        }
    }

    /// Not bound: the engine has no empty-Trash call yet.
    fn emptying(&self) -> Option<Emptying> {
        None
    }

    /// Sent once, on a worker; the row leaves the list at once and comes back only if Beebeeb refuses. `false`: no
    /// such row in the current session's list (or no worker could start).
    fn restore(&mut self, id: &str) -> bool {
        let session = self.engine.session_turn();
        let Some(at) = self
            .current()
            .and_then(|listing| listing.rows.iter().position(|l| l.row.id == id))
        else {
            return false;
        };
        let (engine, answers, item) = (self.engine.clone(), self.answers.clone(), id.to_string());
        let started = std::thread::Builder::new().name("trash-restore".into()).spawn(move || {
            let taken = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.restore_items(&[item.as_str()]).is_ok()
            }))
            .unwrap_or(false);
            let _ = answers.send(Answer::Restored(item, session, taken));
            engine.wake_host();
        });
        if started.is_err() {
            log("trash: could not start a thread for a restore");
            return false;
        }
        if let Some(listing) = self.listing.as_mut() {
            let row = listing.rows.remove(at);
            self.restoring.insert(id.to_string(), row);
        }
        self.dirty = true;
        true
    }

    fn loaded(&self) -> Loaded {
        if !self.engine.has_session() {
            Loaded::SignedOut
        } else if self.current().is_some() {
            Loaded::Ready
        } else {
            self.loaded
        }
    }

    fn opened(&mut self) {
        if !self.engine.has_session() {
            return;
        }
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    fn pump(&mut self) -> Pumped {
        let mut changed = std::mem::take(&mut self.dirty);
        // The session moved on since the list was read: it goes now, on the window's side.
        if self.listing.is_some() && self.current().is_none() {
            self.forget();
            changed = true;
        }
        while let Ok(answer) = self.inbox.try_recv() {
            self.take_in(answer);
            changed = true;
        }
        let notice = self.notice.take();
        Pumped {
            changed: changed || notice.is_some(),
            busy: self.reading.is_some() || !self.restoring.is_empty(),
            notice,
        }
    }

    fn forget(&mut self) {
        self.listing = None;
        self.restoring.clear();
        self.restored.clear();
        self.again = false;
        self.read_at = None;
        self.notice = None;
        self.loaded = Loaded::Loading;
        self.dirty = true;
    }

    /// None: the engine has no count call, and the window must not read the whole Trash for a badge.
    fn count(&self) -> Option<usize> {
        None
    }

    /// Empty Trash, Details, Preview and Delete permanently have no engine call yet.
    fn actions_available(&self) -> bool {
        false
    }
}

/// The server's Trash rows as the view's rows, newest deleted first, with what each was sorted by. Nothing here
/// reads the local state (see the module's doc).
pub(crate) fn rows_of(items: Vec<DesktopTrashItem>, now: DateTime<Local>) -> Vec<Listed> {
    let mut rows: Vec<Listed> = items
        .into_iter()
        .map(|item| {
            let at = seconds_of(&item.updated_at);
            let bytes = if item.is_folder { 0 } else { item.size_bytes.max(0) };
            let kind = kind_of(item.is_folder, mime_for_name(&item.name));
            let size = if item.is_folder {
                String::new()
            } else {
                crate::source::format_size(bytes)
            };
            Listed {
                row: Row {
                    id: item.id,
                    name: item.name,
                    kind,
                    date: at.map(|at| when(at, now)).unwrap_or_default(),
                    size,
                    // Where it was: not bound (the parent's name is local state; see the module's doc).
                    extra: Extra::Words(String::new()),
                    pc: None,
                },
                at: at.unwrap_or(i64::MIN),
                bytes,
            }
        })
        .collect();
    rows.sort_by_key(|listed| std::cmp::Reverse(listed.at));
    rows
}

/// The table's kind of an item: a folder, a picture (no thumbnail of its own here), a sheet, else a document.
pub(crate) fn kind_of(is_folder: bool, mime: Option<&str>) -> Kind {
    match mime {
        _ if is_folder => Kind::Folder,
        Some(m) if m.starts_with("image/") => Kind::Photo(None),
        Some(m) if m.contains("spreadsheet") || m.contains("ms-excel") || m == "text/csv" => Kind::Sheet,
        _ => Kind::Document,
    }
}

/// A server time as Unix seconds: RFC 3339, or a whole number of seconds. `None`: neither.
pub(crate) fn seconds_of(stamp: &str) -> Option<i64> {
    let stamp = stamp.trim();
    DateTime::parse_from_rfc3339(stamp)
        .map(|at| at.timestamp())
        .ok()
        .or_else(|| stamp.parse::<i64>().ok())
}

/// A time as the board shows it, on this computer's clock: "Today, 07:02", "Yesterday, 16:42", else "2 Nov 2026".
pub(crate) fn when(seconds: i64, now: DateTime<Local>) -> String {
    let Some(at) = Local.timestamp_opt(seconds, 0).single() else {
        return String::new();
    };
    let (day, today) = (at.date_naive(), now.date_naive());
    let time = at.format("%H:%M");
    if day == today {
        format!("Today, {time}")
    } else if today.pred_opt() == Some(day) {
        format!("Yesterday, {time}")
    } else {
        at.format("%-d %b %Y").to_string()
    }
}
