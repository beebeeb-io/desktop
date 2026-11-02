//! `shared_roots` — the `EngineBridge` impl for shared-folder names, keys and the shared-root refresh, split out of `engine_bridge.rs`.

use super::*;

impl EngineBridge {
    fn decrypted_direct_shared_root_name(&self, root: &SharedRootMapping) -> Option<String> {
        let sender_public_key = root.sender_public_key.as_deref()?;
        let encrypted_file_key = root.encrypted_file_key.as_deref()?;
        let name_encrypted = root.file_name_encrypted.as_deref()?;
        let file_key = unwrap_direct_shared_file_key(
            self.api.master_key(),
            sender_public_key,
            &root.file_id,
            encrypted_file_key,
        )
        .ok()?;
        decrypt_shared_name_with_key(&file_key, name_encrypted)
    }

    pub(super) async fn shared_folder_material(
        &self,
        root: &SharedRootMapping,
    ) -> Result<(Zeroizing<[u8; 32]>, serde_json::Value), EngineError> {
        let sender_public_key = root
            .sender_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("folder share {} is missing sender_public_key", root.invite_id))?;
        let encrypted_folder_key = root
            .encrypted_folder_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("folder share {} is missing encrypted_folder_key", root.invite_id))?;
        let folder_key = unwrap_folder_share_key(
            self.api.master_key(),
            sender_public_key,
            &root.file_id,
            encrypted_folder_key,
        )?;
        let scope = self.owner_key_scope();
        scope.check()?;
        let response = self.api.get_shared_folder(&root.file_id).await;
        scope.check()?;
        let custody = match response {
            Ok(row) => {
                if !(row["folder_id"].as_str() == Some(&root.file_id)
                    && row["key_version"].as_u64().is_some_and(|v| v > 0)) { return Err(anyhow::anyhow!("invalid folder custody record").into()) };
                // The server returns the caller's copy: L6 for owners, the
                // existing authenticated invite envelope for members. A member
                // cannot open another account's owner/L6 envelope.
                let custody_key = if row["role"] == "member" {
                    let wrapped = row["wrapped_folder_key"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing member custody key"))?;
                    unwrap_folder_share_key(self.api.master_key(), sender_public_key, &root.file_id, wrapped)?
                } else {
                    if !(row["role"] == "owner") { return Err(anyhow::anyhow!("invalid custody role").into()) };
                    let wrap = crate::owner_keys::normalize_wrap(&row["owner_wrapped_folder_key"])?
                        .ok_or_else(|| anyhow::anyhow!("missing owner custody key"))?;
                    let key = beebeeb_sync::key_resolution::unwrap_folder_custody(
                        &beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key()), &root.file_id, &wrap)?;
                    Zeroizing::new(*key.as_bytes())
                };
                if !(*custody_key == *folder_key) { return Err(anyhow::anyhow!("custody and invite keys differ").into()) };
                scope.remember_custody(&root.file_id)?;
                true
            }
            Err(error) if crate::owner_keys::custody_absent(&error) => {
                if !(!self.db.has_seen_folder_custody(&root.file_id)?) { return Err(anyhow::anyhow!("This folder's keys are unavailable").into()) };
                false
            }
            Err(error) => return Err(error.into()),
        };
        let mut folder_keys_response = self.api.get_folder_keys(&root.invite_id).await?;
        scope.check()?;
        folder_keys_response["desktop_has_custody"] = serde_json::json!(custody);
        Ok((folder_key, folder_keys_response))
    }

    pub(super) fn shared_folder_child_key(
        &self,
        folder: &[u8; 32],
        root: &str,
        id: &str,
        kind: u8,
        wrap: &str,
        custody: bool,
    ) -> Result<beebeeb_core::kdf::FileKey, EngineError> {
        if custody {
            return unwrap_folder_child_for_format(folder, root, id, kind, wrap, true);
        }
        self.db.read_without_folder_custody(root, || {
            unwrap_folder_child_for_format(folder, root, id, kind, wrap, false)
        })?.ok_or_else(|| anyhow::anyhow!("This folder's keys are unavailable"))?
    }

    pub(super) fn decrypted_folder_root_name(
        &self,
        root: &SharedRootMapping,
        folder_key: &[u8; 32],
        folder_keys_response: &serde_json::Value,
    ) -> Option<String> {
        let name_encrypted = root.file_name_encrypted.as_deref()?;
        let encrypted_file_key = encrypted_file_key_from_folder_keys_response(folder_keys_response, &root.file_id)?;
        let file_key = self.shared_folder_child_key(folder_key, &root.file_id, &root.file_id, 2,
            encrypted_file_key, folder_keys_response["desktop_has_custody"].as_bool() == Some(true)).ok()?;
        decrypt_shared_name_with_key(&file_key, name_encrypted)
    }

    fn shared_root_display_name(
        &self,
        root: &SharedRootMapping,
        folder_key: Option<&[u8; 32]>,
        folder_keys_response: Option<&serde_json::Value>,
    ) -> String {
        let item_kind = if root.is_folder {
            ItemKind::Folder
        } else {
            ItemKind::File
        };
        let decrypted = if root.is_folder {
            folder_key
                .zip(folder_keys_response)
                .and_then(|(key, keys_response)| self.decrypted_folder_root_name(root, key, keys_response))
        } else {
            self.decrypted_direct_shared_root_name(root)
        };

        decrypted
            .as_deref()
            .and_then(safe_shared_leaf_name)
            .unwrap_or_else(|| shared_placeholder_name(&item_kind).to_string())
    }

    async fn refresh_shared_folder_children(
        &self,
        root: &SharedRootMapping,
        root_display_name: &str,
        folder_key: &[u8; 32],
        folder_keys_response: &serde_json::Value,
        now: i64,
    ) -> Result<(), EngineError> {
        let key_map = folder_keys_map(folder_keys_response);
        let root_path = format!("Shared with me/{root_display_name}");
        let mut queue: VecDeque<(Option<String>, String, String)> =
            VecDeque::from([(None, root.file_id.clone(), root_path)]);

        while let Some((api_parent_id, db_parent_id, parent_path)) = queue.pop_front() {
            let children = self
                .api
                .list_shared_folder_files(&root.file_id, api_parent_id.as_deref())
                .await?;

            for child in children {
                let Some(child_id) = child["id"].as_str().map(str::to_string) else {
                    continue;
                };
                let child_file_key = key_map
                    .get(&child_id)
                    .and_then(|encrypted| self.shared_folder_child_key(folder_key, &root.file_id, &child_id,
                        if item_kind_from_metadata(&child) == ItemKind::Folder { 2 } else { 1 }, encrypted,
                        folder_keys_response["desktop_has_custody"].as_bool() == Some(true)).ok());
                let Some(entry) = apply_shared_metadata_file_row(
                    &self.db,
                    &child,
                    root,
                    now,
                    Some(db_parent_id.clone()),
                    &parent_path,
                    child_file_key.as_ref(),
                )?
                else {
                    continue;
                };

                if entry.item_kind == ItemKind::Folder {
                    queue.push_back((Some(child_id.clone()), child_id, entry.path));
                }
            }
        }

        Ok(())
    }

    pub async fn refresh_shared_roots(&self) -> anyhow::Result<SharedRootRefreshOutcome> {
        let body = self.api.list_shared_roots().await?;
        let roots = shared_roots_from_invite_response(&body);
        let active_shared_root_ids: Vec<String> = roots.iter().map(|root| root.file_id.clone()).collect();
        let now = now_secs();

        for root in &roots {
            let mut folder_material = None;
            if root.is_folder {
                match self.shared_folder_material(root).await {
                    Ok((folder_key, folder_keys_response)) => {
                        folder_material = Some((folder_key, folder_keys_response));
                    }
                    Err(e) => {
                        tracing::warn!(
                            invite_id = %root.invite_id,
                            error = %e,
                            "shared folder key unwrap failed during refresh"
                        );
                    }
                }
            }
            let display_name = self.shared_root_display_name(
                root,
                folder_material.as_ref().map(|(folder_key, _)| &**folder_key),
                folder_material.as_ref().map(|(_, keys_response)| keys_response),
            );
            let root_path = format!("Shared with me/{display_name}");
            crate::reject_unsafe_rel_path(&root_path)
                .map_err(|e| anyhow::anyhow!("unsafe shared root path for {}: {e}", root.file_id))?;

            let item_kind = if root.is_folder {
                ItemKind::Folder
            } else {
                ItemKind::File
            };
            // Spec §7.2 (final review): the row and its contract in one transaction, so no
            // listing sees a new shared root in the person's own namespace.
            self.db.upsert_file_with_contract(
                &FileEntry {
                    file_id: root.file_id.clone(),
                    path: root_path,
                    status: FileStatus::CloudOnly,
                    size_bytes: root.size_bytes,
                    modified_at: root.approved_at.unwrap_or(now),
                    content_hash: None,
                    remote_updated_at: root.approved_at.unwrap_or(0),
                    // upsert_file does not persist these; the contract update sets the
                    // authoritative parent_id/item_kind.
                    parent_id: None,
                    item_kind: item_kind.clone(),
                },
                |contract| {
                    contract.namespace = Namespace::SharedWithMe;
                    contract.parent_id = None;
                    contract.shared_root_id = Some(root.file_id.clone());
                    contract.share_id = Some(root.invite_id.clone());
                    contract.owner_email = root.owner_email.clone();
                    contract.permission_bits = root.permission_bits;
                    contract.item_kind = item_kind;
                    contract.content_type = root.content_type.clone();
                    contract.last_sync_at = now;
                },
            )?;

            if let Some((folder_key, folder_keys_response)) = folder_material
                && let Err(e) = self
                    .refresh_shared_folder_children(root, &display_name, &folder_key, &folder_keys_response, now)
                    .await
            {
                tracing::warn!(
                    invite_id = %root.invite_id,
                    error = %e,
                    "shared folder child refresh failed"
                );
            }
        }

        let removed = self.db.purge_revoked_shared_content(&active_shared_root_ids)?;
        let mut removed_shared_file_ids = Vec::new();
        let mut removed_cache_paths = Vec::new();
        for cache in removed {
            removed_shared_file_ids.push(cache.file_id);
            if let Some(path) = cache.cache_path {
                if let Err(e) = std::fs::remove_file(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(path = %path, error = %e, "failed to remove revoked shared cache file");
                    }
                }
                removed_cache_paths.push(path);
            }
        }

        Ok(SharedRootRefreshOutcome {
            active_shared_root_ids,
            removed_shared_file_ids,
            removed_cache_paths,
        })
    }
}
