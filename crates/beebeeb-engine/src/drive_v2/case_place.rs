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
//! folder, a local read) therefore asks here first whether the place is that row's own. The rule: the entries on disk
//! along the path are spelled as the row spells them ([`spelled_as_on_disk`]), and no other server item of its kind
//! has exactly its path ([`shares_its_exact_path`]). Otherwise the press is a [`DriveError::Conflict`]: the other row's
//! bytes are never opened, renamed, moved, copied or read as this row's. The row stays reachable through every press
//! that goes by its id (Move to Trash, versions, a link, the in-app viewer's download).

use std::collections::HashSet;
use std::io;
use std::path::Path;

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
/// that is the same name under `rule` but spelled otherwise (`Report.pdf` where `rel` says `report.pdf`) is another
/// row's place: `false`. A name with no entry ends the walk (nothing is there yet; a create makes exactly that name),
/// and so does a folder that is not there. `Err`: a folder on the way could not be read.
pub fn spelled_as_on_disk(sync_root: &Path, rel: &str, rule: CaseRule) -> io::Result<bool> {
    if rule == CaseRule::Sensitive {
        // Every name is its own entry: nothing to read.
        return Ok(true);
    }
    let mut dir = sync_root.to_path_buf();
    for name in rel.trim_matches('/').split('/').filter(|name| !name.is_empty()) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error),
        };
        let (mut exact, mut other) = (false, false);
        for entry in entries {
            let found = entry?.file_name();
            let found = found.to_string_lossy();
            if found == name {
                exact = true;
                break;
            }
            other |= rule.same_name(&found, name);
        }
        // The system reaches the name although no entry is spelled so, nor the same name by `same_name` (another Unicode
        // form on APFS, an NTFS case pair `to_lowercase` does not pair, an 8.3 short name): another entry's place.
        if !exact && !other && std::fs::symlink_metadata(dir.join(name)).is_ok() {
            other = true;
        }
        if !exact {
            return Ok(!other);
        }
        dir.push(name);
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

/// Refuse a press at `rel` below `sync_root` when the entries on disk along it are spelled as another row's
/// ([`spelled_as_on_disk`] under [`CaseRule::host`]). A folder that cannot be read refuses too.
pub fn refuse_another_spelling(sync_root: &Path, rel: &str) -> Result<(), DriveError> {
    match spelled_as_on_disk(sync_root, rel, CaseRule::host()) {
        Ok(true) => Ok(()),
        Ok(false) => Err(DriveError::Conflict),
        Err(error) => Err(DriveError::io(error)),
    }
}

/// Refuse a press on the place of `entry` (`None`: the account's folder itself, always its own) unless it is that
/// row's alone: no other server item of its kind at exactly its path, and the entries on disk spelled as it spells them.
pub fn refuse_shared_place(db: &StateDb, sync_root: &Path, entry: Option<&FileEntry>) -> Result<(), DriveError> {
    let Some(entry) = entry else { return Ok(()) };
    if shares_its_exact_path(db, entry)? {
        return Err(DriveError::Conflict);
    }
    refuse_another_spelling(sync_root, &entry.path)
}

/// Of `pruned` (rows a snapshot no longer has, already gone from `db`), the ones whose entry on disk may be removed:
/// not one whose place on disk is spelled as another row's while a row still in the local state has that place under
/// `rule`. That entry is the other row's (`Report.pdf` is still there when the server's `report.pdf` went to Trash),
/// and it stays. A place whose folder cannot be read counts as spelled otherwise, and a local state that cannot be
/// read leaves every such place alone. The order (children before parents) is kept.
pub fn removable_places(db: &StateDb, sync_root: &Path, pruned: &[PrunedRow], rule: CaseRule) -> Vec<PrunedRow> {
    let spelled_otherwise: Vec<bool> = pruned
        .iter()
        .map(|row| !matches!(spelled_as_on_disk(sync_root, &row.path, rule), Ok(true)))
        .collect();
    if !spelled_otherwise.contains(&true) {
        return pruned.to_vec();
    }
    // Rare (two items at one place, one of them gone): read every remaining place once.
    let Ok(rows) = db.file_overview_rows() else {
        return pruned
            .iter()
            .zip(&spelled_otherwise)
            .filter(|(_, otherwise)| !**otherwise)
            .map(|(row, _)| row.clone())
            .collect();
    };
    let held: HashSet<String> = rows
        .iter()
        .filter(|(row, _)| row.status != FileStatus::Trashing && !row.path.trim_matches('/').is_empty())
        .map(|(row, _)| rule.fold(&row.path))
        .collect();
    pruned
        .iter()
        .zip(&spelled_otherwise)
        .filter(|(row, otherwise)| !**otherwise || !held.contains(&rule.fold(&row.path)))
        .map(|(row, _)| row.clone())
        .collect()
}
