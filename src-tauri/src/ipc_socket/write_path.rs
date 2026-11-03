//! `write_path` items split out of `ipc_socket.rs`.

use super::*;

/// Fixed category for an engine error on the write path, for
/// [`log_refused_write`]. The error's text can carry user data (names, paths),
/// so it is classified, never logged.
pub(super) fn write_refusal_category(error: &anyhow::Error) -> &'static str {
    if crate::engine_bridge::engine_cause::<crate::engine_bridge::UnknownItem>(error).is_some() {
        return "unknown_item";
    }
    if crate::engine_bridge::engine_cause::<crate::engine_bridge::FinderStagingUnavailable>(error).is_some() {
        return "staging_root_unavailable";
    }
    if crate::engine_bridge::engine_cause::<crate::engine_bridge::StagedCopyGone>(error).is_some() {
        return "staged_copy_gone";
    }
    if crate::engine_bridge::engine_cause::<crate::engine_bridge::AcceptsClosed>(error).is_some() {
        return "accepts_closed";
    }
    if crate::engine_bridge::engine_cause::<crate::engine_bridge::ItemInTrash>(error).is_some() {
        return "in_trash";
    }
    if crate::engine_bridge::engine_cause::<std::io::Error>(error).is_some() {
        return "io";
    }
    if crate::engine_bridge::engine_cause::<rusqlite::Error>(error).is_some() {
        return "database";
    }
    let text = error.to_string();
    if text.contains("engine is stopping") {
        "engine_stopping"
    } else if text.contains("Upload source is not a file") {
        "contents_not_a_file"
    } else if text.contains("read-only") {
        "write_policy"
    } else if text.contains("did not include") {
        "malformed_request"
    } else {
        "other"
    }
}

/// Whether a remembered write reply may resolve through a provisional id's
/// alias (spec 2026-10-09 §7.2). A create's repeat never does (§12): its item
/// is the real row or the repeat runs again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThroughAlias {
    Direct,
    ThroughProvisionalId,
}

impl ThroughAlias {
    /// The fingerprint's first field is the operation (`create|…`/`modify|…`).
    pub(super) fn for_fingerprint(fingerprint: &str) -> Self {
        if fingerprint.starts_with("modify|") {
            Self::ThroughProvisionalId
        } else {
            Self::Direct
        }
    }

    pub(super) fn resolves_alias(self) -> bool {
        matches!(self, Self::ThroughProvisionalId)
    }
}

pub(super) fn parse_write_kind(kind: &str) -> crate::engine_bridge::FinderWriteItemKind {
    if kind.eq_ignore_ascii_case("folder") || kind.eq_ignore_ascii_case("namespace") {
        crate::engine_bridge::FinderWriteItemKind::Folder
    } else {
        crate::engine_bridge::FinderWriteItemKind::File
    }
}

pub(super) fn parse_write_kind_name(kind: &crate::engine_bridge::FinderWriteItemKind) -> &'static str {
    match kind {
        crate::engine_bridge::FinderWriteItemKind::Folder => "folder",
        crate::engine_bridge::FinderWriteItemKind::File => "file",
    }
}

/// Run a write-queue request, deduplicated by `request_id` when the extension
/// sent one (task 1684). Without a key this is exactly the pre-1684 behaviour:
/// `work` runs inline on this connection's task.
///
/// With a key, `work` runs at most once per key within the TTL: a repeat while
/// the first is still copying waits for and returns its reply, and a repeat
/// after it finished returns the stored reply (refreshed from the current row)
/// if the row it created still exists under the same name and is not being
/// trashed (otherwise the user deleted or renamed it since, and the repeat is a
/// new operation; a delete through this socket also forgets the stored reply). Only queued results are remembered; an error is
/// handed to whoever was already waiting but a later retry runs again.
/// Drop every remembered write reply that describes `file_id`.
pub(super) fn forget_dedup_for_item(dedup: &std::sync::Arc<WriteDedup<IpcResponse>>, file_id: &ItemId) {
    dedup.forget_where(
        |cached| matches!(cached, IpcResponse::WriteQueued { item: Some(item), .. } if item.identifier == file_id.as_str()),
    );
}

/// Decide whether a remembered write reply still describes reality, and bring it
/// up to date. `None` means the item it created is gone, renamed, or on its way
/// to the trash, so the repeat is a NEW operation and must run.
///
/// "Up to date" matters: the reply was built when the first attempt finished,
/// and finalizing the upload since then changed the item's version and status.
/// Serving the old `version_identifier` would hand the system a stale base
/// version for the user's next edit, so the item is rebuilt from the current row.
///
/// `through_alias` (a modify's reply): an item named by a provisional id whose create
/// has landed is rebuilt from the server file and still presented under the provisional
/// id, as the modify was (spec 2026-10-09 §7.2). A create's reply never resolves through
/// the alias: its repeat after the landing runs again, as before (§12).
pub(super) fn refresh_cached_write(
    db: &crate::state_db::StateDb,
    cached: IpcResponse,
    through_alias: ThroughAlias,
) -> Option<IpcResponse> {
    let IpcResponse::WriteQueued {
        item: Some(item),
        ignored,
        message,
    } = cached
    else {
        return Some(cached);
    };
    let (entry, presented) = match db.get_file(&item.identifier) {
        Ok(Some(entry)) => (entry, false),
        Ok(None) if through_alias.resolves_alias() => {
            match db
                .resolve_alias(&item.identifier)
                .ok()
                .flatten()
                .and_then(|server_id| db.get_file(&server_id).ok().flatten())
            {
                Some(entry) => (entry, true),
                None => return None,
            }
        }
        Ok(None) => return None,
        // Cannot tell: prefer returning the stored result over queueing twice.
        Err(_) => {
            return Some(IpcResponse::WriteQueued {
                item: Some(item),
                ignored,
                message,
            });
        }
    };
    if entry.status == crate::state_db::FileStatus::Trashing {
        return None;
    }
    // A failed lookup of the trash queue falls through to "not deleted": the
    // alternative (re-queueing) is only ever a duplicate, never a lost file, but
    // a lookup that cannot run also cannot prove a delete, and the delete path
    // forgets the entry itself.
    if db.has_pending_trash(&entry.file_id).unwrap_or(false) {
        return None;
    }
    let mut fresh = file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE);
    if fresh.filename != item.filename {
        return None;
    }
    if presented {
        fresh.identifier = item.identifier;
    }
    Some(IpcResponse::WriteQueued {
        item: Some(fresh),
        ignored,
        message,
    })
}

pub(super) async fn dedup_write(
    dedup: &std::sync::Arc<WriteDedup<IpcResponse>>,
    db: &std::sync::Arc<crate::state_db::StateDb>,
    request_id: Option<RequestId>,
    fingerprint: String,
    work: impl FnOnce() -> IpcResponse + Send + 'static,
) -> IpcResponse {
    let Some(request_id) = request_id else {
        return work();
    };
    let key = request_id.as_str();
    if key.is_empty() || key.len() > MAX_KEY_BYTES || key.chars().any(|c| c.is_control()) {
        log_refused_write("write", "invalid_request_id");
        return IpcResponse::Error {
            message: format!("request_id must be 1 to {MAX_KEY_BYTES} printable bytes"),
        };
    }
    let validate_db = db.clone();
    let through_alias = ThroughAlias::for_fingerprint(&fingerprint);
    dedup
        .run(
            key,
            &fingerprint,
            std::time::Instant::now,
            move |cached: IpcResponse| refresh_cached_write(&validate_db, cached, through_alias),
            |resp: &IpcResponse| matches!(resp, IpcResponse::WriteQueued { .. }),
            work,
            IpcResponse::Error {
                message: "the Finder write did not complete; try again".into(),
            },
        )
        .await
}

pub(super) fn normalize_parent_id(parent_id: Option<ParentId>) -> Option<ParentId> {
    parent_id.and_then(|value| {
        let trimmed = value.as_str().trim();
        if is_file_provider_root(trimmed) || trimmed.starts_with("namespace:") {
            None
        } else {
            Some(ParentId::new(trimmed))
        }
    })
}

/// The `GetFileStatus` reply: a live row as before; otherwise a provisional id whose create
/// landed is answered with the server file's item under the provisional id (spec §7.2), so
/// the system's item keeps its identity until it applies the swap; otherwise "not found".
pub(crate) fn file_status_response(db: &crate::state_db::StateDb, file_id: &str) -> IpcResponse {
    if let Ok(Some(entry)) = db.get_file(file_id) {
        return IpcResponse::FileStatus(file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE));
    }
    if let Ok(Some(server_id)) = db.resolve_alias(file_id)
        && let Ok(Some(entry)) = db.get_file(&server_id)
    {
        crate::engine_bridge::log_alias_resolved("item", file_id, &server_id);
        let mut item = file_entry_payload_for_db(db, &entry, FP_ROOT_APPLE);
        item.identifier = file_id.to_string();
        return IpcResponse::FileStatus(item);
    }
    IpcResponse::Error {
        message: "not found".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn through_alias_is_the_fingerprint_operation() {
        assert_eq!(
            ThroughAlias::for_fingerprint("modify|file-1|notes.txt|file"),
            ThroughAlias::ThroughProvisionalId
        );
        assert_eq!(ThroughAlias::for_fingerprint("create||notes.txt|file"), ThroughAlias::Direct);
        assert!(!ThroughAlias::Direct.resolves_alias());
        assert!(ThroughAlias::ThroughProvisionalId.resolves_alias());
    }
}
