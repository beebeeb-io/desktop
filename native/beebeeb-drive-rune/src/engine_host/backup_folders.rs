//! Slice A7 (plan40 batch 1): Add a folder and Stop backing up (DW-0737, DW-0738, DW-1410). Part of
//! [`EngineBackupsSource`] (`backups.rs`), which hands `add_folder` and `remove_folder` here through
//! `not_wired(…)`.
//!
//! Not bound yet: the adapter's bodies of before the scaffold, moved unchanged. The settings file holds Windows' six
//! known folders only, so a chosen folder is never added, and there is none to remove. Session-bound like its
//! neighbours once bound.

use super::*;

/// What slice A7 keeps between a press and its answer: its worker's inbox, what it holds for one session and
/// account. The adapter holds one (`EngineBackupsSource::backup_folders`). Empty until A7 binds.
#[derive(Default)]
pub(super) struct BackupFolderCalls {}

impl<E: BackupsEngine> EngineBackupsSource<E> {
    pub(super) fn backup_folders_add_folder(&mut self, _path: &str) -> bool {
        false
    }

    pub(super) fn backup_folders_remove_folder(&mut self, _id: &str, _move_copy_to_trash: bool) {}
}
