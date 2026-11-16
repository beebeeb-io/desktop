//! One place on disk, two server items.
//!
//! The server keeps every item by its id and cannot compare names it never reads, so one folder can hold two items
//! whose names differ only by letter case (`Report.pdf` and `report.pdf`, from another device or the web), or even two
//! with the very same name. The local state keeps both rows, each by its id, and the window lists both: each is a
//! server item (D-T-15). The disk holds ONE entry for them, though: Windows and macOS compare names without case, and
//! no system holds two entries with one name. A placeholder is never created over an existing entry, so the entry is
//! the row's whose name it was created with, and its name on disk is that row's spelling.
//!
//! A press that reaches the disk by a row's path (open, show, rename, move, duplicate, a create or an upload into a
//! folder, a local read) therefore asks here first whether the place is another row's ([`place_is_another_rows`]):
//! another server item of its kind has exactly its path ([`shares_its_exact_path`]), or the entries on disk along the
//! path are spelled otherwise ([`spelled_as_on_disk`]) WHILE another row has that place. A row alone at its place is
//! never refused for its spelling: after a rename by case alone elsewhere its entry keeps the old spelling for a while
//! (a remote rename leaves the entry where it is, `engine_bridge::meta_2`), and the entry is still its own. A place
//! that is another row's is a [`DriveError::Conflict`]: the other row's bytes are never opened, renamed, moved, copied
//! or read as this row's. The row stays reachable through every press that goes by its id (Move to Trash, versions, a
//! link, the in-app viewer's download).

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use super::DriveError;
use crate::state_db::{FileEntry, FileStatus, PrunedRow, StateDb};

/// How the volume under the account's folder compares names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseRule {
    /// Windows (NTFS) and macOS (APFS as it is formatted by default): `report.pdf` and `Report.pdf` are one name.
    Insensitive,
    /// Linux and the rest: two names.
    Sensitive,
}

impl CaseRule {
    /// The rule of the system this build runs on.
    pub fn host() -> Self {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(rule) = TEST_RULE.with(std::cell::Cell::get) {
            return rule;
        }
        Self::of_os(std::env::consts::OS)
    }

    /// The rule of the system `os` names (as `std::env::consts::OS` names it).
    pub fn of_os(os: &str) -> Self {
        match os {
            "windows" | "macos" => Self::Insensitive,
            _ => Self::Sensitive,
        }
    }

    /// Whether `a` and `b` are one name under this rule.
    pub fn same_name(self, a: &str, b: &str) -> bool {
        match self {
            Self::Insensitive => a == b || a.to_lowercase() == b.to_lowercase(),
            Self::Sensitive => a == b,
        }
    }

    /// A path as this rule compares it: trimmed of its outer slashes, and without case where case does not count.
    pub fn fold(self, path: &str) -> String {
        let path = path.trim_matches('/');
        match self {
            Self::Insensitive => path.to_lowercase(),
            Self::Sensitive => path.to_string(),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_RULE: std::cell::Cell<Option<CaseRule>> = const { std::cell::Cell::new(None) };
}

/// Test builds only: run `f` with [`CaseRule::host`] answering `rule` on this thread (a test on Linux plays a volume
/// that compares names without case).
#[cfg(any(test, feature = "test-support"))]
pub fn with_rule_for_test<T>(rule: CaseRule, f: impl FnOnce() -> T) -> T {
    struct Reset(Option<CaseRule>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_RULE.with(|cell| cell.set(self.0));
        }
    }
    let _reset = Reset(TEST_RULE.with(|cell| cell.replace(Some(rule))));
    f()
}

/// Whether the entries already on disk along `rel` (below `sync_root`) carry the names `rel` gives them. An entry
/// that is the same name under `rule` but spelled otherwise (`Report.pdf` where `rel` says `report.pdf`) answers
/// `false`, and so does a name the system reaches with no entry spelled so (an 8.3 short name, another Unicode form):
/// the place is then another entry's, or this row's own under an older spelling. A name with no entry ends the walk
/// (nothing is there yet; a create makes exactly that name), and so does a folder that is not there. `Err`: a folder on
/// the way could not be read.
pub fn spelled_as_on_disk(sync_root: &Path, rel: &str, rule: CaseRule) -> io::Result<bool> {
    spelled_in(&mut Listings::default(), sync_root, rel, rule)
}

/// The names of each folder read so far, so a pass over many rows reads each folder once. `None`: not there.
#[derive(Default)]
struct Listings(HashMap<PathBuf, Option<Vec<String>>>);

fn spelled_in(listings: &mut Listings, sync_root: &Path, rel: &str, rule: CaseRule) -> io::Result<bool> {
    if rule == CaseRule::Sensitive {
        // Every name is its own entry: nothing to read.
        return Ok(true);
    }
    let mut dir = sync_root.to_path_buf();
    for name in rel.trim_matches('/').split('/').filter(|name| !name.is_empty()) {
        if !listings.0.contains_key(&dir) {
            let names = match std::fs::read_dir(&dir) {
                Ok(entries) => Some(
                    entries
                        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                        .collect::<io::Result<Vec<_>>>()?,
                ),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            listings.0.insert(dir.clone(), names);
        }
        let Some(names) = &listings.0[&dir] else {
            return Ok(true);
        };
        if names.iter().any(|found| found == name) {
            dir.push(name);
            continue;
        }
        // The same name by `same_name`, or one the system reaches although no entry is spelled so.
        let other =
            names.iter().any(|found| rule.same_name(found, name)) || std::fs::symlink_metadata(dir.join(name)).is_ok();
        return Ok(!other);
    }
    Ok(true)
}

/// Whether another server item of `entry`'s kind has exactly its path. The two are one entry on disk on every system,
/// and nothing in the local state says whose it is. A row the server never confirmed is not such an item: it folds
/// into the server's row at its path (`model::live_rows`), and the server's row keeps its presses.
pub fn shares_its_exact_path(db: &StateDb, entry: &FileEntry) -> Result<bool, DriveError> {
    let path = entry.path.trim_matches('/');
    if entry.remote_updated_at <= 0 || path.is_empty() {
        return Ok(false);
    }
    for stored in [path.to_string(), format!("/{path}")] {
        let rows = db.file_rows_at_path(&stored).map_err(DriveError::db)?;
        if rows.iter().any(|other| {
            other.file_id != entry.file_id
                && other.status != FileStatus::Trashing
                && other.remote_updated_at > 0
                && other.is_dir() == entry.is_dir()
                && other.path.trim_matches('/') == path
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The place of every row still in the local state (not being trashed) under `rule`. Reads every row: asked only when
/// the disk spells a place otherwise, which is rare.
fn held_places(db: &StateDb, rule: CaseRule, except: Option<&str>) -> Result<HashSet<String>, DriveError> {
    Ok(db
        .file_overview_rows()
        .map_err(DriveError::db)?
        .iter()
        .filter(|(row, _)| {
            row.status != FileStatus::Trashing
                && Some(row.file_id.as_str()) != except
                && !row.path.trim_matches('/').is_empty()
        })
        .map(|(row, _)| rule.fold(&row.path))
        .collect())
}

/// Whether the entry at `entry`'s place below `sync_root` is another row's: another server item of its kind has exactly
/// its path, or the disk spells the place otherwise while another row has it under [`CaseRule::host`]. `Err`: a folder
/// on the way or the local state could not be read.
pub fn place_is_another_rows(db: &StateDb, sync_root: &Path, entry: &FileEntry) -> Result<bool, DriveError> {
    if shares_its_exact_path(db, entry)? {
        return Ok(true);
    }
    let rule = CaseRule::host();
    match spelled_as_on_disk(sync_root, &entry.path, rule) {
        Ok(true) => Ok(false),
        Ok(false) => Ok(held_places(db, rule, Some(&entry.file_id))?.contains(&rule.fold(&entry.path))),
        Err(error) => Err(DriveError::io(error)),
    }
}

/// Refuse a press on the place of `entry` (`None`: the account's folder itself, always its own) when it is another
/// row's ([`place_is_another_rows`]).
pub fn refuse_shared_place(db: &StateDb, sync_root: &Path, entry: Option<&FileEntry>) -> Result<(), DriveError> {
    match entry {
        Some(entry) if place_is_another_rows(db, sync_root, entry)? => Err(DriveError::Conflict),
        _ => Ok(()),
    }
}

/// Of `pruned` (rows a snapshot no longer has, already gone from `db`), the ones whose entry on disk may be removed:
/// not one whose place on disk is spelled otherwise while a row still in the local state has that place under `rule`.
/// That entry is the other row's (`Report.pdf` is still there when the server's `report.pdf` went to Trash), and it
/// stays. A place whose folder cannot be read counts as spelled otherwise, and a local state that cannot be read leaves
/// every such place alone. Each folder is read once. The order (children before parents) is kept.
pub fn removable_places(db: &StateDb, sync_root: &Path, pruned: &[PrunedRow], rule: CaseRule) -> Vec<PrunedRow> {
    let mut listings = Listings::default();
    let spelled_otherwise: Vec<bool> = pruned
        .iter()
        .map(|row| !matches!(spelled_in(&mut listings, sync_root, &row.path, rule), Ok(true)))
        .collect();
    if !spelled_otherwise.contains(&true) {
        return pruned.to_vec();
    }
    // Rare (two items at one place, one of them gone): read every remaining place once.
    let held = held_places(db, rule, None).ok();
    pruned
        .iter()
        .zip(&spelled_otherwise)
        .filter(|(row, otherwise)| {
            !**otherwise || held.as_ref().is_some_and(|held| !held.contains(&rule.fold(&row.path)))
        })
        .map(|(row, _)| row.clone())
        .collect()
}
