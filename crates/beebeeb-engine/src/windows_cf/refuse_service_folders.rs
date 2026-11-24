//! Slice A9 (plan40 batch 1): moving, renaming or deleting Backups, or another folder a Beebeeb service keeps, in
//! File Explorer is refused by Cloud Files, and the person is told why (DW-1182; the same rule and words as the
//! window's DW-0425).
//!
//! Not built yet: [`check`] answers `None` for every item, so nothing more is refused (today's behaviour). The
//! pre-operation delete and rename callbacks already ask it (`callbacks_pre_op.rs`), only for an item this process
//! did not ask to change, that the Backups protection did not refuse, and only while the engine runs. A `Some` is
//! answered to Windows as refused and raises [`crate::host::Notice::ServiceFolderRefused`]
//! (`backup_protect::note_service_folder_refused`). A9 fills [`check`] here and edits no callbacks file.

#![cfg(target_os = "windows")]

use windows::Win32::Storage::CloudFilters::{CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS, CF_OPERATION_TYPE};

/// A refused move, rename or delete of a folder a Beebeeb service keeps. It carries no path and no name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal;

/// Whether `info`'s item is a folder a Beebeeb service keeps, which the operation `op` (Cloud Files' delete or rename
/// acknowledgement, `CF_OPERATION_TYPE_ACK_DELETE` / `CF_OPERATION_TYPE_ACK_RENAME`) may not change. `params` are the
/// callback's own (`None`: Windows gave none). Answers at once, from the state DB alone: Explorer waits for it, so
/// never the network. Nothing yet: `None`.
pub fn check(
    _db: &crate::state_db::StateDb,
    _sync_root: Option<&std::path::Path>,
    _info: &CF_CALLBACK_INFO,
    _params: Option<&CF_CALLBACK_PARAMETERS>,
    _op: CF_OPERATION_TYPE,
) -> Option<Refusal> {
    None
}
