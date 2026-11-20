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
use crate::state_db::StateDb;
use crate::sync_root_owner::Registration;

pub trait SyncRootPort: Send + Sync {
    /// What the OS says about `folder`'s sync-root registration now.
    fn registration(&self, folder: &Path) -> Registration;
    /// Register `folder` as this app's sync root. `Err`: refused, with the OS's code when it gave one.
    fn register(&self, folder: &Path) -> Result<(), Option<i32>>;
    /// Whether the engine lock in `state_dir` is held by a live process (the read half of `LockFile::acquire`).
    fn engine_lock_held(&self, state_dir: &Path) -> bool;
    /// Remove the previous account's synced items from `folder`, the items `db` lists, for an account switch.
    /// Only an item known to be synced and unchanged goes, checked per item at the moment it is deleted. `unasked` are
    /// the paths the engine asked [`may_hold_bytes`](Self::may_hold_bytes) about and was told nothing is there: an
    /// item of any kind found at one of them was made since, is no one's to delete, and refuses the whole removal
    /// before anything is deleted (so does any other item the database has no row for).
    /// `Err`: nothing may be assumed removed beyond what the OS did; the switch then goes no further. An OS with no
    /// way to do it safely refuses.
    fn remove_synced_items(&self, _db: &StateDb, _folder: &Path, _unasked: &[String]) -> Result<(), String> {
        Err("this OS cannot remove another account's synced items".to_string())
    }
    /// Whether the item at `path` (as the database writes it, under `folder`) may hold bytes on this PC. Asked for a
    /// file of unknown content before an account switch: one that may is never deleted unasked (sign-out's D11 rule).
    /// `false` only when the OS proves there are none: no item is there. An OS that cannot tell says `true`.
    fn may_hold_bytes(&self, _folder: &Path, _path: &str) -> bool {
        true
    }
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

    /// Windows' purge in its `UnknownFiles::Refuse` mode: it deletes only items proven clean while it holds each one
    /// (an exclusive handle binds the identity and in-sync check to the deletion), and refuses, deleting nothing, at
    /// any other item: an ordinary file, a placeholder with changes, a file at a path of `unasked` or of a file of
    /// unknown content (made after the engine's check), or one in a folder no row names. The engine asks for the
    /// removal only when the database holds no unsent work (`work_waiting_for_a_person`) and every file of unknown
    /// content was answered `false` by `may_hold_bytes`.
    #[cfg(target_os = "windows")]
    fn remove_synced_items(&self, db: &StateDb, folder: &Path, unasked: &[String]) -> Result<(), String> {
        crate::windows_cf::signout::purge(
            self.identity,
            db,
            Some(folder),
            &crate::windows_cf::signout::UnknownFiles::refuse_at(unasked.iter().map(String::as_str)),
        )
        .map(|_| ())
        .map_err(|_| "the purge refused".to_string())
    }
    /// No item at the path: no bytes. Any item there (an ordinary file, a placeholder whatever is on disk of it, a
    /// folder, something that cannot be read) may hold some. The reparse point itself is asked, never followed, and
    /// nothing is opened, so nothing is fetched.
    #[cfg(target_os = "windows")]
    fn may_hold_bytes(&self, folder: &Path, path: &str) -> bool {
        let at = folder.join(path.trim_start_matches('/').replace('/', "\\"));
        !matches!(std::fs::symlink_metadata(&at), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
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
        // A dead process's lock is free (acquire would overwrite it). PID 4_000_000_000 is above Linux's
        // default `pid_max` (4_194_304) and Windows' ceiling, so it is not alive on any test machine.
        std::fs::write(&lock, held_by(4_000_000_000)).unwrap();
        assert!(
            !os.engine_lock_held(dir.path()),
            "a dead process's lock is free (acquire would overwrite it)"
        );
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

    /// A port that was not taught the question never lets a file of unknown content go: it says "may hold bytes".
    #[test]
    fn a_port_that_cannot_tell_says_the_item_may_hold_bytes() {
        struct Untaught;
        impl SyncRootPort for Untaught {
            fn registration(&self, _folder: &Path) -> Registration {
                Registration::Nobody
            }
            fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
                Ok(())
            }
            fn engine_lock_held(&self, _state_dir: &Path) -> bool {
                false
            }
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(Untaught.may_hold_bytes(dir.path(), "nothing here.docx"));
    }

    /// Windows: no item at the path holds no bytes (also under a folder that is not there: the VM proof's case, a
    /// document made in the app); an ordinary file, even an empty one, and a folder may.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_says_no_bytes_only_where_no_item_is() {
        let dir = tempfile::tempdir().unwrap();
        let os = OsSyncRoots::new(&crate::identity::TEST);
        assert!(!os.may_hold_bytes(dir.path(), "Budget.docx"), "no item");
        assert!(!os.may_hold_bytes(dir.path(), "/Plans/Budget.docx"), "no folder, no item");
        std::fs::write(dir.path().join("Budget.docx"), b"typed, not sent").unwrap();
        assert!(os.may_hold_bytes(dir.path(), "Budget.docx"), "an ordinary file");
        assert!(os.may_hold_bytes(dir.path(), "/Budget.docx"), "the same, as the database writes it");
        std::fs::write(dir.path().join("Empty.docx"), b"").unwrap();
        assert!(os.may_hold_bytes(dir.path(), "Empty.docx"), "an ordinary file is never proven empty of work");
        std::fs::create_dir(dir.path().join("Plans")).unwrap();
        assert!(os.may_hold_bytes(dir.path(), "Plans"), "a folder where the file should be");
    }
}
