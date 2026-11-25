//! RE3-a: the last full listing of each backup, kept between passes so a pass does not list the whole remote tree
//! again (0.9.0 listed it every 10 minutes, one `GET /files` per folder: ~2,700 requests in a burst for an account of
//! 28k backed-up files).
//!
//! A pass uses the kept listing while it is younger than [`FULL_LISTING_EVERY`] and the account's ops since it
//! (`/sync/ops`, one request) touch none of its folders or files; it hands core the kept tree as partial, so core
//! marks nothing removed elsewhere from it (the daily full listing does). Our own uploads only add files (core's
//! records know them), so they leave the kept listing usable.
//!
//! In memory, per process: the first pass after a start lists once. A stop or a sign-out clears it ([`clear`]).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use beebeeb_backup::DeviceTree;
use uuid::Uuid;

/// A backup is listed in full at most this often (and when its listing is lost, touched or its records are empty).
pub const FULL_LISTING_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// Ops that touch a kept listing's files (a rename, a move, a trash elsewhere) list again at most this often; one
/// that trashes or moves a kept FOLDER lists at the next pass (an upload into a trashed folder is never planned).
pub const TOUCHED_LISTING_GAP: Duration = Duration::from_secs(60 * 60);

/// One backup's last full listing.
#[derive(Clone, Debug)]
pub struct KeptListing {
    pub category_folder_id: String,
    /// The tree as listed (`complete` as the listing was).
    pub tree: DeviceTree,
    pub listed_at: Instant,
    /// The op cursor the listing was read at; `None`: listed page by page (no snapshot), so nothing tells what
    /// changed after it and it is not used again.
    pub cursor: Option<i64>,
}

/// Kept listings by (device folder id, category key).
static KEPT: Mutex<Option<HashMap<(String, String), KeptListing>>> = Mutex::new(None);
/// Backups (by category folder id) whose records were rebuilt from a full listing in this process: their records
/// stay empty when nothing matched (an emptied source), and the rebuild is not run again on every pass.
static REBUILT: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Forget every kept listing and rebuild (a stop, a lock, a sign-out).
pub fn clear() {
    if let Ok(mut kept) = KEPT.lock() {
        *kept = None;
    }
    if let Ok(mut rebuilt) = REBUILT.lock() {
        *rebuilt = None;
    }
}

pub fn get(device: &str, category: &str) -> Option<KeptListing> {
    let kept = KEPT.lock().ok()?;
    kept.as_ref()?.get(&(device.to_string(), category.to_string())).cloned()
}

pub fn put(device: &str, category: &str, listing: KeptListing) {
    if let Ok(mut kept) = KEPT.lock() {
        kept.get_or_insert_with(HashMap::new)
            .insert((device.to_string(), category.to_string()), listing);
    }
}

/// The kept listing's ops were read up to `cursor`.
pub fn advance(device: &str, category: &str, cursor: i64) {
    if let Ok(mut kept) = KEPT.lock()
        && let Some(listing) = kept
            .as_mut()
            .and_then(|kept| kept.get_mut(&(device.to_string(), category.to_string())))
    {
        listing.cursor = Some(listing.cursor.map_or(cursor, |at| at.max(cursor)));
    }
}

/// A folder the pass made (or found) under `parent` joins every kept listing that holds `parent`, so the next pass
/// does not look for it again.
pub fn add_folder(parent: Uuid, name: &str, id: Uuid) {
    let Ok(mut kept) = KEPT.lock() else { return };
    for listing in kept.iter_mut().flat_map(|kept| kept.values_mut()) {
        let parent_key = listing
            .tree
            .folders
            .iter()
            .find_map(|(key, folder)| (*folder == parent).then(|| key.clone()));
        if let Some(parent_key) = parent_key {
            let key = if parent_key.is_empty() {
                name.to_string()
            } else {
                format!("{parent_key}/{name}")
            };
            listing.tree.folders.entry(key).or_insert(id);
        }
    }
}

pub fn rebuilt(category_folder_id: &str) -> bool {
    REBUILT
        .lock()
        .ok()
        .is_some_and(|set| set.as_ref().is_some_and(|set| set.contains(category_folder_id)))
}

pub fn mark_rebuilt(category_folder_id: &str) {
    if let Ok(mut set) = REBUILT.lock() {
        set.get_or_insert_with(HashSet::new)
            .insert(category_folder_id.to_string());
    }
}

/// What the ops after a kept listing say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpsVerdict {
    /// Nothing touched it; the ops were read up to the head.
    Quiet,
    /// A file of it was renamed, moved, trashed or deleted elsewhere (or by this pass's own move).
    FileTouched,
    /// One of its folders (or the device folder) was renamed, moved, trashed or deleted.
    FolderTouched,
}

/// Op types that only add (a create, a new version): our own uploads, which a kept listing need not see.
fn adds_only(op_type: &str) -> bool {
    matches!(op_type, "file_create" | "folder_create" | "file_update")
}

/// Judge `ops` against `listing` (and the device folder `device`).
pub fn verdict(listing: &KeptListing, device: &str, ops: &[crate::api_client::SyncOp]) -> OpsVerdict {
    let folders: HashSet<String> = listing
        .tree
        .folders
        .values()
        .map(Uuid::to_string)
        .chain([listing.category_folder_id.clone(), device.to_string()])
        .collect();
    let files: HashSet<String> = listing.tree.files.iter().map(|f| f.remote_id.to_string()).collect();
    let mut verdict = OpsVerdict::Quiet;
    for op in ops {
        if adds_only(&op.op_type) {
            continue;
        }
        let ids = ["id", "file_id", "folder_id"].map(|field| op.payload[field].as_str());
        if ids.iter().flatten().any(|id| folders.contains(*id)) {
            return OpsVerdict::FolderTouched;
        }
        if ids.iter().flatten().any(|id| files.contains(*id)) {
            verdict = OpsVerdict::FileTouched;
        }
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(op_type: &str, id: &str) -> crate::api_client::SyncOp {
        crate::api_client::SyncOp {
            seq_id: 1,
            op_type: op_type.into(),
            payload: serde_json::json!({ "id": id }),
        }
    }

    #[test]
    fn creates_leave_a_kept_listing_and_a_trashed_folder_does_not() {
        let (cat, sub, file) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let listing = KeptListing {
            category_folder_id: cat.to_string(),
            tree: DeviceTree {
                device_folder_live: true,
                category_folder_live: Some(true),
                folders: HashMap::from([(String::new(), cat), ("Trip".to_string(), sub)]),
                files: vec![beebeeb_core::backup::RemoteEntry {
                    key: "Trip/a.jpg".into(),
                    remote_id: file,
                    size: 1,
                    uploaded_at_ms: 0,
                }],
                complete: true,
            },
            listed_at: Instant::now(),
            cursor: Some(0),
        };
        let other = Uuid::new_v4().to_string();
        assert_eq!(
            verdict(
                &listing,
                "dev",
                &[op("file_create", &other), op("file_update", &file.to_string())]
            ),
            OpsVerdict::Quiet
        );
        assert_eq!(verdict(&listing, "dev", &[op("file_trash", &other)]), OpsVerdict::Quiet);
        assert_eq!(
            verdict(&listing, "dev", &[op("file_move", &file.to_string())]),
            OpsVerdict::FileTouched
        );
        assert_eq!(
            verdict(
                &listing,
                "dev",
                &[op("file_rename", &file.to_string()), op("file_trash", &sub.to_string())]
            ),
            OpsVerdict::FolderTouched
        );
        assert_eq!(
            verdict(&listing, "dev", &[op("folder_move", "dev")]),
            OpsVerdict::FolderTouched
        );
    }
}
