//! Slice B1 (plan40 batch 1): the devices' backup line (DW-0601) and, not bound yet, Photo sources' folders
//! (DW-0604). Part of [`EnginePhotos`] (`photos.rs`), which hands `folders` and `backup_status` here.
//!
//! The backup line: `last` is the newest pass time any other device's manifest names
//! ([`Engine::backup_devices_for`], asked for the session and account of the look and read on a worker of its own).
//! The answer is held here; the accessor only reads what is held. `now` stays empty: this PC does not know what
//! another device is doing now (the live "is backing up 12 photos" needs a server field), and nothing is invented.
//! With no pass time named by any device there is no line at all. A late answer (another session or account than
//! the one it was asked under) is dropped. Must not: log a name, an id or a path.
//!
//! The folders: not bound. The engine keeps no per-account "folders from Files that Photos shows" and the state
//! database has no accessor for one outside its own module; see the branch notes.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use beebeeb_engine::app::backup_devices_api::DeviceRead;

use super::*;

/// What slice B1 keeps between a press and its answer: the held backup line and the read on its way. The adapter
/// holds one (`EnginePhotos::photos_folders`).
#[derive(Default)]
pub(super) struct PhotoFolderCalls {
    held: Arc<Mutex<Held>>,
}

/// The backup line for one session and account.
#[derive(Default)]
struct Held {
    /// The session and account everything below was read for.
    stamp: Option<Stamp>,
    /// A read for this stamp is on its way.
    reading: bool,
    /// When the last answer for `stamp` landed (an answer that failed counts: it is asked again after `FRESH_FOR`).
    at: Option<Instant>,
    /// "last backup 09:12", when a device named a pass time.
    last: Option<String>,
}

impl Held {
    /// The held state is for `now`: another session or account empties it.
    fn follow(&mut self, now: &Stamp) {
        if self.stamp.as_ref() != Some(now) {
            *self = Held {
                stamp: Some(now.clone()),
                ..Held::default()
            };
        }
    }

    /// An answer asked under `asked`: kept only while the held state is still for it. `false`: dropped.
    fn take(&mut self, asked: &Stamp, last: Option<String>) -> bool {
        if self.stamp.as_ref() != Some(asked) {
            return false;
        }
        self.reading = false;
        self.at = Some(Instant::now());
        self.last = last;
        true
    }
}

/// "last backup 09:12" for a pass time in Unix milliseconds: the time alone today, else the day first ("last backup
/// Yesterday 09:12"). `None`: no device named a time.
fn last_backup_words(newest_ms: Option<i64>, today: NaiveDate) -> Option<String> {
    let taken = Local.timestamp_millis_opt(newest_ms?).single()?;
    let at = taken.format("%H:%M");
    Some(if taken.date_naive() == today {
        format!("last backup {at}")
    } else {
        format!("last backup {} {at}", day_title(taken.date_naive(), today))
    })
}

impl<C: CredentialStores> EnginePhotos<C> {
    /// Nothing bound for folders: the list is empty (see the module's doc).
    pub(super) fn photos_folders_folders(&self) -> Vec<Folder> {
        Vec::new()
    }

    /// The backup line from what is held. Asks the engine on a worker when nothing is held for this session and
    /// account, or what is held is older than [`FRESH_FOR`]; never waits.
    pub(super) fn photos_folders_backup_status(&self) -> Option<BackupStatus> {
        if !self.engine.has_session() {
            return None;
        }
        let now = stamp(&self.engine);
        let mut held = self.photos_folders.held.lock().ok()?;
        held.follow(&now);
        let stale = held.at.is_none_or(|at| at.elapsed() > FRESH_FOR);
        if stale && !held.reading {
            held.reading = true;
            let engine = self.engine.clone();
            let shared = self.photos_folders.held.clone();
            let asked = now;
            std::thread::spawn(move || {
                let last = engine.backup_devices_for(asked.0).ok().and_then(|read| {
                    let newest = read
                        .devices
                        .iter()
                        .filter_map(|device| match &device.read {
                            DeviceRead::Read(summary) => summary.last_backup_ms,
                            _ => None,
                        })
                        .max();
                    last_backup_words(newest, Local::now().date_naive())
                });
                // Another session or account since the ask: the answer is not kept.
                let still = stamp(&engine) == asked;
                let kept = still && shared.lock().map(|mut held| held.take(&asked, last)).unwrap_or(false);
                if kept {
                    engine.wake_host();
                } else if let Ok(mut held) = shared.lock() {
                    // Only the read of this stamp is ended; a newer stamp's read is its own.
                    if held.stamp.as_ref() == Some(&asked) {
                        held.reading = false;
                    }
                }
            });
        }
        held.last.clone().map(|last| BackupStatus {
            now: String::new(),
            last,
        })
    }
}

#[cfg(test)]
#[path = "photos_folders_tests.rs"]
mod tests;
