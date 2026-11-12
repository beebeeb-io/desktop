//! LD-0113 for the app's own window: what the person may do to a row at or under the Backups root, and the words
//! of a refusal.
//!
//! The window is an APP surface, so the rule is the web app's (Guus: "make sure that Backups folder is protected,
//! same as on web"): core's `check_app_mutation` on the row's [`BackupPosition`], plus the two things the web adds
//! around it (a folder at or under the root is never shared; a device's folder or one backed-up folder is deleted
//! through its own password-confirmed call). One thing the web cannot know and this device can: a row that is this
//! device's own backup of a folder it backs up now answers core's `own_backup_refusal`, as it does in Explorer
//! (a change there only comes back on the next pass). [`crate::backup_protect`] is the DRIVE surface's rule
//! (Explorer, Finder): do not call it for the window, its words send the person "to the app".
//!
//! Must not: queue, delete or call the network (the UI's action does that after [`check`] allowed it), log a
//! refusal's words (the own-backup words carry a path on this device), or decide what the server enforces.

use beebeeb_core::backup::{BackupPosition, MSG_READ_ONLY, Mutation, check_app_mutation, own_backup_refusal};

use crate::backup_protect::{BackupsTree, OwnBackups};
use crate::state_db::StateDb;

/// The server's words for a folder share or link in the Backups tree (LD-0113 Amendment 10), as the web app
/// shows them (`web/src/lib/backups-root.ts` `BACKUPS_NO_FOLDER_SHARE`). Not in core yet: keep the two equal.
pub const MSG_NO_FOLDER_SHARE: &str = "Folders in Backups can't be shared. You can share a single file.";

/// What the password dialog of [`Action::DeleteBackup`] says (the web's `BACKUPS_STEP_UP`).
pub const MSG_DELETE_BACKUP_STEP_UP: &str = "Confirm it's you to delete this backup.";

/// A row's place in the Backups tree, on a listing item. `Default` is a row outside every tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupMark {
    pub position: BackupPosition,
    /// The Backups root itself: the row with the lock and the "Read-only" chip (`MSG_BADGE`), as on the web.
    pub is_protected: bool,
}

impl Default for BackupMark {
    fn default() -> Self {
        Self::at(BackupPosition::Outside)
    }
}

impl BackupMark {
    pub fn at(position: BackupPosition) -> Self {
        Self {
            position,
            is_protected: position == BackupPosition::Root,
        }
    }
}

/// Everything the window's menus and drops can ask about one row. For [`Action::MoveInto`], [`Action::UploadInto`]
/// and [`Action::NewFolderIn`] the row is the DESTINATION folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Trash,
    DeleteForever,
    Rename,
    /// Move this row somewhere else. Ask [`Action::MoveInto`] of the destination as well.
    Move,
    MoveInto,
    Share,
    UploadInto,
    NewFolderIn,
    /// Save a change to the file's content (a new version).
    Edit,
    /// Remove the copy on this PC, keep the one in the cloud. Changes nothing in the cloud, so the Backups rule
    /// never refuses it; a row with changes that are not sent yet is refused by the action itself.
    FreeUpSpace,
    /// Delete a whole backup: a device's folder, or one backed-up folder of a device. Its own call, behind the
    /// password ([`Verdict::NeedsPassword`]); the generic delete never does this.
    DeleteBackup,
}

impl Action {
    pub const ALL: [Action; 11] = [
        Action::Trash,
        Action::DeleteForever,
        Action::Rename,
        Action::Move,
        Action::MoveInto,
        Action::Share,
        Action::UploadInto,
        Action::NewFolderIn,
        Action::Edit,
        Action::FreeUpSpace,
        Action::DeleteBackup,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    /// [`Action::DeleteBackup`] only: allowed once the person confirmed the password
    /// ([`MSG_DELETE_BACKUP_STEP_UP`]); the server refuses the delete without that confirmation.
    NeedsPassword,
    /// [`Action::Trash`] and [`Action::DeleteForever`] on a backup: not through the generic delete. Offer
    /// [`Action::DeleteBackup`] instead.
    UseDeleteBackup,
    /// [`Action::DeleteBackup`] on a row that is not a whole backup (outside the tree, or one item inside a
    /// backup, which the generic delete removes). Never offered, so it has no words.
    NotABackup,
    /// Not allowed; these are the words to show (never log them).
    Refused(String),
}

impl Verdict {
    /// Whether a menu shows the action for this row (anything but a refusal).
    pub fn is_offered(&self) -> bool {
        !matches!(self, Verdict::Refused(_) | Verdict::NotABackup)
    }
}

/// What the rule reads of one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject<'a> {
    pub position: BackupPosition,
    pub is_folder: bool,
    /// The original's path when the row is this device's own backup of a folder it backs up now.
    pub own_active_source: Option<&'a str>,
}

fn refused(words: &str) -> Verdict {
    Verdict::Refused(words.to_string())
}

fn app_rule(position: BackupPosition, mutation: Mutation) -> Verdict {
    match check_app_mutation(position, mutation) {
        Ok(()) => Verdict::Allowed,
        Err(protected) => Verdict::Refused(protected.message.into_owned()),
    }
}

/// The rule, pure: `action` on `subject`. See the module doc for where each answer comes from.
pub fn verdict(subject: &Subject<'_>, action: Action) -> Verdict {
    use BackupPosition::{CategoryFolder, DeviceFolder, Manifest, Outside, Root};
    let position = subject.position;
    if position == Outside {
        return match action {
            // Outside the tree nothing is a backup.
            Action::DeleteBackup => Verdict::NotABackup,
            _ => Verdict::Allowed,
        };
    }
    let is_backup = matches!(position, DeviceFolder | CategoryFolder);
    let mutation = match action {
        Action::FreeUpSpace => return Verdict::Allowed,
        Action::Share => {
            return match position {
                // The manifest is never listed; asked anyway, it is read-only like everything about it.
                Manifest => refused(MSG_READ_ONLY),
                _ if subject.is_folder => refused(MSG_NO_FOLDER_SHARE),
                _ => Verdict::Allowed,
            };
        }
        Action::Trash | Action::DeleteBackup => Mutation::Trash,
        Action::DeleteForever => Mutation::DeletePermanently,
        Action::Rename => Mutation::Rename,
        Action::Move => Mutation::Move,
        Action::MoveInto | Action::UploadInto | Action::NewFolderIn => Mutation::Create,
        Action::Edit => Mutation::Write,
    };
    // The root's own words come first, as in core's drive rule; then this device's own active backup.
    if position != Root
        && position != Manifest
        && let Some(source) = subject.own_active_source
    {
        return Verdict::Refused(own_backup_refusal(source));
    }
    match (app_rule(position, mutation), action) {
        (Verdict::Allowed, Action::DeleteBackup) if is_backup => Verdict::NeedsPassword,
        // A single file or folder inside a backup is deleted by the generic delete (rule 11), not by this call.
        (Verdict::Allowed, Action::DeleteBackup) => Verdict::NotABackup,
        (Verdict::Allowed, Action::Trash | Action::DeleteForever) if is_backup => Verdict::UseDeleteBackup,
        (answer, _) => answer,
    }
}

/// [`verdict`] for the row `file_id`, read from the state database and what this device backs up now. `None`: no
/// such row. A database that cannot be read answers `Err`: the UI refuses the action, it never guesses "allowed".
pub fn check(db: &StateDb, own: &OwnBackups, file_id: &str, action: Action) -> Result<Option<Verdict>, String> {
    Ok(subject_of(db, own, file_id)?.map(|subject| verdict(&subject.borrowed(), action)))
}

/// The actions a menu offers for the row `file_id`: every one [`check`] does not refuse, in [`Action::ALL`]'s
/// order. `None`: no such row.
pub fn allowed_actions(db: &StateDb, own: &OwnBackups, file_id: &str) -> Result<Option<Vec<Action>>, String> {
    Ok(subject_of(db, own, file_id)?.map(|subject| {
        let subject = subject.borrowed();
        Action::ALL
            .into_iter()
            .filter(|action| verdict(&subject, *action).is_offered())
            .collect()
    }))
}

/// A [`Subject`] that owns its path.
struct OwnedSubject {
    position: BackupPosition,
    is_folder: bool,
    own_active_source: Option<String>,
}

impl OwnedSubject {
    fn borrowed(&self) -> Subject<'_> {
        Subject {
            position: self.position,
            is_folder: self.is_folder,
            own_active_source: self.own_active_source.as_deref(),
        }
    }
}

fn subject_of(db: &StateDb, own: &OwnBackups, file_id: &str) -> Result<Option<OwnedSubject>, String> {
    let Some(entry) = db.get_file(file_id).map_err(|_| "read the row".to_string())? else {
        return Ok(None);
    };
    // The roots are read strictly here: `BackupsTree::read` treats a failed read as "no tree" (right for Explorer,
    // which must never be held), and that would allow every action.
    db.backups_roots().map_err(|_| "read the Backups root".to_string())?;
    let tree = BackupsTree::read(db);
    Ok(Some(OwnedSubject {
        position: tree.position(&entry.path),
        is_folder: entry.is_dir(),
        own_active_source: crate::backup_protect::own_active_source_of(&tree, own, &entry.path),
    }))
}

#[cfg(test)]
#[path = "backup_actions_tests.rs"]
mod tests;
