//! Backups' "Other devices" and "Delete this backup" on the real engine: [`DeviceCalls`], the part of
//! `EngineBackupsSource` behind `devices`, `devices_note`, `deleting` and `delete_device_backup` (DW-0737, DW-0739,
//! DW-0742).
//!
//! - The list: [`Engine::backup_devices_for`] (the device folders under the Backups root, each manifest read and
//!   decrypted by the engine and core; this PC left out). Read when the section opens (again only when what is held
//!   is older than [`FRESH_FOR`]), and after every delete that was sent. A device whose manifest this PC cannot read
//!   is a row that says so, never hidden. Whether another device is signed in, and what its backup does now, is not
//!   known on this PC: never drawn.
//! - Delete this backup: [`Engine::delete_device_backup_for`], the account PASSWORD proven first (owner ruling Q4),
//!   then one move to Trash. Only an id the list holds for the session that is here is sent; one delete at a time.
//!   Nothing is removed from the list before the answer; after it, the list is read again and shows what Beebeeb has.
//! - Each read and each delete is stamped with the session AND the account of its press (`stamp::stamp`, on the
//!   window's thread, before the worker starts) and handed to the engine with that session, which sends nothing
//!   unless it is still the session in memory. An answer that lands under another session or account is dropped, and
//!   what the window held of the earlier one goes the moment the stamp moves (in `look`, on the window's side).
//!
//! Thread rule: nothing here waits on the window's thread. Each read and each delete has a worker of its own; it sends
//! its answer down one channel and wakes the window.
//!
//! Must not: log a name, an id, the password or a manifest; keep the password beyond the delete's worker (it is a
//! `Zeroizing` copy, wiped when the worker ends, a panic included); draw a device it did not read as read.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::backup_devices_api::{BackupDevice, BackupDevices, DeviceClass, DeviceRead, DevicesRefused};
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};
use beebeeb_engine::backup_delete::DeleteBackupError;
use zeroize::Zeroizing;

use super::backups::last_backup_words;
use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::backups::source::{Backed, Device, DeviceKind};

#[cfg(test)]
#[path = "backups_devices_tests.rs"]
mod tests;

/// A list read this recently is not read again when the section opens.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// What "Other devices" says while its first read is out.
pub const READING: &str = "Reading your other devices\u{2026}";
/// What it says when the list could not be read (an older list, if any, stays).
pub const NOT_READ: &str = crate::frames::backups::source::DEVICES_NOT_READ;
/// What it says when the Backups listing was cut short.
pub const NOT_ALL: &str = "Not every device could be listed.";
/// What it says when nothing has synced on this PC yet (the keys of the other devices' files are not here).
pub const NOT_SYNCED: &str = concat!("Your other devices show here once ", beebeeb_engine::device_word!(), " has synced.");
/// A device folder whose name does not open on this PC.
pub const UNNAMED: &str = "Unnamed device";

/// Why a delete was not done, in the words the dialog shows. Each says whether anything was deleted.
pub const WRONG_PASSWORD: &str = "That password isn't right. Nothing was deleted.";
pub const TOO_MANY: &str = "Too many tries. Wait a little, then try again. Nothing was deleted.";
pub const SIGNED_OUT: &str = "You're no longer signed in. Nothing was deleted.";
pub const NOT_FOUND: &str = "This backup could not be found. Nothing was deleted.";
pub const NOT_ALLOWED: &str = "This backup can't be deleted from here. Nothing was deleted.";
pub const UNCONFIRMED: &str = "Beebeeb didn't confirm the delete. The list is read again to show what was done.";
pub const NOT_CHECKED: &str = concat!("Couldn't check this backup on ", beebeeb_engine::device_word!(), ". Nothing was deleted.");

/// The engine handle as the device calls use it. The real one is [`Engine`]; tests pass a fake.
pub trait DevicesEngine: Send + Sync + 'static {
    /// The session and the account a press is made under. Never waits.
    fn stamp(&self) -> Stamp;
    /// A session is in memory (signed in, vault open). Never waits.
    fn has_session(&self) -> bool;
    /// The other devices, for `session` only (`Engine::backup_devices_for`). BLOCKS: a worker's.
    fn devices_for(&self, session: SessionGeneration) -> Result<BackupDevices, DevicesRefused>;
    /// Delete one, for `session` only (`Engine::delete_device_backup_for`). BLOCKS: a worker's.
    fn delete_for(&self, session: SessionGeneration, folder_id: &str, password: &[u8])
    -> Result<(), DeleteBackupError>;
    /// Wake the window's loop (an answer is in). Never waits.
    fn wake(&self);
}

impl<C: CredentialStores> DevicesEngine for Engine<C> {
    fn stamp(&self) -> Stamp {
        stamp(self)
    }
    fn has_session(&self) -> bool {
        Engine::has_session(self)
    }
    fn devices_for(&self, session: SessionGeneration) -> Result<BackupDevices, DevicesRefused> {
        self.backup_devices_for(session)
    }
    fn delete_for(
        &self,
        session: SessionGeneration,
        folder_id: &str,
        password: &[u8],
    ) -> Result<(), DeleteBackupError> {
        self.delete_device_backup_for(session, folder_id, password)
    }
    fn wake(&self) {
        self.wake_host();
    }
}

enum Answer {
    Read {
        pressed: Stamp,
        read: Result<BackupDevices, DevicesRefused>,
    },
    Deleted {
        pressed: Stamp,
        id: String,
        result: Result<(), DeleteBackupError>,
    },
}

/// What [`DeviceCalls::pump`] found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DevicesPumped {
    pub changed: bool,
    pub busy: bool,
    pub deleted: Option<Result<(), &'static str>>,
}

/// See the module's doc.
pub struct DeviceCalls<E: DevicesEngine> {
    engine: Arc<E>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// Workers started and not finished (tests wait on it).
    pending: Arc<AtomicUsize>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// The rows read for `seen`, in the engine's order.
    held: Option<Vec<Device>>,
    /// The last read's Backups listing was complete.
    complete: bool,
    /// Why the list is not (or no longer) read, for `seen`.
    note: Option<&'static str>,
    read_at: Option<Instant>,
    /// A read is out for `seen`; `again`: another was asked meanwhile (after a delete), so one more follows it.
    reading: bool,
    again: bool,
    /// The device a delete is on its way for, under `seen`.
    deleting: Option<String>,
    /// The section was opened: a session or account that changes later is read again at once.
    wanted: bool,
    /// Tests only: the next worker waits on the first, before it asks the engine anything.
    #[cfg(test)]
    hold_next: Option<mpsc::Receiver<()>>,
}

impl<E: DevicesEngine> DeviceCalls<E> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<E>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            pending: Arc::default(),
            seen: None,
            held: None,
            complete: true,
            note: None,
            read_at: None,
            reading: false,
            again: false,
            deleting: None,
            wanted: false,
            #[cfg(test)]
            hold_next: None,
        }
    }

    /// No worker is running.
    #[cfg(test)]
    pub(crate) fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }

    /// The session or the account is not the one the last look saw: nothing of the earlier one is kept, here, now.
    fn look(&mut self, now: &Stamp) -> bool {
        if self.seen.as_ref() == Some(now) {
            return false;
        }
        self.seen = Some(now.clone());
        self.held = None;
        self.complete = true;
        self.note = None;
        self.read_at = None;
        self.reading = false;
        self.again = false;
        self.deleting = None;
        true
    }

    /// Whether what is held may be shown: a session is here and it is the one it was read for.
    fn current(&self) -> bool {
        self.engine.has_session() && self.seen.as_ref() == Some(&self.engine.stamp())
    }

    /// Start `job` on a worker of its own for the stamp `pressed`; its answer goes down the channel and wakes the
    /// window. `false`: no worker could be started.
    fn spawn(&mut self, job: impl FnOnce(&E) -> Answer + Send + 'static) -> bool {
        let (engine, answers, pending) = (self.engine.clone(), self.answers.clone(), self.pending.clone());
        #[cfg(test)]
        let hold = self.hold_next.take();
        pending.fetch_add(1, Ordering::SeqCst);
        let started = std::thread::Builder::new()
            .name("backups-devices".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some(hold) = hold {
                    let _ = hold.recv();
                }
                let answer = job(&*engine);
                let _ = answers.send(answer);
                pending.fetch_sub(1, Ordering::SeqCst);
                engine.wake();
            });
        if started.is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            log("backups: could not start a worker for other devices");
            return false;
        }
        true
    }

    /// Read the list for the session that is here. While one is out, one more follows it.
    fn ask_read(&mut self) {
        let Some(pressed) = self.seen.clone() else {
            return;
        };
        if !self.engine.has_session() {
            return;
        }
        if self.reading {
            self.again = true;
            return;
        }
        let stamped = pressed.clone();
        self.reading = self.spawn(move |engine| {
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.devices_for(stamped.0)))
                .unwrap_or_else(|_| {
                    log("backups: reading other devices ended early");
                    Err(DevicesRefused::Call(
                        beebeeb_engine::app::settings_api::AccountCallError::ServerError,
                    ))
                });
            Answer::Read { pressed: stamped, read }
        });
    }

    /// The section opened (or Try again): read when nothing is held for this session, the last read failed, or what
    /// is held is older than [`FRESH_FOR`].
    pub fn opened(&mut self) {
        self.wanted = true;
        let now = self.engine.stamp();
        self.look(&now);
        let fresh = self.held.is_some()
            && self.note != Some(NOT_READ)
            && self.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            self.ask_read();
        }
    }

    /// Take in what the workers answered. An answer counts only for the session AND the account of its press.
    pub fn pump(&mut self) -> DevicesPumped {
        let mut pumped = DevicesPumped::default();
        let now = self.engine.stamp();
        if self.look(&now) {
            pumped.changed = true;
            if self.wanted {
                self.ask_read();
            }
        }
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
                Answer::Read { pressed, read } => {
                    if self.seen.as_ref() != Some(&pressed) {
                        log("backups: other devices read for an earlier session were dropped");
                        continue;
                    }
                    self.reading = false;
                    pumped.changed = true;
                    self.take_read(read);
                    if std::mem::take(&mut self.again) {
                        self.ask_read();
                    }
                }
                Answer::Deleted { pressed, id, result } => {
                    if self.seen.as_ref() != Some(&pressed) {
                        log("backups: a delete's answer for an earlier session was dropped");
                        continue;
                    }
                    if self.deleting.as_deref() == Some(id.as_str()) {
                        self.deleting = None;
                    }
                    pumped.changed = true;
                    if let Err(error) = &result {
                        log(format!("backups: a backup was not deleted ({})", kind(error)));
                    }
                    if reached(&result) {
                        // Nothing was removed before the answer: the list is read again and shows what Beebeeb has.
                        self.ask_read();
                    }
                    pumped.deleted = Some(result.map_err(|error| words(&error)));
                }
            }
        }
        pumped.busy = self.reading || self.deleting.is_some();
        pumped
    }

    fn take_read(&mut self, read: Result<BackupDevices, DevicesRefused>) {
        match read {
            Ok(read) => {
                let today = chrono::Local::now().date_naive();
                self.held = Some(read.devices.iter().map(|device| device_of(device, today)).collect());
                self.complete = read.complete;
                self.note = None;
                self.read_at = Some(Instant::now());
            }
            Err(DevicesRefused::Local(LocalStateRefused::NothingSynced)) => {
                self.held = None;
                self.note = Some(NOT_SYNCED);
            }
            // This PC's local state could not be opened: said, never an empty "no other devices".
            Err(refused @ DevicesRefused::Local(LocalStateRefused::Unreadable)) => {
                log(format!("backups: other devices not read ({})", refused.kind()));
                self.note = Some(NOT_READ);
            }
            // Another account's local state (the section says so), a lock, a session that moved: nothing is shown.
            Err(refused @ (DevicesRefused::Local(_) | DevicesRefused::NotSignedIn)) => {
                log(format!("backups: other devices not read ({})", refused.kind()));
                self.held = None;
                self.note = None;
            }
            // Could not be read: what is held of this session stays, and the list says so. No freshness is stamped.
            Err(refused) => {
                log(format!("backups: other devices not read ({})", refused.kind()));
                self.note = Some(NOT_READ);
            }
        }
    }

    /// The rows for the session that is here (nothing while another's, or none, is).
    pub fn devices(&self) -> Vec<Device> {
        if !self.current() {
            return Vec::new();
        }
        self.held.clone().unwrap_or_default()
    }

    /// What "Other devices" says beside or instead of its rows.
    pub fn note(&self) -> Option<&'static str> {
        if !self.current() {
            return None;
        }
        if let Some(note) = self.note {
            return Some(note);
        }
        match &self.held {
            None if self.reading => Some(READING),
            Some(_) if !self.complete => Some(NOT_ALL),
            _ => None,
        }
    }

    /// A delete is on its way for the session that is here.
    pub fn deleting(&self) -> bool {
        self.current() && self.deleting.is_some()
    }

    /// Delete the backup of the listed device `id` after proving `password`. `true`: sent (the answer comes through
    /// [`Self::pump`]). `false`, nothing sent: no session, a delete already on its way, or an id the list does not
    /// hold for this session (or holds as possibly this PC's own).
    pub fn delete(&mut self, id: &str, password: &str) -> bool {
        // The session and account of the press, read before anything else.
        let pressed = self.engine.stamp();
        if self.look(&pressed) && self.wanted {
            self.ask_read();
        }
        if !self.engine.has_session() || self.deleting.is_some() || password.is_empty() {
            return false;
        }
        // Listed, and not a folder the engine says may be this PC's own (it refuses that delete too).
        let listed = self
            .held
            .as_ref()
            .is_some_and(|rows| rows.iter().any(|row| row.id == id && row.deletable));
        if !listed {
            return false;
        }
        // The worker's own copy, wiped when it ends (a panic included); it is in no type that prints.
        let secret = Zeroizing::new(password.as_bytes().to_vec());
        let (stamped, asked) = (pressed, id.to_string());
        let sent = self.spawn(move |engine| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.delete_for(stamped.0, &asked, &secret)
            }))
            .unwrap_or_else(|_| {
                log("backups: a delete ended early");
                Err(DeleteBackupError::Failed("server"))
            });
            drop(secret);
            Answer::Deleted {
                pressed: stamped,
                id: asked,
                result,
            }
        });
        if sent {
            self.deleting = Some(id.to_string());
        }
        sent
    }
}

/// Whether the delete may have reached Beebeeb (then the list is read again).
fn reached(result: &Result<(), DeleteBackupError>) -> bool {
    matches!(
        result,
        Ok(()) | Err(DeleteBackupError::Offline | DeleteBackupError::Failed("server") | DeleteBackupError::NotFound)
    )
}

/// The dialog's words for a delete that was not done.
fn words(error: &DeleteBackupError) -> &'static str {
    match error {
        DeleteBackupError::WrongPassword => WRONG_PASSWORD,
        DeleteBackupError::RateLimited(_) => TOO_MANY,
        DeleteBackupError::SignedOut => SIGNED_OUT,
        DeleteBackupError::NotFound => NOT_FOUND,
        DeleteBackupError::NotABackup | DeleteBackupError::Refused(_) => NOT_ALLOWED,
        DeleteBackupError::Offline | DeleteBackupError::Failed("server") => UNCONFIRMED,
        DeleteBackupError::Failed(_) => NOT_CHECKED,
    }
}

/// A delete's refusal by name, for a log line (never the rule's words: they can carry a path).
fn kind(error: &DeleteBackupError) -> &'static str {
    match error {
        DeleteBackupError::WrongPassword => "wrong_password",
        DeleteBackupError::RateLimited(_) => "rate_limited",
        DeleteBackupError::SignedOut => "signed_out",
        DeleteBackupError::NotFound => "not_found",
        DeleteBackupError::NotABackup => "not_a_backup",
        DeleteBackupError::Refused(_) => "refused",
        DeleteBackupError::Offline => "offline",
        DeleteBackupError::Failed(code) => code,
    }
}

/// One row as the section draws it, from what the engine read. Never a value it did not read.
fn device_of(device: &BackupDevice, today: chrono::NaiveDate) -> Device {
    let folder_name = || device.folder_name.clone().unwrap_or_else(|| UNNAMED.to_string());
    match &device.read {
        DeviceRead::Read(summary) => {
            let kind = match summary.class {
                DeviceClass::Computer => DeviceKind::Computer,
                DeviceClass::Phone => DeviceKind::Phone,
                DeviceClass::Tablet => DeviceKind::Tablet,
                DeviceClass::Unknown => DeviceKind::Unknown,
            };
            let model = summary.model.clone().unwrap_or_else(|| {
                match kind {
                    DeviceKind::Tablet => "tablet",
                    DeviceKind::Phone => "phone",
                    _ => "device",
                }
                .to_string()
            });
            Device {
                id: device.folder_id.clone(),
                name: summary.name.clone().unwrap_or_else(folder_name),
                model,
                kind,
                backed: Backed::Folders(summary.folders),
                size: None,
                last_backup: summary.last_backup_ms.map(|at| last_backup_words(at, today)),
                signed_in: None,
                deletable: !device.may_be_this_pc,
            }
        }
        DeviceRead::NoStatusFile | DeviceRead::Unreadable | DeviceRead::NotReached => Device {
            id: device.folder_id.clone(),
            name: folder_name(),
            model: "device".to_string(),
            kind: DeviceKind::Unknown,
            backed: Backed::Unreadable,
            size: None,
            last_backup: None,
            signed_in: None,
            deletable: !device.may_be_this_pc,
        },
    }
}
