//! `finder_edit` — the `EngineBridge` impl for Finder/File Provider modify, rename and delete writes, split out of `engine_bridge.rs`.

use super::*;

impl EngineBridge {
    /// [`Self::queue_file_provider_create_from`] reading the contents by path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn queue_file_provider_create(&self, target: FinderWriteTarget) -> anyhow::Result<FpWrite> {
        self.queue_file_provider_create_from(target, None)
    }

    pub fn queue_finder_modify(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_modify_from(target, None)
    }

    /// [`Self::queue_finder_modify`], reading new contents from `contents`
    /// when given; see [`Self::queue_finder_create_from`].
    pub fn queue_finder_modify_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_modify_guarded(target, contents, crate::backup_protect::DriveGuard::Check)
    }

    /// [`Self::queue_finder_modify`] for a caller that applied the drive rule itself (the Windows
    /// watcher, `DriveGuard::Decided`).
    pub fn queue_finder_modify_decided(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_modify_guarded(target, None, crate::backup_protect::DriveGuard::Decided)
    }

    fn queue_finder_modify_guarded(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
        guard: crate::backup_protect::DriveGuard,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let Some((file_id, item_contract)) = self.start_finder_modify(&target, guard)? else {
            return Ok(ignored_finder_item(&target.filename));
        };

        if let Some(contents_path) = target.contents_path.as_deref() {
            let PreparedFinderModify {
                staged,
                staged_path,
                size_bytes,
                modified_at,
                payload,
            } = self.prepare_finder_modify(&target, contents_path, contents, &file_id, item_contract.as_ref())?;
            // Decided from the row BEFORE this write changes it.
            let current_row = self.db.get_file(&file_id)?;
            let current_contract = self.db.get_file_contract_state(&file_id)?;
            let base_version = modify_base_version(
                target.base_version_identifier.as_deref(),
                current_row.as_ref().zip(current_contract.as_ref()),
            );
            // The system keeps the bytes it handed over and is not asked to
            // fetch them back, so the item in the reply must describe them:
            // the row takes the staged copy's size and modification time. The
            // content version is untouched until the upload lands.
            self.db.record_local_write(&file_id, size_bytes, modified_at)?;
            // A landing writes an op's target path to the row: a sync-folder edit (task 1640)
            // names the file's full relative path, so a nested file stays in its folder.
            let target_path = target.rel_path.unwrap_or(target.filename);
            let outcome = self
                .enqueue_finder_operation(
                    OperationKind::UploadVersion,
                    Some(file_id),
                    target.parent_id,
                    Some(target_path),
                    payload,
                    Some(staged_path),
                    base_version,
                    item_contract
                        .as_ref()
                        .and_then(|contract| contract.current_object_version_id.clone()),
                    None,
                )
                .map_err(land_error_to_anyhow)?;
            staged.retain();
            Ok(outcome)
        } else {
            let (kind, payload) = self.metadata_update_payload(&file_id, &target, item_contract.as_ref())?;
            self.enqueue_finder_operation(
                kind,
                Some(file_id),
                target.parent_id,
                Some(target.filename),
                payload,
                None,
                None,
                item_contract
                    .as_ref()
                    .and_then(|contract| contract.current_object_version_id.clone()),
                None,
            )
            .map_err(land_error_to_anyhow)
        }
    }

    /// A metadata-only modify's kind and payload: a move is a change of parent, read against
    /// the row BEFORE the op changes it.
    fn metadata_update_payload(
        &self,
        file_id: &str,
        target: &FinderWriteTarget,
        item_contract: Option<&crate::state_db::FileContractState>,
    ) -> Result<(OperationKind, serde_json::Value), EngineError> {
        let name_encrypted = owned_metadata_for_rename(
            &self.db,
            self.api.master_key(),
            file_id,
            &target.filename,
            target.content_type.as_deref(),
        )?;
        // Spec §7.2 (final review): a move is a change of parent, read against the row, so a
        // move to the top level (no parent) is a move too, and a rename in a folder is a
        // rename. With no row, a parent means a move, as before.
        let moved = match self.db.get_file(file_id)? {
            Some(row) => row.parent_id != target.parent_id,
            None => target.parent_id.is_some(),
        };
        let kind = if moved {
            OperationKind::MoveFile
        } else {
            OperationKind::RenameFile
        };
        let mut payload = serde_json::json!({
            "operation": Op::MetadataUpdate.as_str(),
            "name_encrypted": name_encrypted,
            "base_version_identifier": target.base_version_identifier,
        });
        apply_shared_context(&mut payload, item_contract);
        Ok((kind, payload))
    }

    /// Task 2204 S0.1: a Windows rename or move of a tracked FILE, queued with its row moved to
    /// `new_path` in the same transaction (`StateDb::enqueue_local_rename`). The op is marked
    /// `local_rename`, which holds the row against stale landings and pulls until it is sent.
    /// The checked entry point (`DriveGuard::Check`); the Windows watcher calls
    /// [`Self::queue_local_rename_decided`].
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    pub fn queue_local_rename(&self, target: FinderWriteTarget, new_path: &str) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_local_rename_guarded(target, new_path, crate::backup_protect::DriveGuard::Check)
    }

    /// [`Self::queue_local_rename`] for the Windows watcher, which applied the drive rule itself.
    pub fn queue_local_rename_decided(
        &self,
        target: FinderWriteTarget,
        new_path: &str,
    ) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_local_rename_guarded(target, new_path, crate::backup_protect::DriveGuard::Decided)
    }

    fn queue_local_rename_guarded(
        &self,
        target: FinderWriteTarget,
        new_path: &str,
        guard: crate::backup_protect::DriveGuard,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let Some((file_id, item_contract)) = self.start_finder_modify(&target, guard)? else {
            return Ok(ignored_finder_item(&target.filename));
        };
        let (kind, mut payload) = self.metadata_update_payload(&file_id, &target, item_contract.as_ref())?;
        let row = self.db.get_file(&file_id)?;
        payload["local_rename"] = serde_json::Value::Bool(true);
        // S0.4: where the file was, so a safe save can fold back onto it.
        payload["vacated_path"] = row.as_ref().map(|row| row.path.clone()).into();
        payload["vacated_parent_id"] = row.and_then(|row| row.parent_id).into();
        let mut op = self.finder_operation(
            kind.clone(),
            Some(file_id.clone()),
            target.parent_id.clone(),
            Some(target.filename.clone()),
            payload,
            None,
            None,
            item_contract.and_then(|contract| contract.current_object_version_id),
        )?;
        // S0.4: a safe save's rename, new file and delete arrive within this; until then the
        // rename is not sent, and a file at the vacated path waits.
        op.next_retry_at = now_secs() + RENAME_SETTLE_SECS;
        // L15/A14b: a rename or move of an existing item never claims the
        // owner-only copy (it may be foreign or already wrapped).
        match self
            .db
            .enqueue_local_rename(&op, &file_id, new_path, target.parent_id.as_deref())?
        {
            crate::state_db::FinderEnqueue::AcceptsClosed => Err(anyhow::Error::new(AcceptsClosed).into()),
            crate::state_db::FinderEnqueue::Queued => Ok(FinderWriteOutcome::Queued {
                op_id: op.op_id,
                file_id: Some(file_id),
                kind,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            }),
        }
    }

    /// What every Finder modify checks first: the file id, and the item's contract when it
    /// is shared. `None`: the name is one Finder writes for itself, which is ignored.
    fn start_finder_modify(
        &self,
        target: &FinderWriteTarget,
        guard: crate::backup_protect::DriveGuard,
    ) -> Result<Option<(String, Option<FileContractState>)>, EngineError> {
        // Task 1538 Codex P1 — see `queue_finder_create`'s identical guard.
        if self.is_stopping() {
            return Err(anyhow::anyhow!("engine is stopping; refusing to enqueue a new local write").into());
        }
        if is_ignored_finder_name(&target.filename) {
            return Ok(None);
        }

        let file_id = target
            .file_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Finder modify callback did not include a file id"))?;
        let item_contract = self.ensure_item_allows_shared_write(&file_id, "modify")?;
        if guard == crate::backup_protect::DriveGuard::Check {
            self.refuse_protected_drive_change(&file_id, target, item_contract.as_ref())?;
        }
        Ok(Some((file_id, item_contract)))
    }

    /// LD-0113 (Amendment 6): a drive surface never queues a rename or move of an item in the Backups
    /// tree, nor a move into it; a content edit is queued as before (spec §6.2). Same name and the
    /// same parent (the row's or its contract's: live rows may leave `parent_id` empty) is a content
    /// edit; so is the same name with a parent this device does not know, which shows no change. A
    /// move is a different parent PATH.
    fn refuse_protected_drive_change(
        &self,
        file_id: &str,
        target: &FinderWriteTarget,
        contract: Option<&FileContractState>,
    ) -> Result<(), EngineError> {
        let Some(row) = self.db.get_file(file_id)? else {
            return Ok(());
        };
        let path = row.path.trim_matches('/');
        let (parent_path, leaf) = path.rsplit_once('/').unwrap_or(("", path));
        let renamed = target.filename != leaf;
        let same_parent = target.parent_id == row.parent_id
            || contract.is_some_and(|contract| contract.parent_id == target.parent_id);
        let new_parent = if same_parent {
            Some(parent_path.to_string())
        } else {
            match target.parent_id.as_deref() {
                None => Some(String::new()),
                Some(parent_id) => self
                    .db
                    .get_file(parent_id)?
                    .map(|parent| parent.path.trim_matches('/').to_string()),
            }
        };
        if !renamed && new_parent.as_deref().is_none_or(|new_parent| new_parent == parent_path) {
            return Ok(());
        }
        let mutation = match new_parent.as_deref() {
            Some(new_parent) if new_parent != parent_path => beebeeb_core::backup::Mutation::Move,
            _ => beebeeb_core::backup::Mutation::Rename,
        };
        let new_path = new_parent.map(|new_parent| match new_parent.as_str() {
            "" => target.filename.clone(),
            new_parent => format!("{new_parent}/{}", target.filename),
        });
        let own = crate::backup_protect::own_backups();
        match crate::backup_protect::move_refusal(&self.db, &own, &row.path, mutation, new_path.as_deref()) {
            Some(message) => Err(anyhow::Error::new(crate::backup_protect::DriveChangeRefused { message }).into()),
            None => Ok(()),
        }
    }

    /// A Finder content modify, staged and described: the daemon's own copy of the new
    /// bytes, its size and time, and the upload's metadata. It reads and writes no row:
    /// each entry point decides the base itself. Shared by both modify entry points.
    fn prepare_finder_modify(
        &self,
        target: &FinderWriteTarget,
        contents_path: &str,
        contents: Option<&std::fs::File>,
        file_id: &str,
        item_contract: Option<&FileContractState>,
    ) -> Result<PreparedFinderModify, EngineError> {
        let staged = stage_finder_contents(
            &self.db,
            self.finder_staging_root()?,
            Path::new(contents_path),
            contents,
        )?;
        let staged_path = staged.path().to_string();
        let staged_metadata = std::fs::metadata(&staged_path).ok();
        let size_bytes = staged_metadata.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        let modified_at = staged_metadata
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or_else(now_secs);
        let mime = target
            .content_type
            .as_deref()
            .or_else(|| beebeeb_core::media::guess_mime_type(&target.filename));
        let name_encrypted = owned_metadata_for_name(&self.db, self.api.master_key(), file_id, &target.filename, mime)?;
        let mut payload = serde_json::json!({
            "operation": Op::UploadVersion.as_str(),
            "name_encrypted": name_encrypted,
            "content_type": target.content_type,
            "size_bytes": size_bytes,
            "base_version_identifier": target.base_version_identifier,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut payload, item_contract);
        Ok(PreparedFinderModify {
            staged,
            staged_path,
            size_bytes,
            modified_at,
            payload,
        })
    }

    /// The File Provider modify (spec §8.8; see [`Self::queue_file_provider_create_from`]).
    /// New contents go through one accept transaction that maps the system's base
    /// through the write token (spec §6.1) and queues the upload. A rename or move
    /// carries no content and gets no token.
    ///
    /// The id is resolved first (spec §7.2): a provisional id whose create has landed acts
    /// on the server file, and the reply is presented under the provisional id. An id with
    /// no row and no alias is refused with [`UnknownItem`] before anything is staged, for a
    /// content and a metadata modify alike (§7.3). A name Finder writes for itself stays
    /// ignored, as before, whatever its id.
    #[cfg_attr(not(any(target_os = "macos", any(test, feature = "test-support"))), allow(dead_code))]
    pub fn queue_file_provider_modify_from(
        &self,
        mut target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        let mut present_as = None;
        if let Some(requested_id) = target.file_id.clone()
            && !is_ignored_finder_name(&target.filename)
        {
            let (file_id, presented) = self.resolve_provisional(&requested_id, "modify")?;
            if self.db.get_file(&file_id)?.is_none() {
                return Err(anyhow::Error::new(UnknownItem { requested_id }).into());
            }
            target.file_id = Some(file_id);
            present_as = presented;
        }
        let Some(contents_path) = target.contents_path.clone() else {
            return self.queue_finder_modify_from(target, contents).map(|outcome| FpWrite {
                outcome,
                token: None,
                present_as,
            });
        };
        let Some((file_id, item_contract)) =
            self.start_finder_modify(&target, crate::backup_protect::DriveGuard::Check)?
        else {
            return Ok(FpWrite::plain(ignored_finder_item(&target.filename)));
        };
        let prepared =
            self.prepare_finder_modify(&target, &contents_path, contents, &file_id, item_contract.as_ref())?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = serde_json::to_string(&prepared.payload)?;
        let backup_source_key = crate::known_folder::backup_source_key_for_this_device(&target.filename);
        self.seam("accept:before_tx");
        let accepted = self.db.accept_finder_write(
            &crate::state_db::FinderAccept {
                retire_parked: None,
                op_id: &op_id,
                file_id: &file_id,
                kind: crate::state_db::FinderAcceptKind::Modify {
                    incoming_base: target.base_version_identifier.as_deref(),
                },
                parent_id: target.parent_id.as_deref(),
                target_path: Some(&target.filename),
                metadata_json: &metadata_json,
                payload_path: &prepared.staged_path,
                size_bytes: prepared.size_bytes,
                modified_at: prepared.modified_at,
                backup_source_key: backup_source_key.as_deref(),
                now: now_secs(),
            },
            &|entry, contract| legacy_item_identifiers(entry, contract).to_vec(),
        )?;
        let token = match accepted {
            crate::state_db::AcceptOutcome::Queued { token, .. } => token,
            crate::state_db::AcceptOutcome::ParkedAtOnce { token, reason } => {
                log_parked(&op_id, Some(&file_id), reason);
                token
            }
            // The row went between the check above and the accept: refused the same way
            // (§7.3). The staged copy is dropped with `prepared`; nothing was queued. The
            // request named the provisional id it is presented under, else this one.
            crate::state_db::AcceptOutcome::UnknownItem => {
                return Err(anyhow::Error::new(UnknownItem {
                    requested_id: present_as.unwrap_or(file_id),
                })
                .into());
            }
            crate::state_db::AcceptOutcome::CopyGone => return Err(anyhow::Error::new(StagedCopyGone).into()),
            crate::state_db::AcceptOutcome::AcceptsClosed => return Err(anyhow::Error::new(AcceptsClosed).into()),
            // §7.2: refused; the staged copy is dropped with `prepared`, and nothing was queued.
            crate::state_db::AcceptOutcome::InTrash => return Err(anyhow::Error::new(ItemInTrash).into()),
            // A modify retires no parked write (task 2190's "Keep both" is a create).
            crate::state_db::AcceptOutcome::ParkedMoved | crate::state_db::AcceptOutcome::QueuedRetiring { .. } => {
                return Err(anyhow::anyhow!("a modify retires no parked write").into());
            }
        };
        prepared.staged.retain();
        // L15/A14b: a version of an existing item never claims the owner-only
        // copy — the item may be foreign or already wrapped.
        Ok(FpWrite {
            outcome: FinderWriteOutcome::Queued {
                op_id,
                file_id: Some(file_id),
                kind: OperationKind::UploadVersion,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            },
            token: Some(token),
            present_as,
        })
    }

    /// [`Self::queue_file_provider_modify_from`] reading the contents by path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn queue_file_provider_modify(&self, target: FinderWriteTarget) -> anyhow::Result<FpWrite> {
        self.queue_file_provider_modify_from(target, None)
    }

    pub fn queue_finder_delete(
        &self,
        file_id: &str,
        base_version_identifier: Option<String>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_delete_guarded(
            file_id,
            base_version_identifier,
            crate::backup_protect::DriveGuard::Check,
        )
    }

    /// [`Self::queue_finder_delete`] for the Windows watcher, which applied the drive rule itself.
    pub fn queue_finder_delete_decided(&self, file_id: &str) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_delete_guarded(file_id, None, crate::backup_protect::DriveGuard::Decided)
    }

    fn queue_finder_delete_guarded(
        &self,
        file_id: &str,
        base_version_identifier: Option<String>,
        guard: crate::backup_protect::DriveGuard,
    ) -> anyhow::Result<FinderWriteOutcome> {
        // Task 1538 Codex P1 — see `queue_finder_create`'s identical guard.
        if self.is_stopping() {
            return Err(anyhow::anyhow!("engine is stopping; refusing to enqueue a new local write").into());
        }
        // §7.2 (m-6): a provisional id whose create landed reaches the server file only when
        // the delete's base is that file's held token, the token the create's reply named.
        // Any other base keeps the unknown-item answer below: the system's own resolution of
        // the swap is undocumented, and a delete that does not name our bytes must never
        // trash the person's file.
        if let Some(server_id) = self.db.resolve_alias(file_id)? {
            let held = self
                .db
                .item_presentation(&server_id)?
                .and_then(|presentation| presentation.held)
                .map(|held| held.token());
            if held.is_some() && held == base_version_identifier {
                log_alias_resolved("delete", file_id, &server_id);
                return self.queue_finder_delete_guarded(&server_id, base_version_identifier, guard);
            }
            log_alias_delete_kept(file_id, &server_id);
            return Ok(FinderWriteOutcome::Ignored {
                message: "the item is already gone from Beebeeb".to_string(),
            });
        }
        // Task 1698 (trash ruling): an UNKNOWN item is an idempotent success
        // ("unknown items report success") — the item may have been deleted
        // remotely already, or the replica converged past it. Queueing a
        // doomed server call would retry a guaranteed 404 for hours.
        let item_contract = match self.ensure_item_allows_shared_write(file_id, "delete")? {
            contract if contract.is_none() && self.db.get_file(file_id)?.is_none() => {
                // The id is the request's, from the wire (spec §11 M5).
                let (file_id, id_kind) = wire_id_fields(file_id);
                tracing::info!(file_id, id_kind, "finder delete: unknown item — idempotent success");
                return Ok(FinderWriteOutcome::Ignored {
                    message: "the item is already gone from Beebeeb".to_string(),
                });
            }
            contract => contract,
        };
        // LD-0113 (Amendment 6): a drive surface never queues a trash of an item in the Backups tree.
        if guard == crate::backup_protect::DriveGuard::Check
            && let Some(row) = self.db.get_file(file_id)?
            && let Some(message) = crate::backup_protect::protected_reason(
                &self.db,
                &crate::backup_protect::own_backups(),
                &row.path,
                beebeeb_core::backup::Mutation::Trash,
            )
        {
            return Err(anyhow::Error::new(crate::backup_protect::DriveChangeRefused { message }).into());
        }
        let mut payload = serde_json::json!({
            "operation": Op::Trash.as_str(),
            "base_version_identifier": base_version_identifier,
        });
        apply_shared_context(&mut payload, item_contract.as_ref());
        let outcome = self
            .enqueue_finder_operation(
                OperationKind::TrashFile,
                Some(file_id.to_string()),
                None,
                None,
                payload,
                None,
                parse_base_version_number(base_version_identifier.as_deref()),
                item_contract
                    .as_ref()
                    .and_then(|contract| contract.current_object_version_id.clone()),
                None,
            )
            .map_err(land_error_to_anyhow)?;
        // Task 1698: park the row `Trashing` AFTER a successful enqueue, so
        // the change feed immediately presents the item under the trash
        // container (ruling step 2) while the server trash converges. A
        // failed enqueue leaves the row untouched (nothing hidden without a
        // queued op). Idempotent for the watcher path, which parks first.
        if let Some(entry) = self.db.get_file(file_id)?
            && entry.status != crate::state_db::FileStatus::Trashing
        {
            self.db.set_status(file_id, crate::state_db::FileStatus::Trashing)?;
        }
        Ok(outcome)
    }
}
