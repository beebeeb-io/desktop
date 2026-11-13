//! Trash on the real engine: [`EngineTrashSource`], the `TrashSource` of the running app.
//!
//! What it answers, and from where:
//! - `items` and `totals`: the account's Trash from Beebeeb, every page, each name decrypted in the engine
//!   ([`Engine::trash_listing`]); where an item was, from this computer's local state
//!   (`drive_v2::model_shell::trash_entries`). Newest deleted first. The size is the files' sizes added up.
//! - `restore`: [`Engine::restore_items`], one item; its row leaves the list at once, and the list is read again
//!   once Beebeeb answered, so a refused restore brings the row back.
//! - Not bound: `emptying` (the engine has no empty-Trash call yet), so it is always `None`.
//!
//! Thread rule: nothing here waits for Beebeeb on the window's thread. A read and each restore run on a worker of
//! their own and send their answer down one channel; the window is woken ([`Engine::wake_host`]) and the answer is
//! taken in on the next call the view makes (the trait has no poll of its own). A read is not started while one is
//! under way, and an answer, or a failure, is not asked for again within [`FRESH_FOR`].
//!
//! Honest states: until the signed-in account's Trash has been read (signed out, still reading, a failed first
//! read) `wired()` is false, and the view draws the shell's "not built yet" frame: never an empty Trash it has not
//! read, never sample rows. Rows read for another account are never shown.
//!
//! Must not: log a name, an id, a path or a server's words.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::settings_api::AccountCallError;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::drive_v2::dto::TrashEntry;
use beebeeb_engine::drive_v2::model_shell::trash_entries;
use chrono::{DateTime, Local, TimeZone};

use super::log;
use crate::frames::shared::parts::table::{Extra, Kind, Row};
use crate::frames::trash::source::{Emptying, TrashSource, TrashTotals};

#[cfg(test)]
mod tests;

/// An answer (or a failure) this recent is not asked for again when the view draws.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// Trash on the real engine. See the module's doc.
pub struct EngineTrashSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    state: Mutex<Inner>,
    answers: mpsc::Sender<Answer>,
    inbox: Mutex<mpsc::Receiver<Answer>>,
}

#[derive(Default)]
struct Inner {
    /// The last listing read, and whose account it is.
    listing: Option<Listing>,
    /// When the last read was answered (a failure too).
    read_at: Option<Instant>,
    reading: bool,
    /// Items whose restore is on its way: a listing that lands meanwhile does not show them.
    restoring: HashSet<String>,
    /// Counts the restores Beebeeb answered: a read that began before one of them is stale when it lands.
    generation: u64,
}

struct Listing {
    whose: String,
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
    /// The account it was read for (`None`: it changed during the call), the generation the read began in, and
    /// the rows, or why not.
    Listed(Option<String>, u64, Result<Vec<Listed>, AccountCallError>),
    /// A restore of this item, and whether Beebeeb took it.
    Restored(String, bool),
}

/// Whose account a read is for (`settings::account_of`): in memory only, never waits.
fn account_of<C: CredentialStores>(engine: &Engine<C>) -> Option<String> {
    engine.account_email().or_else(|| engine.user_id())
}

impl<C: CredentialStores> EngineTrashSource<C> {
    /// Nothing is asked until the view first draws Trash.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            state: Mutex::new(Inner::default()),
            answers,
            inbox: Mutex::new(inbox),
        }
    }

    /// Take in what the workers sent, then start a read when the listing is missing, another account's, or old.
    fn sync(&self) -> std::sync::MutexGuard<'_, Inner> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        while let Ok(answer) = inbox.try_recv() {
            match answer {
                Answer::Listed(whose, generation, result) => {
                    state.reading = false;
                    state.read_at = Some(Instant::now());
                    if generation != state.generation {
                        // A restore was answered after this read began: what it says is older than Beebeeb.
                        state.read_at = None;
                        continue;
                    }
                    match (result, whose) {
                        (Ok(rows), Some(whose)) => state.listing = Some(Listing { whose, rows }),
                        (Ok(_), None) => state.listing = None,
                        (Err(kind), _) => {
                            if kind != AccountCallError::NotSignedIn {
                                log(format!("trash: the list did not load ({kind})"));
                            }
                            // A session that ended or a sign-out: nothing of the account stays on screen.
                            // Offline or a server error: the rows already read stay; none read: not wired.
                            if matches!(kind, AccountCallError::SessionEnded | AccountCallError::NotSignedIn) {
                                state.listing = None;
                            }
                        }
                    }
                }
                Answer::Restored(id, taken) => {
                    state.restoring.remove(&id);
                    state.generation += 1;
                    // Read again what Beebeeb has: a refused restore's row comes back.
                    state.read_at = None;
                    if !taken {
                        log("trash: an item was not restored");
                    }
                }
            }
        }
        drop(inbox);
        let whose = account_of(&self.engine);
        let stale = state.read_at.is_none_or(|at| at.elapsed() >= FRESH_FOR);
        let other = state.listing.as_ref().is_some_and(|l| Some(&l.whose) != whose.as_ref());
        if whose.is_some() && !state.reading && (stale || other) {
            self.read(&mut state);
        }
        state
    }

    /// Ask Beebeeb for the Trash on a worker.
    fn read(&self, state: &mut Inner) {
        let (engine, answers, generation) = (self.engine.clone(), self.answers.clone(), state.generation);
        let started = std::thread::Builder::new().name("trash-list".into()).spawn(move || {
            // A panic in this worker must not leave `reading` set for good: send an error.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let before = account_of(&engine);
                let listed = engine.trash_listing().map(|raw| {
                    let db = engine.state_db().ok();
                    rows_of(trash_entries(db.as_ref(), raw), Local::now())
                });
                let whose = before.filter(|whose| account_of(&engine).as_ref() == Some(whose));
                (whose, listed)
            }));
            let answer = match result {
                Ok((whose, listed)) => Answer::Listed(whose, generation, listed),
                Err(_) => Answer::Listed(None, generation, Err(AccountCallError::Offline)),
            };
            let _ = answers.send(answer);
            engine.wake_host();
        });
        state.reading = started.is_ok();
        if started.is_err() {
            log("trash: could not start a thread for the list");
            state.read_at = Some(Instant::now());
        }
    }

    /// The signed-in account's rows, without those whose restore is on its way.
    fn rows(&self, state: &Inner) -> Option<Vec<Row>> {
        let whose = account_of(&self.engine)?;
        let listing = state.listing.as_ref().filter(|l| l.whose == whose)?;
        Some(
            listing
                .rows
                .iter()
                .filter(|listed| !state.restoring.contains(&listed.row.id))
                .map(|listed| listed.row.clone())
                .collect(),
        )
    }
}

#[cfg(test)]
impl<C: CredentialStores> EngineTrashSource<C> {
    /// A read or a restore is on its way (the tests wait for it).
    fn busy(&self) -> bool {
        let state = self.sync();
        state.reading || !state.restoring.is_empty()
    }

    /// The last answer is old: the next call reads again (instead of waiting [`FRESH_FOR`]).
    fn expire(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).read_at = None;
    }
}

impl<C: CredentialStores> TrashSource for EngineTrashSource<C> {
    fn wired(&self) -> bool {
        let state = self.sync();
        self.rows(&state).is_some()
    }

    fn items(&self) -> Vec<Row> {
        let state = self.sync();
        self.rows(&state).unwrap_or_default()
    }

    fn totals(&self) -> TrashTotals {
        let state = self.sync();
        let Some(rows) = self.rows(&state) else {
            return TrashTotals {
                items: 0,
                size: String::new(),
            };
        };
        let bytes = state
            .listing
            .as_ref()
            .map(|l| {
                l.rows
                    .iter()
                    .filter(|listed| !state.restoring.contains(&listed.row.id))
                    .map(|listed| listed.bytes)
                    .sum::<i64>()
            })
            .unwrap_or(0);
        TrashTotals {
            items: rows.len(),
            size: crate::source::format_size(bytes),
        }
    }

    /// Not bound: the engine has no empty-Trash call yet.
    fn emptying(&self) -> Option<Emptying> {
        None
    }

    /// Sent once, on a worker; the row leaves the list at once. `false`: no such row (or no worker could start).
    fn restore(&mut self, id: &str) -> bool {
        let mut state = self.sync();
        let known = self
            .rows(&state)
            .is_some_and(|rows| rows.iter().any(|row| row.id == id));
        if !known {
            return false;
        }
        let (engine, answers, item) = (self.engine.clone(), self.answers.clone(), id.to_string());
        let started = std::thread::Builder::new().name("trash-restore".into()).spawn(move || {
            let taken = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.restore_items(&[item.as_str()]).is_ok()
            }))
            .unwrap_or(false);
            let _ = answers.send(Answer::Restored(item, taken));
            engine.wake_host();
        });
        if started.is_err() {
            log("trash: could not start a thread for a restore");
            return false;
        }
        state.restoring.insert(id.to_string());
        true
    }
}

/// The engine's Trash entries as the view's rows, newest deleted first, with the deleted time each was sorted by.
pub(crate) fn rows_of(entries: Vec<TrashEntry>, now: DateTime<Local>) -> Vec<Listed> {
    let mut rows: Vec<Listed> = entries
        .into_iter()
        .map(|entry| {
            let at = seconds_of(&entry.trashed_at);
            let item = entry.item;
            let kind = kind_of(item.is_folder, item.mime.as_deref());
            let bytes = if item.is_folder {
                0
            } else {
                item.size.unwrap_or(0).max(0)
            };
            let size = if item.is_folder {
                String::new()
            } else {
                crate::source::format_size(bytes)
            };
            let from = entry
                .original_path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(beebeeb_engine::drive_v2::model::ROOT_NAME)
                .to_string();
            let row = Row {
                id: item.id,
                name: item.name,
                kind,
                date: at.map(|at| when(at, now)).unwrap_or_default(),
                size,
                extra: Extra::Words(format!("From {from}")),
                pc: None,
            };
            Listed {
                row,
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
