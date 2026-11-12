//! The three answers of the OS a sync start needs (contract
//! `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`, §4.2): whose sync folder a folder is,
//! "register this folder" (Windows Cloud Files; nothing to do on another OS), and whether another live process
//! holds this app's engine lock.
//!
//! `Engine::start_sync` asks the OS ([`OsSyncRoots`]). A test asks a port of its own through
//! `Engine::start_sync_with` (test builds only), so each answer can be made on any OS.
//!
//! Must not: read a registration from anywhere but `sync_root_owner` (the one reader), or register a folder another
//! way than `windows_cf::register_sync_root` (which asks `sync_root_owner` first).

use std::path::Path;

use crate::identity::EngineIdentity;
use crate::sync_root_owner::Registration;

pub trait SyncRootPort: Send + Sync {
    /// What the OS says about `folder`'s sync-root registration now.
    fn registration(&self, folder: &Path) -> Registration;
    /// Register `folder` as this app's sync root. `Err`: refused, with the OS's code when it gave one.
    fn register(&self, folder: &Path) -> Result<(), Option<i32>>;
    /// Whether the engine lock in `state_dir` is held by a live process (the read half of `LockFile::acquire`).
    fn engine_lock_held(&self, state_dir: &Path) -> bool;
}

/// The OS's answers, for the identity the handle was opened with.
pub struct OsSyncRoots {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    identity: &'static EngineIdentity,
}

impl OsSyncRoots {
    pub fn new(identity: &'static EngineIdentity) -> Self {
        OsSyncRoots { identity }
    }
}

impl SyncRootPort for OsSyncRoots {
    fn registration(&self, folder: &Path) -> Registration {
        crate::sync_root_owner::registered(folder)
    }

    #[cfg(target_os = "windows")]
    fn register(&self, folder: &Path) -> Result<(), Option<i32>> {
        crate::windows_cf::register_sync_root(self.identity, folder).map_err(|error| hresult_of(&error))
    }

    /// No other OS has a sync-root registration to make.
    #[cfg(not(target_os = "windows"))]
    fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
        Ok(())
    }

    fn engine_lock_held(&self, state_dir: &Path) -> bool {
        crate::lockfile::held_by_a_live_process(state_dir)
    }
}

/// The Windows code (`HRESULT`) behind `error`, when Windows gave one.
#[cfg(target_os = "windows")]
pub(crate) fn hresult_of(error: &anyhow::Error) -> Option<i32> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<windows::core::Error>())
        .map(|windows| windows.code().0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read half answers as `LockFile::acquire` would decide: no lock, or a lock it would overwrite (a dead
    /// process, unreadable JSON), is free; a live process's lock, or a lock that cannot be read, is held.
    #[test]
    fn the_lock_is_held_exactly_when_acquire_would_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join(crate::state_paths::LOCK_FILENAME);
        let os = OsSyncRoots::new(&crate::identity::TEST);
        assert!(!os.engine_lock_held(dir.path()), "no lock");
        let held_by = |pid: u32| {
            format!(r#"{{"pid":{pid},"agent":"desktop","hostname":"pc","started_at":"2026-11-12T10:00:00Z"}}"#)
        };
        std::fs::write(&lock, held_by(std::process::id())).unwrap();
        assert!(os.engine_lock_held(dir.path()), "a live process holds it");
        std::fs::write(&lock, "not json").unwrap();
        assert!(
            !os.engine_lock_held(dir.path()),
            "a corrupt lock is overwritten by acquire"
        );
        std::fs::remove_file(&lock).unwrap();
        std::fs::create_dir(&lock).unwrap();
        assert!(
            os.engine_lock_held(dir.path()),
            "a lock that cannot be read refuses, as acquire does"
        );
    }
}
