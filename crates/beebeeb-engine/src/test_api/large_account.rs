//! A large invented account for the fake server: Guus's size (about 40,000 files) with deterministic names, so the
//! native bench (`native/beebeeb-drive-rune/examples/bench_large.rs`) measures the views at the size that hurts.
//!
//! [`seed`] adds items to a [`FakeAccount`] (the account the fake server serves, `server::fixture_account`); the
//! names are encrypted by `add_item` like every other item. The same spec always makes the same ids and names.
//!
//! Layout (the counts are exact): `Camera Roll` holds `biggest_folder` photos and is the biggest folder; the other
//! photos are in one folder per month; `Backups/<device>/<dir>` holds the `backup_records` (files, none of them
//! photos); the rest of the files and folders are documents spread over `Projects` and the root.
//!
//! Must not: be reachable from a release build (this module is in `test_api`); hold a real name or a real key.

use super::fake_account::FakeAccount;

/// How big the account is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LargeAccount {
    /// Every file, photos and backup records included.
    pub files: usize,
    /// Every folder, the ones that hold the backup records included.
    pub folders: usize,
    /// The files that are pictures (a subset of `files`).
    pub photos: usize,
    /// The files under `Backups` (a subset of `files`, none of them a photo).
    pub backup_records: usize,
    /// The items directly in the biggest folder (`Camera Roll`; photos, so a grid has thumbnails to draw).
    pub biggest_folder: usize,
}

impl LargeAccount {
    /// Guus's account on 2026-11-20.
    pub const GUUS: LargeAccount = LargeAccount {
        files: 38_764,
        folders: 2_739,
        photos: 8_816,
        backup_records: 28_500,
        biggest_folder: 5_000,
    };

    /// Whether the spec can be laid out: the biggest folder is photos, and what is left over holds at least the
    /// folders that must exist.
    fn check(&self) {
        assert!(self.biggest_folder <= self.photos, "the biggest folder is photos");
        assert!(
            self.photos + self.backup_records <= self.files,
            "photos + backups fit the files"
        );
        assert!(
            self.folders >= FIXED_FOLDERS + 2,
            "room for the fixed folders and a backup folder"
        );
    }
}

/// `Camera Roll`, `Photos`... counted below: the folders every layout has (root ones, devices, `Projects`).
const FIXED_FOLDERS: usize = 1 /* Camera Roll */ + 1 /* Backups */ + DEVICES.len() + 1 /* Projects */;
const DEVICES: [&str; 3] = ["Laptop", "Desktop", "Père's Mac mini"];
/// How many month folders hold the photos that are not in the Camera Roll (at most; fewer if there are few photos).
const MONTHS: usize = 36;

const WORDS: [&str; 24] = [
    "Annual report",
    "Café receipts",
    "Überweisung",
    "naïve bayes notes",
    "São Paulo trip",
    "Ærø summer",
    "invoice",
    "budget",
    "draft",
    "Kraków plan",
    "meeting",
    "taxes",
    "contract",
    "scan",
    "backup",
    "notes",
    "résumé",
    "Zürich",
    "holiday",
    "design",
    "minutes",
    "final",
    "Åre skiing",
    "photos",
];
const FILE_EXT: [&str; 8] = ["pdf", "docx", "txt", "md", "xlsx", "zip", "csv", "pptx"];
const PHOTO_EXT: [&str; 4] = ["jpg", "jpeg", "heic", "png"];

/// A stable pseudo-random number for item `n` of stream `salt` (splitmix64).
fn mix(salt: u64, n: u64) -> u64 {
    let mut z = n
        .wrapping_add(salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A UUID of the shape the engine reads (`item_class` needs a valid one): `kind` and `n` make it unique.
fn id(kind: u16, n: usize) -> String {
    format!("b{:07x}-0025-4000-8000-{:012x}", kind, n)
}

/// A file name: two words and a number, an extension; every 97th is over 120 characters long.
fn file_name(kind: u64, n: usize, exts: &[&str]) -> String {
    let r = mix(kind, n as u64);
    let (a, b) = (WORDS[(r % 24) as usize], WORDS[((r >> 8) % 24) as usize]);
    let ext = exts[((r >> 16) % exts.len() as u64) as usize];
    if n % 97 == 96 {
        let long =
            format!("{a} - {b} - {a} - {b} - the long version with every page of the scanned original kept together");
        format!("{long} {n:06}.{ext}")
    } else {
        format!("{a} {b} {n:06}.{ext}")
    }
}

/// Add the account of `spec` to `account`. Exact counts: `spec.files` files and `spec.folders` folders (the account
/// is expected to be empty: a fixture account's seven sample items are on top of it).
pub fn seed(account: &FakeAccount, spec: &LargeAccount) {
    spec.check();
    const K_FOLDER: u16 = 1;
    const K_PHOTO: u16 = 2;
    const K_BACKUP: u16 = 3;
    const K_DOC: u16 = 4;
    let folders = std::cell::Cell::new(0usize);
    let folder = |name: &str, parent: Option<&str>| -> String {
        folders.set(folders.get() + 1);
        let fid = id(K_FOLDER, folders.get());
        account.add_item(&fid, parent, name, true, 0);
        fid
    };

    let roll = folder("Camera Roll", None);
    let backups = folder("Backups", None);
    let projects = folder("Projects", None);
    let devices: Vec<String> = DEVICES.iter().map(|d| folder(d, Some(&backups))).collect();

    // Photo folders: one per month for the photos outside the roll; the rest of the folder budget goes to the
    // backup directories and to project folders (a quarter of what is left).
    let outside = spec.photos - spec.biggest_folder;
    let months = if outside == 0 { 0 } else { MONTHS.min(1 + outside / 50) };
    let month_ids: Vec<String> = (0..months)
        .map(|m| folder(&format!("{}-{:02} Camera", 2021 + m / 12, 1 + m % 12), None))
        .collect();
    let left = spec.folders - folders.get();
    let project_dirs = left / 4;
    let backup_dirs = left - project_dirs;
    assert!(backup_dirs > 0, "at least one backup folder");
    let project_ids: Vec<String> = (0..project_dirs)
        .map(|n| folder(&format!("{} {}", WORDS[n % 24], n), Some(&projects)))
        .collect();
    let backup_ids: Vec<String> = (0..backup_dirs)
        .map(|n| {
            folder(
                &format!("{} {n}", WORDS[(n * 7) % 24]),
                Some(&devices[n % devices.len()]),
            )
        })
        .collect();
    assert_eq!(folders.get(), spec.folders);

    // Photos: the roll first, then the months.
    for n in 0..spec.photos {
        let parent = if n < spec.biggest_folder {
            &roll
        } else {
            &month_ids[(n - spec.biggest_folder) % months]
        };
        let name = format!("IMG_{n:05} {}", file_name(5, n, &PHOTO_EXT));
        account.add_item(
            &id(K_PHOTO, n),
            Some(parent),
            &name,
            false,
            1_500_000 + mix(6, n as u64) % 6_000_000,
        );
    }
    // Backup records, round-robin over the backup folders.
    for n in 0..spec.backup_records {
        let size = mix(7, n as u64) % 3_000_000;
        account.add_item(
            &id(K_BACKUP, n),
            Some(&backup_ids[n % backup_dirs]),
            &file_name(8, n, &FILE_EXT),
            false,
            size,
        );
    }
    // The rest: documents, a third at the root, the others in the project folders.
    let rest = spec.files - spec.photos - spec.backup_records;
    for n in 0..rest {
        let parent = if n % 3 == 0 || project_ids.is_empty() {
            None
        } else {
            Some(project_ids[n % project_ids.len()].as_str())
        };
        account.add_item(
            &id(K_DOC, n),
            parent,
            &file_name(9, n, &FILE_EXT),
            false,
            mix(10, n as u64) % 900_000,
        );
    }
}

/// The id of the biggest folder (`Camera Roll`), the first folder [`seed`] makes.
pub fn camera_roll_id() -> String {
    id(1, 1)
}

/// One item as this PC's local state holds it after a sync: what a bench writes into its `state.db` so the app's
/// Files window lists the account without a sync run.
#[derive(Debug, Clone)]
pub struct LocalRow {
    pub id: String,
    pub parent_id: Option<String>,
    /// `/Folder/Sub/name`, built from the names (the way the engine's rows are).
    pub path: String,
    pub is_folder: bool,
    pub size_bytes: u64,
    pub updated_at: i64,
}

/// Every live item of `account`, folders before files, each group by id: parents come first, so a writer can insert
/// the rows in this order.
pub fn local_rows(account: &FakeAccount) -> Vec<LocalRow> {
    let items = account.items.lock().expect("vault items");
    let mut paths: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let folders: Vec<_> = items.values().filter(|i| i.is_folder && !i.is_trashed).collect();
    fn path_of<'a>(
        items: &'a std::collections::HashMap<String, super::fake_account::VaultItem>,
        id: &'a str,
    ) -> String {
        let item = &items[id];
        match &item.parent_id {
            Some(parent) => format!("{}/{}", path_of(items, parent), item.name_plaintext),
            None => format!("/{}", item.name_plaintext),
        }
    }
    let mut rows = Vec::with_capacity(items.len());
    let mut folder_rows: Vec<&super::fake_account::VaultItem> = folders;
    folder_rows.sort_by_key(|i| (path_of(&items, &i.id).matches('/').count(), i.id.clone()));
    for i in folder_rows {
        let path = path_of(&items, &i.id);
        paths.insert(i.id.as_str(), path.clone());
        rows.push(LocalRow {
            id: i.id.clone(),
            parent_id: i.parent_id.clone(),
            path,
            is_folder: true,
            size_bytes: 0,
            updated_at: i.updated_at,
        });
    }
    let mut files: Vec<_> = items.values().filter(|i| !i.is_folder && !i.is_trashed).collect();
    files.sort_by(|a, b| a.id.cmp(&b.id));
    for i in files {
        let parent = i
            .parent_id
            .as_deref()
            .and_then(|p| paths.get(p).cloned())
            .unwrap_or_default();
        rows.push(LocalRow {
            id: i.id.clone(),
            parent_id: i.parent_id.clone(),
            path: format!("{parent}/{}", i.name_plaintext),
            is_folder: false,
            size_bytes: i.size_bytes,
            updated_at: i.updated_at,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::super::fake_account::TEST_PHRASE;
    use super::*;

    fn counts(account: &FakeAccount) -> (usize, usize, usize, usize, usize) {
        let items = account.items.lock().unwrap();
        let live = || items.values().filter(|i| !i.is_trashed);
        let files = live().filter(|i| !i.is_folder).count();
        let folders = live().filter(|i| i.is_folder).count();
        let photos = live()
            .filter(|i| !i.is_folder && i.name_plaintext.starts_with("IMG_"))
            .count();
        let backups = live()
            .filter(|i| !i.is_folder && i.id.starts_with(&format!("b{:07x}", 3)))
            .count();
        let mut per_parent = std::collections::HashMap::new();
        for i in live() {
            *per_parent.entry(i.parent_id.clone()).or_insert(0usize) += 1;
        }
        let biggest = per_parent
            .iter()
            .filter(|(p, _)| p.is_some())
            .map(|(_, n)| *n)
            .max()
            .unwrap_or(0);
        (files, folders, photos, backups, biggest)
    }

    #[test]
    fn large_account_seed_holds_the_counts() {
        let (account, _) = FakeAccount::register("large@example.invalid", "a large invented password", TEST_PHRASE);
        let spec = LargeAccount::GUUS;
        seed(&account, &spec);
        assert_eq!(
            counts(&account),
            (
                spec.files,
                spec.folders,
                spec.photos,
                spec.backup_records,
                spec.biggest_folder
            )
        );
        // Deterministic and with the hard names in it: long, accented, no duplicate ids by construction.
        let items = account.items.lock().unwrap();
        assert!(items.values().any(|i| i.name_plaintext.chars().count() > 100));
        assert!(items.values().any(|i| i.name_plaintext.contains('é')));
        assert!(
            items
                .values()
                .any(|i| i.name_plaintext.contains('ü') || i.name_plaintext.contains('Ü'))
        );
        assert_eq!(items.len(), spec.files + spec.folders);
    }

    #[test]
    fn large_account_small_spec_lays_out() {
        let (account, _) = FakeAccount::register("small@example.invalid", "a small invented password", TEST_PHRASE);
        let spec = LargeAccount {
            files: 300,
            folders: 20,
            photos: 80,
            backup_records: 150,
            biggest_folder: 40,
        };
        seed(&account, &spec);
        assert_eq!(counts(&account), (300, 20, 80, 150, 40));
        let rows = local_rows(&account);
        assert_eq!(rows.len(), 320);
        let mut seen = std::collections::HashSet::new();
        for row in &rows {
            assert!(
                row.parent_id.as_ref().is_none_or(|p| seen.contains(p)),
                "a parent comes first"
            );
            seen.insert(row.id.clone());
            assert!(row.path.starts_with('/'));
        }
        assert!(
            rows.iter()
                .any(|r| r.id == camera_roll_id() && r.path == "/Camera Roll")
        );
    }
}
