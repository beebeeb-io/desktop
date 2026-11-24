//! The devices' backup line (DW-0601), and Photo sources' folders
//! from Files (DW-0604). Part of [`EnginePhotos`] (`photos.rs`), which hands `folders`, a folder's `set_shown`,
//! `add_folder`, `remove_folder` and `backup_status` here.
//!
//! The backup line: `last` is the newest pass time any other device's manifest names
//! ([`Engine::backup_devices_for`], asked for the session and account of the look and read on a worker of its own).
//! The answer is held here; the accessor only reads what is held. `now` stays empty: this PC does not know what
//! another device is doing now (the live "is backing up 12 photos" needs a server field), and nothing is invented.
//! With no pass time named by any device there is no line at all. A late answer (another session or account than
//! the one it was asked under) is dropped. Must not: log a name, an id or a path.
//!
//! The folders: the list is the engine's answer ([`Engine::photo_folders_for`], for the session and account of the
//! look, read on a worker of its own) and is held here; the accessor only reads what is held. It starts a read when
//! nothing is held for this session and account or what is held is older than [`FRESH_FOR`]; the timeline's backup
//! line starts the first one, so the Photo sources dialog opens on the engine's list. A switch, an add or a removal is
//! asked of the engine on a worker, for the session and account of the press, and nothing is drawn as changed before
//! the engine answered: the held list changes only when the engine's list is read again after the change (also after
//! a refusal, so the window shows what is true). The same worker then reads the library again and hands it to `pump`
//! as an ordinary read (`Answer::Read`, numbered at the press, every earlier read marked stale), so a folder switched
//! off takes its photos out of the timeline at that answer. A late answer (another session or account than the one it
//! was asked under) is dropped, and so is a list older than the one held. The frame's `Folder` carries no id: the ids
//! stay here, by the place in the list handed out, for that list only.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use beebeeb_engine::app::backup_devices_api::DeviceRead;
use beebeeb_engine::app::bindings::photos_folders::{PhotoFolder, PhotoFoldersRefused};

use super::*;

/// What slices B1 and K4 keep between a press and its answer: the held backup line, the held folders, and the reads
/// on their way. The adapter holds one (`EnginePhotos::photos_folders`).
#[derive(Default)]
pub(super) struct PhotoFolderCalls {
    held: Arc<Mutex<Held>>,
    /// Photo sources' folders.
    folders: Arc<Mutex<HeldFolders>>,
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

/// Photo sources' folders for one session and account, as the engine last answered.
#[derive(Default)]
struct HeldFolders {
    /// The session and account everything below was read for.
    stamp: Option<Stamp>,
    /// The engine's list for `stamp`, in its order. `None`: not read yet.
    list: Option<Vec<PhotoFolder>>,
    /// A plain read for `stamp` is on its way (a change reads the list again on its own worker).
    reading: bool,
    /// When the last plain read for `stamp` landed (a failed one counts: it is asked again after `FRESH_FOR`).
    at: Option<Instant>,
    /// The number the last read got. A read gets the next one when it starts, and a change's read starts only after
    /// the change was written.
    started: u64,
    /// The number of the list held: an older list never replaces a newer one.
    taken: u64,
    /// A change of this stamp was refused by the engine and the window has not been told yet.
    refused: bool,
}

impl HeldFolders {
    /// The held state is for `now`: another session or account empties it. The numbers keep growing.
    fn follow(&mut self, now: &Stamp) {
        if self.stamp.as_ref() != Some(now) {
            *self = HeldFolders {
                stamp: Some(now.clone()),
                started: self.started,
                taken: self.started,
                ..HeldFolders::default()
            };
        }
    }

    /// The window asks whether a change was refused since it last asked: said once.
    fn take_refusal(&mut self) -> bool {
        std::mem::take(&mut self.refused)
    }

    /// A read starts under `asked`: its number. `None`: the held state is for another stamp, and nothing starts.
    fn start(&mut self, asked: &Stamp) -> Option<u64> {
        if self.stamp.as_ref() != Some(asked) {
            return None;
        }
        self.started += 1;
        Some(self.started)
    }

    /// A list read under `asked` as number `number`: kept only while the held state is still for `asked` and no newer
    /// list is held. `false`: dropped.
    fn take(&mut self, asked: &Stamp, number: u64, list: Vec<PhotoFolder>) -> bool {
        if self.stamp.as_ref() != Some(asked) || number <= self.taken {
            return false;
        }
        self.taken = number;
        self.list = Some(list);
        true
    }
}

/// A change of Photo sources' folders, asked of the engine: the folder's file id.
enum FolderChange {
    Shown(String, bool),
    Add(String),
    Remove(String),
}

/// 12430 as "12,430".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A folder as Photo sources draws it: "Lisbon trip", "212 items, with its subfolders" (DW-0604).
fn folder_of(folder: &PhotoFolder) -> Folder {
    let count = match folder.items {
        1 => "1 item".to_string(),
        n => format!("{} items", thousands(n)),
    };
    Folder {
        name: folder.name.clone(),
        detail: format!("{count}, with its subfolders"),
        shown: folder.shown,
    }
}

/// Starts a read of the folders for `asked` on a worker, unless one is on its way or the held state is for another
/// stamp. Never waits.
fn start_folder_read<C: CredentialStores>(
    engine: &Arc<Engine<C>>,
    shared: &Arc<Mutex<HeldFolders>>,
    held: &mut HeldFolders,
    asked: &Stamp,
) {
    if held.reading {
        return;
    }
    let Some(number) = held.start(asked) else {
        return;
    };
    held.reading = true;
    let (engine, shared, asked) = (engine.clone(), shared.clone(), asked.clone());
    let started = std::thread::Builder::new()
        .name("photos-folders".into())
        .spawn(move || {
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.photo_folders_for(asked.0)))
                .unwrap_or(Err(PhotoFoldersRefused::NotSaved));
            // Another session or account since the ask: the answer is not kept.
            let still = stamp(&engine) == asked;
            let kept = shared
                .lock()
                .map(|mut held| {
                    // Only the read of this stamp is ended; a newer stamp's read is its own.
                    if held.stamp.as_ref() == Some(&asked) {
                        held.reading = false;
                        held.at = Some(Instant::now());
                    }
                    match read {
                        Ok(list) if still => held.take(&asked, number, list),
                        _ => false,
                    }
                })
                .unwrap_or(false);
            if kept {
                engine.wake_host();
            }
        });
    if started.is_err() {
        held.reading = false;
        log("photos: could not start the read");
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
    /// Photo sources' folders from what is held for this session and account, in the engine's order. Asks the engine
    /// on a worker when nothing is held or what is held is older than [`FRESH_FOR`]; never waits.
    pub(super) fn photos_folders_folders(&self) -> Vec<Folder> {
        if !self.engine.has_session() {
            self.forget_folders();
            return Vec::new();
        }
        let now = stamp(&self.engine);
        let Ok(mut held) = self.photos_folders.folders.lock() else {
            return Vec::new();
        };
        held.follow(&now);
        // Never read for this stamp (`at` is empty), or the last read, answered or refused, is older than `FRESH_FOR`.
        if held.at.is_none_or(|at| at.elapsed() > FRESH_FOR) {
            start_folder_read(&self.engine, &self.photos_folders.folders, &mut held, &now);
        }
        held.list.as_deref().unwrap_or_default().iter().map(folder_of).collect()
    }

    /// A folder change (add, remove, switch) was refused by the engine since the last call: `true` once.
    pub(super) fn photos_folders_refused(&mut self) -> bool {
        self.photos_folders
            .folders
            .lock()
            .map(|mut held| held.take_refusal())
            .unwrap_or(false)
    }

    /// No session: no folder name of the earlier account stays in memory. The numbers keep growing, so a read on its
    /// way is not taken later. (`EnginePhotos::forget` does not reach this state; the accessors empty it instead.)
    fn forget_folders(&self) {
        if let Ok(mut held) = self.photos_folders.folders.lock() {
            let started = held.started;
            *held = HeldFolders {
                started,
                taken: started,
                ..HeldFolders::default()
            };
        }
    }

    /// The Folder arm of `set_shown` (Photo sources, Done). Done hands every folder over, changed or not: only a
    /// switch that differs from the engine's last answer is sent.
    pub(super) fn photos_folders_set_shown(&mut self, index: usize, shown: bool) {
        if let Some(id) = self.folder_id_at(index, Some(shown)) {
            self.photos_folders_change(FolderChange::Shown(id, shown));
        }
    }

    /// Add a folder…, once the folder picker chose one. `false`: nothing sent.
    pub(super) fn photos_folders_add(&mut self, folder_id: &str) -> bool {
        if folder_id.is_empty() {
            return false;
        }
        self.photos_folders_change(FolderChange::Add(folder_id.to_string()))
    }

    /// Remove, on a folder of the list handed out. A device: nothing.
    pub(super) fn photos_folders_remove(&mut self, source: SourceRef) {
        let SourceRef::Folder(index) = source else {
            return;
        };
        if let Some(id) = self.folder_id_at(index, None) {
            self.photos_folders_change(FolderChange::Remove(id));
        }
    }

    /// The id of the folder at `index` in the list held for the session and account here, when `shown` is `None` or
    /// differs from its switch.
    fn folder_id_at(&self, index: usize, shown: Option<bool>) -> Option<String> {
        if !self.engine.has_session() {
            return None;
        }
        let now = stamp(&self.engine);
        let held = self.photos_folders.folders.lock().ok()?;
        if held.stamp.as_ref() != Some(&now) {
            return None;
        }
        let folder = held.list.as_ref()?.get(index)?;
        (shown != Some(folder.shown)).then(|| folder.id.clone())
    }

    /// Asks the engine for `change` on a worker, for the session and account of the press; the same worker then reads
    /// the folders and the library again: what shows changes at those answers, never before. `false`: nothing sent.
    fn photos_folders_change(&mut self, change: FolderChange) -> bool {
        if !self.engine.has_session() {
            return false;
        }
        let pressed = stamp(&self.engine);
        self.look(&pressed);
        // The library is read again after the change: a read that began before it is not taken (`pump` reads again
        // once it lands), and no other read starts before this one landed.
        self.stale_up_to = self.reads;
        self.reads += 1;
        let number = self.reads;
        let before = self.reading.replace(number);
        let (engine, answers, shared) = (
            self.engine.clone(),
            self.answers.clone(),
            self.photos_folders.folders.clone(),
        );
        let asked = pressed;
        let started = std::thread::Builder::new()
            .name("photos-folder-change".into())
            .spawn(move || {
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &change {
                    FolderChange::Shown(id, shown) => engine.set_photo_folder_shown_for(asked.0, id, *shown),
                    FolderChange::Add(id) => engine.add_photo_folder_for(asked.0, id),
                    FolderChange::Remove(id) => engine.remove_photo_folder_for(asked.0, id),
                }))
                .unwrap_or(Err(PhotoFoldersRefused::NotSaved));
                if stamp(&engine) != asked {
                    log("photos: the answer of an earlier session was dropped");
                } else if done.is_err() {
                    log("photos: a folder change was refused");
                    // The window says so (`PhotosSource::folder_refused`), not only the log.
                    if let Ok(mut held) = shared.lock()
                        && held.stamp.as_ref() == Some(&asked)
                    {
                        held.refused = true;
                    }
                }
                // The folders as the engine has them now, also after a refusal: the window shows what is true. The
                // number is taken after the write, so a read that began before it never replaces this list.
                let number_now = shared.lock().ok().and_then(|mut held| held.start(&asked));
                if let Some(list_number) = number_now
                    && let Ok(list) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.photo_folders_for(asked.0)))
                            .unwrap_or(Err(PhotoFoldersRefused::NotSaved))
                    && stamp(&engine) == asked
                    && let Ok(mut held) = shared.lock()
                {
                    held.take(&asked, list_number, list);
                }
                // The library again, after the change: `pump` takes it as the read numbered at the press.
                let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    engine.photos_library().map(|library| (library.session, library.photos))
                }))
                .unwrap_or(Err(LocalStateRefused::Unreadable));
                let _ = answers.send(Answer::Read(number, asked, read));
                engine.wake_host();
            });
        if started.is_err() {
            self.reading = before;
            log("photos: could not start the read");
            return false;
        }
        true
    }

    /// The backup line from what is held. Asks the engine on a worker when nothing is held for this session and
    /// account, or what is held is older than [`FRESH_FOR`]; never waits. It also starts the first read of Photo
    /// sources' folders for this session and account, so the dialog opens on the engine's list.
    pub(super) fn photos_folders_backup_status(&self) -> Option<BackupStatus> {
        if !self.engine.has_session() {
            self.forget_folders();
            return None;
        }
        let now = stamp(&self.engine);
        if let Ok(mut folders) = self.photos_folders.folders.lock() {
            folders.follow(&now);
            if folders.list.is_none() && folders.at.is_none() {
                start_folder_read(&self.engine, &self.photos_folders.folders, &mut folders, &now);
            }
        }
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
