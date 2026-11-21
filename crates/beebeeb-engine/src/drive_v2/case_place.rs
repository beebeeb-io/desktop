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
//! that is another row's is a [`DriveError::SharedPlace`]: the other row's bytes are never opened, renamed, moved, copied
//! or read as this row's. The row stays reachable through every press that goes by its id (Move to Trash, versions, a
//! link, the in-app viewer's download).

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use super::DriveError;
use super::dto::{DriveItem, LocalState};
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
struct Listings(HashMap<PathBuf, Option<Names>>);

/// One folder's names: as spelled, and as [`CaseRule::Insensitive`] compares them (a name is looked up once, however
/// many entries the folder holds).
struct Names {
    spelled: HashSet<String>,
    folded: HashSet<String>,
}

fn spelled_in(listings: &mut Listings, sync_root: &Path, rel: &str, rule: CaseRule) -> io::Result<bool> {
    if rule == CaseRule::Sensitive {
        // Every name is its own entry: nothing to read.
        return Ok(true);
    }
    let mut dir = sync_root.to_path_buf();
    for name in rel.trim_matches('/').split('/').filter(|name| !name.is_empty()) {
        if !listings.0.contains_key(&dir) {
            let names = match std::fs::read_dir(&dir) {
                Ok(entries) => {
                    let spelled = entries
                        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                        .collect::<io::Result<HashSet<_>>>()?;
                    let folded = spelled.iter().map(|name| rule.fold(name)).collect();
                    Some(Names { spelled, folded })
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            listings.0.insert(dir.clone(), names);
        }
        let Some(names) = &listings.0[&dir] else {
            return Ok(true);
        };
        if names.spelled.contains(name) {
            dir.push(name);
            continue;
        }
        // The same name under `rule`, or one the system reaches although no entry is spelled so.
        let other = names.folded.contains(&rule.fold(name)) || std::fs::symlink_metadata(dir.join(name)).is_ok();
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

/// The place of every row still in the local state (not being trashed) under `rule`, with the ids of the rows there.
/// Reads every row: asked only when the disk spells a place otherwise, which is rare.
fn places_held(db: &StateDb, rule: CaseRule) -> Result<HashMap<String, Vec<String>>, DriveError> {
    let mut held: HashMap<String, Vec<String>> = HashMap::new();
    for (row, _) in db.file_overview_rows().map_err(DriveError::db)? {
        if row.status != FileStatus::Trashing && !row.path.trim_matches('/').is_empty() {
            held.entry(rule.fold(&row.path)).or_default().push(row.file_id);
        }
    }
    Ok(held)
}

/// Why the entry at a row's place on disk is another row's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clash {
    /// Another server item of its kind has exactly its name in its folder: one entry on every system, and nothing in
    /// the local state says whose.
    SameName,
    /// Another row has its name but for letter case, and the entry on disk carries a spelling that is not this row's
    /// (a volume that compares names without case).
    Case,
}

/// Many places asked in one pass (a folder's listing, a reclaim over many files): each folder on the way is read once,
/// and the places every row holds are read once, the first time a place on disk is spelled otherwise.
pub struct PlaceCheck<'a> {
    db: &'a StateDb,
    sync_root: &'a Path,
    rule: CaseRule,
    listings: Listings,
    held: Option<HashMap<String, Vec<String>>>,
}

impl<'a> PlaceCheck<'a> {
    pub fn new(db: &'a StateDb, sync_root: &'a Path, rule: CaseRule) -> Self {
        Self {
            db,
            sync_root,
            rule,
            listings: Listings::default(),
            held: None,
        }
    }

    pub fn sync_root(&self) -> &'a Path {
        self.sync_root
    }

    /// Whether the entry at `entry`'s place is another row's, and why ([`place_is_another_rows`]). `Err`: a folder on
    /// the way or the local state could not be read.
    pub fn clash(&mut self, entry: &FileEntry) -> Result<Option<Clash>, DriveError> {
        if shares_its_exact_path(self.db, entry)? {
            return Ok(Some(Clash::SameName));
        }
        match spelled_in(&mut self.listings, self.sync_root, &entry.path, self.rule) {
            Ok(true) => Ok(None),
            Ok(false) => {
                if self.held.is_none() {
                    self.held = Some(places_held(self.db, self.rule)?);
                }
                let another = self.held.as_ref().is_some_and(|held| {
                    held.get(&self.rule.fold(&entry.path))
                        .is_some_and(|ids| ids.iter().any(|id| *id != entry.file_id))
                });
                Ok(another.then_some(Clash::Case))
            }
            Err(error) => Err(DriveError::io(error)),
        }
    }

    /// Whether the row `file_id` has its own entry at its place: `false` for a row whose place is another row's, for a
    /// row that is not there, and when that cannot be read.
    pub fn is_own_place(&mut self, file_id: &str) -> bool {
        match self.db.get_file(file_id) {
            Ok(Some(entry)) => matches!(self.clash(&entry), Ok(None)),
            _ => false,
        }
    }
}

/// Whether the entry at `entry`'s place below `sync_root` is another row's: another server item of its kind has exactly
/// its path, or the disk spells the place otherwise while another row has it under [`CaseRule::host`]. `Err`: a folder
/// on the way or the local state could not be read.
pub fn place_is_another_rows(db: &StateDb, sync_root: &Path, entry: &FileEntry) -> Result<bool, DriveError> {
    Ok(PlaceCheck::new(db, sync_root, CaseRule::host()).clash(entry)?.is_some())
}

/// Mark the rows of a listing whose place on disk is another row's ([`PlaceCheck::clash`]): their `local` says so
/// ([`LocalState::NameShared`], [`LocalState::NameSharedButCase`]), as their presses on disk are refused. One pass over
/// `sync_root` under [`CaseRule::host`]: each folder on the way is read once. A row whose place cannot be read, or that
/// the local state does not have, keeps the state it had.
pub fn mark_shared_places(db: &StateDb, sync_root: &Path, items: &mut [DriveItem]) {
    let mut places = PlaceCheck::new(db, sync_root, CaseRule::host());
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    let Ok(entries) = db.get_files_by_ids(&ids) else {
        return;
    };
    for item in items {
        let Some(entry) = entries.get(&item.id) else {
            continue;
        };
        match places.clash(entry) {
            Ok(Some(Clash::SameName)) => item.local = Some(LocalState::NameShared),
            Ok(Some(Clash::Case)) => item.local = Some(LocalState::NameSharedButCase),
            Ok(None) | Err(_) => {}
        }
    }
}

/// Refuse a press on the place of `entry` (`None`: the account's folder itself, always its own) when it is another
/// row's ([`place_is_another_rows`]), with why ([`DriveError::SharedPlace`]).
pub fn refuse_shared_place(db: &StateDb, sync_root: &Path, entry: Option<&FileEntry>) -> Result<(), DriveError> {
    match entry {
        Some(entry) => match PlaceCheck::new(db, sync_root, CaseRule::host()).clash(entry)? {
            Some(clash) => Err(DriveError::SharedPlace(clash)),
            None => Ok(()),
        },
        None => Ok(()),
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
    let held = places_held(db, rule).ok();
    pruned
        .iter()
        .zip(&spelled_otherwise)
        .filter(|(row, otherwise)| {
            !**otherwise
                || held
                    .as_ref()
                    .is_some_and(|held| !held.contains_key(&rule.fold(&row.path)))
        })
        .map(|(row, _)| row.clone())
        .collect()
}

/// Whether another server item still in the local state (not being trashed) has exactly `path`, besides `file_id`.
/// `Err`: the local state could not be read.
fn another_server_item_at(db: &StateDb, path: &str, file_id: &str) -> Result<bool, DriveError> {
    for stored in [path.to_string(), format!("/{path}")] {
        let rows = db.file_rows_at_path(&stored).map_err(DriveError::db)?;
        if rows.iter().any(|other| {
            other.file_id != file_id
                && other.status != FileStatus::Trashing
                && other.remote_updated_at > 0
                && other.path.trim_matches('/') == path
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Of `pruned` (rows a snapshot no longer has, already gone from `db`), the ones whose entry may be removed when another
/// server item still in the local state has exactly the pruned row's path: two server items with one name are one
/// entry on every system, and nothing in the local state says whose it is. Then the entry goes only when `identity_at`
/// reads the pruned row's own id on it (a placeholder's identity, Windows). An entry that names the other row, names
/// nothing or cannot be read stays: it may be the other row's file. The pruned row is logged by its id. A row with no
/// such twin passes as it is, and the order (children before parents) is kept.
pub fn removable_by_identity(
    db: &StateDb,
    sync_root: &Path,
    pruned: Vec<PrunedRow>,
    identity_at: &dyn Fn(&Path) -> Option<String>,
) -> Vec<PrunedRow> {
    pruned
        .into_iter()
        .filter(|row| {
            let path = row.path.trim_matches('/');
            if path.is_empty() || !another_server_item_at(db, path, &row.file_id).unwrap_or(true) {
                return true;
            }
            let place = sync_root.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let own = identity_at(&place).as_deref() == Some(row.file_id.as_str());
            if !own {
                tracing::warn!(
                    file_id = %row.file_id,
                    "pruned row not removed: the placeholder identity there is another item or unknown"
                );
            }
            own
        })
        .collect()
}
