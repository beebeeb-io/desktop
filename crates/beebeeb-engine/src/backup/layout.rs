//! LD-0113 (spec §4.2 rules 3–6, §5.2, §6.1 point 6; plan Task D4 "before the first pass"): where this device's
//! backup lives on the server, resolved once per account and cached in the state DB.
//!
//! 1. **The root** ([`resolve_root_id`]): the cached id; else `GET /backups/root`; else (an account from before the
//!    flag) the root-level listing goes through core's `resolve_root`: `Claim` merges the legacy duplicate roots into
//!    the survivor (core's `plan_merge`, applied in order) and then claims it with `PUT`; `Create` mints an id, seals
//!    `Backups` and `PUT`s it. The server's answer is the root (another device's when it won a race).
//! 2. **This device's folder** ([`resolve_device_folder`]): every folder under the root, with its `.device.json`
//!    read and parsed, goes through core's `find_device_folder`. `AdoptLegacy` (the old mirror's `Backups/GLegion`,
//!    no manifest) and `Create` write the manifest at once; `Existing` is used as it is.
//!
//! A listing that is not complete, or a manifest that could not be fetched, stops the resolution (it is tried again
//! on the next tick): deciding from part of the tree could claim the wrong root, trash a duplicate that still holds
//! something, or start a second device folder next to this device's own.

use std::sync::Arc;

use beebeeb_core::backup::{
    DeviceFolderCandidate, DeviceFolderChoice, DeviceManifest, DeviceSelf, FolderManifest, FolderState, MANIFEST_NAME,
    MergeNode, MergeStep, RootCandidate, RootPlan, find_device_folder, parse_manifest, plan_merge, resolve_root,
    serialize_manifest,
};
use beebeeb_types::backup::Category;
use uuid::Uuid;

use crate::engine_bridge::{BackupChild, EngineBridge};
use crate::state_db::BackupLayout;

/// Why the layout could not be resolved now. A kind, never a name or a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutError(pub &'static str);

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

fn failed(kind: &'static str) -> impl Fn(crate::engine_bridge::EngineError) -> LayoutError {
    move |error| {
        tracing::warn!(
            kind,
            reason = crate::engine_bridge::error_category(&error),
            "backup: layout step failed"
        );
        LayoutError(kind)
    }
}

fn db_failed(kind: &'static str) -> impl Fn(rusqlite::Error) -> LayoutError {
    move |_| LayoutError(kind)
}

/// The label a merge gives the second of two same-named files (core's `keep_both_name`): `IMG_1 (merged, 14.05).heic`.
const MERGE_CLASH_LABEL: &str = "merged";

/// This account's layout: the cached one, else resolved now (the root, then this device's folder) and cached.
/// `enabled`: the folders this device backs up now, written into a new manifest.
pub async fn ensure_layout(
    bridge: &Arc<EngineBridge>,
    me: &DeviceSelf,
    enabled: &[Category],
) -> Result<BackupLayout, LayoutError> {
    let db = bridge.db();
    if let Some(layout) = db.backup_layout().map_err(db_failed("layout_read"))? {
        return Ok(layout);
    }
    let root_id = resolve_root_id(bridge).await?;
    let (device_folder_id, device_folder_name) = resolve_device_folder(bridge, &root_id, me, enabled).await?;
    let layout = BackupLayout {
        root_id,
        device_folder_id,
        device_folder_name,
    };
    if !db.set_backup_layout(&layout).map_err(db_failed("layout_write"))? {
        return Err(LayoutError("no account is bound"));
    }
    tracing::info!(root_id = %layout.root_id, device_folder_id = %layout.device_folder_id, "backup: layout resolved");
    Ok(layout)
}

/// Spec §4.2 rule 5 and §5.2: the account's flagged Backups root, cached once known.
pub async fn resolve_root_id(bridge: &Arc<EngineBridge>) -> Result<String, LayoutError> {
    let db = bridge.db();
    if let Some(id) = db.backup_root().map_err(db_failed("root_read"))? {
        return Ok(id);
    }
    let id = match bridge.backup_root_flagged().await.map_err(failed("root_get"))? {
        Some(id) => id,
        None => claim_or_create_root(bridge).await?,
    };
    if !db.set_backup_root(&id).map_err(db_failed("root_write"))? {
        return Err(LayoutError("no account is bound"));
    }
    Ok(id)
}

/// No flagged root yet: core's `resolve_root` over the whole root-level listing, then the server's `PUT`.
async fn claim_or_create_root(bridge: &Arc<EngineBridge>) -> Result<String, LayoutError> {
    let (listed, complete) = bridge.backup_list_root().await.map_err(failed("root_listing"))?;
    if !complete {
        return Err(LayoutError("the root-level listing is not complete"));
    }
    let candidates: Vec<RootCandidate> = listed
        .iter()
        .filter(|item| item.is_folder)
        .filter_map(|item| {
            Some(RootCandidate {
                id: Uuid::parse_str(&item.id).ok()?,
                name: item.name.clone(),
                is_trashed: item.is_trashed,
                created_at_ms: item.created_at_ms,
                system_role: item.system_role,
            })
        })
        .collect();
    match resolve_root(&candidates) {
        RootPlan::Use(id) => bridge
            .backup_root_put(&id.to_string(), false)
            .await
            .map_err(failed("root_put")),
        RootPlan::Claim { id, merge_from } => {
            if !merge_from.is_empty() {
                merge_duplicate_roots(bridge, id, &merge_from).await?;
            }
            bridge
                .backup_root_put(&id.to_string(), false)
                .await
                .map_err(failed("root_claim"))
        }
        RootPlan::Create => {
            let id = bridge.backup_mint_id().map_err(failed("root_mint"))?;
            bridge.backup_root_put(&id, true).await.map_err(failed("root_create"))
        }
    }
}

/// A name for a node whose name cannot be opened: unique, so it never merges with or clashes against another and is
/// moved as it is.
fn unopened_name(id: &str) -> String {
    format!("\u{0}{id}")
}

/// The live tree under `id`, every level listed (a merge has no depth limit). An incomplete listing is an error:
/// the merge would otherwise trash a duplicate that still holds what was not listed.
async fn merge_tree(bridge: &Arc<EngineBridge>, id: Uuid, name: String) -> Result<MergeNode, LayoutError> {
    let mut root = MergeNode {
        id,
        name,
        is_folder: true,
        children: Vec::new(),
    };
    // (path of child indexes from the root, folder id) still to list.
    let mut queue: std::collections::VecDeque<(Vec<usize>, String)> =
        std::collections::VecDeque::from([(Vec::new(), id.to_string())]);
    while let Some((at, folder)) = queue.pop_front() {
        let (children, complete) = bridge
            .backup_list_children(&folder)
            .await
            .map_err(failed("merge_listing"))?;
        if !complete {
            return Err(LayoutError("a merge listing is not complete"));
        }
        let mut node = &mut root;
        for index in &at {
            node = &mut node.children[*index];
        }
        for BackupChild {
            id, name, is_folder, ..
        } in children
        {
            let Ok(uuid) = Uuid::parse_str(&id) else {
                return Err(LayoutError("a merge listing holds an id that is not a uuid"));
            };
            if is_folder {
                let mut path = at.clone();
                path.push(node.children.len());
                queue.push_back((path, id.clone()));
            }
            node.children.push(MergeNode {
                id: uuid,
                name: name.unwrap_or_else(|| unopened_name(&id)),
                is_folder,
                children: Vec::new(),
            });
        }
    }
    Ok(root)
}

/// Spec §4.2 rule 6: fold the legacy duplicate roots into the survivor (core's `plan_merge`), in order. A step that
/// fails stops the merge; replaying the plan later is safe (a step only names a row an earlier one left in place).
async fn merge_duplicate_roots(
    bridge: &Arc<EngineBridge>,
    survivor: Uuid,
    duplicates: &[Uuid],
) -> Result<(), LayoutError> {
    let survivor_tree = merge_tree(bridge, survivor, beebeeb_core::backup::ROOT_NAME.to_string()).await?;
    let mut duplicate_trees = Vec::with_capacity(duplicates.len());
    for id in duplicates {
        duplicate_trees.push(merge_tree(bridge, *id, beebeeb_core::backup::ROOT_NAME.to_string()).await?);
    }
    let hhmm = chrono::Local::now().format("%H.%M").to_string();
    let steps = plan_merge(&survivor_tree, &duplicate_trees, MERGE_CLASH_LABEL, &hhmm);
    let (mut moved, mut trashed) = (0usize, 0usize);
    for step in &steps {
        match step {
            MergeStep::Move { id, new_parent } => {
                bridge
                    .backup_move(&id.to_string(), &new_parent.to_string(), None)
                    .await
                    .map_err(failed("merge_move"))?;
                moved += 1;
            }
            MergeStep::MoveRenamed {
                id,
                new_parent,
                new_name,
            } => {
                bridge
                    .backup_move(&id.to_string(), &new_parent.to_string(), Some(new_name))
                    .await
                    .map_err(failed("merge_move"))?;
                moved += 1;
            }
            MergeStep::Trash { id } => {
                bridge
                    .backup_trash(&id.to_string())
                    .await
                    .map_err(failed("merge_trash"))?;
                trashed += 1;
            }
        }
    }
    tracing::info!(
        duplicates = duplicates.len(),
        moved,
        trashed,
        "backup: legacy duplicate roots merged"
    );
    Ok(())
}

/// What `.device.json` in the device folder `folder_id` says: absent, mine, another device's, or unreadable. More
/// than one: this device's if one is, else the first that parses.
async fn read_manifest(
    bridge: &Arc<EngineBridge>,
    folder_id: &str,
    me: &DeviceSelf,
) -> Result<FolderManifest, LayoutError> {
    let (children, complete) = bridge
        .backup_list_children(folder_id)
        .await
        .map_err(failed("device_listing"))?;
    if !complete {
        return Err(LayoutError("a device folder listing is not complete"));
    }
    let mut found = Vec::new();
    for child in children
        .iter()
        .filter(|c| !c.is_folder && c.name.as_deref() == Some(MANIFEST_NAME))
    {
        let bytes = bridge
            .backup_read_small_file(&child.id, child.chunk_count, child.size)
            .await
            .map_err(failed("manifest_read"))?;
        let parsed = bytes
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| parse_manifest(&text).ok());
        found.push(parsed);
    }
    if found.is_empty() {
        return Ok(FolderManifest::Absent);
    }
    let mine = |m: &DeviceManifest| {
        m.device_id
            .as_deref()
            .is_some_and(|id| beebeeb_core::backup::same_device_id(id, &me.device_id))
    };
    let parsed: Vec<DeviceManifest> = found.into_iter().flatten().collect();
    Ok(match parsed.iter().find(|m| mine(m)).or(parsed.first()) {
        Some(manifest) => FolderManifest::Parsed(manifest.clone()),
        None => FolderManifest::Unreadable,
    })
}

/// This device's manifest (spec §4.2 rule 4, v1): its id, name, platform and the folders it backs up now.
pub fn device_manifest(me: &DeviceSelf, enabled: &[Category]) -> DeviceManifest {
    DeviceManifest {
        version: beebeeb_core::backup::MANIFEST_VERSION,
        device_id: Some(me.device_id.clone()),
        device_name: me.name.clone(),
        platform: Some(me.platform),
        folders: enabled
            .iter()
            .map(|category| FolderState {
                category: category.key(),
                enabled: true,
                last_pass_at_ms: None,
                items: 0,
                other: serde_json::Map::new(),
            })
            .collect(),
        other: serde_json::Map::new(),
    }
}

/// Spec §4.2 rule 3: this device's folder under `root_id`, as (id, name). Adopting the old mirror's folder or
/// creating one writes the manifest at once.
pub async fn resolve_device_folder(
    bridge: &Arc<EngineBridge>,
    root_id: &str,
    me: &DeviceSelf,
    enabled: &[Category],
) -> Result<(String, String), LayoutError> {
    let (children, complete) = bridge
        .backup_list_children(root_id)
        .await
        .map_err(failed("root_children"))?;
    if !complete {
        return Err(LayoutError("the Backups listing is not complete"));
    }
    let mut candidates = Vec::new();
    for child in children.iter().filter(|c| c.is_folder) {
        let Ok(folder_id) = Uuid::parse_str(&child.id) else {
            return Err(LayoutError("the Backups listing holds an id that is not a uuid"));
        };
        let manifest = read_manifest(bridge, &child.id, me).await?;
        candidates.push(DeviceFolderCandidate {
            folder_id,
            name: child.name.clone().unwrap_or_else(|| unopened_name(&child.id)),
            manifest,
        });
    }
    let name_of = |id: Uuid| {
        candidates
            .iter()
            .find(|c| c.folder_id == id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    };
    let manifest = serialize_manifest(&device_manifest(me, enabled));
    match find_device_folder(me, &candidates) {
        DeviceFolderChoice::Existing(id) => Ok((id.to_string(), name_of(id))),
        DeviceFolderChoice::AdoptLegacy(id) => {
            let folder = id.to_string();
            bridge
                .backup_write_small_file(&folder, MANIFEST_NAME, manifest.as_bytes(), "application/json")
                .await
                .map_err(failed("manifest_write"))?;
            tracing::info!(device_folder_id = %folder, "backup: the old mirror's device folder adopted; manifest written");
            Ok((folder, name_of(id)))
        }
        DeviceFolderChoice::Create { name } => {
            let folder = bridge
                .backup_create_folder(root_id, &name)
                .await
                .map_err(failed("device_folder_create"))?;
            bridge
                .backup_write_small_file(&folder, MANIFEST_NAME, manifest.as_bytes(), "application/json")
                .await
                .map_err(failed("manifest_write"))?;
            tracing::info!(device_folder_id = %folder, "backup: device folder created; manifest written");
            Ok((folder, name))
        }
    }
}
