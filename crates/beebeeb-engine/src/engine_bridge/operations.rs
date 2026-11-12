//! `operations` — the `EngineBridge` impl for the operation queue: feed, restore and enqueue helpers, split out of `engine_bridge.rs`.

use super::*;

/// The ops that CREATE an item of this device's own: only these owe the
/// owner-only copy. A version, rename or move of an existing item does not.
fn creates_an_item(kind: &OperationKind, metadata: &serde_json::Value) -> bool {
    match kind {
        OperationKind::CreateFolder => true,
        OperationKind::UploadVersion | OperationKind::UploadFile => is_create_file_operation(metadata),
        _ => false,
    }
}

impl EngineBridge {
    #[cfg(any(test, feature = "test-support"))]
    pub fn version_conflict_feed(&self) -> anyhow::Result<Vec<VersionConflictEntry>> {
        version_conflict_feed_from_db(self.db.as_ref())
    }

    pub fn queue_restore_version(
        &self,
        file_id: &str,
        version_id: &str,
        last_error: Option<String>,
    ) -> anyhow::Result<PendingOperation> {
        let now = now_secs();
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::RestoreVersion,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(
                serde_json::json!({
                    "operation": Op::RestoreVersion.as_str(),
                    "version_id": version_id,
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: Some(version_id.to_string()),
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error,
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(op)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn enqueue_finder_operation(
        &self,
        kind: OperationKind,
        file_id: Option<String>,
        parent_id: Option<String>,
        target_path: Option<String>,
        metadata: serde_json::Value,
        payload_path: Option<String>,
        base_version: Option<i64>,
        base_object_version_id: Option<String>,
        folder: Option<crate::state_db::FinderFolderRow<'_>>,
    ) -> Result<FinderWriteOutcome, EngineError> {
        // L15/A14b: a CREATE of this device's own item in a shared folder stays
        // owner-only. A version/rename/move of an existing item must not claim
        // it: that item may be foreign or already wrapped.
        let owner_only = creates_an_item(&kind, &metadata)
            .then(|| self.owner_only_notice(file_id.as_deref(), parent_id.as_deref()))
            .flatten();
        let op = self.finder_operation(
            kind.clone(),
            file_id.clone(),
            parent_id,
            target_path,
            metadata,
            payload_path,
            base_version,
            base_object_version_id,
        )?;
        let op_id = op.op_id.clone();
        // Spec 2026-10-10 §5.2 step 6: the sign-out's fence is read in the transaction that queues the op, and a
        // folder's row is written in it too.
        match self.db.enqueue_finder_change(&op, folder)? {
            crate::state_db::FinderEnqueue::AcceptsClosed => Err(anyhow::Error::new(AcceptsClosed).into()),
            crate::state_db::FinderEnqueue::Queued => Ok(FinderWriteOutcome::Queued {
                op_id,
                file_id,
                kind,
                ignored: false,
                // L15/A14b: an owner write into a shared folder says it stays
                // owner-only instead of the plain queue copy.
                message: owner_only.unwrap_or("queued for encrypted sync").to_string(),
            }),
        }
    }

    /// The op a Finder or sync-folder write queues (`enqueue_finder_operation`'s first half).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finder_operation(
        &self,
        kind: OperationKind,
        file_id: Option<String>,
        parent_id: Option<String>,
        target_path: Option<String>,
        metadata: serde_json::Value,
        payload_path: Option<String>,
        base_version: Option<i64>,
        base_object_version_id: Option<String>,
    ) -> Result<PendingOperation, EngineError> {
        let op_id = uuid::Uuid::new_v4().to_string();
        let now = now_secs();
        // Task 0811: tag known-folder backup ops with their origin key (derived
        // from the server-relative path the upload threads as `target_path`), so
        // disabling that folder can purge exactly its queued ops. `None` for any
        // normal user upload — those are never purged by a folder disable. The
        // `_this_device` form pins segment 2 to THIS machine's sanitized device
        // name (the only value the mirror writes), so a user's own
        // `Backup/<other>/<CatalogName>/…` file is never mis-tagged + wrongly
        // purged (review fix).
        let backup_source_key = target_path
            .as_deref()
            .and_then(crate::known_folder::backup_source_key_for_this_device);
        let op = PendingOperation {
            op_id: op_id.clone(),
            kind: kind.clone(),
            file_id: file_id.clone(),
            parent_id,
            target_path,
            metadata_json: Some(serde_json::to_string(&metadata)?),
            payload_path,
            base_version,
            base_object_version_id,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued from Finder; upload worker not yet attached".to_string()),
            backup_source_key,
            created_at: now,
            updated_at: now,
        };
        Ok(op)
    }

    pub(super) fn enqueue_pin_tree_operation(
        &self,
        file_id: &str,
        toggle: PinToggle,
        now: i64,
    ) -> Result<(), EngineError> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::PinTree,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(
                serde_json::json!({
                    "operation": Op::PinTree.as_str(),
                    "pinned": toggle.is_pin(),
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued recursive pin state; hydration worker not yet attached".to_string()),
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn enqueue_hydrate_operation(&self, entry: &FileEntry, now: i64) -> Result<(), EngineError> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::HydrateFile,
            file_id: Some(entry.file_id.clone()),
            parent_id: None,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": Op::HydrateFile.as_str(),
                    "reason": "recursive_pin",
                })
                .to_string(),
            ),
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: now,
            last_error: Some("queued pinned content hydration; transfer worker not yet attached".to_string()),
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };
        self.db.enqueue_operation(&op)?;
        Ok(())
    }
}
