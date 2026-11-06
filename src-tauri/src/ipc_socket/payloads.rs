//! `payloads` items split out of `ipc_socket.rs`.

use super::*;

pub(crate) fn file_entry_payload_for_db(
    db: &crate::state_db::StateDb,
    entry: &crate::state_db::FileEntry,
    parent_identifier: &str,
) -> FileProviderItemPayload {
    // Spec §5.3: the row and the queue come from ONE read, so the predicate never
    // sees a row from before a landing beside a queue from after it.
    let read = db.item_presentation(&entry.file_id);
    builder_seam();
    // §9.1: a Finder upload that has not parked keeps the item writable and not evictable
    // (ITEM.h:531-533). A Trashing row keeps its trash presentation (m-8).
    let shown;
    let entry = match &read {
        Ok(Some(p)) if p.unparked_finder_upload && entry.status != crate::state_db::FileStatus::Trashing => {
            shown = crate::state_db::FileEntry {
                status: crate::state_db::FileStatus::Uploading,
                ..entry.clone()
            };
            &shown
        }
        _ => entry,
    };
    if read.is_ok() {
        presentation_read_ok(&mut presentation_warned(), &entry.file_id);
    }
    let mut payload = match &read {
        Ok(Some(p)) => file_entry_payload(entry, &p.contract, parent_identifier),
        Ok(None) => file_entry_payload_without_contract(entry, parent_identifier),
        Err(error) => {
            // A failed read (a SQLite error, or a held write without its base) is
            // presented as round 3 did, from the contract alone, with the capabilities
            // kept. When the row holds a write token, the numeric content version this
            // gives is not the one the system holds, so the system reads it as a content
            // change and fetches the item again (ITEM.h:93-95). That fetch is served from
            // the queued write's staged copy, or, once the write landed, from the server's
            // same bytes (spec §7.4): an extra fetch, never other bytes. The held columns
            // alone are not a fallback: without the queue they cannot tell a write that is
            // queued or landed here from one a remote change has replaced, and naming the
            // token for that one would hide the remote change. A fixed category only: the
            // error can carry a path (spec §11). Once per item and kind of failure per
            // interval, with how many were held back.
            let kind = crate::engine_bridge::error_category(error);
            let due = presentation_warn_due(
                &mut presentation_warned(),
                &entry.file_id,
                kind,
                std::time::Instant::now(),
            );
            if let Some(held_back) = due {
                tracing::warn!(
                    file_id = %entry.file_id,
                    category = "presentation_read_failed",
                    kind,
                    held_back,
                    "item presented from its contract alone"
                );
            }
            match db.get_file_contract_state(&entry.file_id).ok().flatten() {
                Some(contract) => file_entry_payload(entry, &contract, parent_identifier),
                None => file_entry_payload_without_contract(entry, parent_identifier),
            }
        }
    };
    if let Ok(Some(p)) = &read {
        // Rule 1: the bytes the system holds keep the name their write was given,
        // while that write is queued or is what the server holds now.
        match crate::write_token::held_token(
            p.held.as_ref(),
            p.held_write_queued,
            p.contract.current_version,
            p.contract.current_object_version_id.as_deref(),
        ) {
            Some(token) => payload.content_version = Some(token),
            None => {
                // Housekeeping only; correctness comes from the predicate. Guarded by
                // the write this read saw, so a token set since is never cleared.
                if let Some(held) = &p.held {
                    let _ = db.clear_held_if(&entry.file_id, &held.write_id);
                }
            }
        }
    }
    // Task 1697: real dates + child count, stamped where the DB is at hand.
    // `modified_at` conflates "server updated_at" and "conflict detected at"
    // (see the FileEntry doc comment) but is the best mtime the daemon has.
    payload.modified_at = Some(entry.modified_at.max(entry.remote_updated_at));
    if payload.created_at.is_none() {
        payload.created_at = payload.modified_at;
    }
    if entry.is_dir() {
        payload.child_item_count = Some(db.child_item_count(&entry.file_id).unwrap_or(0));
    }
    // Task 1698 (trash ruling): a Trashing row that is TOP of trash (its
    // parent row is not itself Trashing) is presented under the SYSTEM trash
    // container, wherever it used to live. Children of a trashed folder keep
    // their real parent so the trash view mirrors the server trash's shape.
    if entry.status == crate::state_db::FileStatus::Trashing && !parent_row_is_trashing(db, entry) {
        payload.parent_identifier = FP_TRASH_APPLE.to_string();
    }
    crate::backup_protect::with_own_backups(|own| apply_protection(db, own, entry, &mut payload));
    payload
}

/// LD-0113 §6.3, Amendment 6 ("Protect the whole tree"): every item at or under the Backups root loses
/// delete, trash, rename and move in Finder, and adding items (no New Folder, no drop into the tree),
/// and carries core's words for each, which the extension refuses with as a second line. Reading and
/// downloading stay. Their metadata version says so, so Finder reads the capabilities again when
/// protection starts or ends.
///
/// Amendment 7 (Guus, 19:30): a protected item is read-only too: it loses `CAP_WRITE` and carries the
/// words for a content change. Amendment 8a (20:04): the root carries the "Read-only" badge.
pub(super) fn apply_protection(
    db: &crate::state_db::StateDb,
    own: &crate::backup_protect::OwnBackups,
    entry: &crate::state_db::FileEntry,
    payload: &mut FileProviderItemPayload,
) {
    use beebeeb_core::backup::Mutation;
    let [Some(delete), rename, moved, written] = crate::backup_protect::protected_reasons(
        db,
        own,
        &entry.path,
        [Mutation::Trash, Mutation::Rename, Mutation::Move, Mutation::Write],
    ) else {
        return;
    };
    payload.protected = true;
    payload.capabilities &= !(CAP_DELETE | CAP_TRASH | CAP_RENAME | CAP_REPARENT | CAP_ADD_SUBITEMS | CAP_WRITE);
    payload.protected_reason = Some(delete);
    payload.protected_rename_reason = rename;
    payload.protected_move_reason = moved;
    payload.protected_write_reason = written;
    payload.read_only_badge =
        crate::backup_protect::BackupsTree::read(db).position(&entry.path) == beebeeb_core::backup::BackupPosition::Root;
    if let Some(version) = payload.metadata_version.as_mut() {
        version.push_str(":protected");
    }
}

pub(super) fn file_entry_payload(
    entry: &crate::state_db::FileEntry,
    contract: &crate::state_db::FileContractState,
    fallback_parent_identifier: &str,
) -> FileProviderItemPayload {
    let parent_identifier = contract.parent_id.as_deref().unwrap_or(fallback_parent_identifier);
    let kind = match contract.item_kind {
        crate::state_db::ItemKind::Folder => "folder",
        crate::state_db::ItemKind::File => "file",
    };
    let mut capabilities = capabilities_for_status(&entry.status);
    if contract.permission_bits != 0 {
        capabilities = 0;
        if contract.permission_bits & crate::state_db::PERMISSION_READ != 0 {
            capabilities |= CAP_READ;
        }
        if contract.permission_bits & crate::state_db::PERMISSION_WRITE != 0
            && matches!(
                entry.status,
                crate::state_db::FileStatus::Local
                    | crate::state_db::FileStatus::Uploading
                    | crate::state_db::FileStatus::Conflict
                    | crate::state_db::FileStatus::CloudOnly
            )
        {
            capabilities |= CAP_WRITE | CAP_RENAME | CAP_DELETE;
        }
    }
    // The contract branch zeroes `capabilities` above, so the folder
    // add-subitems grant is re-applied here, after the permission rebuild
    // (task 1694). Task 1697: live items may also be reparented and trashed
    // (drag-move / drag-to-Trash; trash semantics: task 1698 ruling).
    let capabilities = with_folder_add_subitems(kind, &entry.status, capabilities);

    // Task 1697: version split. contentVersion changes only when the content
    // identity changes (server version + content hash when present);
    // metadataVersion tracks mtime/size/parent/name so a rename no longer
    // forces a content re-download. Both identifiers lead with the server
    // version and never with `remote_updated_at`: the extension sends the
    // content version back as the base of its next write.
    let content_version =
        crate::engine_bridge::item_content_version(contract.current_version, entry.content_hash.as_deref());
    let metadata_version = format!(
        "{}:{}:{}:{}:{}",
        entry.modified_at,
        entry.size_bytes,
        parent_identifier,
        filename_from_path(&entry.path),
        entry.status.as_str()
    );

    FileProviderItemPayload {
        identifier: entry.file_id.clone(),
        parent_identifier: parent_identifier.to_string(),
        filename: filename_from_path(&entry.path),
        kind: kind.to_string(),
        size_bytes: entry.size_bytes,
        content_type: contract.content_type.clone(),
        status: file_status_string(&entry.status).to_string(),
        capabilities,
        version_identifier: Some(crate::engine_bridge::item_version_identifier(
            contract.current_version,
            entry.modified_at,
            entry.size_bytes,
        )),
        // Dates/child count are stamped by `file_entry_payload_for_db`, where
        // the DB handle is available.
        created_at: None,
        modified_at: None,
        child_item_count: None,
        content_version: Some(content_version),
        metadata_version: Some(metadata_version),
        pinned: matches!(contract.effective_pin_state(), crate::state_db::PinState::Pinned),
        // L12: the mark comes from the id's class, never from a server field.
        added_by_someone_else: beebeeb_core::item_id::is_foreign_item_id(&entry.file_id).unwrap_or(false),
        // LD-0113: `file_entry_payload_for_db` decides protection, where the DB is at hand.
        protected: false,
        protected_reason: None,
        protected_rename_reason: None,
        protected_move_reason: None,
        protected_write_reason: None,
        read_only_badge: false,
    }
}

pub(super) fn file_entry_payload_without_contract(
    entry: &crate::state_db::FileEntry,
    parent_identifier: &str,
) -> FileProviderItemPayload {
    let status = file_status_string(&entry.status);
    // Kind is derivable WITHOUT a contract: `FileEntry.item_kind` is read
    // from the `files.item_kind` column by the SELECT mappers (written only
    // by `set_file_contract_state`). It used to be hardcoded to "file",
    // which mis-reported folders AND lost them the folder add-subitems
    // grant (task 1694).
    let kind = match entry.item_kind {
        crate::state_db::ItemKind::Folder => "folder",
        crate::state_db::ItemKind::File => "file",
    };
    let capabilities = with_folder_add_subitems(kind, &entry.status, capabilities_for_status(&entry.status));

    // Task 1697: version split without a contract. With no contract there is
    // no known server version, so both identifiers lead with 0, which the
    // write path reads as "no base" (`parse_base_version_number` ignores 0).
    // The row's wall-clock stamp is not a version and is never used as one.
    let content_version = crate::engine_bridge::item_content_version(0, None);
    let metadata_version = format!(
        "{}:{}:{}:{}:{}",
        entry.modified_at,
        entry.size_bytes,
        parent_identifier,
        filename_from_path(&entry.path),
        status
    );

    FileProviderItemPayload {
        identifier: entry.file_id.clone(),
        parent_identifier: parent_identifier.to_string(),
        filename: filename_from_path(&entry.path),
        kind: kind.to_string(),
        size_bytes: entry.size_bytes,
        content_type: None,
        status: status.to_string(),
        capabilities,
        version_identifier: Some(crate::engine_bridge::item_version_identifier(
            0,
            entry.modified_at,
            entry.size_bytes,
        )),
        created_at: None,
        modified_at: None,
        child_item_count: None,
        content_version: Some(content_version),
        metadata_version: Some(metadata_version),
        // No contract = no pin state to resolve (fail-safe: unpinned).
        pinned: false,
        // L12: the mark comes from the id's class, never from a server field.
        added_by_someone_else: beebeeb_core::item_id::is_foreign_item_id(&entry.file_id).unwrap_or(false),
        // LD-0113: `file_entry_payload_for_db` decides protection, where the DB is at hand.
        protected: false,
        protected_reason: None,
        protected_rename_reason: None,
        protected_move_reason: None,
        protected_write_reason: None,
        read_only_badge: false,
    }
}

pub(super) fn capabilities_for_status(status: &crate::state_db::FileStatus) -> u32 {
    match status {
        // `Trashing` (a locally-deleted file whose server-trash is pending) gets
        // no write/rename/delete caps — the item is on its way out; it is grouped
        // with the other read-only terminal states.
        crate::state_db::FileStatus::CloudOnly
        | crate::state_db::FileStatus::Downloading
        | crate::state_db::FileStatus::Error
        | crate::state_db::FileStatus::Trashing => CAP_READ,
        crate::state_db::FileStatus::Conflict => CAP_READ | CAP_RENAME,
        // Task 1697: live items may be reparented (drag-move) and trashed.
        // Trash semantics: task 1698 ruling (the trash-container reparent maps
        // to the server trash op); the capability only stops Finder from
        // blocking the gesture outright.
        crate::state_db::FileStatus::Local | crate::state_db::FileStatus::Uploading => {
            CAP_READ | CAP_WRITE | CAP_RENAME | CAP_DELETE | CAP_REPARENT | CAP_TRASH
        }
    }
}

pub(super) fn file_status_string(status: &crate::state_db::FileStatus) -> &'static str {
    match status {
        crate::state_db::FileStatus::CloudOnly => "cloud_only",
        crate::state_db::FileStatus::Downloading => "downloading",
        crate::state_db::FileStatus::Local => "local",
        crate::state_db::FileStatus::Uploading => "uploading",
        crate::state_db::FileStatus::Conflict => "conflict",
        crate::state_db::FileStatus::Error => "error",
        crate::state_db::FileStatus::Trashing => "trashing",
    }
}

pub(super) fn filename_from_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}
