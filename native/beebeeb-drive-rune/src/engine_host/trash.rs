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
//! - the sidebar's count: the server's own number ([`Engine::trash_count_for`], `count.rs`), read apart from the list
//!   (counting by listing would decrypt the whole Trash for a badge); none until Beebeeb has said it.
//! - `restore`: [`Engine::restore_items_for`], one item, for the session of the press.
//! - `purge` (Empty Trash, Delete permanently): [`Engine::purge_trash_for`] after the account PASSWORD is proven for
//!   each item (`purge.rs`).
//! - Details and Preview (`actions_available`: while a list of the session in memory is here): Details is drawn from
//!   the listing's own row (`frames::item`), with nothing read from the local state. Preview is read from Beebeeb
//!   only, in memory (`engine_host::item_preview`: the reader never asks for this PC's copy or the local state's
//!   facts), with no file written to disk and no editor.
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
//! row. A press whose restore was not sent because the session moved on (a lock, a sign-out) is told to the account
//! that pressed, once its Trash is listed again under a session of its own AND is on screen
//! ([`TrashSource::on_screen`]), naming the item as that list shows it then: only the item's id is held across the
//! lock or the sign-out, never its name. Kept while no account is here; never told to another account, and dropped
//! the moment another account is here (nothing of an earlier account is kept through another's session); dropped
//! unsaid when the item is no longer in that account's Trash.
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
use crate::frames::trash::source::{Emptying, Loaded, Notice, Pumped, PurgeEnd, TrashSource, TrashTotals};

mod count;
mod purge;
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
    /// Restores that were not sent because their session moved on, owed to the account that pressed: told by the
    /// first poll that finds that account's Trash listed again and on screen (a lock or a sign-out may be in between);
    /// kept while no account is here, dropped when another account is. Outlives `forget`, and holds no name of an
    /// item (ids only).
    not_done: Option<NotDone>,
    /// Whether the Trash section is on screen (the window's poll says so; `opened` too).
    shown: bool,
    /// Tests only: the next restore waits on the first, on its worker, before it calls the engine, and says on the
    /// second that the engine's call ended and its answer is in the inbox.
    #[cfg(test)]
    next_restore: Option<(mpsc::Receiver<()>, mpsc::Sender<()>)>,
    /// Something changed outside an answer (a restore started, a forget): the next pump says so.
    dirty: bool,
    /// Changes with every change to the rows held (`TrashSource::rev`).
    rev: u64,
    /// The delete for good on its way, and how the last one ended (told once, by the next pump).
    purging: Option<purge::Purging>,
    purge_end: Option<PurgeEnd>,
    /// The sidebar's count (`count.rs`).
    counted: count::CountState,
}

struct Listing {
    session: SessionGeneration,
    /// Newest deleted first.
    rows: Vec<Listed>,
}

/// Restores a lock or a sign-out cut before they were sent: the account that pressed, and the items by id. Never a
/// name: each is looked up in that account's own list when the words are said.
struct NotDone {
    account: String,
    ids: Vec<String>,
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
    /// A delete for good got this far (the number gone), under the session and account of its press.
    PurgeProgress(Stamp, usize),
    /// A delete for good ended, under the session and account of its press.
    Purged(Stamp, beebeeb_engine::app::trash_purge_api::Purged),
    /// A delete of many for good (one proof, one call) ended, under the session and account of its press.
    BulkPurged(Stamp, beebeeb_engine::app::trash_bulk_api::BulkPurged),
    /// The count read with this number, the session and account it was asked under, and the server's number or why not.
    Counted(u64, Stamp, Result<u64, AccountCallError>),
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
            not_done: None,
            shown: false,
            #[cfg(test)]
            next_restore: None,
            dirty: false,
            rev: 0,
            purging: None,
            purge_end: None,
            counted: count::CountState::default(),
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
        self.rev += 1;
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
                // The count moved (or may have): asked again, under whoever is here now.
                self.count_moved();
                // The row in flight is this press's only when it left the list under the session of this press: an
                // answer of an earlier session never takes out a row a later press put there.
                let row = match self.restoring.get(&id) {
                    Some((session, _)) if *session == pressed.0 => self.restoring.remove(&id).map(|(_, row)| row),
                    _ => None,
                };
                // Drawn only for the session AND the account of the press. The session's turn only moves forward,
                // so a stamp that is current now was current for the whole call.
                if !(self.engine.has_session() && pressed == stamp(&self.engine)) {
                    self.answered_late(id, pressed, result);
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
            Answer::PurgeProgress(pressed, done) => self.purge_progress(pressed, done),
            Answer::Purged(pressed, result) => {
                self.purge_ended(pressed, result);
                self.count_moved();
            }
            Answer::BulkPurged(pressed, result) => {
                self.bulk_purge_ended(pressed, result);
                self.count_moved();
            }
            Answer::Counted(number, asked, result) => self.take_count(number, asked, result),
        }
    }
}

impl<C: CredentialStores> EngineTrashSource<C> {
    /// A restore of `id` answered under another session or account than its press: no row is touched and nothing of
    /// it is drawn. The account that pressed is told when the restore was certainly not made (by `pump`, which says
    /// it to that account only); when it may have been made, the list is read again.
    fn answered_late(&mut self, id: String, pressed: Stamp, result: Result<(), AccountCallError>) {
        if let Err(kind) = &result {
            log(format!("trash: an item was not restored ({kind})"));
        }
        match result {
            // Still in Trash for certain: the engine sent nothing (the session of the press is gone: a lock, a
            // sign-out, another sign-in).
            Err(AccountCallError::NotSignedIn) => self.owe_not_done(pressed.1, id),
            // Sent under the session of the press: taken, refused (a gateway's error may hide a restore that was
            // made), or never answered. A list read since, or being read now, may be out of date: the next opening
            // reads again, and a list on screen or on its way is read again (whoever is here is asked for their
            // own list, under their own session).
            _ => {
                self.read_at = None;
                if self.current().is_some() || self.reading.is_some() {
                    self.read();
                }
            }
        }
    }

    /// The restore of `id`, pressed under `account`, was not sent: owed to that account, with any other it is owed
    /// already. Another account's owed words go (they are never said to this one). An unnamed session (no email, no
    /// user id in memory) cannot be told.
    fn owe_not_done(&mut self, account: Option<String>, id: String) {
        let Some(account) = account else {
            return;
        };
        if let Some(owed) = self.not_done.as_mut().filter(|owed| owed.account == account) {
            if !owed.ids.contains(&id) {
                owed.ids.push(id);
            }
            return;
        }
        self.not_done = Some(NotDone { account, ids: vec![id] });
    }

    /// The words owed to an account whose restore a lock or a sign-out cut (`not_done`), put in `notice` when this
    /// poll may say them: that account is here, its Trash is listed under a session of its own and is on screen, and
    /// no other notice takes this poll. Kept while no account is here (signed out) and while its own Trash is not
    /// listed or not shown; dropped when another account is here, and dropped unsaid when none of its items is in
    /// its Trash any more. `true`: they would be said now but another notice took this poll (poll again soon).
    fn say_not_done(&mut self, notice: &mut Option<Notice>) -> bool {
        let Some(owed) = self.not_done.take() else {
            return false;
        };
        match account_of(&self.engine) {
            // Another account is here: never said to it, and nothing of the earlier account is kept through its
            // session.
            Some(here) if here != owed.account => false,
            Some(_) if self.shown && self.current().is_some() => {
                if notice.is_some() {
                    self.not_done = Some(owed);
                    return true;
                }
                let names = self.names_in_trash(&owed.ids);
                if !names.is_empty() {
                    *notice = Some(Notice::RestoreNotDone(names));
                }
                false
            }
            // Its own account, before its Trash is listed again or while Trash is not on screen; or no account at
            // all (signed out, or the session not read back yet): kept.
            _ => {
                self.not_done = Some(owed);
                false
            }
        }
    }

    /// The names, as this session's Trash shows them, of those of `ids` still in it: in the list, or taken out of
    /// it by a restore of this session that is still on its way. Never logged.
    fn names_in_trash(&self, ids: &[String]) -> Vec<String> {
        let now = self.engine.session_turn();
        let in_list = self.current().into_iter().flat_map(|listing| listing.rows.iter());
        let in_flight = self
            .restoring
            .values()
            .filter(|(session, _)| *session == now)
            .map(|(_, listed)| listed);
        in_list
            .chain(in_flight)
            .filter(|listed| ids.contains(&listed.row.id))
            .map(|listed| listed.row.name.clone())
            .collect()
    }
}

#[cfg(test)]
impl<C: CredentialStores> EngineTrashSource<C> {
    /// A read or a restore is on its way (the tests wait for it).
    fn busy(&self) -> bool {
        self.reading.is_some() || !self.restoring.is_empty() || self.counted.busy()
    }

    /// The last answer is old: the next opening reads again (instead of waiting [`FRESH_FOR`]).
    fn expire(&mut self) {
        self.read_at = None;
    }
}

impl<C: CredentialStores> TrashSource for EngineTrashSource<C> {
    fn rev(&self) -> Option<u64> {
        self.current().map(|_| self.rev)
    }

    fn retry(&mut self) {
        self.read_at = None;
        self.opened();
    }

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

    /// While Empty Trash is on its way: how many are gone for good, of how many.
    fn emptying(&self) -> Option<Emptying> {
        self.emptying_now()
    }

    /// Sent once, on a worker; the row leaves the list at once and comes back only if Beebeeb refuses. `false`: no
    /// such row in the current session's list (or no worker could start).
    fn restore(&mut self, id: &str) -> bool {
        // The session and account of the press, read BEFORE the list the item is chosen from is looked at: the
        // item is sent for this session or not at all.
        let pressed = stamp(&self.engine);
        let Some(at) = self
            .current()
            .filter(|listing| listing.session == pressed.0)
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
            let _ = answers.send(Answer::Restored(item, stamped, result));
            #[cfg(test)]
            if let Some((_, ended)) = &hooks {
                let _ = ended.send(());
            }
            engine.wake_host();
        });
        if started.is_err() {
            log("trash: could not start a thread for a restore");
            return false;
        }
        if let Some(listing) = self.listing.as_mut() {
            let row = listing.rows.remove(at);
            self.rev += 1;
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
        self.shown = true;
        if !self.engine.has_session() {
            return;
        }
        // The badge is read again too: the section is where it is looked at hardest.
        self.count_moved();
        let fresh = self.current().is_some() && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.read();
        }
    }

    /// A finished check: a list that was read in this session is read again; otherwise the next opening reads.
    fn refresh(&mut self) {
        if !self.engine.has_session() {
            return;
        }
        if self.listing.is_some() {
            self.read();
        } else {
            self.read_at = None;
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
        // The badge is read from every section: a session without one of its own asks for it.
        self.count_if_missing();
        let mut notice = self.notice.take();
        let purge = self.purge_end.take();
        // Words for the account that pressed: said once its Trash is listed again under a session of its own and is
        // on screen (a notice handed to a window showing something else would be wiped unseen when Trash opens);
        // never to another account (`say_not_done`).
        let waiting_to_say = self.say_not_done(&mut notice);
        Pumped {
            changed: changed || notice.is_some() || purge.is_some(),
            busy: self.reading.is_some()
                || !self.restoring.is_empty()
                || waiting_to_say
                || self.purging.is_some()
                || self.counted.busy(),
            notice,
            purge,
            wake_in: self.count_due_in(),
        }
    }

    fn on_screen(&mut self, shown: bool) {
        self.shown = shown;
    }

    fn forget(&mut self) {
        self.rev += 1;
        self.listing = None;
        self.restoring.clear();
        self.restored.clear();
        self.again = false;
        self.read_at = None;
        self.notice = None;
        self.purging = None;
        self.purge_end = None;
        self.forget_count();
        self.loaded = Loaded::Loading;
        self.dirty = true;
    }

    /// The server's own count (`count.rs`): none until it has said one for this session and account.
    fn count(&self) -> Option<usize> {
        self.badge()
    }

    fn server_count(&self) -> Option<usize> {
        self.badge()
    }

    /// Details and Preview: while a list of the session in memory is here (an item is looked up in it).
    fn actions_available(&self) -> bool {
        self.current().is_some()
    }

    /// Empty Trash and Delete permanently: the engine's permanent delete, after the account password is proven.
    fn purge_available(&self) -> bool {
        true
    }

    fn purge_batch(&self) -> (TrashTotals, usize) {
        self.first_batch()
    }

    fn purge(&mut self, id: Option<&str>, password: &str) -> bool {
        self.start_purge(id, password)
    }

    fn purging(&self) -> bool {
        self.purging.is_some()
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
                    bytes: (!item.is_folder).then_some(bytes as u64),
                    at,
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
