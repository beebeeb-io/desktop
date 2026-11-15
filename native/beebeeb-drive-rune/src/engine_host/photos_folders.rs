//! Slice B1 (plan40 batch 1): Photo sources' folders (DW-0604) and the devices' backup line (DW-0601). Part of
//! [`EnginePhotos`] (`photos.rs`), which hands `folders` and `backup_status` here through `not_wired(…)`.
//!
//! Not bound yet: the adapter's bodies of before the scaffold, moved unchanged. The engine has no "folder from Files
//! that Photos shows" (every photo and video of the account shows already), and this PC does not know what another
//! device's backup is doing now. Session-bound like its neighbours once bound (`stamp.rs`).

use super::*;

/// What slice B1 keeps between a press and its answer: its worker's inbox, what it holds for one session and
/// account. The adapter holds one (`EnginePhotos::photos_folders`). Empty until B1 binds.
#[derive(Default)]
pub(super) struct PhotoFolderCalls {}

impl<C: CredentialStores> EnginePhotos<C> {
    pub(super) fn photos_folders_folders(&self) -> Vec<Folder> {
        Vec::new()
    }

    pub(super) fn photos_folders_backup_status(&self) -> Option<BackupStatus> {
        None
    }
}
