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
//! - `restore`: [`Engine::restore_items_for`], one item, for the session of the press. Not bound: `emptying`, and the actions behind Empty Trash,
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
//! A restore is bound to the session AND the account of the press: both are read on the window's thread before the
//! list is looked at, the engine sends nothing unless the session in memory is still that one (asked under its
//! session-write lock, with the token copy), and an answer that lands under another session or account touches no
//! row. A press whose restore was not made because the session moved on is told to the account that pressed, once
//! its session is in memory again, without a name (none is held across a lock); never to another account.
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
    /// Rows taken out of the list while their restore is on its way (put back if Beebeeb refuses), each with the
    /// session of its press: only that press's answer takes it out again.
    restoring: HashMap<String, (SessionGeneration, Listed)>,
    /// Items Beebeeb restored, with the number of the last read that began before it answered: a read up to that
    /// number may still list them and must not bring them back.
    restored: HashMap<String, u64>,
    /// A notice for the next pump.
    notice: Option<Notice>,
    /// The account whose restore was not made because its session moved on: told by the first poll that finds that
    /// account's session in memory (a lock may be in between); dropped when another account, or none, is here.
    /// Outlives `forget`, and holds no name of an item.
    not_done_for: Option<String>,
    /// Tests only: the next restore waits on the first, on its worker, before it calls the engine, and says on the
    /// second that the engine's call ended.
    #[cfg(test)]
    next_restore: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
    /// Something changed outside an answer (a restore started, a forget): the next pump says so.
    dirty: bool,
}

struct Listing {
    session: SessionGeneration,
    /// Newest deleted first.
    rows: Vec<Listed>,
}

/// One row, with what it was sorted by and the bytes it adds to the total.
#[derive(Clone, PartialEq)]
pub(crate) struct Listed {
    pub(crate) row: Row,
    /// Deleted, in Unix seconds (`i64::MIN`: not known, last).
    pub(crate) at: i64,
    /// A file's size; a folder's 0.
    pub(crate) bytes: i64,
}

/// Never the row: it holds a decrypted name, and a `{:?}` must not print one.
impl std::fmt::Debug for Listed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listed")
            .field("at", &self.at)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

enum Answer {
    /// The read with this number, and the list with its session, or why not.
    Listed(u64, Result<(SessionGeneration, Vec<Listed>), AccountCallError>),
    /// A restore of this item, the session and account of its press, and whether Beebeeb took it.
    Restored(String, Stamp, Result<(), AccountCallError>),
}

/// The session and the account a restore was pressed under.
type Stamp = (SessionGeneration, Option<String>);

/// Whose account is here: the stored email, else the user id the server named (as the Settings source names it).
/// In memory only: never waits.
fn account_of<C: CredentialStores>(engine: &Engine<C>) -> Option<String> {
    engine.account_email().or_else(|| engine.user_id())
}

/// The session and account a press on `engine` is made under. Taken on the window's thread, before a worker starts.
fn stamp<C: CredentialStores>(engine: &Engine<C>) -> Stamp {
    (engine.session_turn(), account_of(engine))
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
            not_done_for: None,
            #[cfg(test)]
            next_restore: None,
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
            Answer::Restored(id, pressed, result) => {
                // The row in flight is this press's only when it left the list under the session of this press: an
                // answer of an earlier session never takes out a row a later press put there.
                let row = match self.restoring.get(&id) {
                    Some((session, _)) if *session == pressed.0 => self.restoring.remove(&id).map(|(_, row)| row),
                    _ => None,
                };
                // Drawn only for the session AND the account of the press. The session's turn only moves forward,
                // so a stamp that is current now was current for the whole call.
                if !(self.engine.has_session() && pressed == stamp(&self.engine)) {
                    self.answered_late(pressed, result);
                    return;
                }
                match result {
                    Ok(()) => {
                        self.restored.insert(id, self.reads);
                    }
                    Err(kind) => {
                        log(format!("trash: an item was not restored ({kind})"));
                        if let (Some(row), Some(listing)) = (row, self.listing.as_mut()) {
                            self.notice = Some(Notice::RestoreRefused(row.row.name.clone()));
                            listing.rows.push(row);
                            listing.rows.sort_by_key(|listed| std::cmp::Reverse(listed.at));
                        }
                    }
                }
                // Read again what Beebeeb has now.
                self.read();
            }
        }
    }
}

impl<C: CredentialStores> EngineTrashSource<C> {
    /// A restore answered under another session or account than its press: no row is touched and nothing of it is
    /// drawn. The account that pressed is told when the restore was certainly not made; when it may have been, what
    /// is listed for that account is read again.
    fn answered_late(&mut self, pressed: Stamp, result: Result<(), AccountCallError>) {
        // The account that pressed is the one this engine names now (its vault may be locked).
        let theirs = pressed.1.is_some() && pressed.1 == account_of(&self.engine);
        if let Err(kind) = &result {
            log(format!("trash: an item was not restored ({kind})"));
        }
        if !theirs {
            return;
        }
        match result {
            // Still in Trash for certain: the engine sent nothing (the session of the press is gone: a lock, a
            // sign-out, another sign-in), or Beebeeb answered and said no.
            Err(AccountCallError::NotSignedIn | AccountCallError::ServerError | AccountCallError::SessionEnded) => {
                self.not_done_for = pressed.1;
            }
            // Taken under the session of the press, or sent with an answer that never came: a list read since may
            // be out of date. The next opening reads again, and a list on screen is read again now.
            _ => {
                self.read_at = None;
                if self.current().is_some() {
                    self.read();
                }
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
        // The session and account of the press, read BEFORE the list the item is chosen from is looked at: the
        // item is sent for this session or not at all.
        let pressed = stamp(&self.engine);
        let Some(at) = self
            .current()
            .filter(|listing| listing.session == pressed.0 && pressed.1.is_some())
            .and_then(|listing| listing.rows.iter().position(|l| l.row.id == id))
        else {
            return false;
        };
        let (engine, answers, item) = (self.engine.clone(), self.answers.clone(), id.to_string());
        let stamped = pressed.clone();
        #[cfg(test)]
        let hooks = self.next_restore.take();
        let started = std::thread::Builder::new().name("trash-restore".into()).spawn(move || {
            #[cfg(test)]
            if let Some((hold, _)) = &hooks {
                let _ = hold.recv();
            }
            // The guard is the engine's: it copies the token only while the session in memory is still the one of
            // the press, asked under its session-write lock, and sends nothing otherwise. A check here before the
            // call could not be atomic with the send, so there is none to rely on: the session of the press is
            // handed over and the engine decides. A panic in this worker must not leave the row in flight for
            // good: it answers as an error.
            let session = stamped.0;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.restore_items_for(session, &[item.as_str()])
            }))
            .unwrap_or(Err(AccountCallError::Offline));
            #[cfg(test)]
            if let Some((_, ended)) = &hooks {
                let _ = ended.send(());
            }
            let _ = answers.send(Answer::Restored(item, stamped, result));
            engine.wake_host();
        });
        if started.is_err() {
            log("trash: could not start a thread for a restore");
            return false;
        }
        if let Some(listing) = self.listing.as_mut() {
            let row = listing.rows.remove(at);
            self.restoring.insert(id.to_string(), (pressed.0, row));
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
        let mut notice = self.notice.take();
        // Words for the account that pressed: said once its session is in memory (after an unlock the engine wakes
        // the window, which polls); never to another account, and dropped when another, or none, is here.
        if let Some(account) = self.not_done_for.take() {
            if Some(&account) == account_of(&self.engine).as_ref() {
                if self.engine.has_session() && notice.is_none() {
                    notice = Some(Notice::RestoreNotDone);
                } else {
                    self.not_done_for = Some(account);
                }
            }
        }
        let waiting_to_say = self.not_done_for.is_some() && self.engine.has_session();
        Pumped {
            changed: changed || notice.is_some(),
            busy: self.reading.is_some() || !self.restoring.is_empty() || waiting_to_say,
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
