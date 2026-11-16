//! Slice A7 (plan40 batch 1): Add a folder and Stop backing up (DW-0737, DW-0738, DW-1410): a folder this PC backs up,
//! added or removed. Folds wave-3 S11 in: which folder a known folder OneDrive moved is backed up from.
//!
//! - [`Engine::add_backup_folder_for`]: DW-0738's picker answered with a folder. The rule ([`check_chosen`], here and
//!   nowhere else): Beebeeb's own folder and any folder inside it can't be chosen, nor a folder that holds it, a folder
//!   already backed up (a chosen one, or a known folder's location), or anything that is not a folder on this PC. A
//!   folder that passes joins the settings file's backup folders (`backup_chosen_folders`, its key `custom:<name>` in
//!   the backup set) under core's custom name (`custom_category_name`: safe, unique, never changed after), its backup
//!   starts over in the state database (no records, no stop, no last pass), and the runner passes it like a known
//!   folder from its next tick (`backup::sources::chosen_sources`).
//! - [`Engine::remove_backup_folder_for`]: DW-1410. The folder leaves the backup set first (settings, then its pending
//!   uploads purged and the protection refreshed: it is no longer this PC's active backup). "Stop and keep the
//!   backup" ends there: its backup stays in Backups, read-only. "Stop and move the backup to Trash" then proves the
//!   ACCOUNT PASSWORD and moves that one folder's backup to Trash through the engine's one delete of a backup
//!   (`Engine::delete_device_backup_for`: core's OPAQUE step-up, one single-use token, one move to Trash); a refusal
//!   before anything moved puts the folder back as it was ([`remove_steps`]); asked to move a backup this PC has not
//!   found, nothing changes at all. No file on this PC is touched, either way: nothing here opens, moves or deletes
//!   anything under the folder's path.
//! - [`Engine::choose_backup_source_for`] (S11): the answer to "which folder do you want to back up?" for a moved
//!   known folder, written to `known_folder_source`; its question is settled and the protection follows.
//!
//! Per account (review of #463, P3-8): the settings file is the PC's, so each chosen folder carries its owner's tag
//! ([`owner_tag`], a digest of the account's id, never an email or an id in clear; never made from an address, P3-5).
//! Only that account backs it up ([`chosen_for`]: the runner, [`runner_chosen`], and the protection read the folders of
//! the account the local data is bound to), lists it and removes it; nothing is written for another account. A folder
//! without a tag (written by a test build before folders were per account; no released build wrote one) is nobody's:
//! passed, listed and removable for no account, the local data's owner included (lead ruling 2026-11-16 01:04: fail
//! closed, the person adds the folder again). One an earlier build tagged by its address is nobody's too, until its own
//! account's next start tags it by that account's id ([`retag_at_start`], lead ruling 2026-11-16 05:47). Names stay
//! unique across every folder in the file, untagged ones included (their `custom:<name>` switch keys share the one
//! backup set).
//!
//! Never backed up twice (ruling 5.1, review of #464 P3-3): a chosen folder may not be, hold or lie inside another of
//! the account's chosen folders, nor a known folder whose switch is on. The rule follows the known folder's switch:
//! while Documents is off, a folder inside it may be chosen, and Documents is then not switched on while that folder is
//! backed up ([`switch_on_refusal`], which names it). [`nesting`] is the rule's one comparison (P3-6).
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and refused, with nothing written, once that session is no longer the one in memory.
//!
//! Must not: log a path, a folder's name, an id or the password; forge a step-up (the password is proven to the server
//! or nothing moves); delete anything in Beebeeb for good; touch the chosen folder on this PC.

use std::path::{Path, PathBuf};

use beebeeb_core::backup::custom_category_name;
use sha2::{Digest, Sha256};

use crate::account::SessionGeneration;
use crate::account_binding::{Identity, canonical_email};
use crate::app::backups_api::BackupsError;
use crate::app::{CredentialStores, Engine, LocalStateRefused, OwnedState};
use crate::backup::kfm::{self, KnownFolderPaths};
use crate::backup::sources::{self, ChosenFolder, SourceRefusal};
use crate::backup_delete::DeleteBackupError;
use crate::config::{DesktopConfig, KnownFolderSource};
use crate::identity::EngineIdentity;
use crate::known_folder::{KNOWN_FOLDERS, known_folder_by_key};
use crate::state_db::StateDb;

#[cfg(test)]
#[path = "backup_folders_tests.rs"]
mod tests;

/// Why a folder was not added. Nothing was written in any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddRefused {
    /// Beebeeb's own folder in File Explorer (the sync folder), or a folder inside it (DW-0738's rule).
    InsideSyncFolder,
    /// A folder that holds Beebeeb's own folder.
    HoldsSyncFolder,
    /// Backed up already: a folder chosen before, or where one of the known folders is.
    AlreadyBackedUp,
    /// Inside a folder backed up already (a chosen one, or a known folder such as Documents): its files are in that
    /// backup, so it would be backed up twice.
    InsideBackedUp,
    /// Holds a folder backed up already: that folder's files would be backed up twice.
    HoldsBackedUp,
    /// Where one of the known folders is (Documents, ...) while its switch is off: a known folder is switched on in the
    /// list, never added (P3-3).
    KnownFolderOff,
    /// Not a folder on this PC (a file, a folder that is gone, a path that is not absolute).
    NotAFolder,
    /// The local state or the settings file refused (signed out, another session, unreadable, ...).
    Backups(BackupsError),
}

impl AddRefused {
    /// What the window says, one sentence (it adds that nothing was changed).
    pub fn words(self) -> &'static str {
        match self {
            Self::InsideSyncFolder => {
                "Beebeeb's own folder, and any folder inside it, can't be chosen. Its files are in Beebeeb already."
            }
            Self::HoldsSyncFolder => {
                "This folder holds Beebeeb's own folder, so it can't be chosen. Choose a folder inside it."
            }
            Self::AlreadyBackedUp => "This folder is backed up already.",
            Self::InsideBackedUp => "This folder is inside a folder that is backed up already.",
            Self::HoldsBackedUp => {
                "This folder holds a folder that is backed up already. Choose one of them, not both."
            }
            Self::KnownFolderOff => "This folder is in the list already. Switch it on there to back it up.",
            Self::NotAFolder => "This isn't a folder on this PC.",
            Self::Backups(_) => "Couldn't add this folder. Try again in a moment.",
        }
    }

    /// The refusal's name, for a log line (no path, no name).
    pub fn kind(self) -> &'static str {
        match self {
            Self::InsideSyncFolder => "inside_sync_folder",
            Self::HoldsSyncFolder => "holds_sync_folder",
            Self::AlreadyBackedUp => "already_backed_up",
            Self::InsideBackedUp => "inside_backed_up",
            Self::HoldsBackedUp => "holds_backed_up",
            Self::KnownFolderOff => "known_folder",
            Self::NotAFolder => "not_a_folder",
            Self::Backups(error) => error.kind(),
        }
    }
}

/// Why "Stop backing up" did not do all it was asked. Each says whether anything changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveRefused {
    /// No folder the person chose has this id (removed meanwhile). Nothing changed.
    NotChosen,
    /// Asked to move the backup to Trash, but this PC has not found the folder's backup (nothing synced yet, or the
    /// backup is not in its list yet). Nothing changed: the folder is still backed up, and nothing was sent.
    NoCopyFound,
    /// The local state or the settings file refused. Nothing changed (a half-done removal is put back).
    Backups(BackupsError),
    /// The move to Trash was refused before anything was sent (the password, the rule, the session): the folder was
    /// put back in the backup set as it was. Nothing changed.
    PasswordRefused(DeleteBackupError),
    /// The folder is no longer backed up, but the move to Trash may not have happened (no answer, or the server's
    /// error): its backup may still be in Backups.
    TrashNotDone(DeleteBackupError),
}

impl RemoveRefused {
    /// What the window says, whole sentences that say whether anything changed.
    pub fn words(&self) -> &'static str {
        match self {
            Self::NotChosen => "This folder is no longer in the list. Nothing was changed.",
            Self::NoCopyFound => {
                "This PC hasn't found this folder's backup yet, so it can't be moved to Trash. Nothing was changed."
            }
            Self::Backups(_) => "Couldn't stop backing up this folder. Nothing was changed.",
            Self::PasswordRefused(DeleteBackupError::WrongPassword) => {
                "That password isn't right. Nothing was changed."
            }
            Self::PasswordRefused(DeleteBackupError::RateLimited(_)) => {
                "Too many tries. Wait a minute, then try again. Nothing was changed."
            }
            Self::PasswordRefused(DeleteBackupError::SignedOut) => "You're no longer signed in. Nothing was changed.",
            Self::PasswordRefused(DeleteBackupError::Refused(_) | DeleteBackupError::NotABackup) => {
                "This backup can't be moved to Trash from here. Nothing was changed."
            }
            Self::PasswordRefused(_) => "Couldn't move this backup to Trash. Nothing was changed.",
            Self::TrashNotDone(_) => {
                "This folder is no longer backed up. Beebeeb didn't confirm the move to Trash, so its backup may still be in Backups."
            }
        }
    }

    /// The refusal's name, for a log line (no path, no name, never the server's words).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotChosen => "not_chosen",
            Self::NoCopyFound => "not_found",
            Self::Backups(error) => error.kind(),
            Self::PasswordRefused(_) => "trash_refused",
            Self::TrashNotDone(_) => "trash_unconfirmed",
        }
    }

    /// The folder left the backup set (only [`RemoveRefused::TrashNotDone`]).
    pub fn stopped_backing_up(&self) -> bool {
        matches!(self, Self::TrashNotDone(_))
    }
}

/// DW-0738's rule: whether `path` may be chosen while Beebeeb's own folder is `sync_root` and `taken` are the folders
/// this PC backs up already. Compared as the backup's own rule compares (`backup::sources`: component by component,
/// without case on Windows and macOS, as given and as resolved). Reads only whether `path` is a folder.
pub fn check_chosen(path: &Path, sync_root: &Path, taken: &[PathBuf]) -> Result<(), AddRefused> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(AddRefused::NotAFolder);
    }
    // A sync folder that cannot be placed is never guessed around: refused.
    if !sync_root.is_absolute() {
        return Err(AddRefused::Backups(BackupsError::Settings));
    }
    let inside = matches!(
        sources::check_source(path, sync_root),
        Err(SourceRefusal::InsideSyncRoot)
    );
    if inside {
        return Err(AddRefused::InsideSyncFolder);
    }
    let holds = matches!(
        sources::check_source(sync_root, path),
        Err(SourceRefusal::InsideSyncRoot)
    );
    if holds {
        return Err(AddRefused::HoldsSyncFolder);
    }
    match nested_with(path, taken, |folder| folder.as_path()) {
        Some((how, _)) => Err(add_refusal(how)),
        None => Ok(()),
    }
}

/// How a folder lies against another, for the rule that nothing is backed up twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nesting {
    /// The same folder.
    Same,
    /// Inside the other, at any depth.
    Inside,
    /// Holds the other, at any depth.
    Holds,
}

/// How `path` lies against `other` (P3-6: the double-backup rule's one comparison, for the add, its re-check under the
/// settings lock, a switch turned on and the re-tag): compared as the backup's own rule compares (`backup::sources`:
/// component by component, without case on Windows and macOS, as given and as resolved). A sibling whose name only
/// starts the same way is not nested. `None`: apart.
pub fn nesting(path: &Path, other: &Path) -> Option<Nesting> {
    let under = |inner: &Path, outer: &Path| {
        outer.is_absolute() && matches!(sources::check_source(inner, outer), Err(SourceRefusal::InsideSyncRoot))
    };
    if sources::same_folder(other, path) {
        Some(Nesting::Same)
    } else if under(path, other) {
        Some(Nesting::Inside)
    } else if under(other, path) {
        Some(Nesting::Holds)
    } else {
        None
    }
}

/// The first of `others` that `path` nests with ([`nesting`]), and how; the same folder before any other.
pub fn nested_with<'a, T>(path: &Path, others: &'a [T], at: impl Fn(&T) -> &Path) -> Option<(Nesting, &'a T)> {
    let same = others
        .iter()
        .find(|other| nesting(path, at(other)) == Some(Nesting::Same));
    same.map(|other| (Nesting::Same, other)).or_else(|| {
        others
            .iter()
            .find_map(|other| nesting(path, at(other)).map(|how| (how, other)))
    })
}

/// An add's refusal for a folder that nests `how` with one backed up already.
fn add_refusal(how: Nesting) -> AddRefused {
    match how {
        Nesting::Same => AddRefused::AlreadyBackedUp,
        Nesting::Inside => AddRefused::InsideBackedUp,
        Nesting::Holds => AddRefused::HoldsBackedUp,
    }
}

fn digest(key: &str) -> String {
    crate::hexfmt::encode(&Sha256::digest(key.as_bytes()))
}

/// The one place a chosen folder's tag is made from an owner key (P3-6): [`owner_tag`] makes a tag here and
/// [`is_owners`] matches through [`owner_tag`], so the two never disagree. A plain SHA-256 digest today; a keyed digest
/// (a secret of this PC's or of the account's) slots in here alone (P3-5 (a)/(b): an open decision).
fn tag_of(owner_key: &str) -> String {
    digest(owner_key)
}

/// The tag a chosen folder carries for `owner` (P3-8): the digest of `user:<id>` (the owner key the backup's own rows
/// use), so the PC's settings file never holds an id in clear. Never made from an address (P3-5, lead ruling
/// 2026-11-16 05:47: an address can be guessed and checked against its digest). `None`: the id is not known (nothing
/// is added then).
pub fn owner_tag(owner: &Identity) -> Option<String> {
    owner.user_id.as_deref().map(|id| tag_of(&format!("user:{id}")))
}

/// The tag an earlier build made for a folder added while its owner's id was not known: the digest of
/// `email:<canonical email>`. Read once, by [`retag_by_id`] for that address's own account; never matched by a read.
fn address_tag(email: &str) -> Option<String> {
    let canonical = canonical_email(email);
    (!canonical.is_empty()).then(|| tag_of(&format!("email:{canonical}")))
}

/// Whether `folder` is `owner`'s: its tag is `owner`'s ([`owner_tag`], by the id alone). Untagged, tagged by an
/// address (until [`retag_by_id`]), or no owner: never.
pub fn is_owners(folder: &ChosenFolder, owner: Option<&Identity>) -> bool {
    match (folder.owner.as_deref(), owner.and_then(owner_tag)) {
        (Some(tag), Some(owners)) => tag == owners,
        _ => false,
    }
}

/// P3-5 (lead ruling 2026-11-16 05:47): the folders an earlier build tagged by `owner`'s OWN address carry `owner`'s
/// id tag from now on, so a re-tagged entry no longer holds a digest of an address. Only an entry whose tag is the
/// digest of `owner`'s canonical address is changed, once: another account's (by id or by its own address), an
/// untagged one and one tagged by id stay as they are. One that would back `owner`'s folder up twice (it is, holds or
/// lies inside a folder `owner`'s id tag holds already) stays as it was: nobody's (fail closed; the person sees the one
/// folder). An entry left as it was (that one, or one whose account has not started since, or started with another
/// address) keeps its address digest and stays nobody's. Nothing without both the id and the address. How many were
/// re-tagged.
pub fn retag_by_id(config: &mut DesktopConfig, owner: &Identity) -> usize {
    let (Some(by_id), Some(by_address)) = (owner_tag(owner), owner.email.as_deref().and_then(address_tag)) else {
        return 0;
    };
    let mut retagged = 0;
    for at in 0..config.backup_chosen_folders.len() {
        if config.backup_chosen_folders[at].owner.as_deref() != Some(by_address.as_str()) {
            continue;
        }
        let owners: Vec<PathBuf> = config
            .backup_chosen_folders
            .iter()
            .filter(|folder| folder.owner.as_deref() == Some(by_id.as_str()))
            .map(|folder| folder.path.clone())
            .collect();
        if nested_with(&config.backup_chosen_folders[at].path, &owners, PathBuf::as_path).is_some() {
            continue;
        }
        config.backup_chosen_folders[at].owner = Some(by_id.clone());
        retagged += 1;
    }
    retagged
}

/// [`retag_by_id`] on the settings file at `path`, under its lock (written only when something changed).
pub fn retag_in(path: &Path, owner: &Identity) -> Result<usize, String> {
    DesktopConfig::update_at(path, |config| {
        let retagged = retag_by_id(config, owner);
        (retagged > 0, retagged)
    })
}

/// The sync start's part of P3-5, right after the local data was bound to `owner` (the session's account): its
/// address-tagged folders take its id tag ([`retag_by_id`]). Best effort: a settings file that cannot be read or written
/// keeps them nobody's until a later start. Logs a count alone.
pub fn retag_at_start(identity: &'static EngineIdentity, owner: &Identity) {
    if owner.user_id.is_none() || owner.email.is_none() {
        return;
    }
    let retagged = DesktopConfig::path_for(identity).and_then(|path| retag_in(&path, owner));
    match retagged {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "backups: chosen folders were marked with the account id"),
        Err(_) => tracing::warn!("backups: chosen folders could not be marked with the account id"),
    }
}

/// `owner`'s chosen folders alone, in the file's order. An untagged folder is nobody's here; no owner: none.
pub fn chosen_for(chosen: &[ChosenFolder], owner: Option<&Identity>) -> Vec<ChosenFolder> {
    chosen
        .iter()
        .filter(|folder| is_owners(folder, owner))
        .cloned()
        .collect()
}

/// The runner's read of the chosen folders: those of the account `db` is bound to ([`chosen_for`] with `db`'s recorded
/// owner). An untagged folder is passed for nobody; a state with no owner recorded passes nothing.
pub fn runner_chosen(db: &StateDb, config: &DesktopConfig) -> Vec<ChosenFolder> {
    chosen_for(&config.backup_chosen_folders, db.owner().ok().flatten().as_ref())
}

/// A folder of an account's list, as the double-backup rule reads it.
#[derive(Debug, Clone)]
struct Place {
    /// Its key in the backup set (`custom:<name>`, or the known folder's).
    key: String,
    /// The name the list shows for it.
    name: String,
    path: PathBuf,
    /// A known folder (switched in the list, never added), not a chosen one.
    known: bool,
    /// Its switch is on: it is backed up now.
    on: bool,
}

/// The folders in `owner`'s list: its chosen ones (an untagged one, or another account's, is not; no owner: none),
/// then each known folder at its default and at its current location (`paths`: `kfm::known_folder_paths`, or a test's
/// pairs), each with its switch.
fn places(config: &DesktopConfig, owner: Option<&Identity>, paths: impl Fn(&str) -> KnownFolderPaths) -> Vec<Place> {
    let mut found: Vec<Place> = chosen_for(&config.backup_chosen_folders, owner)
        .into_iter()
        .map(|folder| Place {
            key: folder.key(),
            on: config.known_folder_enabled(&folder.key()),
            name: folder.name,
            path: folder.path,
            known: false,
        })
        .collect();
    for folder in KNOWN_FOLDERS {
        let both = paths(folder.key);
        for path in both.default.into_iter().chain(both.current) {
            found.push(Place {
                key: folder.key.to_string(),
                name: folder.display_name.to_string(),
                path,
                known: true,
                on: config.known_folder_enabled(folder.key),
            });
        }
    }
    found
}

/// Whether a new folder is compared with `place` (P3-3): every chosen folder of the account's (on or stopped: it stays
/// in the list and may be backed up again) and each known folder whose switch is on. A known folder that is off backs
/// nothing up, so a folder inside it may be chosen.
fn takes(place: &Place) -> bool {
    !place.known || place.on
}

/// Where the folders `owner` backs up already are ([`takes`]): its chosen ones (an untagged one is nobody's, so
/// its folder can be added again) and each known folder whose switch is on, at its default and current location.
pub fn taken_paths(config: &DesktopConfig, owner: &Identity) -> Vec<PathBuf> {
    taken_paths_with(config, owner, kfm::known_folder_paths)
}

/// [`taken_paths`] with the known folders' locations from `paths` (the tests' folder pairs).
pub fn taken_paths_with(
    config: &DesktopConfig,
    owner: &Identity,
    paths: impl Fn(&str) -> KnownFolderPaths,
) -> Vec<PathBuf> {
    places(config, Some(owner), paths)
        .into_iter()
        .filter(takes)
        .map(|place| place.path)
        .collect()
}

/// Why a folder's switch was not turned on (P3-3): it nests with another folder of the account's that is backed up
/// now. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchRefused {
    /// The name the list shows for the other folder.
    pub other: String,
    /// How the folder being switched on lies against it.
    pub how: Nesting,
}

impl SwitchRefused {
    /// What the window says, naming the other folder (its name in the list; never a path). The window adds that
    /// nothing was changed.
    pub fn words(&self) -> String {
        let other = &self.other;
        match self.how {
            Nesting::Same => format!("This folder is backed up already, as {other}."),
            Nesting::Inside => {
                format!("This folder is inside {other}, which is backed up already, so its files are in that backup.")
            }
            Nesting::Holds => format!(
                "This folder holds {other}, which is backed up already. Stop backing up {other} first, so nothing is backed up twice."
            ),
        }
    }
}

/// Whether `owner`'s folder `key` (a known folder, or a chosen one of `owner`'s: "Back up again") may be switched on
/// (P3-3): refused while it is, holds or lies inside another folder of `owner`'s whose switch is on, naming that folder.
/// A known folder is compared with the chosen folders alone (two known folders are not compared, as before); a chosen
/// one with the known folders and the other chosen ones. Another account's chosen folders, and untagged ones, are never
/// compared nor named. `None`: allowed, also for a key that is not in `owner`'s list (the switch refuses an unknown key
/// itself).
pub fn switch_on_refusal(config: &DesktopConfig, key: &str, owner: Option<&Identity>) -> Option<SwitchRefused> {
    switch_on_refusal_with(config, key, owner, kfm::known_folder_paths)
}

/// [`switch_on_refusal`] with the known folders' locations from `paths` (the tests' folder pairs).
pub fn switch_on_refusal_with(
    config: &DesktopConfig,
    key: &str,
    owner: Option<&Identity>,
    paths: impl Fn(&str) -> KnownFolderPaths,
) -> Option<SwitchRefused> {
    let all = places(config, owner, paths);
    let known = all.iter().any(|place| place.key == key && place.known);
    let backed_up: Vec<&Place> = all
        .iter()
        .filter(|place| place.key != key && place.on && !(known && place.known))
        .collect();
    all.iter()
        .filter(|place| place.key == key)
        .find_map(|place| nested_with(&place.path, &backed_up, |other| other.path.as_path()))
        .map(|(how, other)| SwitchRefused {
            other: other.name.clone(),
            how,
        })
}

/// The names of the folders in this device's backup (`Backups/<device>/*`) the local state knows, so a new custom
/// name never lands in the backup of a folder removed earlier and kept (P3-10: a pass adopts a live folder of its name,
/// `backup::host`'s `category_folder`).
///
/// No layout yet (nothing passed since this local state began, so which device folder is this PC's is not known): the
/// names in EVERY device's backup the local rows know (each Backups root, by its role, then each device folder in it).
/// A name another device uses is skipped too; only the new folder's name changes for it. Rows not pulled yet hold no
/// name: that case stays open (the pass's to close).
pub fn backup_folder_names(db: &StateDb) -> Result<Vec<String>, BackupsError> {
    let folders_in = |parent: &str| -> Result<Vec<(String, String)>, BackupsError> {
        let children = db.children_of(Some(parent)).map_err(|_| BackupsError::State)?;
        Ok(children
            .into_iter()
            .filter(|(entry, _)| entry.is_dir())
            .filter_map(|(entry, _)| {
                let name = entry.path.rsplit('/').next()?.to_string();
                (!name.is_empty()).then_some((entry.file_id, name))
            })
            .collect())
    };
    if let Some(layout) = db.backup_layout().map_err(|_| BackupsError::State)? {
        return Ok(folders_in(&layout.device_folder_id)?
            .into_iter()
            .map(|(_, name)| name)
            .collect());
    }
    let mut names = Vec::new();
    for (root_id, _) in db.backups_roots().map_err(|_| BackupsError::State)? {
        for (device_id, _) in folders_in(&root_id)? {
            names.extend(folders_in(&device_id)?.into_iter().map(|(_, name)| name));
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// The settings' part of an add for `owner`: `path` joins the chosen folders under core's custom name (unique among
/// EVERY account's chosen names, their switch keys share the backup set, and `in_backup`, the names already in this
/// device's backup), tagged as `owner`'s, and its key joins the backup set. Its key back. Refused, with nothing
/// changed, when the file now holds a folder of `owner`'s it is, holds or lies inside, or a known folder whose switch
/// is on (another add or a switch landed since the rule ran: read from the file this call holds; an untagged folder is
/// nobody's, so its folder may be added again, under a name of its own), when it is where a known folder is while that
/// one is off (switched, never added), or when `owner` has no tag.
pub fn add_to(
    config: &mut DesktopConfig,
    path: &Path,
    in_backup: &[String],
    owner: &Identity,
) -> Result<String, AddRefused> {
    add_to_with(config, path, in_backup, owner, kfm::known_folder_paths)
}

/// [`add_to`] with the known folders' locations from `paths` (the tests' folder pairs).
pub fn add_to_with(
    config: &mut DesktopConfig,
    path: &Path,
    in_backup: &[String],
    owner: &Identity,
    paths: impl Fn(&str) -> KnownFolderPaths,
) -> Result<String, AddRefused> {
    let tag = owner_tag(owner).ok_or(AddRefused::Backups(BackupsError::Refused(
        LocalStateRefused::IdentityUnknown,
    )))?;
    let all = places(config, Some(owner), paths);
    let taken: Vec<&Place> = all.iter().filter(|place| takes(place)).collect();
    if let Some((how, _)) = nested_with(path, &taken, |place| place.path.as_path()) {
        return Err(add_refusal(how));
    }
    if all
        .iter()
        .any(|place| !takes(place) && nesting(path, &place.path) == Some(Nesting::Same))
    {
        return Err(AddRefused::KnownFolderOff);
    }
    let mut taken: Vec<&str> = config
        .backup_chosen_folders
        .iter()
        .map(|folder| folder.name.as_str())
        .collect();
    taken.extend(in_backup.iter().map(String::as_str));
    let raw = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = ChosenFolder {
        name: custom_category_name(&raw, &taken),
        path: path.to_path_buf(),
        owner: Some(tag),
        folder_id: None,
    };
    let key = folder.key();
    config.backup_chosen_folders.push(folder);
    config.set_known_folder(&key, true);
    Ok(key)
}

/// What a removal took out of the settings, to put it back exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub folder: ChosenFolder,
    /// Its place in the list.
    pub at: usize,
    /// It was in the backup set (on).
    pub was_on: bool,
}

/// The settings' part of a removal: the chosen folder `key` leaves the list and the backup set. `None`: no such folder.
pub fn remove_from(config: &mut DesktopConfig, key: &str) -> Option<Removed> {
    let at = config
        .backup_chosen_folders
        .iter()
        .position(|folder| folder.key() == key)?;
    let folder = config.backup_chosen_folders.remove(at);
    let was_on = config.known_folder_enabled(key);
    config.set_known_folder(key, false);
    Some(Removed { folder, at, was_on })
}

/// Undo [`remove_from`]: the folder back in its place, its switch as it was.
pub fn put_back_into(config: &mut DesktopConfig, removed: &Removed) {
    let key = removed.folder.key();
    if !config.backup_chosen_folders.iter().any(|folder| folder.key() == key) {
        let at = removed.at.min(config.backup_chosen_folders.len());
        config.backup_chosen_folders.insert(at, removed.folder.clone());
    }
    config.set_known_folder(&key, removed.was_on);
}

/// Record `folder_id` as the folder of `owner`'s chosen folder `key`'s backup (the pass, before it creates that
/// folder: `backup::host`). Only `owner`'s entry is written (P3-8; an untagged one is nobody's). `false`: no such
/// entry (removed meanwhile, or another account's), nothing written.
pub fn record_folder_id(config: &mut DesktopConfig, key: &str, owner: &Identity, folder_id: &str) -> bool {
    match config
        .backup_chosen_folders
        .iter_mut()
        .find(|folder| folder.key() == key && is_owners(folder, Some(owner)))
    {
        Some(folder) => {
            folder.folder_id = Some(folder_id.to_string());
            true
        }
        None => false,
    }
}

/// The state database's part of an add: the folder's backup starts over (no records, no last pass, no stop, and no
/// folder remembered for its key: an earlier folder of the same name, removed, may have passed into a backup the person
/// kept, and the new one never continues there, P3-10), and the protection takes it in (it is this PC's active backup
/// now). Run after the settings file holds it.
pub fn start_over(identity: &'static EngineIdentity, db: &StateDb, key: &str) -> Result<(), BackupsError> {
    let now = chrono::Utc::now().timestamp_millis();
    db.restart_backup_pass(key, now).map_err(|_| BackupsError::State)?;
    db.clear_backup_stop(key).map_err(|_| BackupsError::State)?;
    db.set_backup_category_folder(key, None)
        .map_err(|_| BackupsError::State)?;
    crate::backup_protect::refresh_own_backups(identity, db);
    Ok(())
}

/// The state database's part of a removal: the folder's pending uploads go, and the protection lets it go (it is no
/// longer this PC's active backup, so its backup may be moved to Trash). Run after the settings file let it go. What is
/// backed up already stays; nothing on this PC is touched. How many uploads went.
pub fn leave_set(identity: &'static EngineIdentity, db: &StateDb, key: &str) -> Result<usize, BackupsError> {
    let purged = db.purge_backup_source_ops(key).map_err(|_| BackupsError::State)?;
    crate::backup_protect::refresh_own_backups(identity, db);
    tracing::info!(purged, "backups: a chosen folder was removed; its pending uploads went");
    Ok(purged)
}

/// The id of `folder`'s backup in Beebeeb (`Backups/<device>/<name>`): the folder whose id its entry holds
/// (`ChosenFolder::folder_id`, recorded by its pass), while the local state holds it in this device's folder. Never
/// found by its name: another folder of that name (a backup the person kept) is not this one's. `None`: no id (it never
/// passed), no layout yet, or that folder is not in this device's folder as the local state knows it. Only that one
/// folder: never another folder's backup, never the device's.
pub fn backup_copy_of(db: &StateDb, folder: &ChosenFolder) -> Result<Option<String>, BackupsError> {
    let Some(id) = folder.folder_id.as_deref() else {
        return Ok(None);
    };
    let Some(layout) = db.backup_layout().map_err(|_| BackupsError::State)? else {
        return Ok(None);
    };
    let children = db
        .children_of(Some(&layout.device_folder_id))
        .map_err(|_| BackupsError::State)?;
    Ok(children
        .into_iter()
        .map(|(entry, _)| entry)
        .find(|entry| entry.is_dir() && entry.file_id == id)
        .map(|entry| entry.file_id))
}

/// Whether a refused move to Trash may have moved something in Beebeeb (then nothing is put back: the folder stays out
/// of the set). `NotFound` is not one (P3-12): before anything is sent it is an id that is no file id or no such row,
/// and from the server (404) the move found nothing to move; either way nothing moved.
fn may_have_reached(error: &DeleteBackupError) -> bool {
    matches!(error, DeleteBackupError::Offline | DeleteBackupError::Failed("server"))
}

/// DW-1410 as one sequence, its steps handed in (the engine's: the settings file and the state database, the
/// password-proven move to Trash; a test's: fakes):
/// 0. With `password` and no `copy` (this PC has not found the folder's backup): refused before anything, nothing
///    changed (P3-9: "done" would claim a move to Trash that never happened).
/// 1. `leave`: the folder leaves the backup set. A refusal there changed nothing (`leave` puts back what it did).
/// 2. Only with `password` and a `copy` (its backup's id): `trash` proves the password and moves that one folder to
///    Trash. Refused before anything moved: `put_back` puts the folder back as it was, nothing changed. Refused after
///    it may have moved something: the folder stays out of the set, and the refusal says so.
///
/// Without a password ("Stop and keep the backup") `trash` is never called.
pub fn remove_steps<Undo>(
    password: Option<&[u8]>,
    copy: Option<&str>,
    leave: impl FnOnce() -> Result<Undo, BackupsError>,
    trash: impl FnOnce(&str, &[u8]) -> Result<(), DeleteBackupError>,
    put_back: impl FnOnce(Undo),
) -> Result<(), RemoveRefused> {
    if password.is_some() && copy.is_none() {
        return Err(RemoveRefused::NoCopyFound);
    }
    let undo = leave().map_err(RemoveRefused::Backups)?;
    let (Some(password), Some(copy)) = (password, copy) else {
        return Ok(());
    };
    match trash(copy, password) {
        Ok(()) => Ok(()),
        Err(error) if may_have_reached(&error) => Err(RemoveRefused::TrashNotDone(error)),
        Err(error) => {
            put_back(undo);
            Err(RemoveRefused::PasswordRefused(error))
        }
    }
}

/// The settings file without the folder an add wrote, best effort (an add whose state part failed is not left half
/// done).
fn undo_add(identity: &'static EngineIdentity, key: &str) {
    let undone = DesktopConfig::update_for(identity, |config| (remove_from(config, key).is_some(), ()));
    if undone.is_err() {
        tracing::warn!("backups: a failed add could not be taken out of the settings file");
    }
}

/// Put a removed folder back, best effort: the settings file, then the protection.
fn restore(identity: &'static EngineIdentity, handle: Option<&OwnedState<'_>>, removed: &Removed) {
    let written = DesktopConfig::update_for(identity, |config| {
        put_back_into(config, removed);
        (true, ())
    });
    if written.is_err() {
        tracing::warn!("backups: a chosen folder could not be put back in the settings file");
    }
    if let Some(handle) = handle {
        let _ = handle.with(|db| crate::backup_protect::refresh_own_backups(identity, db));
    }
}

/// S11: the state database's part of a moved known folder's answer: its question is settled, the protection follows.
pub fn source_chosen(identity: &'static EngineIdentity, db: &StateDb, key: &str) -> Result<(), BackupsError> {
    db.settle_backup_source_question(key).map_err(|_| BackupsError::State)?;
    crate::backup_protect::refresh_own_backups(identity, db);
    Ok(())
}

impl<C: CredentialStores> Engine<C> {
    /// The local state for a change asked under `session`: `None` while nothing has synced on this computer (the
    /// settings file alone is written then, as `set_backup_folder` does). Refused once `session` is not the one in
    /// memory, before anything is read.
    fn backup_folders_state(&self, session: SessionGeneration) -> Result<Option<OwnedState<'_>>, BackupsError> {
        if self.session_turn() != session {
            return Err(BackupsError::Refused(LocalStateRefused::SessionChanged));
        }
        let handle = match self.local_state_for_changes() {
            Ok(handle) => handle,
            Err(LocalStateRefused::NothingSynced) => return Ok(None),
            Err(refused) => return Err(BackupsError::Refused(refused)),
        };
        if handle.session() != session {
            return Err(BackupsError::Refused(LocalStateRefused::SessionChanged));
        }
        handle.still_this_session().map_err(BackupsError::Refused)?;
        Ok(Some(handle))
    }

    /// Whose chosen folders a call under `handle` reads and writes (P3-8): with a local state, its recorded owner; with
    /// nothing synced on this PC yet, the session's own identity. Refused when neither is known.
    fn chosen_asker(&self, handle: Option<&OwnedState<'_>>) -> Result<Identity, BackupsError> {
        match handle {
            Some(handle) => match handle.with(StateDb::owner).map_err(BackupsError::Refused)? {
                Ok(Some(owner)) => Ok(owner),
                Ok(None) => Err(BackupsError::Refused(LocalStateRefused::IdentityUnknown)),
                Err(_) => Err(BackupsError::State),
            },
            None => self.session_identity().map_err(BackupsError::Refused),
        }
    }

    /// The chosen folders in `chosen` the signed-in account sees under `handle` ([`chosen_for`]); none when whose they
    /// are cannot be told. For the overview and "Back up again" (`backups_api`).
    pub(crate) fn chosen_folders_seen(
        &self,
        handle: Option<&OwnedState<'_>>,
        chosen: &[ChosenFolder],
    ) -> Vec<ChosenFolder> {
        match self.chosen_asker(handle) {
            Ok(asker) => chosen_for(chosen, Some(&asker)),
            Err(_) => Vec::new(),
        }
    }

    /// Whether `key` is one of the signed-in account's chosen folders ("Back up again" after its stop,
    /// `set_backup_folder`). While the local state is refused, any folder with that key answers (as before folders were
    /// per account): the caller's own handle refuses next, with its reason, and writes nothing.
    pub(crate) fn chosen_key_seen(&self, key: &str) -> bool {
        let Ok(config) = DesktopConfig::load_for(self.identity) else {
            return false;
        };
        let seen = match self.local_state() {
            Ok(handle) => self.chosen_folders_seen(Some(&handle), &config.backup_chosen_folders),
            Err(LocalStateRefused::NothingSynced) => self.chosen_folders_seen(None, &config.backup_chosen_folders),
            Err(_) => config.backup_chosen_folders,
        };
        seen.iter().any(|folder| folder.key() == key)
    }

    /// Whose folders a switch under `handle` compares with (P3-3, [`switch_on_refusal`]): the local data's owner, or
    /// the session's account while nothing has synced. `None` when that cannot be told: then no chosen folder is
    /// compared (none is passed for an owner that cannot be told either).
    pub(crate) fn switch_asker(&self, handle: Option<&OwnedState<'_>>) -> Option<Identity> {
        self.chosen_asker(handle).ok()
    }

    /// The words for a switch of `key` that was refused with `BackupsError::Nested` (P3-3): the folder it nests with,
    /// named as the settings file and the local state hold it now. `None`: nothing nests with it now (it may be
    /// switched on again), or whose folders they are cannot be told. Reads only. BLOCKS: a worker's.
    pub fn switch_on_refused(&self, key: &str) -> Option<SwitchRefused> {
        let config = DesktopConfig::load_for(self.identity).ok()?;
        let asker = match self.local_state() {
            Ok(handle) => self.switch_asker(Some(&handle)),
            Err(LocalStateRefused::NothingSynced) => self.switch_asker(None),
            Err(_) => None,
        }?;
        switch_on_refusal(&config, key, Some(&asker))
    }

    /// WRITES: add `path` (DW-0738's picker's answer) to the folders this PC backs up, for the session a person
    /// pressed under, as that account's. The rule is [`check_chosen`]; see the module's doc. Its key back
    /// (`custom:<name>`, the id the window lists it under). Never runs a pass here: the runner's next tick passes it.
    /// BLOCKS: a worker's.
    pub fn add_backup_folder_for(&self, session: SessionGeneration, path: &Path) -> Result<String, AddRefused> {
        let handle = self.backup_folders_state(session).map_err(AddRefused::Backups)?;
        let owner = self.chosen_asker(handle.as_ref()).map_err(AddRefused::Backups)?;
        let identity = self.identity;
        let sync_root = self
            .sync_root()
            .map_err(|_| AddRefused::Backups(BackupsError::Settings))?;
        let config = DesktopConfig::load_for(identity).map_err(|_| AddRefused::Backups(BackupsError::Settings))?;
        check_chosen(path, &sync_root, &taken_paths(&config, &owner))?;
        let in_backup = match &handle {
            Some(handle) => handle
                .with(backup_folder_names)
                .map_err(|refused| AddRefused::Backups(BackupsError::Refused(refused)))?
                .map_err(AddRefused::Backups)?,
            None => Vec::new(),
        };
        // Written under the settings file's lock: the duplicate is checked again against what the file holds now.
        let key = DesktopConfig::update_for(identity, |config| match add_to(config, path, &in_backup, &owner) {
            Ok(key) => (true, Ok(key)),
            Err(refused) => (false, Err(refused)),
        })
        .map_err(|_| AddRefused::Backups(BackupsError::Settings))??;
        if let Some(handle) = &handle {
            let started = handle
                .with(|db| start_over(identity, db, &key))
                .map_err(BackupsError::Refused)
                .and_then(|started| started);
            if let Err(error) = started {
                undo_add(identity, &key);
                return Err(AddRefused::Backups(error));
            }
        }
        tracing::info!("backups: a chosen folder was added");
        Ok(key)
    }

    /// WRITES: stop backing up the chosen folder `id` (DW-1410), for the session a person pressed under. Without
    /// `trash_password` its backup stays in Backups ("Stop and keep the backup"); with it, the account password is
    /// proven and that one folder's backup moves to Trash ("Stop and move the backup to Trash"). See
    /// [`remove_steps`] for what a refusal leaves. Only a folder of this account's. Never touches the folder on this
    /// PC. BLOCKS: a worker's.
    pub fn remove_backup_folder_for(
        &self,
        session: SessionGeneration,
        id: &str,
        trash_password: Option<&[u8]>,
    ) -> Result<(), RemoveRefused> {
        let handle = self.backup_folders_state(session).map_err(RemoveRefused::Backups)?;
        let owner = self.chosen_asker(handle.as_ref()).map_err(RemoveRefused::Backups)?;
        let identity = self.identity;
        let config = DesktopConfig::load_for(identity).map_err(|_| RemoveRefused::Backups(BackupsError::Settings))?;
        // Only a folder of this account's (P3-8): another account's is "not in the list" here.
        let Some(folder) = chosen_for(&config.backup_chosen_folders, Some(&owner))
            .into_iter()
            .find(|folder| folder.key() == id)
        else {
            return Err(RemoveRefused::NotChosen);
        };
        let key = folder.key();
        // Its backup, found while the folder is still in the set (only when it goes to Trash).
        let copy = match (&handle, trash_password) {
            (Some(handle), Some(_)) => handle
                .with(|db| backup_copy_of(db, &folder))
                .map_err(|refused| RemoveRefused::Backups(BackupsError::Refused(refused)))?
                .map_err(RemoveRefused::Backups)?,
            _ => None,
        };
        let leave = || -> Result<Removed, BackupsError> {
            let removed = DesktopConfig::update_for(identity, |config| match remove_from(config, &key) {
                Some(removed) => (true, Some(removed)),
                None => (false, None),
            })
            .map_err(|_| BackupsError::Settings)?
            .ok_or(BackupsError::UnknownFolder)?;
            if let Some(handle) = &handle {
                let left = handle
                    .with(|db| leave_set(identity, db, &key))
                    .map_err(BackupsError::Refused)
                    .and_then(|left| left);
                if let Err(error) = left {
                    restore(identity, Some(handle), &removed);
                    return Err(error);
                }
            }
            Ok(removed)
        };
        let removed = remove_steps(
            trash_password,
            copy.as_deref(),
            leave,
            |copy, password| self.delete_device_backup_for(session, copy, password),
            |removed: Removed| restore(identity, handle.as_ref(), &removed),
        );
        match &removed {
            Ok(()) => tracing::info!("backups: a chosen folder stopped backing up"),
            Err(refused) => tracing::info!(
                kind = refused.kind(),
                "backups: a chosen folder was not stopped as asked"
            ),
        }
        removed
    }

    /// S11 (wave 3, folded into A7): which folder the known folder `key`, moved by OneDrive's Known Folder Move, is
    /// backed up from (LD-0113 Amendment 9, "Ask once"), for the session a person pressed under (P3-14: bound to that
    /// session like its neighbours). The answer is written to the settings file (`known_folder_source`), its question
    /// is settled and the protection follows. Refused, with nothing written, for a key that is not one of the six, once
    /// `session` is not the one in memory, and unless the local state is the signed-in account's or nothing has
    /// synced on this computer yet. BLOCKS: a worker's.
    pub fn choose_backup_source_for(
        &self,
        session: SessionGeneration,
        key: &str,
        choice: KnownFolderSource,
    ) -> Result<(), BackupsError> {
        if known_folder_by_key(key).is_none() {
            return Err(BackupsError::UnknownFolder);
        }
        let handle = self.backup_folders_state(session)?;
        let identity = self.identity;
        DesktopConfig::update_for(identity, |config| {
            let was = config.known_folder_source.insert(key.to_string(), choice);
            (was != Some(choice), ())
        })
        .map_err(|_| BackupsError::Settings)?;
        if let Some(handle) = &handle {
            handle
                .with(|db| source_chosen(identity, db, key))
                .map_err(BackupsError::Refused)??;
        }
        Ok(())
    }
}
