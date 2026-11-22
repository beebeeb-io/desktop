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

pub use super::actions_data::Read;
use crate::source::DriveItem;

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

/// Where a file went that has no preview in the app (`frames::open_file`, steps 2 and 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outside {
    /// The app the system has for its kind.
    DefaultApp,
    /// The system has none: its file manager shows the file selected in its folder.
    Explorer,
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
    /// A change of one of these is still on its way: nothing was sent a second time.
    InFlight,
    /// Some of the items changed, some did not (the rows show which).
    NotAll,
    /// Free up space left it: it has a change that is not uploaded yet (its bytes here are the only copy).
    NotSent,
    /// Free up space left it: something in it is kept on this PC. Asking again changes nothing.
    HoldsKept,
    /// This system cannot do it at all (Free up space for a selection, off Windows): nothing was changed, and
    /// Try again would change nothing either.
    NotOnThisSystem,
    /// This app cannot do it yet (a link to a folder, to several items, a view-only link): nothing was sent.
    NotYet,
    /// Copy link left it: the item is a folder, and only a file has a link here. Nothing was sent, and Try again
    /// would change nothing.
    NotAFile,
    /// Copy link left it: the file already has a link, and that link cannot be copied again yet. No second link
    /// was made, and Try again would change nothing.
    HasLink,
    /// Cancel upload left it: its copy (the only one: it was never uploaded) cannot go to the Recycle Bin (none on
    /// its drive, too big for it, or the move failed), and it is never deleted for good. The file and its upload
    /// stay as they were.
    NotToRecycleBin,
    /// New file left it: the disk of this PC has no room for it. Nothing was made; asking again after making room
    /// works.
    NoRoom,
    /// New file left it: more than a new file is made with here. Nothing was made, and Try again would change
    /// nothing.
    TooLarge,
    /// Another item here has the very same name, and this PC holds one entry for the two: the press would reach the
    /// other item's file, so nothing was done. Asking again changes nothing; renaming one on the web does.
    NameShared,
    /// Another item here has the same name but for capital letters, and this PC (which does not tell them apart)
    /// holds one entry for the two: the press would reach the other item's file, so nothing was done.
    NameSharedButCase,
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
    /// A link with choices (the Share dialog), as distinct from the plain Copy link.
    ShareLink,
    /// Share… that was refused before its dialog opened (a protected item): said, with no Try
    /// again, because there is nothing to repeat but the plain link nobody asked for.
    Share,
    Open,
    ShowInFileExplorer,
    Upload,
    /// Rename in the row (DW-0410).
    Rename,
    /// New folder (DW-0417).
    NewFolder,
    /// Create file in the New file dialog (DW-1522).
    NewFile,
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
    /// Try again on ONE upload, from the Uploads panel (as distinct from a drop that failed).
    RetryUpload,
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
            Action::ShareLink => "make the link",
            Action::Share => "share this",
            Action::Open => "open",
            Action::ShowInFileExplorer => "show in File Explorer",
            Action::Upload => "upload",
            Action::Rename => "rename",
            Action::NewFolder => "make the folder",
            Action::NewFile => "make the file",
            Action::Move => "move",
            Action::FreeUpSpace => "free up space",
            Action::OpenInFileExplorer => "open File Explorer",
            Action::RestoreVersion => "go back to that version",
            Action::OpenWith => "open it in another app",
            Action::KeepVersion => "keep that version",
            Action::Invite => "invite someone to this",
            Action::PauseUploads => "pause the uploads",
            Action::RetryUpload => "try that upload again",
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
    #[allow(dead_code)]
    Open(Vec<String>),
    ShowInFileExplorer(Vec<String>),
    /// Into the folder with this id (`None`: the root).
    Upload(Option<String>, Vec<std::path::PathBuf>),
    /// The folder with this id (`None`: the root) is now sorted like this.
    Sort(Option<String>, super::Sort),
    /// A folder with this name, in the folder with this id (`None`: the root).
    NewFolder(Option<String>, String),
    /// An empty file with this name, in the folder with this id (`None`: the root).
    NewFile(Option<String>, String),
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
    /// The file with this id went outside the app, to the app the system has for its kind (`open_file` step 2).
    OpenDefault(String),
    /// The file with this id went outside the app, to the file manager with it selected (`open_file` step 3).
    OpenRevealed(String),
    /// The same for a path that is not an item of the drive.
    OpenPathDefault(std::path::PathBuf),
    OpenPathRevealed(std::path::PathBuf),
    /// The file with this id (its copy kept on this PC) opened in the app with this id (DW-1562).
    OpenInApp(String, String),
    /// The file with this id that changed in two places: what is kept (DW-0429).
    KeepVersion(String, super::actions_data::Keep),
    /// The upload with this id is tried again now (DW-0427).
    RetryUpload(String),
    /// The account left: what was uploading is cancelled and forgotten.
    ForgetUploads,
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
    /// A change of what this PC holds (`KeepOffline`, `RemoveOffline`, `FreeUpSpace`) was tried: per item asked,
    /// what the engine says of it now and whether the change was made. Never inferred from the action.
    Local { action: Action, items: Vec<LocalOutcome> },
    /// A move to Trash (`Action::Trash`) or its Undo (`Action::Restore`) was tried: per item asked, whether
    /// Beebeeb made it. Only from a source that answers later.
    Trash { action: Action, items: Vec<TrashOutcome> },
    /// The upload with this id was cancelled (copy in the Recycle Bin, change dropped).
    Cancelled { id: String },
    /// A link (`Action::CopyLink`, `Action::ShareLink`) was asked for: the link Beebeeb accepted, for the
    /// clipboard, or why there is none. Only from a source that answers later.
    Link {
        action: Action,
        answer: Result<super::link::LinkText, ActionError>,
    },
}

pub use super::outcomes::{LocalOutcome, TrashOutcome};

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

/// An app Windows reports for a type, as Open With lists it (DW-1562).
pub use super::open_with_menu::AppChoice;

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
    /// Put the share link of these on the clipboard. `Ok`: it is there, or (a source that answers later)
    /// accepted: the link arrives as `Change::Link` and the WINDOW copies it (only a handler's `Tx` can).
    fn copy_link(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    /// Open these files in their apps (downloaded and decrypted on this PC first).
    #[allow(dead_code)]
    fn open(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    fn show_in_file_explorer(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    /// Upload these files of this PC into the folder with this id (`None`: the root).
    fn upload(&mut self, folder: Option<&str>, files: &[std::path::PathBuf]) -> Result<(), ActionError>;
    /// Make a folder with this name in the folder with this id (`None`: the root). It arrives
    /// through [`FilesSource::changes`] (`Change::Made`). The name is encrypted on this PC first.
    fn new_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), ActionError>;
    /// Make an empty file with this name in the folder with this id (`None`: the root): New file's Create file
    /// (DW-1522). It uploads as any new file there, encrypted on this PC first; a taken name is refused, never
    /// overwritten. Default: a source that cannot make one.
    fn new_file(&mut self, _folder: Option<&str>, _name: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
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
    /// Does this source answer `keep_offline`, `free_up_space`, `trash`, `restore`, `copy_link` and `share_link`
    /// LATER (`Ok` is "accepted"; what happened arrives through [`FilesSource::changes`])? Then the window draws
    /// nothing of the change before that, and asks `changes` every frame. Default: answered in the call.
    fn answers_later(&self) -> bool {
        false
    }
    /// Wake the window once more after this long: a message set between presses goes at its own time. Only a
    /// source that answers later needs it (the window has no other wake then).
    fn wake_after(&mut self, _after: std::time::Duration) {}
    /// After an `upload` that was refused: the files of it that were NOT copied in, so Try again sends those
    /// alone (an empty list: every file is in, there is nothing to try again). `None`: nothing of it was copied
    /// (or the source cannot say).
    fn upload_left(&mut self) -> Option<Vec<std::path::PathBuf>> {
        None
    }

    // ---- The action dialogs (lane files-actions). Each has a default for a source that knows
    // nothing: no sample data in a build that ships, and every action refused. ----

    /// Is an accepted link still unanswered? `false`: its answer was dropped, and the window forgets the press.
    fn link_pending(&self) -> bool {
        true
    }
    /// Make a link to these with these options and put it on the clipboard (DW-0407). The link is never drawn;
    /// a source that answers later hands it over once, in `Change::Link`, for the clipboard only.
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
    /// The item with this id, as this account's local state holds it. `None`: not known, or not this account's.
    fn item_of(&self, _id: &str) -> Option<DriveItem> {
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
    /// The Versions or the Details dialog of this FILE opened: read its versions if what is held is old.
    fn ask_versions(&mut self, _item: &DriveItem) {}
    /// Where the read of a file's versions stands (what [`FilesSource::versions`] is empty for).
    fn versions_state(&self, _item: &DriveItem) -> Read {
        Read::Answered
    }
    /// What Preview shows of a file (DW-0408). `None`: no preview for it (DW-0408b).
    fn preview(&mut self, _item: &DriveItem) -> Option<super::actions_data::PreviewOf> {
        None
    }
    /// Where the preview of `item` stands, for a source that answers `preview` later (`Shown` is handed over once).
    fn preview_later(&mut self, _item: &DriveItem) -> Option<super::actions_data::Previewed> {
        None
    }
    /// The panel closed or moved to another file: stop the read, wipe what it decrypted.
    fn cancel_preview(&mut self) {}
    /// Windows' app chooser for this file; it is decrypted for the app picked (DW-0408b).
    fn open_with(&mut self, _item: &DriveItem) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Steps 2 and 3 of `frames::open_file`: the file (its copy on this PC) with the app the system has for its
    /// kind, else the system's file manager with the file selected. Only `open_file` calls it, after the in-app
    /// preview was ruled out. Never decrypts or copies anything for the app.
    fn open_outside(&mut self, _item: &DriveItem) -> Result<Outside, ActionError> {
        Err(ActionError::Unavailable)
    }
    /// [`FilesSource::open_outside`] for a file that is not an item of the drive (the licence notice beside the exe).
    fn open_path_outside(&mut self, _path: &std::path::Path) -> Result<Outside, ActionError> {
        Err(ActionError::Unavailable)
    }
    /// The apps Windows reports for this file's type (DW-1562), at most 8, never Beebeeb. Called on the window's thread
    /// while the menu is drawn: never waits (what is held; asked once in the background). Empty: none known.
    fn apps_for(&self, _item: &DriveItem) -> Vec<AppChoice> {
        Vec::new()
    }
    /// Open the copy of this file kept on this PC in `app` (of `apps_for`). Nothing is fetched, decrypted or copied.
    fn open_in_app(&mut self, _item: &DriveItem, _app: &AppChoice) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Is this file kept on this PC AND are its bytes here, now? Open With hands only such a copy to another app. The
    /// engine answers from its own facts; the default, for a source with no engine, is the row's mark.
    fn kept_here(&self, item: &DriveItem) -> bool {
        item.pc_status.as_deref() == Some("available_offline")
    }
    /// A session ended (a lock, a sign-out, another account): what it asked to start and has not begun goes.
    fn forget_starts(&mut self) {}
    /// The files that changed in two places and wait for a choice, the first to answer first (DW-0429).
    fn conflicts(&self) -> Vec<super::actions_data::Conflict> {
        Vec::new()
    }
    /// Answer the conflict of the file with this id. Whatever is kept, no version is lost: the
    /// other one stays in the file's version history, or beside it (Keep both).
    fn keep_version(&mut self, _file: &str, _keep: super::actions_data::Keep) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// The copies coming down now, each with how far it has come (DW-0412). Empty: none is, or this
    /// source cannot tell.
    fn downloads(&self) -> Vec<super::keeping::Download> {
        Vec::new()
    }
    /// The account left (a sign-out, another account): forget what was read of the copies coming down.
    fn forget_downloads(&mut self) {}
    /// What uploads now: what stopped, what moves, what waits (DW-0426, DW-0427). `None`: nothing.
    fn uploads(&self) -> Option<super::uploads::Uploads> {
        None
    }
    /// Try the upload with this id again now.
    fn retry_upload(&mut self, _id: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// Cancel the upload with this id (a NEW file only): the copy goes to the Recycle Bin, the change is dropped.
    /// A source that answers later says what it did as [`Change::Cancelled`].
    fn cancel_upload(&mut self, _id: &str) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// The Uploads panel opened: read again if what is held is old.
    fn ask_uploads(&mut self) {}
    /// Can this source cancel THIS upload? `false` (the default): the row menu does not offer it.
    fn cancels_upload(&self, _id: &str) -> bool {
        false
    }
    /// The account that was uploading left: forget what is shown of its uploads. Unsent changes are the
    /// engine's sign-out's, never this call's. Default: a source that keeps no uploads.
    fn forget_uploads(&mut self) {}
    /// Pause every upload, or let them go on.
    fn pause_uploads(&mut self, _pause: bool) -> Result<(), ActionError> {
        Err(ActionError::Unavailable)
    }
    /// The id of the Backups folder, which the sidebar's Backups shows (DW-0422). `None`: this
    /// source knows none, and the section stays on the shell's "not built yet" frame.
    fn backups_root(&self) -> Option<String> {
        None
    }
    /// The sidebar's Backups has no folder to show (`backups_root` is `None`): the source may look again, now and
    /// then, in case the folder came to be. Called on the window's thread: it must not wait.
    fn look_for_backups(&mut self) {}
    /// The window now shows this folder (`None`: the top of Files): read what the source needs for it.
    fn folder_shown(&mut self, _folder: Option<&str>) {}
    /// Why `item` (with no item: anything in `folder`) cannot be moved, renamed or deleted, with
    /// the core's words for it (DW-0425). `None`: it can.
    fn protection(&self, _folder: Option<&str>, _item: Option<&DriveItem>) -> Option<super::protected::Protection> {
        None
    }
    /// What a row of a protected folder is beyond a folder: a device, a service (DW-0422).
    fn backup_entry(&self, _item: &DriveItem) -> Option<super::protected::BackupEntry> {
        None
    }
    /// The one-time "the local file list was rebuilt" words, once, after the engine moved a damaged state.db aside
    /// (144b); `None` every call after, and for a source with no engine.
    fn take_rebuilt_notice(&mut self) -> Option<String> {
        None
    }
    /// The note over a folder a service manages (DW-0424).
    fn folder_notice(&self, _folder: Option<&str>) -> Option<super::protected::FolderNotice> {
        None
    }
}

fn ids(items: &[DriveItem]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

pub use super::row_facts::{details_of_row, kind_of};

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
    /// The apps Open With lists (DW-1562). `None`: the board's (Microsoft Word for a Word file).
    pub apps: Option<Vec<AppChoice>>,
    /// Ids the engine would say are not here yet (kept, still coming down): `kept_here` is false whatever the row says.
    pub not_here: Vec<String>,
    /// Whether the system has an app for the kind of file asked (`open_outside`): a test turns it off.
    pub no_default_app: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// How often a session end told it to forget its starts. Shared like `calls` (and kept out of them).
    pub starts_forgotten: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// What New folder, Rename and Move changed in the board's folders: shared with the
    /// [`BoardDrive`] that lists them, so a change is still there when the folder is shown again.
    pub edits: SharedEdits,
    /// What the action dialogs changed: versions, the files that wait for a choice.
    pub more: super::actions_data::More,
    /// A source that would accept a change of a protected item: so a test can show that the
    /// WINDOW refuses it before it asks (`protected_tests.rs`).
    pub lenient: bool,
}

/// What the fixture's actions changed in the board's folders.
#[derive(Default)]
pub struct Edits {
    /// Folders made in this session.
    pub(super) made: Vec<DriveItem>,
    /// New names, by id.
    pub(super) names: std::collections::HashMap<String, String>,
    /// New parents, by id (`None`: the root).
    pub(super) parents: std::collections::HashMap<String, Option<String>>,
}

pub type SharedEdits = std::sync::Arc<std::sync::Mutex<Edits>>;

pub(super) fn edits_of(shared: &SharedEdits) -> std::sync::MutexGuard<'_, Edits> {
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
                | Action::ShareLink
                | Action::Rename
                | Action::Move
                | Action::FreeUpSpace
        );
        if self.lenient {
            return Ok(());
        }
        if changes && items.iter().any(|i| i.is_protected) {
            return Err(ActionError::Protected);
        }
        // Nothing in the protected tree is moved, renamed, deleted, copied or shared (DW-0425).
        let alters = matches!(
            action,
            Action::Trash | Action::Duplicate | Action::CopyLink | Action::Rename | Action::Move
        );
        if alters
            && items
                .iter()
                .any(|i| super::protected_fixture::protection(None, Some(i)).is_some())
        {
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
        Some(super::keeping::sync_word(&self.more.downloads))
    }

    fn downloads(&self) -> Vec<super::keeping::Download> {
        self.more.downloads.clone()
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
        self.lenient || (folder != Some(BACKUPS_ID) && !super::protected_fixture::holds(folder))
    }

    fn backups_root(&self) -> Option<String> {
        Some(BACKUPS_ID.to_string())
    }

    fn protection(&self, folder: Option<&str>, item: Option<&DriveItem>) -> Option<super::protected::Protection> {
        super::protected_fixture::protection(folder, item)
    }

    fn backup_entry(&self, item: &DriveItem) -> Option<super::protected::BackupEntry> {
        super::protected_fixture::entry(item)
    }

    fn folder_notice(&self, folder: Option<&str>) -> Option<super::protected::FolderNotice> {
        super::protected_fixture::notice(folder)
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

    #[allow(dead_code)]
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

    fn new_file(&mut self, folder: Option<&str>, name: &str) -> Result<(), ActionError> {
        self.refuse(Action::NewFile, &[])?;
        if !self.can_add(folder) {
            return Err(ActionError::Protected);
        }
        self.record(Call::NewFile(folder.map(str::to_string), name.to_string()));
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
        self.refuse(Action::ShareLink, items)?;
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

    fn open_outside(&mut self, item: &DriveItem) -> Result<Outside, ActionError> {
        self.refuse(Action::Open, std::slice::from_ref(item))?;
        if self.no_default_app.load(std::sync::atomic::Ordering::SeqCst) {
            self.record(Call::OpenRevealed(item.id.clone()));
            Ok(Outside::Explorer)
        } else {
            self.record(Call::OpenDefault(item.id.clone()));
            Ok(Outside::DefaultApp)
        }
    }

    fn open_path_outside(&mut self, path: &std::path::Path) -> Result<Outside, ActionError> {
        if self.no_default_app.load(std::sync::atomic::Ordering::SeqCst) {
            self.record(Call::OpenPathRevealed(path.to_path_buf()));
            Ok(Outside::Explorer)
        } else {
            self.record(Call::OpenPathDefault(path.to_path_buf()));
            Ok(Outside::DefaultApp)
        }
    }

    fn open_with(&mut self, item: &DriveItem) -> Result<(), ActionError> {
        self.refuse(Action::OpenWith, std::slice::from_ref(item))?;
        self.record(Call::OpenWith(item.id.clone()));
        Ok(())
    }

    fn apps_for(&self, item: &DriveItem) -> Vec<AppChoice> {
        match &self.apps {
            Some(apps) => apps.clone(),
            None => super::open_with_menu::board_apps(&item.name),
        }
    }

    fn open_in_app(&mut self, item: &DriveItem, app: &AppChoice) -> Result<(), ActionError> {
        self.refuse(Action::OpenWith, std::slice::from_ref(item))?;
        self.record(Call::OpenInApp(item.id.clone(), app.id.clone()));
        Ok(())
    }

    fn kept_here(&self, item: &DriveItem) -> bool {
        !self.not_here.contains(&item.id) && item.pc_status.as_deref() == Some("available_offline")
    }

    fn forget_starts(&mut self) {
        self.starts_forgotten.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

    fn forget_uploads(&mut self) {
        self.record(Call::ForgetUploads);
        self.more.uploads = None;
    }

    fn retry_upload(&mut self, id: &str) -> Result<(), ActionError> {
        self.refuse(Action::RetryUpload, &[])?;
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

    fn cancels_upload(&self, _id: &str) -> bool {
        true
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

pub use super::board_drive::{BoardDrive, NEW_FOLDER_ID};
