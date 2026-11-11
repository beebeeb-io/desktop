//! The board's samples this group needs of the Files window: a new account (DW-0114: no files, no
//! counts, 0 B of 1 TB) and the sync state without a connection (DW-0121). Everything else
//! answers as the Files fixture does, so "Add files" and a drop work on them.

use crate::frames::files::Sort;
use crate::frames::files::source::{
    Account, ActionError, Change, Counts, Details, FilesFixture, FilesSource, StorageSplit,
};
use crate::source::{Crumb, DriveItem, DriveSource, Listing, SourceError, StorageSummary};

/// The drive of a new account: an empty root, and no other folder.
pub struct EmptyDrive;

/// "0 B of 1 TB".
pub const STORAGE: StorageSummary = StorageSummary {
    used_bytes: 0,
    quota_bytes: 1_000_000_000_000,
};

impl DriveSource for EmptyDrive {
    fn list(&self, folder_id: Option<&str>) -> Result<Listing, SourceError> {
        if folder_id.is_some() {
            return Err(SourceError::NotFound);
        }
        let root = Crumb {
            id: None,
            name: "Files".into(),
        };
        Ok(Listing {
            folder: root.clone(),
            path: vec![root],
            items: Vec::new(),
        })
    }

    fn storage(&self) -> Option<StorageSummary> {
        Some(STORAGE)
    }
}

/// The Files fixture with what a board frame of this group says differently: a new account's
/// counts (DW-0114), the sync state's word without a connection (DW-0121). `None`: the fixture's.
#[derive(Default)]
pub struct SampleFiles {
    pub files: FilesFixture,
    pub counts: Option<Counts>,
    pub split: Option<StorageSplit>,
    pub sync: Option<String>,
}

impl SampleFiles {
    /// DW-0114: nothing shared, nothing in Trash, nothing kept; 0 B of files and of photos.
    pub fn new_account() -> Self {
        SampleFiles {
            counts: Some(Counts {
                shared_waiting: 0,
                trash: 0,
                available_offline: 0,
            }),
            split: Some(StorageSplit {
                files_bytes: 0,
                photos_bytes: 0,
            }),
            ..SampleFiles::default()
        }
    }

    /// DW-0121: the engine says there is no connection.
    pub fn offline() -> Self {
        SampleFiles {
            sync: Some(OFFLINE.to_string()),
            ..SampleFiles::default()
        }
    }
}

/// The sync state's word without a connection (`FilesSource::sync_state`).
pub const OFFLINE: &str = "Offline";

impl FilesSource for SampleFiles {
    fn counts(&self) -> Option<Counts> {
        self.counts.or_else(|| self.files.counts())
    }
    fn account(&self) -> Option<Account> {
        self.files.account()
    }
    fn storage_split(&self) -> Option<StorageSplit> {
        self.split.or_else(|| self.files.storage_split())
    }
    fn sync_state(&self) -> Option<String> {
        self.sync.clone().or_else(|| self.files.sync_state())
    }
    fn details(&self, item: &DriveItem, folder: &str) -> Details {
        self.files.details(item, folder)
    }
    fn changed_label(&self, item: &DriveItem) -> Option<String> {
        self.files.changed_label(item)
    }
    fn can_add(&self, folder: Option<&str>) -> bool {
        self.files.can_add(folder)
    }
    fn sort_of(&self, folder: Option<&str>) -> Option<Sort> {
        self.files.sort_of(folder)
    }
    fn set_sort(&mut self, folder: Option<&str>, sort: Sort) {
        self.files.set_sort(folder, sort);
    }
    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.files.trash(items)
    }
    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.files.restore(items)
    }
    fn duplicate(&mut self, items: &[DriveItem], taken: &[&str]) -> Result<(), ActionError> {
        self.files.duplicate(items, taken)
    }
    fn keep_offline(&mut self, items: &[DriveItem], keep: bool) -> Result<(), ActionError> {
        self.files.keep_offline(items, keep)
    }
    fn copy_link(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.files.copy_link(items)
    }
    fn open(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.files.open(items)
    }
    fn show_in_file_explorer(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.files.show_in_file_explorer(items)
    }
    fn upload(&mut self, folder: Option<&str>, files: &[std::path::PathBuf]) -> Result<(), ActionError> {
        self.files.upload(folder, files)
    }
    fn changes(&mut self) -> Vec<Change> {
        self.files.changes()
    }
}
