//! The first run on this computer, for a host without a UI toolkit (the native first-run steps): whether it is
//! still to do, marking it done, the sync folder a person chooses in it, and the one switch of its steps that
//! lives in the settings file (notifications).
//!
//! "Still to do" is the settings file's `native_setup_done` (sync-start contract §5.1): [`decide_first_run`]
//! writes it once when the native app first opens on this computer (done for an install whose settings already
//! name a sync folder, so an upgrade from 0.8.x never sees the first run), `mark_first_run_done_for` writes it
//! when the person ends the first run.
//!
//! The sync folder: [`Engine::check_sync_folder`] is the rule before a start (`crate::sync_folder_rule`);
//! [`Engine::keep_sync_root`] writes the folder a start succeeded in to the settings, once, and only when they
//! name none, so a home folder that moves later cannot move the sync folder.
//!
//! The session-bound writes are for the session a person pressed under and no other: a session that moved on (a
//! lock, a sign-out, another sign-in) writes nothing. They are this computer's settings, not the account's: the
//! check is that the person who pressed is still the one signed in. Each BLOCKS (it reads or writes the settings
//! file, or asks the OS): a worker's, never the UI thread's. Must not: log a path.

use std::path::{Path, PathBuf};

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::config::DesktopConfig;
use crate::identity::EngineIdentity;
use crate::sync_folder_rule::{self, Facts, OnDisk, Owner, SyncFolderRefused};
use crate::sync_root_owner::{self, Registration};

#[cfg(test)]
#[path = "setup_api_tests.rs"]
mod tests;

/// Why a first-run read or write did not happen. Nothing was written in either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupRefused {
    /// The session of the press is not the one in memory any more (or a session change is under way).
    SessionMoved,
    /// The settings file could not be read or written.
    Settings,
}

impl SetupRefused {
    /// The refusal's name, for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            Self::SessionMoved => "session_moved",
            Self::Settings => "settings",
        }
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Whether this computer still has its first run to do. A settings file that cannot be read is an error, never
    /// a "yes": a first run is not shown to a person because a file did not read.
    pub fn first_run_due(&self) -> Result<bool, SetupRefused> {
        DesktopConfig::load_for(self.identity)
            .map(|config| config.native_setup_done != Some(true))
            .map_err(|_| SetupRefused::Settings)
    }

    /// WRITES the settings file: the first run on this computer is done, and nothing else. `session` is
    /// [`Engine::session_turn`] as the host read it AT THE PRESS.
    pub fn mark_first_run_done_for(&self, session: SessionGeneration) -> Result<(), SetupRefused> {
        self.setup_write(session, |config| {
            let changed = config.native_setup_done != Some(true);
            config.native_setup_done = Some(true);
            changed
        })
    }

    /// The sync folder the settings name, if they name one. `Err`: the settings could not be read.
    pub fn settings_sync_root(&self) -> Result<Option<PathBuf>, SetupRefused> {
        DesktopConfig::load_for(self.identity)
            .map(|config| config.sync_root)
            .map_err(|_| SetupRefused::Settings)
    }

    /// The folder a first run suggests: the identity's sync folder in the home folder.
    pub fn sync_root_suggestion(&self) -> PathBuf {
        crate::config::default_sync_root_suggestion(self.identity)
    }

    /// WRITES the settings file: `folder` is the sync folder from now on, only when the settings name none (the
    /// first start that worked decides; no later start moves it). `Ok(true)`: it was written.
    pub fn keep_sync_root(&self, folder: &Path) -> Result<bool, SetupRefused> {
        DesktopConfig::update_for(self.identity, |config| {
            let free = config.sync_root.is_none();
            if free {
                config.sync_root = Some(folder.to_path_buf());
            }
            (free, free)
        })
        .map_err(|_| SetupRefused::Settings)
    }

    /// Whether `folder` may be the sync folder (`crate::sync_folder_rule`): asked before the first start, for the
    /// folder a person chose. Reads the disk and asks the OS; creates, registers and writes nothing.
    pub fn check_sync_folder(&self, folder: &Path) -> Result<(), SyncFolderRefused> {
        let Some(text) = folder.to_str() else {
            return Err(SyncFolderRefused::NotAFullPath);
        };
        let facts = self.sync_folder_facts(folder);
        match sync_folder_rule::refusal(text, &facts) {
            Some(refused) => {
                tracing::warn!(kind = refused.kind(), "a folder was refused as the sync folder");
                Err(refused)
            }
            None => Ok(()),
        }
    }

    /// What the OS says about `folder` and this computer, for the rule.
    fn sync_folder_facts(&self, folder: &Path) -> Facts {
        let text = |path: PathBuf| path.to_str().map(str::to_string);
        let on_disk = match std::fs::symlink_metadata(folder) {
            Ok(found) if found.file_type().is_dir() => OnDisk::Folder,
            Ok(_) => OnDisk::NotAFolder,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => OnDisk::Missing,
            Err(_) => OnDisk::Unreadable,
        };
        // The owner check is the start's own (`sync_root_owner`): this app's provider, or nobody, is free.
        let registered = sync_root_owner::registered(folder);
        let owner = match (
            &registered,
            sync_root_owner::refusal(self.identity, &registered, folder),
        ) {
            (_, None) => Owner::Free,
            (Registration::Unreadable, Some(_)) => Owner::Unreadable,
            (_, Some(_)) => Owner::Another,
        };
        let settings = DesktopConfig::path_for(self.identity)
            .ok()
            .and_then(|file| file.parent().map(Path::to_path_buf));
        Facts {
            windows: cfg!(target_os = "windows"),
            home: dirs::home_dir().and_then(text),
            system: system_folders(),
            own: [Some(self.config.state_dir.clone()), settings]
                .into_iter()
                .flatten()
                .filter_map(text)
                .collect(),
            other_builds: text(crate::config::default_sync_root_suggestion(self.identity.other_build()))
                .into_iter()
                .collect(),
            remote_drive: on_a_network_drive(folder),
            on_disk,
            owner,
        }
    }

    /// WRITES the settings file: whether this computer shows Beebeeb's notifications at all (`notify_allow`, the
    /// switch Settings › Notifications reads). `session` as for [`Engine::mark_first_run_done_for`].
    pub fn set_notifications_allowed_for(&self, session: SessionGeneration, on: bool) -> Result<(), SetupRefused> {
        self.setup_write(session, |config| {
            let changed = config.notify_allow != on;
            config.notify_allow = on;
            changed
        })
    }

    /// The one path of this file's writes: only while the session in memory is still `session`, asked with no
    /// session change half done; then the settings file, under its own lock.
    fn setup_write(
        &self,
        session: SessionGeneration,
        change: impl FnOnce(&mut DesktopConfig) -> bool,
    ) -> Result<(), SetupRefused> {
        self.acct
            .while_session_is(&self.writes, session, |_| ())
            .ok_or(SetupRefused::SessionMoved)?;
        DesktopConfig::update_for(self.identity, |config| (change(config), ())).map_err(|_| SetupRefused::Settings)
    }
}

/// WRITES the settings file, once per computer: decide whether the native app's first run is still to do here
/// (contract §5.1). Called by the native app's `open()` before anything else writes. Settings that already say
/// are left as they are; settings that name a sync folder are an install that was set up before (0.8.x): done;
/// any other: to do. `Ok(true)`: the first run is to do.
pub fn decide_first_run(identity: &'static EngineIdentity) -> Result<bool, String> {
    DesktopConfig::update_for(identity, |config| {
        let undecided = config.native_setup_done.is_none();
        if undecided {
            config.native_setup_done = Some(config.sync_root.is_some());
        }
        (undecided, config.native_setup_done != Some(true))
    })
}

/// The folders this computer names as the system's own (Windows' environment), beside the ones the rule knows.
fn system_folders() -> Vec<String> {
    if !cfg!(target_os = "windows") {
        return Vec::new();
    }
    [
        "SystemRoot",
        "windir",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "ProgramData",
    ]
    .into_iter()
    .filter_map(|name| std::env::var(name).ok())
    .filter(|folder| !folder.is_empty())
    .collect()
}

/// Whether `folder`'s drive is a network drive. A mapped drive reads like any drive letter: only Windows knows.
#[cfg(target_os = "windows")]
fn on_a_network_drive(folder: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    /// `DRIVE_REMOTE` of `GetDriveTypeW`.
    const DRIVE_REMOTE: u32 = 4;
    let Some(Component::Prefix(prefix)) = folder.components().next() else {
        return false;
    };
    let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
        // A share is refused by its text; a device path is not a full path.
        return false;
    };
    let root: Vec<u16> = std::ffi::OsStr::new(&format!("{}:\\", letter as char))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call; the call only reads it.
    let kind = unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(root.as_ptr())) };
    kind == DRIVE_REMOTE
}

#[cfg(not(target_os = "windows"))]
fn on_a_network_drive(_folder: &Path) -> bool {
    false
}
