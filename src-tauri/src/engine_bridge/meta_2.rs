//! `meta_2` items split out of `engine_bridge.rs`.

use super::*;

/// The identifiers a build before the server-version-led format reported for
/// this row: the content version and the full version identifier, both led by
/// `max(current_version, remote_updated_at)`. Used only to recognise them.
pub(super) fn legacy_item_identifiers(entry: &FileEntry, contract: &FileContractState) -> [String; 2] {
    let leading = contract.current_version.max(entry.remote_updated_at);
    [
        item_content_version(leading, entry.content_hash.as_deref()),
        item_version_identifier(leading, entry.modified_at, entry.size_bytes),
    ]
}

pub(crate) fn parse_base_version_number(version_identifier: Option<&str>) -> Option<i64> {
    version_identifier.and_then(|value| {
        value
            .split(':')
            .next()
            .and_then(|part| part.parse::<i64>().ok())
            .filter(|version| *version > 0)
    })
}

pub(super) fn operation_metadata(op: &PendingOperation) -> Result<serde_json::Value, EngineError> {
    Ok(op
        .metadata_json
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({})))
}

pub(super) fn shared_roots_from_invite_response(body: &serde_json::Value) -> Vec<SharedRootMapping> {
    body.get("invites")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(shared_root_from_invite)
        .collect()
}

pub(super) fn shared_invite_id_matches(invite: &serde_json::Value, share_id: &str) -> bool {
    string_field(invite, &["id", "invite_id"]).as_deref() == Some(share_id)
}

pub(super) fn shared_root_from_invite(invite: &serde_json::Value) -> Option<SharedRootMapping> {
    if invite.get("status").and_then(|value| value.as_str()) != Some("approved") {
        return None;
    }

    let invite_id = string_field(invite, &["id", "invite_id"])?;
    let file_id = string_field(invite, &["file_id"])?;
    let is_folder = invite
        .get("is_folder_share")
        .and_then(|value| value.as_bool())
        .or_else(|| invite.get("is_folder").and_then(|value| value.as_bool()))
        .unwrap_or(false);
    let item_kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
    let display_name = shared_placeholder_name(&item_kind).to_string();
    let content_type = string_field(invite, &["mime_type", "content_type"]);
    let size_bytes = invite.get("size_bytes").and_then(|value| value.as_i64()).unwrap_or(0);
    let mut permission_bits = PERMISSION_READ;
    if invite
        .get("can_reshare")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        permission_bits |= PERMISSION_SHARE;
    }
    if shared_invite_allows_write(invite) {
        permission_bits |= PERMISSION_WRITE;
    }

    Some(SharedRootMapping {
        invite_id,
        file_id,
        display_name,
        is_folder,
        size_bytes,
        content_type,
        owner_email: string_field(invite, &["sender_email", "owner_email", "shared_by"]),
        sender_public_key: string_field(invite, &["sender_public_key", "owner_public_key"]),
        encrypted_file_key: string_field(invite, &["encrypted_file_key", "wrapped_file_key"]),
        encrypted_folder_key: string_field(invite, &["encrypted_folder_key"]),
        file_name_encrypted: string_field(invite, &["file_name_encrypted", "name_encrypted"]),
        permission_bits,
        approved_at: invite
            .get("approved_at")
            .and_then(|value| value.as_str())
            .and_then(parse_rfc3339_secs),
    })
}

pub(super) fn shared_invite_allows_write(invite: &serde_json::Value) -> bool {
    if ["can_write", "can_edit", "editable"]
        .iter()
        .any(|key| invite.get(*key).and_then(|value| value.as_bool()).unwrap_or(false))
    {
        return true;
    }
    ["permission", "capability", "permissions", "role", "access"]
        .iter()
        .filter_map(|key| invite.get(*key).and_then(|value| value.as_str()))
        .any(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "write" | "edit" | "editable" | "editor" | "admin" | "owner"
            )
        })
}

pub(super) fn decode_x25519_public_key(label: &str, value: &str) -> Result<[u8; 32], EngineError> {
    let bytes = decode_standard_b64(label, value)?;
    if bytes.len() != 32 {
        return Err(anyhow::anyhow!("{label} must be 32 bytes, got {}", bytes.len()).into());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub(super) fn derive_recipient_share_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    file_id: &str,
) -> Result<Zeroizing<[u8; 32]>, EngineError> {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*recipient_master_key);
    let recipient_private = beebeeb_core::opaque::derive_x25519_private(&master_key);
    let sender_public = decode_x25519_public_key("sender_public_key", sender_public_key_b64)?;
    let shared_secret = beebeeb_core::opaque::x25519_shared_secret(&recipient_private, &sender_public)
        .map_err(|e| anyhow::anyhow!("derive X25519 shared secret: {e}"))?;
    Ok(beebeeb_core::opaque::derive_share_key(
        &shared_secret,
        file_id.as_bytes(),
    ))
}

pub(super) fn unwrap_key_frame(wrap_key: &[u8; 32], encrypted_key_b64: &str, label: &str) -> Result<Zeroizing<[u8; 32]>, EngineError> {
    let raw = decode_standard_b64(label, encrypted_key_b64)?;
    let frame_key = beebeeb_core::kdf::FileKey::from_bytes(*wrap_key);
    let plaintext = Zeroizing::new(
        beebeeb_core::encrypt::decrypt_chunk_raw(&frame_key, &raw)
            .map_err(|_| anyhow::anyhow!("{label} decrypt failed"))?,
    );
    if plaintext.len() != 32 {
        let len = plaintext.len();
        return Err(anyhow::anyhow!("{label} decrypted key must be 32 bytes, got {len}").into());
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&plaintext);
    Ok(out)
}

pub(super) fn unwrap_direct_shared_file_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    file_id: &str,
    encrypted_file_key_b64: &str,
) -> Result<beebeeb_core::kdf::FileKey, EngineError> {
    let share_key = derive_recipient_share_key(recipient_master_key, sender_public_key_b64, file_id)?;
    let file_key = unwrap_key_frame(&share_key, encrypted_file_key_b64, "encrypted_file_key")?;
    Ok(beebeeb_core::kdf::FileKey::from_bytes(*file_key))
}

pub(super) fn unwrap_folder_share_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    folder_id: &str,
    encrypted_folder_key_b64: &str,
) -> Result<Zeroizing<[u8; 32]>, EngineError> {
    let share_key = derive_recipient_share_key(recipient_master_key, sender_public_key_b64, folder_id)?;
    unwrap_key_frame(&share_key, encrypted_folder_key_b64, "encrypted_folder_key")
}

pub(super) fn encrypted_file_key_from_folder_keys_response<'a>(
    folder_keys_response: &'a serde_json::Value,
    file_id: &str,
) -> Option<&'a str> {
    folder_keys_response
        .get("keys")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .find(|entry| entry.get("file_id").and_then(|value| value.as_str()) == Some(file_id))
        .and_then(|entry| entry.get("encrypted_file_key").and_then(|value| value.as_str()))
}

/// Which way a folder's child key is unwrapped: under the folder's custody (the owner L6
/// envelope) or with the invite's own wrap. The kind, not a bare flag, at every call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyCustody {
    Custody,
    Invite,
}

impl KeyCustody {
    /// The folder-keys record's `desktop_has_custody` says which wrap this device holds.
    pub(super) fn from_folder_keys(folder_keys: &serde_json::Value) -> Self {
        if folder_keys["desktop_has_custody"].as_bool() == Some(true) {
            KeyCustody::Custody
        } else {
            KeyCustody::Invite
        }
    }

    pub(super) fn is_custody(self) -> bool {
        matches!(self, KeyCustody::Custody)
    }
}

pub(super) fn unwrap_folder_child_for_format(folder: &[u8;32], root: &str, id: &str, kind: u8, wrap: &str, custody: KeyCustody) -> Result<beebeeb_core::kdf::FileKey, EngineError> {
    if custody.is_custody() {
        let frame = base64::engine::general_purpose::STANDARD.decode(wrap)?;
        Ok(beebeeb_sync::key_resolution::unwrap_child_key(
            &beebeeb_core::kdf::FileKey::from_bytes(*folder), root, id, kind, &frame)?)
    } else { unwrap_child_file_key(folder, wrap) }
}

pub(super) fn unwrap_child_file_key(
    folder_key: &[u8; 32],
    encrypted_file_key_b64: &str,
) -> Result<beebeeb_core::kdf::FileKey, EngineError> {
    #[cfg(test)]
    LEGACY_CHILD_READS.with(|n| n.set(n.get()+1));
    let file_key = unwrap_key_frame(folder_key, encrypted_file_key_b64, "encrypted_file_key")?;
    Ok(beebeeb_core::kdf::FileKey::from_bytes(*file_key))
}

#[cfg(test)]
pub(super) fn unwrap_folder_share_file_key(
    recipient_master_key: &[u8; 32],
    sender_public_key_b64: &str,
    folder_id: &str,
    encrypted_folder_key_b64: &str,
    file_id: &str,
    folder_keys_response: &serde_json::Value,
) -> Result<beebeeb_core::kdf::FileKey, EngineError> {
    let folder_key = unwrap_folder_share_key(
        recipient_master_key,
        sender_public_key_b64,
        folder_id,
        encrypted_folder_key_b64,
    )?;
    let encrypted_file_key = encrypted_file_key_from_folder_keys_response(folder_keys_response, file_id)
        .ok_or_else(|| anyhow::anyhow!("folder share is missing encrypted_file_key for {file_id}"))?;
    unwrap_child_file_key(&folder_key, encrypted_file_key)
}

pub(super) fn folder_keys_map(folder_keys_response: &serde_json::Value) -> HashMap<String, String> {
    folder_keys_response
        .get("keys")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.get("file_id")?.as_str()?.to_string(),
                entry.get("encrypted_file_key")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

pub(super) fn item_kind_from_metadata(f: &serde_json::Value) -> ItemKind {
    if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    }
}

pub(super) fn shared_placeholder_name(item_kind: &ItemKind) -> &'static str {
    match item_kind {
        ItemKind::Folder => "Encrypted folder",
        ItemKind::File => "Encrypted file",
    }
}

/// Task 2413: a shared item's decrypted name, trimmed as before. `Ok(Some(name))` when it is one name this host can
/// hold (the rule own names meet, [`crate::stored_path::check_name_for`]), `Ok(None)` when it is blank (the caller
/// shows the placeholder name, as for a name that does not decrypt), and `Err` when it breaks the rule: the row is
/// refused, as an own row is (several such names under the one placeholder name would share a path).
pub(super) fn shared_leaf_name(name: &str) -> Result<Option<String>, beebeeb_core::path_safety::UnsafeName> {
    let leaf = name.trim();
    if leaf.is_empty() {
        return Ok(None);
    }
    crate::stored_path::check_name_for(leaf, crate::stored_path::host_is_windows())?;
    Ok(Some(leaf.to_string()))
}

/// Task 2413: a server row the store refused. Logged by id and rule only (never a name); nothing is written, and a row
/// stored before stays exactly as it was. Always `None`: the row is neither applied nor reported as changed.
fn refused_server_row(file_id: &str, rule: &beebeeb_core::path_safety::UnsafeName) -> Option<FileEntry> {
    tracing::warn!(file_id = %file_id, rule = %rule, "server row not stored: a name on its path is not one this device can hold");
    None
}

pub(crate) fn decrypt_shared_name_with_key(file_key: &beebeeb_core::kdf::FileKey, name_encrypted: &str) -> Option<String> {
    beebeeb_core::encrypt::decrypt_file_metadata_with_key(file_key, name_encrypted).ok().map(|m| m.name)
}

/// A shared row's own name: its decrypted name, or the placeholder name when it does not decrypt (or is blank).
/// Task 2413: `Err` when the decrypted name is not one name this host can hold ([`shared_leaf_name`]).
pub(super) fn shared_name_from_metadata(
    f: &serde_json::Value,
    item_kind: &ItemKind,
    file_key: Option<&beebeeb_core::kdf::FileKey>,
) -> Result<String, beebeeb_core::path_safety::UnsafeName> {
    if let (Some(key), Some(name_encrypted)) = (file_key, f["name_encrypted"].as_str()) {
        if let Some(name) = decrypt_shared_name_with_key(key, name_encrypted) {
            if let Some(leaf) = shared_leaf_name(&name)? {
                return Ok(leaf);
            }
        }
    }

    Ok(shared_placeholder_name(item_kind).to_string())
}

pub(super) fn apply_shared_metadata_file_row(
    db: &StateDb,
    f: &serde_json::Value,
    root: &SharedRootMapping,
    now: i64,
    parent_id: Option<String>,
    parent_rel_path: &str,
    file_key: Option<&beebeeb_core::kdf::FileKey>,
) -> Result<Option<FileEntry>, EngineError> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }

    let existing = db.get_file(file_id)?;
    let size = f["size_bytes"].as_i64().or_else(|| f["size"].as_i64()).unwrap_or(0);
    let remote_updated = json_timestamp_secs(f, &["updated_at", "uploaded_at", "created_at"]).unwrap_or(now);
    let item_kind = item_kind_from_metadata(f);
    // Task 2413: the leaf is one name (or the placeholder name), and the store checks every segment of the path. A
    // refused row is skipped, never an error that would stop the rest of the folder's refresh.
    let path = match shared_name_from_metadata(f, &item_kind, file_key)
        .and_then(|leaf| crate::stored_path::compose(parent_rel_path, &leaf))
    {
        Ok(path) => path,
        Err(rule) => return Ok(refused_server_row(file_id, &rule)),
    };
    let status = existing
        .as_ref()
        .map(|entry| entry.status.clone())
        .unwrap_or(FileStatus::CloudOnly);

    let entry = FileEntry {
        file_id: file_id.to_string(),
        path,
        status,
        size_bytes: size,
        modified_at: remote_updated,
        content_hash: existing.as_ref().and_then(|entry| entry.content_hash.clone()),
        remote_updated_at: remote_updated,
        parent_id: parent_id.clone(),
        item_kind: item_kind.clone(),
    };
    // Spec §7.2 (final review): the row and its contract in one transaction, so no listing
    // sees a new shared row in the person's own namespace.
    let written = db.upsert_shared_server_row(&entry, |contract| {
        contract.namespace = Namespace::SharedWithMe;
        contract.parent_id = parent_id;
        contract.shared_root_id = Some(root.file_id.clone());
        contract.share_id = Some(root.invite_id.clone());
        contract.owner_email = root.owner_email.clone();
        contract.permission_bits = root.permission_bits;
        contract.item_kind = item_kind;
        contract.content_type = f["content_type"]
            .as_str()
            .or_else(|| f["mime_type"].as_str())
            .map(str::to_string);
        contract.current_version = f["current_version"]
            .as_i64()
            .or_else(|| f["version"].as_i64())
            .or_else(|| f["version_number"].as_i64())
            .unwrap_or(contract.current_version);
        contract.current_object_version_id = f["current_object_version_id"]
            .as_str()
            .or_else(|| f["object_version_id"].as_str())
            .map(str::to_string)
            .or(contract.current_object_version_id.take());
        contract.last_sync_at = now;
    })?;
    if let crate::stored_path::ServerRowWrite::Refused(rule) = written {
        return Ok(refused_server_row(file_id, &rule));
    }
    Ok(Some(entry))
}

pub(super) fn apply_shared_context(metadata: &mut serde_json::Value, contract: Option<&FileContractState>) {
    let Some(contract) = contract else {
        return;
    };
    metadata["shared_root_id"] = serde_json::json!(contract.shared_root_id);
    metadata["share_id"] = serde_json::json!(contract.share_id);
    metadata["permission_bits"] = serde_json::json!(contract.permission_bits);
    metadata["uploaded_by"] = serde_json::json!("authenticated_desktop_user");
}

pub(super) fn apply_metadata_file_row(
    db: &StateDb,
    f: &serde_json::Value,
    namespace: Namespace,
    shared_root_id: Option<String>,
    share_id: Option<String>,
    permission_bits: i64,
    now: i64,
    master_key: &[u8; 32],
    // Server-relative path of the PARENT directory ("" at the vault root).
    // The decrypted leaf name is composed under it so nested files/folders
    // get a full `<parent>/<leaf>` path that round-trips with the upload
    // watcher's `relative_db_path` (also '/'-joined, no leading slash).
    parent_rel_path: &str,
) -> Result<Option<FileEntry>, EngineError> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }

    let existing = db.get_file(file_id)?;
    let size = f["size_bytes"].as_i64().or_else(|| f["size"].as_i64()).unwrap_or(0);
    let remote_updated = f["updated_at"].as_i64().unwrap_or(0);
    // Task 2413: a refused own row is remembered, so an op under it waits without asking for a re-snapshot.
    let refuse = |rule: beebeeb_core::path_safety::UnsafeName| -> Result<Option<FileEntry>, EngineError> {
        db.note_refused_server_row(file_id)?;
        Ok(refused_server_row(file_id, &rule))
    };
    let leaf = match crate::owner_keys::stored_name(db, file_id, master_key)? {
        Some(name) => name,
        None => match resolve_relative_path(f, file_id, master_key) {
            Ok(name) => name,
            Err(rule) => return refuse(rule),
        },
    };
    // Compose the nested path. An empty leaf means the name didn't decrypt;
    // keep it empty so the placeholder seeder skips the row (and retries next
    // tick) rather than seeding a bare parent dir. Task 2413: the leaf must be
    // ONE name and every segment of the path one this host can hold (the store
    // checks the path), or the row is refused and nothing about it is written (a
    // stored row stays as it was).
    let path = match crate::stored_path::compose(parent_rel_path, &leaf) {
        Ok(path) => path,
        Err(rule) => return refuse(rule),
    };
    // Task 2204 S0.3: while a Windows rename of this file is queued (parked included), the row's
    // path and parent are where the file is on disk; the server's are still the old ones.
    let held = match existing.as_ref() {
        // Only `handle_rename` (Windows) queues these ops, so other builds skip the query.
        Some(entry) if cfg!(any(target_os = "windows", test)) && !entry.is_dir() && db.has_queued_local_rename(file_id)? => {
            Some(entry.clone())
        }
        _ => None,
    };
    let path = held.as_ref().map_or(path, |entry| entry.path.clone());
    let status = existing
        .as_ref()
        .map(|entry| entry.status.clone())
        .unwrap_or(FileStatus::CloudOnly);
    // Task 1640 (review N1, condition 1): a remote move or rename gives the row another place;
    // the fetch candidate described the entry at the old one.
    let moved = existing.as_ref().is_some_and(|entry| entry.path != path);

    let item_kind = if f["is_folder"].as_bool().unwrap_or(false)
        || f["kind"].as_str() == Some("folder")
        || f["type"].as_str() == Some("folder")
    {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    let is_file = item_kind == ItemKind::File;
    let server_parent_id = match held.as_ref() {
        Some(entry) => entry.parent_id.clone(),
        None => f["parent_id"].as_str().map(str::to_string),
    };

    let mut entry = FileEntry {
        file_id: file_id.to_string(),
        path,
        status,
        size_bytes: size,
        modified_at: remote_updated,
        content_hash: existing.as_ref().and_then(|entry| entry.content_hash.clone()),
        remote_updated_at: remote_updated,
        // NOTE: parent_id/item_kind are NOT persisted by upsert_file — the
        // contract write in the same transaction (below) writes the
        // authoritative `files.parent_id` / `files.item_kind` columns. These
        // struct fields exist so the in-memory `FileEntry` returned to callers
        // (notably the windows_cf placeholder seeder reading via list_by_status)
        // carries the correct folder/parent classification.
        parent_id: server_parent_id.clone(),
        item_kind: item_kind.clone(),
    };
    // Spec §7.2 (final review): the row and its contract in one transaction, so no listing
    // sees a new nested row without its parent.
    // Task 2204 S0.3 (review-155): the hold is decided again inside the write's own
    // transaction, so a Windows rename committed after the reads above is never written over.
    let written = db.upsert_server_file_row(&entry, |contract| {
        contract.namespace = namespace;
        contract.parent_id = server_parent_id;
        contract.shared_root_id = shared_root_id;
        contract.share_id = share_id;
        contract.permission_bits = permission_bits;
        contract.item_kind = item_kind;
        contract.content_type = f["content_type"]
            .as_str()
            .or_else(|| f["mime_type"].as_str())
            .map(str::to_string);
        contract.current_version = f["current_version"]
            .as_i64()
            .or_else(|| f["version"].as_i64())
            .or_else(|| f["version_number"].as_i64())
            .unwrap_or(contract.current_version);
        contract.current_object_version_id = f["current_object_version_id"]
            .as_str()
            .or_else(|| f["object_version_id"].as_str())
            .map(str::to_string)
            .or(contract.current_object_version_id.take());
        // Task 2190 (plan §7.1 E3): a `Local` row's base is never invented from the server's
        // version. 0 stays unknown; an edit on an unknown base is held, not sent (task 2229).
        contract.last_sync_at = now;
    })?;
    entry.path = match written {
        crate::stored_path::ServerRowWrite::Written(path) => path,
        crate::stored_path::ServerRowWrite::Refused(rule) => return Ok(refused_server_row(file_id, &rule)),
    };
    apply_system_role_field(db, file_id, f)?;
    // Task 1640 (review F1): a move changes no byte, but the candidate is never promoted at the
    // new place: it becomes mixed, its version kept.
    if moved {
        db.poison_fetched_content(file_id)?;
    }
    // Task 2191 (D12, review-114 F-P1-7, R2-P2-2): a remote rename or move leaves this file's
    // placeholder at the old path (T9), and the new path may still hold another file's. Both
    // places are recorded for the tick's retry (Windows), which keeps one only when the identity
    // there names a file of unknown content living elsewhere (`unknown_check::verify_orphan`).
    if moved && cfg!(any(target_os = "windows", test)) && is_file {
        if let Some(previous) = existing.as_ref() {
            db.put_unknown_orphan(&previous.path, file_id, true)?;
        }
        db.put_unknown_orphan(&entry.path, file_id, true)?;
    }
    Ok(Some(entry))
}

/// LD-0113 (spec §5.4): a server row's `system_role` (`"backups_root"` or `null`) into
/// `files.system_role`. A row that does not carry the field leaves the stored role as it is: an older
/// server never sends it (so a new row stays `None`), and the rows this desktop builds itself from a
/// sync op or a parent's rename (`synthesize_op_row`, `rebase_folder_descendants`) carry only the
/// fields that op changed. An explicit `null`, or a role this build does not know, clears it. The
/// `folder_system_role` sync op sets it through [`super::syncflow`].
pub(super) fn apply_system_role_field(db: &StateDb, file_id: &str, f: &serde_json::Value) -> Result<(), EngineError> {
    if let Some(raw) = f.get("system_role") {
        let role = raw.as_str().and_then(beebeeb_types::backup::SystemRole::parse);
        if db.set_system_role(file_id, role)? {
            // Zero-knowledge: the id and the role's wire string, never a name.
            tracing::info!(file_id = %file_id, role = role.map_or("none", |r| r.as_str()), "system role of a folder changed");
            crate::backup_protect::system_role_changed(db, file_id);
        }
    }
    Ok(())
}

/// Task 1640 (review F3): whether `dir` holds only what this PC keeps from a deletion elsewhere:
/// originals that still wait to be removed or whose entry was dismissed (review-151d P2-1),
/// folders that themselves hold only that, and names the sync folder ignores (an Office owner
/// file). Anything else is the person's, and the folder is wanted again. Fails closed (review-151d
/// P3-5): what cannot be read, and a tree deeper than 32, count as held, so a folder the server
/// removed is never created again on an error.
pub(super) fn held_only_for_kept(db: &StateDb, sync_root: &Path, dir: &Path, depth: usize) -> bool {
    if depth >= 32 {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return true;
        };
        let path = entry.path();
        if entry.file_name().to_str().is_some_and(is_ignored_finder_name) {
            continue;
        }
        let Some(rel) = relative_db_path(sync_root, &path) else {
            return true;
        };
        let held = if entry.file_type().map_or(true, |kind| kind.is_dir()) {
            db.is_removed_folder(&rel).unwrap_or(true) && held_only_for_kept(db, sync_root, &path, depth + 1)
        } else {
            db.is_kept_original(&rel).unwrap_or(true)
        };
        if !held {
            return false;
        }
    }
    true
}

/// Apply one server metadata row during [`sync_tick`]'s recursive walk,
/// running the same three-way decision the flat sweep used (new → cloud_only;
/// local + remote-moved → conflict check; otherwise refresh metadata). On
/// success returns `Some((resolved_rel_path, item_kind))` so the BFS can
/// recurse into folders using the row's full nested path; returns `None` only
/// when the row is unusable (empty id / undecryptable name).
pub(super) fn process_metadata_row(
    bridge: &EngineBridge,
    f: &serde_json::Value,
    parent_rel_path: &str,
    now_secs: i64,
    source: RowSource,
    // Task 2190 (plan §7.1 E1): the detector reports no `ConflictDetected`; a concurrent edit is
    // the parked op. Kept so the tick's conflict plumbing stays as it is.
    _conflicts: &mut Vec<ConflictDetected>,
) -> Result<Option<(String, ItemKind)>, EngineError> {
    let file_id = f["id"].as_str().unwrap_or_default();
    if file_id.is_empty() {
        return Ok(None);
    }
    let remote_updated = f["updated_at"].as_i64().unwrap_or(0);

    // §6.3.2: a snapshot node fills a row at version 0, or raises an older one, in any
    // status but Trashing and Conflict. Only the version (and the size) changes here; the
    // short-circuit below still holds for every other field.
    #[cfg(target_os = "macos")]
    if source == RowSource::Snapshot
        && let Some(node_version) = f["version_number"].as_i64()
    {
        let is_uploading = f["is_uploading"].as_bool() == Some(true);
        let size = f["size_bytes"].as_i64().unwrap_or(0);
        match bridge
            .db()
            .apply_snapshot_version(file_id, node_version, size, is_uploading)?
        {
            crate::state_db::SnapshotVersion::Raised { old, new } => tracing::warn!(
                file_id = %file_id,
                old_version = old,
                new_version = new,
                "file version raised from the snapshot"
            ),
            crate::state_db::SnapshotVersion::Filled { resolved_ops } => {
                for op_id in resolved_ops {
                    tracing::warn!(
                        op_id = %op_id,
                        file_id = %file_id,
                        base_version = node_version,
                        "queued write based on the version the snapshot reported"
                    );
                }
            }
            crate::state_db::SnapshotVersion::Unchanged | crate::state_db::SnapshotVersion::SkippedUploading => {}
        }
    }

    // Helper to refresh metadata + return the resolved path/kind. The single
    // place that writes the row's nested path and folder/file classification.
    let apply = |bridge: &EngineBridge| -> Result<Option<(String, ItemKind)>, EngineError> {
        let entry = apply_metadata_file_row(
            bridge.db(),
            f,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            now_secs,
            bridge.api().master_key(),
            parent_rel_path,
        )?;
        Ok(entry.map(|e| (e.path, e.item_kind)))
    };

    match bridge.db().get_file(file_id)? {
        // (0) Locally-deleted, server-trash pending (task 0802). The row is in
        // `Trashing` because the user deleted the file and the `TrashFile` op has
        // already succeeded (op removed) OR is still in flight. A `/sync/snapshot`
        // that STILL lists this file is just propagation lag — the server has set
        // `is_trashed=TRUE` but the snapshot read hasn't caught up. We MUST NOT
        // re-materialise the row: neither re-insert it nor flip it back to
        // `CloudOnly`, which would re-mint the placeholder (the "deleted file comes
        // back" race). Keep it EXACTLY as-is (`Trashing`, no placeholder). When the
        // trash finally propagates the file drops out of the snapshot and
        // `prune_absent` removes the row; if the `file_trash` op echo arrives first
        // `apply_sync_op` deletes it. Either way it converges WITHOUT resurrecting.
        Some(entry) if entry.status == FileStatus::Trashing => {
            // Still report path/kind so a (rare) trashing folder's independent
            // children continue to enumerate; do NOT touch this row's status.
            Ok(Some((entry.path, entry.item_kind)))
        }
        // (1a) New to us but still an in-progress upload (`is_uploading`): it
        // has no completed content yet — hydrating it would 409 — so minting a
        // placeholder shows a broken duplicate in Finder/Explorer (flow 7: an
        // interrupted upload's orphan row). Skip it; once `complete` lands the
        // row is listed with `is_uploading = false` and materialises normally.
        None if f["is_uploading"].as_bool() == Some(true) => Ok(None),
        None => {
            // (1) New to us — insert as cloud_only. base = remote.
            apply(bridge)
        }
        Some(entry) if entry.status == FileStatus::Local => {
            // (2) Local copy + remote moved? Nothing to check if the
            // timestamps haven't drifted past base. Still report the row's
            // path/kind so a folder we already have locally is still descended.
            // Snapshot rows only: an op row's `updated_at` is a local
            // same-second stamp, not a freshness token (see `RowSource::Op`).
            if source == RowSource::Snapshot && remote_updated <= entry.remote_updated_at {
                return Ok(Some((entry.path, entry.item_kind)));
            }

            // Task 2190 (follow-up 1, plan §7.2 as corrected after review-169 #7): a `Local` row has no
            // pending write (a queued or parked one makes the row `Uploading` or `Error`, branch 3), so
            // the pull re-anchors it to the server's version. An edit on disk that the scan has not
            // queued yet keeps its base (1640's agreed content, which this write never touches), is
            // queued on it, refused as stale and parked: the parked op is the conflict (E1). The pull
            // reads no file: `content_hash` is not read either (E2), and no hash is taken on a cold
            // cache just to choose a log line.
            // Task 2413: `apply` writes the size and both times from the row in its one transaction (it wrote them over
            // a separate write here before), so a row the store refuses keeps them as they were too.
            apply(bridge)
        }
        Some(entry) => {
            // (3) Pending download/upload/conflict/error — let the dedicated
            // path own its transitions, but still refresh cloud_only/downloading
            // metadata (and always report path/kind so folders are descended).
            if matches!(entry.status, FileStatus::CloudOnly | FileStatus::Downloading) {
                return apply(bridge);
            }
            Ok(Some((entry.path, entry.item_kind)))
        }
    }
}
