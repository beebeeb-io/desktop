//! The account's OTHER devices' backups, for a host without a UI toolkit (the native Backups section), and "Delete
//! this backup" of one of them (DW-0737, DW-0739, DW-0742).
//!
//! - [`Engine::backup_devices_for`] lists the device folders under the account's Backups root and reads each one's
//!   manifest (`.device.json`) the way the backup pass reads this PC's own (`backup::layout`): the listing and the
//!   download through the bridge, the item's key and the content's decryption in core (`owned_key_and_pin`,
//!   `ContentRecord::decryptor`), the text parsed by core's `parse_manifest`. No crypto of its own; the plaintext is
//!   wiped once parsed.
//! - This PC is never listed: its folder (the layout's `device_folder_id`) and any folder whose manifest is this PC's
//!   (core's `same_device_id`) are left out. While no layout names this PC's folder yet (a reset PC's first sync), a
//!   folder of this PC's device-folder name whose manifest does not name another device MAY be this PC's: it is
//!   listed as it reads, marked [`BackupDevice::may_be_this_pc`], and never deleted ([`may_be_this_pcs`]).
//! - A device whose manifest this PC cannot open (no key, no record, a name that does not open, not JSON) is a row
//!   that says so ([`DeviceRead::Unreadable`]), never hidden and never guessed. A device folder that could not be
//!   listed or downloaded is [`DeviceRead::NotReached`] for that row only.
//! - What a manifest says, and nothing more: desktops write theirs once (the folders on at setup, no pass time), iOS
//!   writes its model and never its sync time. So a row counts the backed-up folders ACTUALLY listed in the device's
//!   folder, and has a last backup only where the file names one.
//! - The root is the cached one, else read with `GET /backups/root`. Never claimed or created here (that is the
//!   backup pass's, `layout::resolve_root_id`). No root: no other devices.
//!
//! [`Engine::delete_device_backup_for`] is `backup_delete::delete_backup` for the session of the press: the rule
//! (`backup_actions::check`: only a row that answers `NeedsPassword` is sent), core's OPAQUE step-up with the account
//! PASSWORD, one single-use token, one `DELETE /api/v1/files/{id}` (a move to Trash). The engine's rule is followed
//! (it refuses this PC's actively backed-up folders), and before it this PC's own device folder is refused here, not
//! only left out of the list: the layout's `device_folder_id`; while there is no layout, a device folder that
//! [`may_be_this_pcs`] (its manifest is read first). Both before the password is proven, nothing sent.
//!
//! Session rule: the token (and the vault key) leave the session's lock together, as a person's call
//! (`Engine::person_session`: a locked screen is refused there, before anything is copied), only while the session in
//! memory is the press's; the local state is read through the account-bound handle of that same session, and the
//! answer is handed over only while it is still that session.
//!
//! Both BLOCK on the handle's runtime: a worker's, never the UI thread's.
//!
//! Must not: log a name, an id, a manifest's text, the password or a token; list this PC; draw a device it could not
//! read as read; delete before the password is proven; retry a delete.

use std::sync::Arc;

use beebeeb_core::backup::{
    device_folder_name, find_device_folder, fold_name, parse_manifest, same_device_id, DeviceFolderCandidate,
    DeviceFolderChoice, DeviceManifest, DeviceSelf, FolderManifest, TakenName, MANIFEST_NAME,
};
use beebeeb_types::backup::Platform;
use zeroize::Zeroizing;

use super::settings_api::{kind_of, AccountCallError};
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::backup_delete::{self, DeleteBackupError};
use crate::engine_bridge::{BackupChild, EngineBridge, EngineError};
use crate::state_db::StateDb;

#[cfg(test)]
#[path = "backup_devices_api_tests.rs"]
mod tests;

/// The longest device model kept from a manifest (`device_model`): a label, never a document.
const MODEL_MAX: usize = 64;

/// Why a delete of this PC's own device folder is refused (shown, never logged; no name in it).
const THIS_PC: &str = concat!(
    "This may be ",
    device_word!(),
    "'s own backup. Nothing was deleted."
);

/// What kind of device a manifest describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Computer,
    Phone,
    Tablet,
    /// The manifest names no platform and no model.
    Unknown,
}

/// What one device's manifest says, as far as it says it.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceSummary {
    /// The device's own name (`device_name`); `None` when the manifest's is empty.
    pub name: Option<String>,
    pub class: DeviceClass,
    /// The phone's or tablet's model (`device_model`, iOS), for a sentence ("Photos still on the iPhone ...").
    pub model: Option<String>,
    /// The backed-up folders listed in the device's folder now (not the manifest's setup-time list).
    pub folders: u32,
    /// The newest pass time the manifest names (milliseconds); `None` when it names none (today: always, see the
    /// module doc).
    pub last_backup_ms: Option<i64>,
}

impl std::fmt::Debug for DeviceSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceSummary")
            .field("name", &self.name.as_ref().map(|_| "<redacted>"))
            .field("class", &self.class)
            .field("model", &self.model.as_ref().map(|_| "<redacted>"))
            .field("folders", &self.folders)
            .field("last_backup_ms", &self.last_backup_ms)
            .finish()
    }
}

/// How one device's manifest was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceRead {
    Read(DeviceSummary),
    /// The device's folder holds no manifest (an old mirror's folder).
    NoStatusFile,
    /// It is there and does not open on this PC (no key, no record, a name or a frame that does not open, not JSON).
    Unreadable,
    /// The device's folder could not be listed, or its manifest could not be downloaded, now.
    NotReached,
}

/// Another device's backup: its folder under the Backups root.
#[derive(Clone, PartialEq, Eq)]
pub struct BackupDevice {
    /// The device folder's file id: what "Delete this backup" names.
    pub folder_id: String,
    /// The folder's name, opened on this PC; `None` when it does not open.
    pub folder_name: Option<String>,
    pub read: DeviceRead,
    /// It may be this PC's own folder (no layout names this PC's yet, see [`may_be_this_pcs`]): never offered for
    /// delete, and [`Engine::delete_device_backup_for`] refuses it.
    pub may_be_this_pc: bool,
}

impl std::fmt::Debug for BackupDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupDevice")
            .field("folder_id", &"<redacted>")
            .field("folder_name", &self.folder_name.as_ref().map(|_| "<redacted>"))
            .field("read", &self.read)
            .field("may_be_this_pc", &self.may_be_this_pc)
            .finish()
    }
}

/// The account's other devices, read for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupDevices {
    pub session: SessionGeneration,
    pub devices: Vec<BackupDevice>,
    /// `false`: the Backups listing was cut short; there may be devices it did not list.
    pub complete: bool,
}

/// Why the other devices were not read. Nothing is shown of them in any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicesRefused {
    /// No session in memory, or not the session of the ask: nothing was sent.
    NotSignedIn,
    /// The local state is not handed out (another account's, locked, nothing synced, ...).
    Local(LocalStateRefused),
    /// The root could not be read or listed.
    Call(AccountCallError),
}

impl DevicesRefused {
    /// The refusal's name, for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not_signed_in",
            Self::Local(refused) => refused.kind(),
            Self::Call(AccountCallError::SessionEnded) => "session_ended",
            Self::Call(AccountCallError::Offline) => "offline",
            Self::Call(_) => "failed",
        }
    }
}

/// The part of the bridge the read uses: a folder's live children, a small file's plaintext.
pub(crate) trait DeviceTree {
    async fn children(&self, parent: &str) -> Result<(Vec<BackupChild>, bool), EngineError>;
    async fn small_file(&self, file: &BackupChild) -> Result<Option<Vec<u8>>, EngineError>;
}

impl DeviceTree for EngineBridge {
    async fn children(&self, parent: &str) -> Result<(Vec<BackupChild>, bool), EngineError> {
        self.backup_list_children(parent).await
    }
    async fn small_file(&self, file: &BackupChild) -> Result<Option<Vec<u8>>, EngineError> {
        self.backup_read_small_file(&file.id, file.record, file.chunk_count, file.size)
            .await
    }
}

fn call_error(error: &EngineError) -> AccountCallError {
    match error {
        EngineError::Api(error) => kind_of(error),
        _ => AccountCallError::ServerError,
    }
}

/// A short label from a manifest's free text, or `None`.
fn label(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(MODEL_MAX).collect())
}

/// The iOS camera-roll categories' `last_sync` (RFC 3339), newest first, when a manifest names one.
fn ios_last_sync_ms(manifest: &DeviceManifest) -> Option<i64> {
    manifest
        .other
        .get("backups")?
        .as_object()?
        .values()
        .filter_map(|category| category.get("last_sync")?.as_str())
        .filter_map(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map(|at| at.timestamp_millis())
        .max()
}

/// What `manifest` says, with the backed-up folders listed beside it (`folders`). Pure.
pub fn summary_of(manifest: &DeviceManifest, folders: usize) -> DeviceSummary {
    let model = manifest
        .other
        .get("device_model")
        .and_then(|model| model.as_str())
        .and_then(label);
    let tablet = model.as_deref().is_some_and(|model| model.contains("iPad"));
    let class = match manifest.platform {
        Some(Platform::Windows | Platform::Macos | Platform::Linux | Platform::Cli) => DeviceClass::Computer,
        Some(Platform::Ios | Platform::Android) | None if tablet => DeviceClass::Tablet,
        Some(Platform::Ios | Platform::Android) => DeviceClass::Phone,
        None if model.is_some() => DeviceClass::Phone,
        None => DeviceClass::Unknown,
    };
    let desktop_pass = manifest
        .folders
        .iter()
        .filter_map(|folder| folder.last_pass_at_ms)
        .max();
    DeviceSummary {
        name: label(&manifest.device_name),
        class,
        model: model.filter(|_| matches!(class, DeviceClass::Phone | DeviceClass::Tablet)),
        folders: u32::try_from(folders).unwrap_or(u32::MAX),
        last_backup_ms: desktop_pass.into_iter().chain(ios_last_sync_ms(manifest)).max(),
    }
}

/// One device folder: its manifest read and parsed, whether it is this PC's, and the manifest itself (for core's
/// choice of this PC's folder).
async fn read_device<T: DeviceTree>(
    tree: &T,
    folder_id: &str,
    me: Option<&DeviceSelf>,
) -> (DeviceRead, bool, Option<DeviceManifest>) {
    let Ok((children, complete)) = tree.children(folder_id).await else {
        return (DeviceRead::NotReached, false, None);
    };
    let folders = children.iter().filter(|child| child.is_folder).count();
    // A file whose name does not open could be the manifest: then the device is unreadable, never "no status file".
    let unopened = children.iter().any(|child| !child.is_folder && child.name.is_none());
    let mut parsed = Vec::new();
    let mut failed = false;
    for file in children
        .iter()
        .filter(|child| !child.is_folder && child.name.as_deref() == Some(MANIFEST_NAME))
    {
        match tree.small_file(file).await {
            Err(_) => return (DeviceRead::NotReached, false, None),
            Ok(None) => failed = true,
            Ok(Some(bytes)) => {
                let bytes = Zeroizing::new(bytes);
                match std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|text| parse_manifest(text).ok())
                {
                    Some(manifest) => parsed.push(manifest),
                    None => failed = true,
                }
            }
        }
    }
    let is_me = |manifest: &DeviceManifest| {
        me.is_some_and(|me| {
            manifest
                .device_id
                .as_deref()
                .is_some_and(|id| same_device_id(id, &me.device_id))
        })
    };
    if let Some(mine) = parsed.iter().find(|manifest| is_me(manifest)) {
        return (DeviceRead::Read(summary_of(mine, folders)), true, None);
    }
    match parsed.first() {
        Some(manifest) => (
            DeviceRead::Read(summary_of(manifest, folders)),
            false,
            Some(manifest.clone()),
        ),
        None if failed || unopened => (DeviceRead::Unreadable, false, None),
        // The manifest may be in the part that was not listed.
        None if !complete => (DeviceRead::NotReached, false, None),
        None => (DeviceRead::NoStatusFile, false, None),
    }
}

/// Every device folder under `root_id` but this PC's, each read. This PC's folder: `this_pc` (the layout's), any
/// whose manifest is `me`'s, and, while no layout names it, the folder core's `find_device_folder` would adopt as this
/// PC's (the old mirror's folder of its exact name, no manifest). `Err`: the root itself could not be listed. The
/// second value is `false` when the root's listing was cut short.
pub(crate) async fn read_devices<T: DeviceTree>(
    tree: &T,
    root_id: &str,
    this_pc: Option<&str>,
    me: Option<&DeviceSelf>,
    pc_name: Option<&str>,
) -> Result<(Vec<BackupDevice>, bool), AccountCallError> {
    let (children, complete) = tree.children(root_id).await.map_err(|error| call_error(&error))?;
    let mut read = Vec::new();
    for folder in children.iter().filter(|child| child.is_folder) {
        if this_pc == Some(folder.id.as_str()) {
            continue;
        }
        let (device, mine, manifest) = read_device(tree, &folder.id, me).await;
        if !mine {
            read.push((folder, device, manifest));
        }
    }
    let adopted = match (this_pc, me) {
        (None, Some(me)) => adopted_by_this_pc(me, &read),
        _ => None,
    };
    let devices = read
        .into_iter()
        .filter(|(folder, _, _)| adopted.is_none_or(|id| uuid::Uuid::parse_str(&folder.id).ok() != Some(id)))
        .map(|(folder, read, manifest)| BackupDevice {
            folder_id: folder.id.clone(),
            folder_name: folder.name.clone(),
            read,
            // A layout names this PC's folder (left out above): no other row is this PC's.
            may_be_this_pc: this_pc.is_none()
                && may_be_this_pcs(folder.name.as_deref(), owner(false, manifest.as_ref()), me, pc_name),
        })
        .collect();
    Ok((devices, complete))
}

/// The folder core would take as this PC's (`find_device_folder`) among those read, when no layout names one yet.
fn adopted_by_this_pc(
    me: &DeviceSelf,
    read: &[(&BackupChild, DeviceRead, Option<DeviceManifest>)],
) -> Option<uuid::Uuid> {
    let candidates: Vec<DeviceFolderCandidate> = read
        .iter()
        .filter_map(|(folder, device, manifest)| {
            Some(DeviceFolderCandidate {
                folder_id: uuid::Uuid::parse_str(&folder.id).ok()?,
                // A name that does not open never matches by name (as `layout::unopened_name`).
                name: folder.name.clone().unwrap_or_else(|| format!("\u{0}{}", folder.id)),
                manifest: match (device, manifest) {
                    (DeviceRead::Read(_), Some(manifest)) => FolderManifest::Parsed(manifest.clone()),
                    (DeviceRead::NoStatusFile, _) => FolderManifest::Absent,
                    _ => FolderManifest::Unreadable,
                },
            })
        })
        .collect();
    match find_device_folder(me, &candidates) {
        DeviceFolderChoice::Existing(id) | DeviceFolderChoice::AdoptLegacy(id) => Some(id),
        DeviceFolderChoice::Create { .. } => None,
    }
}

/// Whose a device folder's manifest says it is, as [`read_device`] read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    ThisPc,
    /// A manifest that names a device id, not this PC's.
    Another,
    /// No manifest, one that does not open or names no device, or none reached.
    Unknown,
}

/// `mine` and the manifest [`read_device`] answered.
fn owner(mine: bool, manifest: Option<&DeviceManifest>) -> Owner {
    match manifest {
        _ if mine => Owner::ThisPc,
        Some(manifest) if manifest.device_id.is_some() => Owner::Another,
        _ => Owner::Unknown,
    }
}

/// Whether `name` is this PC's device-folder name as core makes it (`device_folder_name`): the bare name or one with
/// a collision suffix (`TestPC (a3f2)`, widened 4 digits at a time), compared as core folds names. Pure.
fn is_this_pcs_name(name: &str, me: &DeviceSelf) -> bool {
    let folded = fold_name(name);
    let mut taken: Vec<TakenName> = Vec::new();
    loop {
        let candidate = device_folder_name(&me.name, &me.device_id, &taken);
        if fold_name(&candidate) == folded {
            return true;
        }
        // The widest suffix is answered even when taken: every form was asked.
        if taken.iter().any(|t| t.name == candidate) {
            return false;
        }
        taken.push(TakenName {
            name: candidate,
            device_id: None,
        });
    }
}

/// `name` is the device-folder name core makes of this PC's computer name `pc` (`clean_name`), bare or with any
/// collision suffix: without a device id the suffix this PC would get is not known, so every one counts. Pure.
fn is_this_pcs_computer_name(name: &str, pc: &str) -> bool {
    let base = fold_name(&device_folder_name(pc, "", &[]));
    let folded = fold_name(name);
    folded == base
        || folded
            .strip_prefix(base.as_str())
            .and_then(|rest| rest.strip_prefix(" ("))
            .is_some_and(|rest| rest.ends_with(')'))
}

/// While no layout names this PC's device folder: whether the device folder `name` (`None`: its name does not open
/// here) may be this PC's own. Its manifest names this PC; or it names no device, and the folder has this PC's
/// device-folder name or a name that does not open.
///
/// `me` is this PC's device identity; `pc_name` its computer name, asked only without `me` (no machine id: macOS
/// until its id lands, a Linux without one). Without `me` no backup pass ran on this PC, so no manifest names it: a
/// manifest naming any device is another device's, and the name alone decides. Without either, nothing can be told
/// apart: every folder that does not name a device may be this PC's. Pure.
pub(crate) fn may_be_this_pcs(
    name: Option<&str>,
    owner: Owner,
    me: Option<&DeviceSelf>,
    pc_name: Option<&str>,
) -> bool {
    match owner {
        Owner::ThisPc => return true,
        Owner::Another => return false,
        Owner::Unknown => {}
    }
    let Some(name) = name else {
        return true;
    };
    match (me, pc_name) {
        (Some(me), _) => is_this_pcs_name(name, me),
        (None, Some(pc)) => is_this_pcs_computer_name(name, pc),
        (None, None) => true,
    }
}

/// This PC's computer name for [`may_be_this_pcs`]: `me`'s, else the OS's as `device_self` names it.
pub(crate) fn this_pc_name(identity: &crate::identity::EngineIdentity, me: Option<&DeviceSelf>) -> Option<String> {
    me.map(|me| me.name.clone())
        .or_else(|| crate::backup::device::device_name_for(identity))
}

/// Two server ids for the same item (as UUIDs when both are).
fn same_item(a: &str, b: &str) -> bool {
    match (uuid::Uuid::parse_str(a), uuid::Uuid::parse_str(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The refusal for `folder_id` when it is, or may be, this PC's own device folder; `None` otherwise (the rest is the
/// Backups rule's, in `backup_delete`). With a layout: its `device_folder_id`. Without one: a device folder (a row
/// directly under a Backups root) is read first ([`read_device`] over `tree`) and refused when it
/// [`may_be_this_pcs`]. A local state that cannot be read refuses (a read error never lifts a protection).
pub(crate) async fn own_folder_refusal<T: DeviceTree>(
    db: &StateDb,
    tree: &T,
    folder_id: &str,
    me: Option<&DeviceSelf>,
    pc_name: Option<&str>,
) -> Option<DeleteBackupError> {
    let state = || Some(DeleteBackupError::Failed("state"));
    match db.backup_layout() {
        Err(_) => return state(),
        Ok(Some(layout)) => {
            return same_item(&layout.device_folder_id, folder_id).then(|| DeleteBackupError::Refused(THIS_PC.into()));
        }
        Ok(None) => {}
    }
    let entry = match db.get_file(folder_id) {
        Err(_) => return state(),
        // No such row: the rule answers it (nothing is sent).
        Ok(None) => return None,
        Ok(Some(entry)) => entry,
    };
    let Ok(roots) = db.backups_roots() else {
        return state();
    };
    let device_folder = entry
        .parent_id
        .as_deref()
        .is_some_and(|parent| roots.iter().any(|(root, _)| same_item(root, parent)));
    if !device_folder {
        return None;
    }
    let name = entry.path.rsplit('/').next().filter(|name| !name.is_empty());
    let (_, mine, manifest) = read_device(tree, folder_id, me).await;
    may_be_this_pcs(name, owner(mine, manifest.as_ref()), me, pc_name)
        .then(|| DeleteBackupError::Refused(THIS_PC.into()))
}

impl<C: CredentialStores> Engine<C> {
    /// The account's other devices' backups, for the session a person opened the section under (`session` is
    /// [`Engine::session_turn`] as the host read it). See the module's doc. BLOCKS: a worker's.
    pub fn backup_devices_for(&self, session: SessionGeneration) -> Result<BackupDevices, DevicesRefused> {
        // A session mutex a panic poisoned: nothing is sent.
        if self.acct.session.is_poisoned() {
            return Err(DevicesRefused::NotSignedIn);
        }
        // The token and the vault key leave the session's lock together, as copies wiped when this call ends, as a
        // person's call: a locked screen copies nothing.
        let (token, key) = match self.person_session(|current, generation| {
            (generation == session).then(|| (current.token.clone(), Zeroizing::new(current.master_key)))
        }) {
            Ok(Some(held)) => held,
            Err(refused) => {
                return Err(refused.said_as(
                    DevicesRefused::Local(LocalStateRefused::Locked),
                    DevicesRefused::NotSignedIn,
                ));
            }
            Ok(None) => return Err(DevicesRefused::NotSignedIn),
        };
        let state = self.local_state().map_err(DevicesRefused::Local)?;
        if state.session() != session {
            return Err(DevicesRefused::NotSignedIn);
        }
        let me = crate::backup::device::device_self(self.identity);
        let pc_name = this_pc_name(self.identity, me.as_ref());
        let read = state
            .with_shared(|db| {
                // The root: the layout's (this PC resolved it), else the one cached on its own, else the server's.
                let layout = db.backup_layout().ok().flatten();
                let this_pc = layout.as_ref().map(|layout| layout.device_folder_id.clone());
                let cached = match layout {
                    Some(layout) => Ok(Some(layout.root_id)),
                    None => db.backup_root(),
                };
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                self.runtime.block_on(async {
                    let root = match cached {
                        Ok(Some(root)) => Some(root),
                        // Never claimed or created here: the server's flagged root, or none.
                        Ok(None) => bridge.backup_root_flagged().await.map_err(|error| call_error(&error))?,
                        Err(_) => return Err(AccountCallError::ServerError),
                    };
                    match root {
                        Some(root) => {
                            read_devices(&bridge, &root, this_pc.as_deref(), me.as_ref(), pc_name.as_deref()).await
                        }
                        None => Ok((Vec::new(), true)),
                    }
                })
            })
            // The session moved while the read ran: nothing of it is handed over.
            .map_err(|_| DevicesRefused::NotSignedIn)?;
        let (devices, complete) = read.map_err(DevicesRefused::Call)?;
        Ok(BackupDevices {
            session,
            devices,
            complete,
        })
    }

    /// WRITES (a move to Trash): delete the backup `folder_id` after proving `password`, for the session a person
    /// pressed under and no other. Sent at most once. Any delete that reached Beebeeb asks the sync engine to take the
    /// server's list again. See the module's doc for the rule. BLOCKS: a worker's.
    pub fn delete_device_backup_for(
        &self,
        session: SessionGeneration,
        folder_id: &str,
        password: &[u8],
    ) -> Result<(), DeleteBackupError> {
        if self.acct.session.is_poisoned() {
            return Err(DeleteBackupError::SignedOut);
        }
        // The token and the vault key (for the read of a folder that may be this PC's) leave the session's lock
        // together, as a person's call: a locked screen copies nothing (as the Trash deletes, review of #424).
        let (token, key) = self
            .person_session(|current, generation| {
                (generation == session).then(|| {
                    (
                        Zeroizing::new(current.token.clone()),
                        Zeroizing::new(current.master_key),
                    )
                })
            })
            .ok()
            .flatten()
            .ok_or(DeleteBackupError::SignedOut)?;
        let state = self.local_state().map_err(|refused| match refused {
            LocalStateRefused::SessionChanged | LocalStateRefused::SignedOut | LocalStateRefused::SessionEnded => {
                DeleteBackupError::SignedOut
            }
            _ => DeleteBackupError::Failed("state"),
        })?;
        if state.session() != session {
            return Err(DeleteBackupError::SignedOut);
        }
        let base = self.config.api_base.trim_end_matches('/').to_string();
        let identity = self.identity;
        // What the delete answered, kept for a session that moves while it runs (`with_shared` refuses after it then).
        let mut answered = None;
        let deleted = state
            .with_shared(|db| {
                // What this PC backs up now, read fresh; a settings file that cannot be read keeps the runner's last
                // answer (a read error never lifts a protection).
                let own = crate::backup::own_backups(identity, &db).unwrap_or_else(crate::backup_protect::own_backups);
                let me = crate::backup::device::device_self(identity);
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token.to_string(), *key));
                let bridge = EngineBridge::new(identity, db.clone(), api);
                let answer = self.runtime.block_on(async {
                    // This PC's own device folder: refused before the password is proven.
                    let pc_name = this_pc_name(identity, me.as_ref());
                    if let Some(refused) =
                        own_folder_refusal(&db, &bridge, folder_id, me.as_ref(), pc_name.as_deref()).await
                    {
                        return Err(refused);
                    }
                    backup_delete::delete_backup(&base, &self.client, token.as_str(), &db, &own, folder_id, password)
                        .await
                });
                answered = Some(answer.clone());
                answer
            })
            // The session moved: before the delete ran, or while it ran. Its own answer is not handed over; what it may
            // have done is.
            .unwrap_or_else(|_| after_session_moved(answered.as_ref()));
        if sent(&deleted) {
            self.request_resnapshot();
        }
        deleted
    }
}

/// The answer of a delete whose session moved. Moved before the delete ran (`answered` is
/// `None`), or the delete was refused before it could move anything (the password, the rule, a backup not found):
/// signed out, and nothing changed is true. The delete may have moved the backup to Trash (done, or no answer, or the
/// server's error): unconfirmed (`Failed("server")`, the answer every caller reads as "may have reached Beebeeb": the
/// sync engine reads the list again, "Stop and move the backup to Trash" keeps the folder out of the set and says its
/// backup may have moved), never "signed out, nothing changed".
fn after_session_moved(answered: Option<&Result<(), DeleteBackupError>>) -> Result<(), DeleteBackupError> {
    match answered {
        Some(Ok(()) | Err(DeleteBackupError::Offline | DeleteBackupError::Failed("server"))) => {
            Err(DeleteBackupError::Failed("server"))
        }
        _ => Err(DeleteBackupError::SignedOut),
    }
}

/// Whether the delete may have reached Beebeeb (then the sync engine takes the server's list again).
fn sent(result: &Result<(), DeleteBackupError>) -> bool {
    match result {
        Ok(()) => true,
        Err(DeleteBackupError::Offline | DeleteBackupError::Failed("server") | DeleteBackupError::NotFound) => true,
        Err(_) => false,
    }
}
