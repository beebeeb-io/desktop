//! The answer of the pre-operation delete and rename callbacks (`callbacks.rs`, LD-0113 §6.3), and what the person
//! is told when one is refused. Part of `callbacks.rs`.
//!
//! Refused, in this order: an item the Backups protection refuses (`delete_refusal` / `rename_refusal`: its words
//! become a notification, `backup_protect::note_refusal`); else, slice A9, a folder a Beebeeb service keeps
//! ([`super::super::refuse_service_folders::check`], which answers `None` until A9 fills it: then nothing changes),
//! raising `Notice::ServiceFolderRefused`. Never this process's own delete or rename, and never without a live
//! bridge (a start or a stop in progress): the answer is then STATUS_SUCCESS, as it always was.

#![cfg(target_os = "windows")]

use windows::Win32::Storage::CloudFilters::{CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS, CF_OPERATION_TYPE};

/// Answer Windows for `op` (`CF_OPERATION_TYPE_ACK_DELETE` or `CF_OPERATION_TYPE_ACK_RENAME`): refused when
/// `refusal` holds the Backups protection's words or a service folder is refused, else allowed; then tell the person.
///
/// SAFETY: `info` must be the live callback's info and `params` its parameters (or null), as for
/// [`super::ack_pre_operation`].
pub(super) unsafe fn answer(
    info: &CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
    op: CF_OPERATION_TYPE,
    refusal: Option<String>,
) {
    let service = if refusal.is_none() && !super::from_this_process(info) {
        super::super::callback_bridge(info.CallbackContext).and_then(|lease| {
            // SAFETY: valid for the duration of the callback, like `info`.
            let params = (!params.is_null()).then(|| unsafe { &*params });
            let root = super::super::sync_root();
            super::super::refuse_service_folders::check(lease.db(), root.as_deref(), info, params, op)
        })
    } else {
        None
    };
    // SAFETY: the keys come from this live callback.
    unsafe { super::ack_pre_operation(info, op, refusal.is_some() || service.is_some()) };
    if let Some(words) = refusal {
        crate::backup_protect::note_refusal(&words);
    }
    if service.is_some() {
        crate::backup_protect::note_service_folder_refused();
    }
}
