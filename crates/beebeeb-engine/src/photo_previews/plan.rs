//! Which photos get a preview made on this PC.
//!
//! A candidate is a photo of the signed-in account's local state that:
//! - is a picture this PC decodes ([`encode::decodable`]: never HEIC, RAW, TIFF or a video);
//! - is `Local` (its bytes were here at the last reconcile), and not being uploaded;
//! - has an owner-class id (`item_id::item_class == Owner`: the preview key is derived from it);
//! - is listed by the server among the account's OWN media (the per-file read answers the owner's files only: a
//!   photo in a folder shared with this account is never listed, so never a source);
//! - has no held write queued in the local state (a change the server does not hold yet);
//! - has no preview at exactly the setting's format and size yet, unless this is a rebuild.
//!
//! `Local` is the row's status as of the last reconcile pass: the file can have been freed since (File Explorer's
//! "Free up space"). So right before a candidate is opened, [`ready_to_read`] asks its attributes only
//! ([`crate::cloud_attrs`]) and its size on disk: a placeholder, or a file whose size is not the row's, is skipped and
//! never opened (opening a placeholder downloads it).
//!
//! No remainder is kept: every run plans again from the server's state, so a run that was stopped goes on where the
//! server says it ended. A rebuild starts again from the first photo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use beebeeb_core::item_id::{ItemClass, item_class};

use super::encode::decodable;
use super::{PreviewSetting, PreviewState};
use crate::drive_v2::model::{leaf_name, live_rows};
use crate::state_db::{FileStatus, StateDb};

/// A photo of the local state, as the plan reads it. Holds a path: its `Debug` prints none.
#[derive(Clone, PartialEq)]
pub struct LocalPhoto {
    pub id: String,
    /// The path under the sync folder, as the local state records it.
    pub rel_path: String,
    pub name: String,
    pub bytes: u64,
    pub status: FileStatus,
    pub held_write: bool,
}

impl std::fmt::Debug for LocalPhoto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalPhoto")
            .field("bytes", &self.bytes)
            .field("status", &self.status)
            .field("held_write", &self.held_write)
            .finish_non_exhaustive()
    }
}

/// Every photo this PC could decode in `db`, newest first. `None`: the local state could not be read.
pub fn local_photos(db: &StateDb) -> Option<Vec<LocalPhoto>> {
    let mut rows: Vec<_> = live_rows(db)
        .ok()?
        .into_iter()
        .filter(|row| !row.is_dir())
        .filter_map(|row| {
            let name = leaf_name(&row);
            decodable(&name).then_some((row, name))
        })
        .collect();
    rows.sort_by(|a, b| {
        b.0.modified_at
            .cmp(&a.0.modified_at)
            .then_with(|| a.0.path.cmp(&b.0.path))
    });
    let mut photos = Vec::with_capacity(rows.len());
    for (row, name) in rows {
        let held_write = db
            .item_presentation(&row.file_id)
            .ok()?
            .is_some_and(|presentation| presentation.held_write_queued);
        photos.push(LocalPhoto {
            id: row.file_id,
            rel_path: row.path,
            name,
            bytes: u64::try_from(row.size_bytes).unwrap_or(0),
            status: row.status,
            held_write,
        });
    }
    Some(photos)
}

/// Whether `photo` may be a source at all (every rule but the stored preview's).
pub fn may_be_source(photo: &LocalPhoto, owned: &HashMap<String, PreviewState>) -> bool {
    photo.status == FileStatus::Local
        && !photo.held_write
        && decodable(&photo.name)
        && matches!(item_class(&photo.id), Ok(ItemClass::Owner))
        && owned.contains_key(&photo.id)
}

/// The photos to make a preview for, in order (newest first). `owned`: the server's per-file facts of the account's
/// own media, by id. `force`: a rebuild (a preview that already matches is made again).
pub fn plan<'a>(
    photos: &'a [LocalPhoto],
    owned: &HashMap<String, PreviewState>,
    setting: PreviewSetting,
    force: bool,
) -> Vec<&'a LocalPhoto> {
    photos
        .iter()
        .filter(|photo| may_be_source(photo, owned))
        .filter(|photo| force || !owned[&photo.id].matches(setting.format, setting.px))
        .collect()
}

/// The original of `photo` under `sync_root`, when every byte of it is on this PC now and its size is the one the
/// local state records: asked from the attributes and the size only, so nothing is fetched. `None`: skip it, never
/// open it.
pub fn ready_to_read(sync_root: &Path, photo: &LocalPhoto) -> Option<PathBuf> {
    let path = crate::engine_bridge::local_file_path_under_sync_root(sync_root, &photo.rel_path).ok()?;
    if !crate::cloud_attrs::file_bytes_here(&path) {
        return None;
    }
    let on_disk = std::fs::symlink_metadata(&path).ok()?.len();
    (on_disk == photo.bytes).then_some(path)
}
