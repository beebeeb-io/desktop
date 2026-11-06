//! `listing` items split out of `ipc_socket.rs`.

use super::*;

/// The change-feed item assembly, extracted from the `ListChanges` arm so
/// tests can drive it without a live socket. `Deleted` rows are gone; the
/// replica only needs the identifier. For every other kind the FULL item
/// payload rides along so `didUpdateItems` needs no second round-trip. A row
/// that vanished since the change was recorded yields no item (the Swift
/// enumerator skips it; its later `deleted` row reports the exit).
pub(crate) fn file_provider_change_payloads(
    db: &crate::state_db::StateDb,
    changes: Vec<crate::state_db::FileChange>,
) -> Vec<FileProviderChangePayload> {
    let backups = crate::backup_protect::BackupsTree::read(db);
    changes
        .into_iter()
        .map(|change| {
            // LD-0113 (core's `is_hidden_backup_item`): a device folder's manifest is never listed. Its
            // change goes out as a delete, so a Finder that listed one before this build drops it.
            let hidden = !matches!(change.kind, crate::state_db::FpChangeKind::Deleted)
                && db.get_file(&change.file_id).ok().flatten().is_some_and(|entry| backups.is_hidden(&entry.path));
            (change, hidden)
        })
        .map(|(change, hidden)| FileProviderChangePayload {
            item: match change.kind {
                _ if hidden => None,
                crate::state_db::FpChangeKind::Deleted => None,
                _ => db
                    .get_file(&change.file_id)
                    .ok()
                    .flatten()
                    .filter(|entry| {
                        // Task 1701 + Codex P1 (PR #99 review): shared content
                        // is webapp-only by ruling
                        // (.claude/tasks/decisions/finder-sidebar-single-root.md).
                        // A live `SharedWithMe` row has a nil contract parent,
                        // so its ride-along payload would fall back to
                        // FP_ROOT_APPLE; the Swift materialized filter treats a
                        // root parent as always materialized (and fails open
                        // while the set is unknown), which would surface shared
                        // files directly beneath "Beebeeb". Report the change
                        // row with NO item — the Swift enumerator skips
                        // payload-less rows, identical to a row that vanished.
                        db.get_file_contract_state(&entry.file_id)
                            .ok()
                            .flatten()
                            .map(|contract| contract.namespace != crate::state_db::Namespace::SharedWithMe)
                            .unwrap_or(true)
                    })
                    .map(|entry| file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE)),
            },
            file_id: change.file_id,
            kind: if hidden { crate::state_db::FpChangeKind::Deleted.as_str() } else { change.kind.as_str() }.to_string(),
            old_parent_id: change.old_parent_id,
            new_parent_id: change.new_parent_id,
        })
        .collect()
}

pub(crate) fn list_file_provider_items(
    db: &crate::state_db::StateDb,
    container_id: &str,
) -> Vec<FileProviderItemPayload> {
    // LD-0113 (core's `is_hidden_backup_item`): a device folder's manifest is never listed, in a folder
    // or in the trash.
    let backups = crate::backup_protect::BackupsTree::read(db);
    // Task 1698: the SYSTEM trash container lists the trash view — the
    // Trashing rows that are TOP of trash (their parent row is not itself
    // Trashing; the children of a trashed folder enumerate INSIDE that
    // folder, mirroring the server trash's shape).
    if is_trash_container(container_id) {
        return db
            .list_files()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.status == crate::state_db::FileStatus::Trashing && !parent_row_is_trashing(db, entry))
            .filter(|entry| !backups.is_hidden(&entry.path))
            .map(|entry| file_entry_payload_for_db(db, &entry, FP_TRASH_APPLE))
            .collect();
    }
    if is_file_provider_root(container_id) {
        // Task 1701 (ruling: `.claude/tasks/decisions/finder-sidebar-single-root.md`):
        // Finder shows ONE root folder — "Beebeeb", the user's own files,
        // matching the Windows app. The root container's children are the
        // user's REAL top-level tree; the synthetic `My files` /
        // `Shared with me` / `Offline` / `Conflicts` namespace containers are
        // no longer enumerated. Shared items are not a Finder surface for
        // now (webapp only); Offline/Conflicts data is untouched — it simply
        // stops being a synthetic Finder folder (native offline affordances
        // are task 1698, conflicts the dialog design pass).
        //
        // Task 1698: Trashing rows are excluded — a trashed item left "Beebeeb"
        // (the system reparented it into the trash container); surfacing it at
        // the root would duplicate it in Finder.
        //
        // Spec 2026-10-09 §7.2 (final review): the root lists the rows with no
        // parent, as a folder lists the rows whose parent it is, and as every
        // reply names the parent. A path with no folder in it decided this
        // before, so a Finder create or save that wrote its name alone as the
        // path was listed at the top level whatever its parent. A row with no
        // name (it did not decrypt) is still left out.
        return db
            .list_files()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.status != crate::state_db::FileStatus::Trashing)
            .filter(|entry| {
                entry.parent_id.is_none()
                    && !entry.path.trim_matches('/').is_empty()
                    && db
                        .get_file_contract_state(&entry.file_id)
                        .ok()
                        .flatten()
                        .map(|contract| contract.namespace == crate::state_db::Namespace::MyFiles)
                        .unwrap_or(true)
            })
            .map(|entry| file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE))
            .collect();
    }

    // Real folders: children by the contract's parent (the server's parent
    // UUID, which is what `files.parent_id` stores).
    //
    // Task 1698: Trashing children are listed only when the CONTAINER itself
    // is trashed (they are the trash view's folder contents); a live folder
    // stops listing children the moment they move to the trash container.
    let container_is_trashed = db
        .get_file(container_id)
        .ok()
        .flatten()
        .map(|entry| entry.status == crate::state_db::FileStatus::Trashing)
        .unwrap_or(false);
    db.list_files()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            db.get_file_contract_state(&entry.file_id)
                .ok()
                .flatten()
                .and_then(|contract| contract.parent_id)
                .as_deref()
                == Some(container_id)
        })
        .filter(move |entry| container_is_trashed || entry.status != crate::state_db::FileStatus::Trashing)
        .filter(|entry| !backups.is_hidden(&entry.path))
        .map(|entry| file_entry_payload_for_db(db, &entry, container_id))
        .collect()
}

/// Task 1698: is `entry`'s PARENT row itself in `Trashing`? A child inside a
/// trashed folder stays parented to the folder (trash-view content); a row
/// whose parent row is NOT trashed is TOP of trash — presented directly under
/// the trash container.
pub(super) fn parent_row_is_trashing(db: &crate::state_db::StateDb, entry: &crate::state_db::FileEntry) -> bool {
    entry
        .parent_id
        .as_deref()
        .and_then(|parent_id| db.get_file(parent_id).ok().flatten())
        .map(|parent| parent.status == crate::state_db::FileStatus::Trashing)
        .unwrap_or(false)
}

pub(super) fn is_trash_container(container_id: &str) -> bool {
    // FP_TRASH_APPLE or its short form; one definition, which the write accept reads too.
    crate::state_db::is_file_provider_trash(container_id)
}

/// Task 1694 rule (lead decision, 2026-10-01): every LIVE folder accepts
/// adding sub-items. Finder refuses a drop into any item whose
/// `.allowsAddingSubItems` is unset, which is why drops onto Beebeeb folders
/// showed the blocked icon everywhere. The grant is gated ONLY on kind +
/// status — deliberately NOT on `permission_bits`: a read-only shared root
/// is still a live folder, and the daemon rejects unauthorized writes at the
/// write path anyway. `Trashing` is the read-only terminal state (the item
/// is on its way out) and keeps the bit unset. Files never get the bit.
/// `capabilities_for_status` stays status-only; the folder grant is applied
/// at the payload construction sites where the kind is known.
pub(super) fn with_folder_add_subitems(kind: &str, status: &crate::state_db::FileStatus, capabilities: u32) -> u32 {
    if kind == "folder" && !matches!(status, crate::state_db::FileStatus::Trashing) {
        capabilities | CAP_ADD_SUBITEMS
    } else {
        capabilities
    }
}

pub(super) fn is_file_provider_root(container_id: &str) -> bool {
    container_id == FP_ROOT
        || container_id == FP_ROOT_APPLE
        || container_id == "rootContainer"
        || container_id.is_empty()
        || container_id.to_ascii_lowercase().contains("root")
}
