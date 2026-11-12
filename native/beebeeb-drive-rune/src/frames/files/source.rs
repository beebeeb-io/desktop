//! `FilesSource`: what the Files frames ask beyond a folder's listing (`crate::source::DriveSource`):
//! one item's facts for the details panel, and the actions of the file menu.
//!
//! Everything the window says that is not in the listing comes through this trait: the sidebar's
//! counts, the account, the storage split, the sync state. A source that does not know says `None`
//! and the window leaves the place empty; it never shows a sample in its place.
//!
//! An action is ASKED here and answered at once with "accepted" or why not. What it then changes
//! in the folder arrives later through [`FilesSource::changes`] (a copy that now exists, an action
//! that failed after all), so an engine never has to finish the work inside the call.
//!
//! `FilesFixture` holds exactly what the board draws (DW-0401, DW-0406, DW-1104) and records every
//! call, so a test can say which engine call an action made. `NoEngine` is the source of a window
//! with nothing behind it (`--db`): it knows nothing and refuses every action.
//! The engine's implementation is another lane's.

use crate::source::{DriveItem, format_size};
use crate::ui::file_list::format_date;

/// What the details panel shows of one item. `None`: the source does not know, and the row is left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    /// "PDF document"
    pub kind: String,
    /// "1.2 MB", or a folder's "48 items"
    pub size: String,
    pub changed: String,
    pub changed_on: Option<String>,
    pub added: Option<String>,
    pub where_: String,
    pub versions: Option<String>,
    pub shared: String,
    /// The Available offline card's second line: "Online only now · 1.2 MB".
    pub offline_note: String,
    pub kept_offline: bool,
    /// The preview is the file's own picture (a photo), not a page.
    pub picture: bool,
}

/// Why an action did not happen. The window says it in words (DW-0109), never as a code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    /// Backups and service folders cannot change (DW-0425).
    Protected,
    /// It needs a connection, and there is none.
    Offline,
    /// Nothing is connected that could do it (a window that only reads a database).
    Unavailable,
    /// It was tried and did not work.
    Failed,
    /// The folder already has an item with this name (DW-0410b): the source saw it, the window did not.
    NameTaken,
}

/// What a person asked for, in the words the failure message uses (DW-0109: "Couldn't rename.").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Trash,
    Restore,
    Duplicate,
    KeepOffline,
    RemoveOffline,
    CopyLink,
    Open,
    ShowInFileExplorer,
    Upload,
    /// Rename in the row (DW-0410).
    Rename,
    /// New folder (DW-0417).
    NewFolder,
    /// Move to another folder (DW-0409).
    Move,
    /// Free up space: the copy on this PC goes, the item stays in Beebeeb.
    FreeUpSpace,
    /// Open the folder shown in File Explorer (the toolbar's button).
    OpenInFileExplorer,
    /// Go back to an earlier version (DW-0413).
    RestoreVersion,
    /// Open in another app… (DW-0408b).
    OpenWith,
    /// Answer a file that changed in two places (DW-0429).
    KeepVersion,
    /// Invite someone… (DW-0407 to DW-0514).
    Invite,
    /// Pause all, Resume all (DW-0427).
    PauseUploads,
    /// Cancel upload (DW-0427).
    CancelUpload,
}

impl Action {
    /// "Couldn't <this>."
    pub fn words(self) -> &'static str {
        match self {
            Action::Trash => "move to Trash",
            Action::Restore => "put it back",
            Action::Duplicate => "duplicate",
            Action::KeepOffline => "keep on this PC",
            Action::RemoveOffline => "remove from this PC",
            Action::CopyLink => "copy the link",
            Action::Open => "open",
            Action::ShowInFileExplorer => "show in File Explorer",
            Action::Upload => "upload",
            Action::Rename => "rename",
            Action::NewFolder => "make the folder",
            Action::Move => "move",
            Action::FreeUpSpace => "free up space",
            Action::OpenInFileExplorer => "open File Explorer",
            Action::RestoreVersion => "go back to that version",
            Action::OpenWith => "open it in another app",
            Action::KeepVersion => "keep that version",
            Action::Invite => "invite someone to this",
            Action::PauseUploads => "pause the uploads",
            Action::CancelUpload => "cancel the upload",
        }
    }
}

/// One call an action made on the source, in the engine's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Trash(Vec<String>),
    Restore(Vec<String>),
    Duplicate(Vec<String>),
    KeepOffline(Vec<String>, bool),
    CopyLink(Vec<String>),
    Open(Vec<String>),
    ShowInFileExplorer(Vec<String>),
    /// Into the folder with this id (`None`: the root).
    Upload(Option<String>, Vec<std::path::PathBuf>),
    /// The folder with this id (`None`: the root) is now sorted like this.
    Sort(Option<String>, super::Sort),
    /// A folder with this name, in the folder with this id (`None`: the root).
    NewFolder(Option<String>, String),
    /// The item with this id gets this name.
    Rename(String, String),
    /// These go into the folder with this id (`None`: the root).
    Move(Vec<String>, Option<String>),
    /// The copies of these on this PC go; the items stay in Beebeeb.
    FreeUpSpace(Vec<String>),
    /// File Explorer opens on the folder with this id (`None`: the root of the sync folder).
    OpenInFileExplorer(Option<String>),
    /// A link to these, made with these options and put on the clipboard (DW-0407).
    ShareLink(Vec<String>, super::actions_data::ShareOptions),
    /// The file with this id goes back to the version with this id (DW-0413).
    RestoreVersion(String, String),
    /// Windows' app chooser for the file with this id (DW-0408b).
    OpenWith(String),
    /// The file with this id that changed in two places: what is kept (DW-0429).
    KeepVersion(String, super::actions_data::Keep),
    /// The upload with this id is tried again now (DW-0427).
    RetryUpload(String),
    /// The upload with this id is cancelled; nothing of it stays.
    CancelUpload(String),
    /// Uploads pause (`true`) or go on.
    PauseUploads(bool),
}

/// What an accepted action changed since the window last asked.
#[derive(Clone, Debug)]
pub enum Change {
    /// A copy now stands next to the item with the id `after` (DW-0406).
    Copied { after: String, item: Box<DriveItem> },
    /// A folder was made (DW-0417); `item.parent_id` says where.
    Made { item: Box<DriveItem> },
    /// An accepted action did not work after all.
    Failed(Action, ActionError),
}

/// The counts beside the sidebar's sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    /// Shares that wait for an answer.
    pub shared_waiting: u32,
    pub trash: u32,
    pub available_offline: u32,
}

/// Whose account this is: the sidebar's last row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub email: String,
    /// The avatar's letters.
    pub initials: String,
}

/// What the storage in use is made of: the meter's two parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageSplit {
    pub files_bytes: i64,
    pub photos_bytes: i64,
}

pub trait FilesSource: Send + 'static {
    /// `None`: not known (nothing is connected that counts them).
    fn counts(&self) -> Option<Counts>;
    fn account(&self) -> Option<Account>;
    fn storage_split(&self) -> Option<StorageSplit>;
    /// The status line's word for sync ("Up to date"). `None`: nothing syncs here.
    fn sync_state(&self) -> Option<String>;
    fn details(&self, item: &DriveItem, folder: &str) -> Details;
    /// A date the list shows in other words than its own ("Just now" for a copy made a moment ago).
    fn changed_label(&self, item: &DriveItem) -> Option<String>;
    /// May things be added to the folder with this id (`None`: the root)? Not to Backups, a service
    /// folder, Trash or Shared with me (DW-0414, DW-0481).
    fn can_add(&self, folder: Option<&str>) -> bool;
    /// The sort this folder was left with (DW-0404: "Each folder remembers its own sort").
    fn sort_of(&self, folder: Option<&str>) -> Option<super::Sort>;
    fn set_sort(&mut self, folder: Option<&str>, sort: super::Sort);
    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    /// Copies of these, each next to its original; they arrive through [`FilesSource::changes`].
    /// `taken`: the folder's names as the window shows them.
    fn duplicate(&mut self, items: &[DriveItem], taken: &[&str]) -> Result<(), ActionError>;
    fn keep_offline(&mut self, items: &[DriveItem], keep: bool) -> Result<(), ActionError>;
    /// Put the share link of these on the clipboard. `Ok`: it is there.
    fn copy_link(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    /// Open these files in their apps (downloaded and decrypted on this PC first).
    fn open(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    fn show_in_file_explorer(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    /// Upload these files of this PC into the folder with this id (`None`: the root).
    fn upload(&mut self, folder: Option<&str>, files: &[std::path::PathBuf]) -> Result<(), ActionError>;
    /// Make a folder with this name in the folder with this id (`None`: the root). It arrives
    /// through [`FilesSource::changes`] (`Change::Made`). The name is encrypted on this PC first.
    fn new_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), ActionError>;
    /// Give the item this name (already checked by `names::check_on` against the rows shown).
    fn rename(&mut self, item: &DriveItem, name: &str) -> Result<(), ActionError>;
    /// Move these into the folder with this id (`None`: the root). `NameTaken`: the folder has
    /// one of the names, and nothing was moved.
    fn move_to(&mut self, items: &[DriveItem], folder: Option<&str>) -> Result<(), ActionError>;
    /// Remove the copies of these from this PC; they stay in Beebeeb and download again when opened.
    fn free_up_space(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    /// Open the folder with this id (`None`: the root) in File Explorer. The engine maps the id
    /// to its path under the sync folder.
    fn open_in_file_explorer(&mut self, folder: Option<&str>) -> Result<(), ActionError>;
    /// What accepted actions changed since the last call, oldest first.
    fn changes(&mut self) -> Vec<Change>;

    // ---- The action dialogs (lane files-actions). Each has a default for a source that knows
    // nothing: no sample data in a build that ships, and every action refused. ----

    /// Make a link to these with these options and put it on the clipboard (DW-0407). The link
    /// itself is never handed to the window: it is not drawn anywhere.
    fn share_link(
        &mut self,
        _items: &[DriveItem],
        _options: &super::actions_data::ShareOptions,
    ) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// The id of the share people are invited to for this item, as the Shared section knows it
    /// (DW-0514). `None`: there is none to invite to.
    fn share_of(&self, _item: &DriveItem) -> Option<String> {
        None
    }
    /// The versions of a file, the current one first (DW-0413). Empty: not known.
    fn versions(&self, _item: &DriveItem) -> Vec<super::actions_data::Version> {
        Vec::new()
    }
    /// Make the version with this id the current one. No version is deleted by it.
    fn restore_version(&mut self, _item: &DriveItem, _version: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// What Preview shows of a file (DW-0408). `None`: no preview for it (DW-0408b).
    fn preview(&mut self, _item: &DriveItem) -> Option<super::actions_data::PreviewOf> {
        None
    }
    /// Windows' app chooser for this file; it is decrypted for the app picked (DW-0408b).
    fn open_with(&mut self, _item: &DriveItem) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// The files that changed in two places and wait for a choice, the first to answer first (DW-0429).
    fn conflicts(&self) -> Vec<super::actions_data::Conflict> {
        Vec::new()
    }
    /// Answer the conflict of the file with this id. Whatever is kept, no version is lost: the
    /// other one stays in the file's version history, or beside it (Keep both).
    fn keep_version(&mut self, _file: &str, _keep: super::actions_data::Keep) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// What uploads now: what stopped, what moves, what waits (DW-0426, DW-0427). `None`: nothing.
    fn uploads(&self) -> Option<super::uploads::Uploads> {
        None
    }
    /// Try the upload with this id again now.
    fn retry_upload(&mut self, _id: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Cancel the upload with this id; nothing is left half-uploaded.
    fn cancel_upload(&mut self, _id: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Pause every upload, or let them go on.
    fn pause_uploads(&mut self, _pause: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
}

fn ids(items: &[DriveItem]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

/// What a row itself says about the item, for a source that knows no more.
pub fn details_of_row(item: &DriveItem, folder: &str) -> Details {
    let size = if item.is_folder {
        item.child_count.map(|c| format!("{c} items")).unwrap_or_default()
    } else {
        item.size.map(format_size).unwrap_or_default()
    };
    let kept = item.pc_status.as_deref() == Some("available_offline");
    let place = match item.pc_status.as_deref() {
        Some("online_only") => "Online only now",
        Some("on_this_pc") => "Downloaded",
        Some("available_offline") => "Kept on this PC",
        _ => "",
    };
    let offline_note = match (place.is_empty(), size.is_empty()) {
        (false, false) => format!("{place} \u{00B7} {size}"),
        (false, true) => place.to_string(),
        (true, _) => size.clone(),
    };
    let shared = if item.shared_type.is_some() {
        "Anyone with the link".to_string()
    } else if item.shared_badges.is_empty() {
        "Only you".to_string()
    } else {
        item.shared_badges
            .iter()
            .map(|b| b.initials.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    Details {
        kind: kind_of(item).to_string(),
        size,
        changed: format_date(&item.modified_at),
        changed_on: None,
        added: None,
        where_: folder.to_string(),
        versions: None,
        shared,
        offline_note,
        kept_offline: kept,
        picture: item.has_thumbnail,
    }
}

pub fn kind_of(item: &DriveItem) -> &'static str {
    if item.is_folder {
        return "Folder";
    }
    match item.mime.as_deref() {
        Some("application/pdf") => "PDF document",
        Some(m) if m.contains("spreadsheet") => "Spreadsheet",
        Some(m) if m.contains("presentation") || m.contains("keynote") => "Presentation",
        Some(m) if m.starts_with("image/heic") || m.starts_with("image/heif") => "HEIC image",
        Some(m) if m.starts_with("image/") => "Image",
        Some(m) if m.starts_with("video/") => "Video",
        Some(m) if m.starts_with("audio/") => "Audio",
        Some(m) if m.contains("document") => "Document",
        _ => "File",
    }
}

/// "Contract draft.pdf" → "Contract draft copy.pdf"; taken → "Contract draft copy 2.pdf" (DW-0406).
pub fn copy_name(name: &str, taken: &[&str]) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let first = format!("{stem} copy{ext}");
    if !taken.contains(&first.as_str()) {
        return first;
    }
    (2..)
        .map(|n| format!("{stem} copy {n}{ext}"))
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .unwrap_or(first)
}

/// The source of a window with nothing behind it but a listing (`--db`): it knows no counts, no
/// account and no sync state, and refuses every action with [`ActionError::Unavailable`], so
/// nothing in the window claims a change that no engine made.
#[derive(Default)]
pub struct NoEngine;

impl FilesSource for NoEngine {
    fn counts(&self) -> Option<Counts> {
        None
    }
    fn account(&self) -> Option<Account> {
        None
    }
    fn storage_split(&self) -> Option<StorageSplit> {
        None
    }
    fn sync_state(&self) -> Option<String> {
        None
    }
    fn details(&self, item: &DriveItem, folder: &str) -> Details {
        details_of_row(item, folder)
    }
    fn changed_label(&self, _: &DriveItem) -> Option<String> {
        None
    }
    fn can_add(&self, _: Option<&str>) -> bool {
        false
    }
    fn sort_of(&self, _: Option<&str>) -> Option<super::Sort> {
        None
    }
    fn set_sort(&mut self, _: Option<&str>, _: super::Sort) {}
    fn trash(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn restore(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn duplicate(&mut self, _: &[DriveItem], _: &[&str]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn keep_offline(&mut self, _: &[DriveItem], _: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn copy_link(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn open(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn show_in_file_explorer(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn upload(&mut self, _: Option<&str>, _: &[std::path::PathBuf]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn new_folder(&mut self, _: Option<&str>, _: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn rename(&mut self, _: &DriveItem, _: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn move_to(&mut self, _: &[DriveItem], _: Option<&str>) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn free_up_space(&mut self, _: &[DriveItem]) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn open_in_file_explorer(&mut self, _: Option<&str>) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    fn changes(&mut self) -> Vec<Change> {
        Vec::new()
    }
}

/// The board's sample account and every action answered in memory. `fail` makes the next calls
/// of an action fail, so a test (and a board frame of a failure) can show what the window says.
#[derive(Default)]
pub struct FilesFixture {
    /// Every call, in order. Shared, so a test keeps a handle after the window owns the fixture.
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<Call>>>,
    /// How many items this session moved to Trash and did not take back.
    pub trashed: u32,
    /// Ids of the copies made in this session: the board shows them as changed "Just now" on "This PC".
    copies: Vec<String>,
    /// Actions that fail when asked, and why. Shared like `calls`.
    pub fail: std::sync::Arc<std::sync::Mutex<Vec<(Action, ActionError)>>>,
    /// Actions that are accepted and fail afterwards (the failure arrives through `changes`).
    pub fail_later: Vec<(Action, ActionError)>,
    sorts: std::collections::HashMap<Option<String>, super::Sort>,
    changes: Vec<Change>,
    /// What New folder, Rename and Move changed in the board's folders: shared with the
    /// [`BoardDrive`] that lists them, so a change is still there when the folder is shown again.
    pub edits: SharedEdits,
    /// What the action dialogs changed: versions, the files that wait for a choice.
    pub more: super::actions_data::More,
}

/// What the fixture's actions changed in the board's folders.
#[derive(Default)]
pub struct Edits {
    /// Folders made in this session.
    made: Vec<DriveItem>,
    /// New names, by id.
    names: std::collections::HashMap<String, String>,
    /// New parents, by id (`None`: the root).
    parents: std::collections::HashMap<String, Option<String>>,
}

pub type SharedEdits = std::sync::Arc<std::sync::Mutex<Edits>>;

fn edits_of(shared: &SharedEdits) -> std::sync::MutexGuard<'_, Edits> {
    shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The fixture's protected folder: the board's Backups row.
pub const BACKUPS_ID: &str = "f-backups";

impl FilesFixture {
    fn record(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(call);
    }

    fn refuse(&self, action: Action, items: &[DriveItem]) -> Result<(), ActionError> {
        let fail = self.fail.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&(_, why)) = fail.iter().find(|(a, _)| *a == action) {
            return Err(why);
        }
        let changes = matches!(
            action,
            Action::Trash
                | Action::Duplicate
                | Action::KeepOffline
                | Action::RemoveOffline
                | Action::CopyLink
                | Action::Rename
                | Action::Move
                | Action::FreeUpSpace
        );
        if changes && items.iter().any(|i| i.is_protected) {
            return Err(ActionError::Protected);
        }
        Ok(())
    }
}

impl FilesSource for FilesFixture {
    fn counts(&self) -> Option<Counts> {
        // The board's sample: 1 share waiting, 12 in Trash, 3 kept offline.
        Some(Counts {
            shared_waiting: 1,
            trash: 12 + self.trashed,
            available_offline: 3,
        })
    }

    fn account(&self) -> Option<Account> {
        Some(Account {
            name: "Anna Visser".into(),
            email: "anna@northwind.studio".into(),
            initials: "AV".into(),
        })
    }

    fn storage_split(&self) -> Option<StorageSplit> {
        // DW-0401: 240 GB of 1 TB: Files 24 GB, Photos 216 GB.
        Some(StorageSplit {
            files_bytes: 24_000_000_000,
            photos_bytes: 216_000_000_000,
        })
    }

    fn sync_state(&self) -> Option<String> {
        Some("Up to date".into())
    }

    fn details(&self, item: &DriveItem, folder: &str) -> Details {
        let row = details_of_row(item, folder);
        // The facts the board draws for the rows it selects; for any other row only what the row itself says.
        let (changed_on, added, versions, shared) = if self.copies.contains(&item.id) {
            ("This PC", "28 Oct 2026", "1 version", "Only you")
        } else {
            match item.id.as_str() {
                "contract" => ("Anna's iPhone", "28 Oct 2026", "3 versions", "Sam Okafor, pending"),
                "harbour" => ("Anna's iPhone", "28 Oct 2026", "1 version", "Anyone with the link"),
                _ => return row,
            }
        };
        Details {
            changed: self.changed_label(item).unwrap_or(row.changed.clone()),
            changed_on: Some(changed_on.to_string()),
            added: Some(added.to_string()),
            versions: Some(match self.more.version_count(&item.id) {
                Some(count) => super::actions_data::versions_words(count),
                None => versions.to_string(),
            }),
            shared: shared.to_string(),
            ..row
        }
    }

    fn changed_label(&self, item: &DriveItem) -> Option<String> {
        let made = edits_of(&self.edits).made.iter().any(|made| made.id == item.id);
        (made || self.copies.contains(&item.id)).then(|| "Just now".to_string())
    }

    fn can_add(&self, folder: Option<&str>) -> bool {
        folder != Some(BACKUPS_ID)
    }

    fn sort_of(&self, folder: Option<&str>) -> Option<super::Sort> {
        self.sorts.get(&folder.map(str::to_string)).copied()
    }

    fn set_sort(&mut self, folder: Option<&str>, sort: super::Sort) {
        let folder = folder.map(str::to_string);
        self.record(Call::Sort(folder.clone(), sort));
        self.sorts.insert(folder, sort);
    }

    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.refuse(Action::Trash, items)?;
        self.record(Call::Trash(ids(items)));
        self.trashed += items.len() as u32;
        Ok(())
    }

    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.refuse(Action::Restore, items)?;
        self.record(Call::Restore(ids(items)));
        self.trashed = self.trashed.saturating_sub(items.len() as u32);
        Ok(())
    }

    fn duplicate(&mut self, items: &[DriveItem], taken: &[&str]) -> Result<(), ActionError> {
        self.refuse(Action::Duplicate, items)?;
        self.record(Call::Duplicate(ids(items)));
        if let Some(&(action, why)) = self.fail_later.iter().find(|(a, _)| *a == Action::Duplicate) {
            self.changes.push(Change::Failed(action, why));
            return Ok(());
        }
        let mut taken: Vec<String> = taken.iter().map(|n| n.to_string()).collect();
        for item in items {
            let id = format!("{}-copy-{}", item.id, self.copies.len() + 1);
            self.copies.push(id.clone());
            let names: Vec<&str> = taken.iter().map(String::as_str).collect();
            let name = copy_name(&item.name, &names);
            taken.push(name.clone());
            self.changes.push(Change::Copied {
                after: item.id.clone(),
                item: Box::new(DriveItem {
                    id,
                    name,
                    // A new file: on this PC, its own versions start now, and it is not shared (DW-0406).
                    shared_type: None,
                    shared_badges: Vec::new(),
                    pc_status: Some("on_this_pc".into()),
                    ..item.clone()
                }),
            });
        }
        Ok(())
    }

    fn keep_offline(&mut self, items: &[DriveItem], keep: bool) -> Result<(), ActionError> {
        self.refuse(
            if keep {
                Action::KeepOffline
            } else {
                Action::RemoveOffline
            },
            items,
        )?;
        self.record(Call::KeepOffline(ids(items), keep));
        Ok(())
    }

    fn copy_link(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.refuse(Action::CopyLink, items)?;
        self.record(Call::CopyLink(ids(items)));
        Ok(())
    }

    fn open(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.refuse(Action::Open, items)?;
        self.record(Call::Open(ids(items)));
        Ok(())
    }

    fn show_in_file_explorer(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.refuse(Action::ShowInFileExplorer, items)?;
        self.record(Call::ShowInFileExplorer(ids(items)));
        Ok(())
    }

    fn upload(&mut self, folder: Option<&str>, files: &[std::path::PathBuf]) -> Result<(), ActionError> {
        self.refuse(Action::Upload, &[])?;
        if !self.can_add(folder) {
            return Err(ActionError::Protected);
        }
        self.record(Call::Upload(folder.map(str::to_string), files.to_vec()));
        // Something uploads now: the board's sample of it (DW-0426).
        self.more.uploads = Some(super::uploads::board_uploads());
        Ok(())
    }

    fn new_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), ActionError> {
        self.refuse(Action::NewFolder, &[])?;
        if !self.can_add(parent) {
            return Err(ActionError::Protected);
        }
        self.record(Call::NewFolder(parent.map(str::to_string), name.to_string()));
        if let Some(&(action, why)) = self.fail_later.iter().find(|(a, _)| *a == Action::NewFolder) {
            self.changes.push(Change::Failed(action, why));
            return Ok(());
        }
        let mut edits = edits_of(&self.edits);
        let item = DriveItem {
            // The first folder made is the one DW-1106 shows.
            id: match edits.made.len() {
                0 => NEW_FOLDER_ID.to_string(),
                n => format!("{NEW_FOLDER_ID}-{}", n + 1),
            },
            parent_id: parent.map(str::to_string),
            name: name.to_string(),
            is_folder: true,
            size: None,
            child_count: Some(0),
            modified_at: NOW.into(),
            mime: None,
            has_thumbnail: false,
            shared_type: None,
            shared_badges: Vec::new(),
            pc_status: Some("on_this_pc".into()),
            is_protected: false,
        };
        edits.made.push(item.clone());
        drop(edits);
        self.changes.push(Change::Made { item: Box::new(item) });
        Ok(())
    }

    fn rename(&mut self, item: &DriveItem, name: &str) -> Result<(), ActionError> {
        self.refuse(Action::Rename, std::slice::from_ref(item))?;
        self.record(Call::Rename(item.id.clone(), name.to_string()));
        let mut edits = edits_of(&self.edits);
        if let Some(made) = edits.made.iter_mut().find(|made| made.id == item.id) {
            made.name = name.to_string();
        }
        edits.names.insert(item.id.clone(), name.to_string());
        Ok(())
    }

    fn move_to(&mut self, items: &[DriveItem], folder: Option<&str>) -> Result<(), ActionError> {
        self.refuse(Action::Move, items)?;
        if !self.can_add(folder) {
            return Err(ActionError::Protected);
        }
        self.record(Call::Move(ids(items), folder.map(str::to_string)));
        let mut edits = edits_of(&self.edits);
        for item in items {
            if let Some(made) = edits.made.iter_mut().find(|made| made.id == item.id) {
                made.parent_id = folder.map(str::to_string);
            }
            edits.parents.insert(item.id.clone(), folder.map(str::to_string));
        }
        Ok(())
    }

    fn free_up_space(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.refuse(Action::FreeUpSpace, items)?;
        self.record(Call::FreeUpSpace(ids(items)));
        Ok(())
    }

    fn open_in_file_explorer(&mut self, folder: Option<&str>) -> Result<(), ActionError> {
        self.refuse(Action::OpenInFileExplorer, &[])?;
        self.record(Call::OpenInFileExplorer(folder.map(str::to_string)));
        Ok(())
    }

    fn share_link(
        &mut self,
        items: &[DriveItem],
        options: &super::actions_data::ShareOptions,
    ) -> Result<(), ActionError> {
        self.refuse(Action::CopyLink, items)?;
        self.record(Call::ShareLink(ids(items), options.clone()));
        Ok(())
    }

    fn share_of(&self, item: &DriveItem) -> Option<String> {
        // The Shared section's sample has one share with people: Contract draft.pdf's.
        (item.id == "contract").then(|| "p-contract".to_string())
    }

    fn versions(&self, item: &DriveItem) -> Vec<super::actions_data::Version> {
        self.more
            .versions(item, &self.details(item, ""), self.copies.contains(&item.id))
    }

    fn restore_version(&mut self, item: &DriveItem, version: &str) -> Result<(), ActionError> {
        self.refuse(Action::RestoreVersion, std::slice::from_ref(item))?;
        self.record(Call::RestoreVersion(item.id.clone(), version.to_string()));
        let all = self.versions(item);
        if self.more.restore(item, all, version) {
            Ok(())
        } else {
            Err(ActionError::Failed)
        }
    }

    fn preview(&mut self, item: &DriveItem) -> Option<super::actions_data::PreviewOf> {
        super::actions_data::More::preview(item)
    }

    fn open_with(&mut self, item: &DriveItem) -> Result<(), ActionError> {
        self.refuse(Action::OpenWith, std::slice::from_ref(item))?;
        self.record(Call::OpenWith(item.id.clone()));
        Ok(())
    }

    fn conflicts(&self) -> Vec<super::actions_data::Conflict> {
        self.more.conflicts.clone()
    }

    fn keep_version(&mut self, file: &str, keep: super::actions_data::Keep) -> Result<(), ActionError> {
        self.refuse(Action::KeepVersion, &[])?;
        let Some(conflict) = self.more.conflicts.iter().find(|c| c.file == file).cloned() else {
            return Err(ActionError::Failed);
        };
        self.record(Call::KeepVersion(file.to_string(), keep));
        let item = super::actions_data::More::conflict_item(&conflict);
        let all = self.versions(&item);
        if keep == super::actions_data::Keep::Both {
            // The other device's version becomes a file beside this one, its device's name added.
            self.changes.push(Change::Copied {
                after: file.to_string(),
                item: Box::new(DriveItem {
                    id: format!("{file}-from-other-device"),
                    name: super::actions_data::kept_beside_name(&conflict.name, &conflict.there.device),
                    ..item
                }),
            });
        }
        self.more.answered(&conflict, keep, all);
        Ok(())
    }

    fn uploads(&self) -> Option<super::uploads::Uploads> {
        self.more.uploads.clone()
    }

    fn retry_upload(&mut self, id: &str) -> Result<(), ActionError> {
        self.refuse(Action::Upload, &[])?;
        self.record(Call::RetryUpload(id.to_string()));
        // It waits its turn again.
        if let Some(upload) = self
            .more
            .uploads
            .as_mut()
            .and_then(|u| u.items.iter_mut().find(|u| u.id == id))
        {
            upload.state = super::uploads::UploadState::Waiting("Waiting to upload".to_string());
        }
        Ok(())
    }

    fn cancel_upload(&mut self, id: &str) -> Result<(), ActionError> {
        self.refuse(Action::CancelUpload, &[])?;
        self.record(Call::CancelUpload(id.to_string()));
        if let Some(uploads) = self.more.uploads.as_mut() {
            uploads.items.retain(|u| u.id != id);
        }
        Ok(())
    }

    fn pause_uploads(&mut self, pause: bool) -> Result<(), ActionError> {
        self.refuse(Action::PauseUploads, &[])?;
        self.record(Call::PauseUploads(pause));
        if let Some(uploads) = self.more.uploads.as_mut() {
            uploads.paused = pause;
        }
        Ok(())
    }

    fn changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }
}

/// When the fixture's session is: a folder made now reads "Just now" (`changed_label`).
const NOW: &str = "2026-10-28T12:00:00Z";

/// The board's folders: `BoardFixtureSource` holds the root (DW-0401); this honours the folder id.
/// Lisbon trip holds the seven rows of DW-0420, "New folder" nothing (DW-1106). A folder the board
/// never opens (Backups: "4 devices", Documents: "48 items") cannot be read here: it has things in
/// it that the board does not draw, so it shows neither invented rows nor "Nothing here yet".
/// An id that is no folder: `NotFound`.
pub struct BoardDrive {
    root: crate::source::BoardFixtureSource,
    /// What the fixture's actions changed (`FilesFixture::edits`).
    edits: SharedEdits,
}

/// The folder DW-0417 makes and DW-1106 shows.
pub const NEW_FOLDER_ID: &str = "f-new";

impl BoardDrive {
    pub fn new() -> Self {
        BoardDrive {
            root: crate::source::BoardFixtureSource::new(),
            edits: SharedEdits::default(),
        }
    }

    /// The board's folders, showing what this fixture's New folder, Rename and Move change.
    pub fn sharing(edits: &SharedEdits) -> Self {
        BoardDrive {
            root: crate::source::BoardFixtureSource::new(),
            edits: edits.clone(),
        }
    }

    /// The board's rows of one folder, before any change: the root's, Lisbon trip's.
    fn drawn(&self, folder: Option<&str>) -> Option<Vec<DriveItem>> {
        match folder {
            None => crate::source::DriveSource::list(&self.root, None)
                .ok()
                .map(|listing| listing.items),
            Some("f-lisbon") => Some(lisbon_trip("f-lisbon")),
            _ => None,
        }
    }
}

impl Default for BoardDrive {
    fn default() -> Self {
        Self::new()
    }
}

fn lisbon_trip(parent: &str) -> Vec<DriveItem> {
    let item = |id: &str, name: &str, modified: &str| DriveItem {
        id: id.into(),
        parent_id: Some(parent.into()),
        name: name.into(),
        is_folder: false,
        size: None,
        child_count: None,
        modified_at: modified.into(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: Some("online_only".into()),
        is_protected: false,
    };
    let folder = |id: &str, name: &str, modified: &str, count: i64| DriveItem {
        is_folder: true,
        child_count: Some(count),
        ..item(id, name, modified)
    };
    let file = |id: &str, name: &str, modified: &str, size: i64, mime: &str| DriveItem {
        size: Some(size),
        mime: Some(mime.into()),
        has_thumbnail: mime.starts_with("image/") || mime.starts_with("video/"),
        ..item(id, name, modified)
    };
    const MB: f64 = 1024.0 * 1024.0;
    let mb = |n: f64| (n * MB) as i64;
    vec![
        folder("f-day1", "Day 1", "2026-10-09T10:00:00Z", 48),
        folder("f-day2", "Day 2", "2026-10-10T10:00:00Z", 61),
        DriveItem {
            pc_status: Some("on_this_pc".into()),
            ..file(
                "tickets",
                "Tickets.pdf",
                "2026-10-08T10:00:00Z",
                220 * 1024,
                "application/pdf",
            )
        },
        file("alfama", "Alfama.heic", "2026-10-09T10:00:00Z", mb(3.4), "image/heic"),
        file("belem", "Belem.heic", "2026-10-09T10:00:00Z", mb(4.1), "image/heic"),
        file(
            "tram28",
            "Tram 28.mov",
            "2026-10-10T10:00:00Z",
            mb(82.0),
            "video/quicktime",
        ),
        file("sunset", "Sunset.heic", "2026-10-10T10:00:00Z", mb(3.8), "image/heic"),
    ]
}

impl crate::source::DriveSource for BoardDrive {
    fn list(&self, folder_id: Option<&str>) -> Result<crate::source::Listing, crate::source::SourceError> {
        use crate::source::{Crumb, Listing, SourceError};
        let root = self.root.list(None)?;
        let edits = edits_of(&self.edits);
        // Every item this fixture knows, where the board drew it.
        let mut known: Vec<DriveItem> = root.items.clone();
        known.extend(lisbon_trip("f-lisbon"));
        known.extend(edits.made.iter().cloned());
        for item in &mut known {
            if let Some(name) = edits.names.get(&item.id) {
                item.name = name.clone();
            }
            if let Some(parent) = edits.parents.get(&item.id) {
                item.parent_id = parent.clone();
            }
        }
        let made = |id: &str| edits.made.iter().any(|item| item.id == id);
        let folder = match folder_id {
            None => root.folder.clone(),
            // The folder DW-1106 shows, before anything made it.
            Some(NEW_FOLDER_ID) if !made(NEW_FOLDER_ID) => Crumb {
                id: Some(NEW_FOLDER_ID.to_string()),
                name: "New folder".to_string(),
            },
            Some(id) => known
                .iter()
                .find(|i| i.id == id && i.is_folder)
                .map(|i| Crumb {
                    id: Some(id.to_string()),
                    name: i.name.clone(),
                })
                .ok_or(SourceError::NotFound)?,
        };
        // A folder the board never opens has things in it that the board does not draw: it
        // cannot be read here, whatever was moved into it. A folder made here starts empty.
        let drawn = match folder_id {
            Some(id) if id == NEW_FOLDER_ID || made(id) => true,
            other => self.drawn(other).is_some(),
        };
        if !drawn {
            return Err(SourceError::Unreadable);
        }
        // The board's own order: its rows where it drew them, then what came in.
        let here = |item: &&DriveItem| item.parent_id.as_deref() == folder_id;
        let items: Vec<DriveItem> = known.iter().filter(here).cloned().collect();
        // The way up: every folder above this one.
        let mut path = vec![folder.clone()];
        let mut at = folder_id.map(str::to_string);
        while let Some(id) = at {
            let parent = known.iter().find(|i| i.id == id).and_then(|i| i.parent_id.clone());
            path.push(match &parent {
                None => root.folder.clone(),
                Some(up) => Crumb {
                    id: Some(up.clone()),
                    name: known
                        .iter()
                        .find(|i| &i.id == up)
                        .map(|i| i.name.clone())
                        .unwrap_or_default(),
                },
            });
            at = parent;
        }
        path.reverse();
        Ok(Listing { path, folder, items })
    }

    fn storage(&self) -> Option<crate::source::StorageSummary> {
        self.root.storage()
    }
}
