//! `syncflow` items split out of `engine_bridge.rs`.

use super::*;

/// How many times one pass of `process_due_operations` may list its due work. Spec D7 (tasks 1888, 2226): on Windows
/// and Linux a change in a folder made here is not due until that folder's create landed, so a pass that lands a
/// folder create lists again for what the landing made due; a new tree up to this many levels deep goes up in one
/// pass. macOS has no gate and lists once, as before.
#[cfg(target_os = "macos")]
pub(super) const DUE_LISTINGS_PER_PASS: usize = 1;

#[cfg(not(target_os = "macos"))]
pub(super) const DUE_LISTINGS_PER_PASS: usize = 16;

pub(crate) fn sync_folder_content(
    db: &StateDb,
    file_id: &str,
    path: &Path,
    os_in_sync: Option<bool>,
) -> Result<SyncFolderContent, EngineError> {
    if db.sync_folder_upload_queued(file_id)? {
        return Ok(SyncFolderContent::Queued);
    }
    // Task 2191 (G-engine): a file of unknown content (synced before task 1640) may hold an edit
    // that 0.8.12 marked in sync, so the platform's in-sync state never makes it Agreed.
    if os_in_sync == Some(true) && !db.is_unknown_content(file_id)? {
        return Ok(SyncFolderContent::Agreed);
    }
    let Some(contract) = db.get_file_contract_state(file_id)? else {
        return Ok(SyncFolderContent::Unknown);
    };
    let (agreed, base) = match contract.local_hash {
        Some(agreed) => (agreed, contract.local_base_version),
        // Task 1640 (review N1): no agreed content yet, but range fetches served the bytes: the
        // candidate decides like a baseline. Its bytes are exactly version `version`'s, so equal
        // bytes are agreed, and any other is a change sent on `version` (never too high: a server
        // that moved refuses it as stale).
        None => match db.fetched_content(file_id)? {
            Some((version, Some(hash))) => (hash, version),
            // Mixed: the fetches served more than one version, so the bytes are none of them.
            // Never agreed; a change is sent on the oldest version served.
            Some((version, None)) => return Ok(SyncFolderContent::Changed { base: Some(version) }),
            None => return Ok(SyncFolderContent::Unknown),
        },
    };
    if cached_sha256_file_hex(file_id, path)? == agreed {
        return Ok(SyncFolderContent::Agreed);
    }
    Ok(SyncFolderContent::Changed {
        base: Some(base).filter(|version| *version > 0),
    })
}

/// The `init` guard (spec §6.3.1), checked where the request is built: a File Provider
/// write that replaces a file (`InitKind::FinderReplace`) is never sent without a base. A missing
/// base, or one that does not fit the server's `i32` (which
/// [`upload_init_request_for_operation`] would drop as "no base"), parks it as
/// `base_unknown`. A create sends no base, and uploads without a write id (Windows, the
/// watcher) keep today's bases.
pub(super) fn guard_init_base(base_version: Option<i64>, kind: InitKind) -> Result<(), EngineError> {
    if !matches!(kind, InitKind::FinderReplace) {
        return Ok(());
    }
    match base_version.map(i32::try_from) {
        Some(Ok(_)) => Ok(()),
        _ => Err(anyhow::Error::new(ParkNow(ParkReason::BaseUnknown)).into()),
    }
}

/// SF-0001 (L13): assert that a persisted session still targets the item the op
/// is writing. A create session must be self-minted (its id was chosen by this
/// desktop); a replace session's `server_file_id` must equal the local id the
/// op is updating. Anything else means the server handed back a different item
/// than the client chose, so the op must abort.
///
pub(super) fn ensure_session_item_id(session: &crate::state_db::UploadResume, local_file_id: LocalFileId<'_>) -> Result<(), EngineError> {
    let ours = if session.is_create {
        session.self_minted
    } else {
        session.server_file_id == local_file_id.as_str()
    };
    if ours {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "server_changed_item_id: the upload init answered with an id this device did not choose"
    ).into())
}

/// The server no longer accepts writes to this upload session: 404 (session
/// or file row gone — e.g. reaped by the stale-upload sweep), 410, or 400
/// (session not writable / chunk plan no longer matches / a chunk the client
/// believed acknowledged is missing at `complete`). Resuming cannot succeed;
/// the op must start a fresh session.
pub(super) fn upload_session_is_gone(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .filter_map(reqwest::Error::status)
        .any(|status| matches!(status.as_u16(), 400 | 404 | 410))
}

/// The class of an `init` 409 somewhere in `error`'s chain (spec §8.4).
pub(crate) fn init_conflict_class(error: &anyhow::Error) -> Option<crate::api_client::InitConflictClass> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<crate::api_client::InitConflict>())
        .map(|conflict| conflict.class)
}

/// Spec §7.2 (final review): the parent an upload's metadata step sends. A File Provider save
/// records in its accept whether it moved the file, or follows a queued save that did
/// (`parent_changed`): then its parent, the top level as `null`; otherwise none. An upload with
/// no record (a create, the watcher's, Windows', an earlier build's) sends its parent when it
/// has one, as before.
pub(super) fn upload_parent_change<'a>(
    op: &'a PendingOperation,
    metadata: &serde_json::Value,
) -> crate::api_client::ParentChange<'a> {
    use crate::api_client::ParentChange;
    match metadata.get("parent_changed").and_then(serde_json::Value::as_bool) {
        Some(true) => ParentChange::to(op.parent_id.as_deref()),
        Some(false) => ParentChange::Unchanged,
        None => op
            .parent_id
            .as_deref()
            .map_or(ParentChange::Unchanged, ParentChange::Folder),
    }
}

pub(crate) fn local_file_path_under_sync_root(sync_root: &Path, rel_path: &str) -> Result<PathBuf, EngineError> {
    crate::reject_unsafe_rel_path(rel_path)
        .map_err(|e| anyhow::anyhow!("local path must stay under the sync root: {e}"))?;
    Ok(sync_root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

pub(super) fn record_moved_to_trash_activity(db: &StateDb, file_id: &str, rel_path: &str, occurred_at: i64) -> Result<(), EngineError> {
    db.record_local_activity(LocalActivityEventInput {
        event_type: LocalActivityKind::MovedToTrash,
        file_id: Some(file_id.to_string()),
        file_name: display_name_for_path(rel_path),
        rel_path: Some(rel_path.to_string()),
        occurred_at,
    })?;
    Ok(())
}

/// Re-snapshots that must fail in a row before a failed one no longer ends the tick (spec §6.3.2).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) const FAILED_SNAPSHOTS_BEFORE_THE_QUEUE_RUNS: u32 = 3;

/// A requested re-snapshot failed (spec §6.3.2). On macOS the request survives a failure, so the
/// next tick tries again. The first two failures in a row end the tick, as before; from the third
/// on, the tick goes on with the ops pull from `cursor` and the queue runs: an upload whose base is
/// known needs no snapshot, and one still waiting for its base keeps waiting at the claim. Before,
/// a snapshot that kept failing for a reason waiting does not cure (a local database error, an
/// endpoint that times out on a large tree) stopped every upload on the Mac. A successful
/// snapshot resets the count. Elsewhere the request was taken before the snapshot, so a failure
/// ends only this tick.
pub(super) fn after_failed_snapshot(bridge: &EngineBridge, cursor: i64, error: anyhow::Error) -> Result<i64, EngineError> {
    #[cfg(target_os = "macos")]
    {
        let failures = bridge.failed_snapshots.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        if failures >= FAILED_SNAPSHOTS_BEFORE_THE_QUEUE_RUNS {
            tracing::warn!(
                failures,
                reason = error_category(error.as_ref()),
                "sync_tick: re-snapshot failed again; pulling ops and running the queue without it"
            );
            return Ok(cursor);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (bridge, cursor);
    Err(error.into())
}

#[cfg(test)]
pub async fn sync_tick(
    bridge: &EngineBridge,
    // See `sync_tick_outcome` for the `sync_root` contract.
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
) -> anyhow::Result<Vec<ConflictDetected>> {
    Ok(sync_tick_outcome(bridge, sync_root).await?.conflicts)
}

/// The real tick body — same behavior as [`sync_tick`], plus the applied item
/// ids for the working-set signal (T3).
pub async fn sync_tick_outcome(
    bridge: &EngineBridge,
    // The on-disk vault root. Needed on Windows so the deletion-reconcile paths
    // (`apply_sync_op` / `apply_snapshot`) can locate and remove the on-disk Cloud
    // Files placeholder of a remotely-deleted row — not just its DB row (task
    // 0806). Unused on macOS/Linux (the OS extension owns the namespace there);
    // `#[cfg_attr]` silences the unused warning on those builds.
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
) -> anyhow::Result<SyncTickOutcome> {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut conflicts: Vec<ConflictDetected> = Vec::new();
    let mut applied_item_ids: Vec<String> = Vec::new();
    bridge.maintain_owned_keys(now_secs).await?;

    // ── /sync delta engine (task 0789) ────────────────────────────────────────
    //
    // Replaces the old per-folder full-tree `/files` BFS re-walk (`1 + folders`
    // requests/tick, never pruned server deletions, self-saturated the per-IP
    // 429 limit). Two cheap paths, gated on whether the op cursor has EVER been
    // persisted (`Option`, NOT a 0 sentinel):
    //
    //   cursor UNSET (never bootstrapped) → BOOTSTRAP via ONE `GET /sync/snapshot`:
    //     the snapshot is the authoritative non-trashed tree, so we (a) apply
    //     every node through the SAME ingest path the BFS used
    //     (`process_metadata_row`, which keeps conflict detection identical +
    //     flows rows through `upsert_file` for Windows `refresh_placeholders`),
    //     (b) PRUNE every own-tree row the snapshot omits (the
    //     deletion-reconciliation fix), and (c) store the snapshot's `seq_id` as
    //     the cursor — which may legitimately be 0. The server returns
    //     `MAX(seq_id) … unwrap_or(0)` and `sync_ops` is NOT backfilled, so a
    //     pre-existing vault (or a brand-new one before its first create-op
    //     lands) snapshots at seq_id 0. seq_id 0 is therefore a VALID
    //     bootstrapped cursor, NOT an "unset" sentinel: gating on `== 0` would
    //     re-snapshot + re-prune the WHOLE tree every tick forever and never
    //     reach the cheap ops path (defeating the 429 fix).
    //
    //   cursor SET (incl. Some(0)) → CATCH-UP via ONE `GET /sync/ops?since={cursor}`:
    //     apply each op by `op_type` and advance the cursor to the max applied
    //     `seq_id`. With `since=0` the server returns every op `> 0`, which is
    //     exactly right for a just-bootstrapped empty/low op log. ~1
    //     request/tick in steady state.
    //
    // Gap / since-too-old fallback: if applying ops can't proceed coherently
    // (the cursor is ahead of the server, or an op references a row we can't
    // place), we re-bootstrap from a fresh snapshot — the snapshot is always
    // authoritative, so a re-bootstrap can never lose data.

    // A pending re-snapshot request (e.g. a `file_restore` op the previous tick
    // couldn't materialise from its `{id}`-only payload) forces a bootstrap this
    // tick regardless of the cursor. On macOS the request is read here and cleared
    // only after the bootstrap succeeded (spec §6.3.2, I-2(b)): a failed snapshot
    // keeps it, and a request made while the snapshot ran survives the clear.
    // Elsewhere the flag is taken here, once.
    #[cfg(target_os = "macos")]
    let resnapshot_request = {
        // §6.3.2: while any upload waits for its base, every pass asks for a snapshot.
        if bridge.db().has_base_pending_uploads()? {
            bridge.db().request_resnapshot()?;
        }
        bridge.db().peek_resnapshot_request()?
    };
    #[cfg(target_os = "macos")]
    let needs_resnapshot = resnapshot_request.is_some();
    #[cfg(not(target_os = "macos"))]
    let needs_resnapshot = bridge.db().take_needs_resnapshot()?;

    let cursor = match bridge.db().get_sync_cursor()? {
        // Never bootstrapped, or a re-snapshot was explicitly requested → full
        // snapshot. (Some(0) is a real cursor and does NOT bootstrap here.)
        None => {
            let applied = bootstrap_from_snapshot(bridge, sync_root, now_secs, &mut conflicts).await?;
            #[cfg(target_os = "macos")]
            {
                bridge.failed_snapshots.store(0, Ordering::SeqCst);
                if let Some(seen) = resnapshot_request {
                    bridge.db().clear_resnapshot_request(seen)?;
                }
            }
            applied_item_ids.extend(applied);
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: true,
            });
        }
        Some(c) if needs_resnapshot => {
            tracing::info!("sync_tick: re-snapshot requested (gap recovery); bootstrapping");
            match bootstrap_from_snapshot(bridge, sync_root, now_secs, &mut conflicts).await {
                Ok(applied) => {
                    #[cfg(target_os = "macos")]
                    {
                        bridge.failed_snapshots.store(0, Ordering::SeqCst);
                        if let Some(seen) = resnapshot_request {
                            bridge.db().clear_resnapshot_request(seen)?;
                        }
                    }
                    applied_item_ids.extend(applied);
                    return Ok(SyncTickOutcome {
                        conflicts,
                        applied_item_ids: dedupe(applied_item_ids),
                        reached_server: true,
                    });
                }
                Err(e) => after_failed_snapshot(bridge, c, e.into())?,
            }
        }
        Some(c) => c,
    };

    let ops = match bridge.api().sync_ops(cursor).await {
        Ok(ops) => ops,
        Err(e) => {
            // Transient list failure (network / lingering 429 after backoff): do
            // NOT advance the cursor and do NOT fall back to a (heavier)
            // snapshot — just skip this tick and retry next time. The cursor is
            // unchanged, so no op is missed.
            tracing::warn!(error = %e, cursor, "sync_tick: /sync/ops failed; retrying next tick");
            // The pull did not come back: `Ok`, but not a tick that reached the server (spec D11, review P1).
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: false,
            });
        }
    };

    // New servers carry the authoritative per-user head in the delta response.
    // Idle ticks need no full tree fetch. Keep the snapshot probe only for a
    // reset (cursor ahead) or an old server that does not advertise its head.
    if ops.ops.is_empty() {
        if ops.head_seq.is_some_and(|head| head >= cursor) {
            // The delta came back (empty, at the head): this tick reached the server.
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: true,
            });
        }
        // `now_secs` (captured at tick start) is the prune freshness cutoff: it
        // predates this snapshot fetch, so it can't be later than any row a
        // concurrent completion stamps during the fetch.
        let fetched_at = now_secs;
        match bridge.api().sync_snapshot().await {
            Ok(mut snap) if snap.seq_id < cursor => {
                snap.nodes=bridge.prepare_owned_rows(&snap.nodes).await?.into_iter().map(|p|p.row).collect();
                tracing::warn!(
                    cursor,
                    server_seq = snap.seq_id,
                    "sync_tick: cursor ahead of server op-log head; re-bootstrapping from snapshot"
                );
                let applied = apply_snapshot(bridge, sync_root, &snap, now_secs, fetched_at, &mut conflicts)?;
                applied_item_ids.extend(applied);
            }
            Ok(_) => { /* cursor still valid, nothing to do */ }
            Err(e) => {
                tracing::warn!(error = %e, "sync_tick: snapshot freshness probe failed; retrying next tick");
            }
        }
        return Ok(SyncTickOutcome {
            conflicts,
            applied_item_ids: dedupe(applied_item_ids),
            reached_server: true,
        });
    }

    // Apply the delta ops in order, advancing the cursor to the max seq_id.
    let mut max_seq = cursor;
    for original in &ops.ops {
        let prepared = prepare_owned_sync_op(bridge, original).await?;
        bridge.owner_key_scope().check()?;
        max_seq = max_seq.max(original.seq_id);
        let Some(op) = prepared else { continue; };
        match apply_sync_op(bridge, sync_root, &op, now_secs, &mut conflicts) {
            Ok(applied) => applied_item_ids.extend(applied),
            Err(e) => {
                // A failed op applied nothing: it contributes no ids.
                tracing::warn!(error = %e, op_type = %op.op_type, seq_id = op.seq_id, "sync_tick: op apply error; skipping op");
            }
        }
    }
    if max_seq > cursor {
        bridge.db().set_sync_cursor(max_seq)?;
    }
    Ok(SyncTickOutcome {
        conflicts,
        applied_item_ids: dedupe(applied_item_ids),
        reached_server: true,
    })
}

/// Enrich actual sparse events before entering the synchronous reconciler.
pub(super) async fn prepare_owned_sync_op(bridge:&EngineBridge,op:&crate::api_client::SyncOp)->Result<Option<crate::api_client::SyncOp>, EngineError> {
    if !matches!(op.op_type.as_str(),"file_create"|"folder_create"|"file_update"|"file_rename"|"folder_rename"|"file_move"|"folder_move") {return Ok(Some(op.clone()));}
    let Some(id)=op.payload["id"].as_str() else{return Ok(Some(op.clone()))};
    let known_foreign=bridge.db().get_owned_key_record(id)?.is_some_and(|r|r.foreign_added || r.owner_wrap.is_some());
    let mut row=synthesize_op_row(bridge,id,op,op.payload["new_name_encrypted"].as_str().or_else(||op.payload["name_encrypted"].as_str()));
    if known_foreign && op.payload["name_encrypted"].is_null() && op.payload["new_name_encrypted"].is_null() {row=serde_json::json!({"id":id});}
    let mut prepared=bridge.prepare_owned_rows(&[row]).await?;
    let Some(prepared)=prepared.pop() else{return Ok(None)};
    let mut result=op.clone();result.payload=prepared.row;
    if matches!(op.op_type.as_str(),"file_move"|"folder_move") {result.payload["new_parent_id"]=result.payload["parent_id"].clone();}
    if matches!(op.op_type.as_str(),"file_rename"|"folder_rename") {result.payload["new_name_encrypted"]=result.payload["name_encrypted"].clone();}
    Ok(Some(result))
}

/// Pull a fresh `/sync/snapshot` and reconcile the whole mirror against it, then
/// store its `seq_id` as the new cursor. The bootstrap path (cursor unset) AND
/// the gap-recovery path both funnel through here so the behaviour is identical.
pub(super) async fn bootstrap_from_snapshot(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> Result<Vec<String>, EngineError> {
    // `now_secs` is captured at tick start, BEFORE this request — so it is the
    // prune freshness cutoff that never prunes a row a concurrent local
    // completion stamps while this snapshot is in flight (see `prune_absent`).
    let fetched_at = now_secs;
    let mut snapshot = bridge.api().sync_snapshot().await?;
    snapshot.nodes=bridge.prepare_owned_rows(&snapshot.nodes).await?.into_iter().map(|p|p.row).collect();
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut applied = apply_snapshot(bridge, sync_root, &snapshot, now_secs, fetched_at, conflicts)?;
    // §6.3.3: an upload still waiting for its base after this successful snapshot counts
    // one more; at the 10th it parks with its bytes. The park changes how its item is
    // presented (spec §9.1), and the snapshot did not list the file, so its id joins the
    // tick's applied ids, which the tick's one working-set signal carries.
    #[cfg(target_os = "macos")]
    for (op_id, file_id) in bridge.db().note_snapshot_for_base_pending(now_secs)? {
        log_parked(&op_id, file_id.as_deref(), ParkReason::BaseUnknown);
        applied.extend(file_id);
    }
    Ok(applied)
}

/// Reconcile the local mirror against an already-fetched snapshot:
///   1. order nodes parent-before-child and compute each node's parent rel path,
///   2. ingest every node through `process_metadata_row` (same conflict logic +
///      `upsert_file` placeholder-freshness flow the old BFS used),
///   3. PRUNE every own-tree row the snapshot omits (deletion reconciliation),
///      EXCEPT rows touched at/after `snapshot_fetched_at` (a just-completed
///      upload) and EXCEPT the whole tree when the snapshot is suspiciously empty
///      (both handled inside `prune_absent`),
///   4. store the snapshot's `seq_id` as the cursor.
///
/// `snapshot_fetched_at` is the wall-clock second the snapshot HTTP fetch began;
/// it is the prune freshness cutoff so an upload re-keyed mid-flight survives.
///
/// Split out from `bootstrap_from_snapshot` so it's directly unit-testable with
/// a synthetic `SyncSnapshot` (no HTTP).
pub(super) fn apply_snapshot(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    snapshot: &crate::api_client::SyncSnapshot,
    now_secs: i64,
    snapshot_fetched_at: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> Result<Vec<String>, EngineError> {
    // Task 1697 review fix (T3): every ingested and pruned item id flows back
    // to the tick, which signals the File Provider working set with the union.
    let mut applied: Vec<String> = Vec::new();
    // Resolve each node's PARENT relative path. The snapshot is a flat node list
    // (each with `parent_id`); the old BFS got nesting for free by listing
    // folder-by-folder. Here we topologically order so a parent's resolved path
    // is known before its children, then thread it into `process_metadata_row`
    // exactly as the BFS threaded the folder's path.
    let order = order_snapshot_nodes(&snapshot.nodes);

    let mut seen: HashSet<String> = HashSet::new();
    // file_id → resolved FULL relative path (so children can prefix their leaf).
    let mut resolved_paths: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for f in order {
        let file_id = f["id"].as_str().unwrap_or_default();
        if file_id.is_empty() {
            continue;
        }
        if !seen.insert(file_id.to_string()) {
            continue;
        }
        // Parent's resolved path ("" at the vault root, or when the parent
        // didn't resolve — the leaf then sits at root, matching BFS behaviour
        // where an unresolved parent frame simply wasn't descended).
        let parent_rel_path = match f["parent_id"].as_str().filter(|pid| !pid.is_empty()) {
            Some(pid) => resolved_paths.get(pid).cloned().unwrap_or_else(|| {
                // If the parent is missing from this snapshot but the child already
                // exists locally, keep its current parent prefix for this ingest.
                // A concurrent folder trash can legitimately produce a partial
                // snapshot that still lists the folder's children but not the folder;
                // re-rooting those children before `prune_absent` runs would hide
                // them from the absent-folder subtree sweep.
                let existing_parent = existing_parent_rel_path(bridge, file_id);
                if !existing_parent.is_empty() {
                    tracing::debug!(
                        file_id = %file_id,
                        parent_id = %pid,
                        parent_rel_path = %existing_parent,
                        "sync_tick: preserving existing parent path for snapshot node whose parent is absent"
                    );
                }
                existing_parent
            }),
            None => String::new(),
        };

        if let Some((rel_path, _kind)) =
            process_metadata_row(bridge, f, &parent_rel_path, now_secs, RowSource::Snapshot, conflicts)?
        {
            if !rel_path.is_empty() {
                resolved_paths.insert(file_id.to_string(), rel_path);
            }
            applied.push(file_id.to_string());
        }
    }

    // Deletion reconciliation: anything in the local own-tree the snapshot did
    // NOT mention was deleted/trashed server-side. `prune_absent` excludes shared
    // rows, rows with a pending upload/op, rows touched at/after the snapshot
    // fetch (a just-completed upload the snapshot predates), and refuses to prune
    // the whole tree on a suspicious EMPTY snapshot (see its doc) — so neither an
    // in-flight local create nor a just-uploaded file is ever collateral.
    let pruned = bridge.db().prune_absent(&seen, snapshot_fetched_at)?;
    if !pruned.is_empty() {
        tracing::info!(count = pruned.len(), "sync_tick: pruned rows absent from snapshot");
        // task 0806: removing only the DB row left the on-disk Cloud Files
        // placeholder ghosting in Explorer. Now ALSO remove each pruned row's
        // placeholder. `prune_absent` returns rows CHILDREN-BEFORE-PARENTS (orphan
        // descendants of a pruned folder precede the folder), which is exactly the
        // order placeholder removal needs (leaf files before their directory).
        remove_pruned_placeholders(bridge.db(), sync_root, &pruned);
        // A pruned row is a REMOTE deletion Finder must see (T3).
        applied.extend(pruned.iter().map(|row| row.file_id.clone()));
    }

    bridge.db().set_sync_cursor(snapshot.seq_id)?;
    Ok(applied)
}

/// Order snapshot nodes so every node appears AFTER its parent (parents-first),
/// which lets `apply_snapshot` resolve a parent's path before its children.
///
/// Roots (no `parent_id`, or a `parent_id` not present in the node set — a
/// shared-into / cross-tree parent) come first; remaining nodes are emitted as
/// their parents become available. Any nodes left in a `parent_id` cycle the
/// server should never produce are appended at the end so they're still ingested
/// (just possibly with an empty parent path), never dropped.
pub(super) fn order_snapshot_nodes(nodes: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    use std::collections::HashMap;
    let present: HashSet<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    // children[parent_id] = [node, …]
    let mut children: HashMap<&str, Vec<&serde_json::Value>> = HashMap::new();
    let mut roots: Vec<&serde_json::Value> = Vec::new();
    for n in nodes {
        match n["parent_id"].as_str() {
            Some(pid) if present.contains(pid) => children.entry(pid).or_default().push(n),
            _ => roots.push(n), // root, or parent outside the snapshot set
        }
    }
    let mut ordered: Vec<&serde_json::Value> = Vec::with_capacity(nodes.len());
    let mut queue: VecDeque<&serde_json::Value> = roots.into_iter().collect();
    let mut emitted: HashSet<&str> = HashSet::new();
    while let Some(n) = queue.pop_front() {
        let Some(id) = n["id"].as_str() else { continue };
        if !emitted.insert(id) {
            continue;
        }
        ordered.push(n);
        if let Some(kids) = children.get(id) {
            for k in kids {
                queue.push_back(k);
            }
        }
    }
    // Defensive: emit any node not reached (a parent_id cycle) so nothing is lost.
    for n in nodes {
        if let Some(id) = n["id"].as_str() {
            if emitted.insert(id) {
                ordered.push(n);
            }
        }
    }
    ordered
}

/// Apply ONE `/sync/ops` delta to the local mirror by `op_type`. Mirrors the
/// server's op vocabulary (`server/.../routes/sync.rs` + the `emit_sync_op`
/// call sites in `routes/files.rs` / `routes/uploads.rs`):
///   * `file_create` / `folder_create` → ingest the new row (cloud_only),
///   * `file_update`                   → refresh the existing row's metadata,
///   * `file_rename` / `folder_rename` → update the leaf name (re-derives path),
///   * `file_move`   / `folder_move`   → re-parent the row,
///   * `file_trash`  / `file_delete`   → remove the local mirror row,
///   * `file_restore`                  → request a re-snapshot ({id}-only payload
///     can't rebuild the row; the authoritative snapshot re-materialises it).
///
/// Each op payload is `{ id, … }`. For create/update/rename/move we synthesise a
/// `serde_json::Value` row and feed it through the SAME `process_metadata_row`
/// ingest path the snapshot uses, so conflict detection + placeholder freshness
/// are identical across both paths. Nested paths ARE resolved from the op's
/// `parent_id`/`new_parent_id` against the local mirror (`parent_rel_path_by_id`),
/// matching how `apply_snapshot` threads parent paths — because the Windows
/// placeholder seeder (`populate_placeholders`) derives the on-disk location
/// purely from the row's stored `path`, NOT from `parent_id`, and there is no
/// periodic snapshot to fix a mis-prefixed row later. When a parent isn't locally
/// known yet, the op requests a re-snapshot so nesting converges next tick.
pub(super) fn apply_sync_op(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    op: &crate::api_client::SyncOp,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> Result<Vec<String>, EngineError> {
    // Task 1697 review fix (T3): the ids of items this op actually changed on
    // the local mirror — what the runner's working-set signal must carry.
    let mut applied: Vec<String> = Vec::new();
    let payload = &op.payload;
    let id = payload["id"].as_str().unwrap_or_default();
    if id.is_empty() {
        return Ok(applied);
    }

    match op.op_type.as_str() {
        // Task 1698 (trash ruling — full sync): the two server-side deletion
        // kinds now mean different things locally.
        //
        // `file_trash` — the server TRASH (is_trashed=TRUE, recoverable): the
        // mirror flips the subtree to `Trashing` (the macOS Trash view) and
        // keeps the rows; each flipped row records a `Modified` change so the
        // replica moves it into the trash container. A `Trashing` row is the
        // server echo of the LOCAL delete-in-flight (0802) — the row already
        // IS the trash view, nothing to flip.
        //
        // `file_delete` — the server PERMANENT delete (password-confirmed in
        // the app/web, or the retention janitor): the mirror row leaves the
        // trash view for good — the row is deleted and a `deleted` change is
        // recorded. This is the convergence path that replaced prune_absent's
        // 0802 sweep (flipped by task 1698).
        "file_trash" => match bridge.db().get_file(id)? {
            Some(entry) if entry.status == crate::state_db::FileStatus::Trashing => {
                // PR #100 review (Codex P1): the root may already be parked
                // `Trashing` by the LOCAL trash flow — `queue_finder_delete`
                // parks ONLY the folder row, its descendants stay unmarked
                // until this server echo arrives. This echo IS that arrival:
                // mark the still-unmarked subtree (mark_subtree_trashing
                // no-ops the parked root and every already-Trashing row, so
                // no duplicate change rows) — otherwise prune_absent's
                // immediate-parent protection cannot cover grandchildren and
                // they drop out of the trash view.
                let marked = bridge.db().mark_subtree_trashing(id)?;
                remove_pruned_placeholders(bridge.db(), sync_root, &marked);
                applied.extend(marked.iter().map(|row| row.file_id.clone()));
            }
            Some(_) => {
                // Mark the subtree Trashing (the trash view), remove the
                // on-disk placeholders exactly as the delete path did (the
                // item is hidden locally — that part of 0802 stands), and
                // report the ids so the working-set signal fires.
                let marked = bridge.db().mark_subtree_trashing(id)?;
                remove_pruned_placeholders(bridge.db(), sync_root, &marked);
                applied.push(id.to_string());
                applied.extend(marked.iter().map(|row| row.file_id.clone()));
            }
            None => {
                let orphans = bridge.db().delete_orphaned_children_of_absent_folder(id)?;
                if !orphans.is_empty() {
                    tracing::info!(
                        folder_id = %id,
                        count = orphans.len(),
                        "sync_tick: trashed folder had no local row but \
                         {} orphaned child(ren) — pruned (task 0828)",
                        orphans.len()
                    );
                    remove_pruned_placeholders(bridge.db(), sync_root, &orphans);
                    // The orphaned children left the mirror (T3); the folder
                    // itself never existed locally, so it contributes nothing.
                    applied.extend(orphans.iter().map(|row| row.file_id.clone()));
                }
            }
        },
        "file_delete" => match bridge.db().get_file(id)? {
            Some(_) => {
                // The item is GONE server-side (irreversible): drop the row —
                // Trashing rows leave the trash view, live rows leave the tree
                // (unchanged 0806 behavior for a live row).
                let removed = bridge.db().delete_file_subtree(id)?;
                remove_pruned_placeholders(bridge.db(), sync_root, &removed);
                applied.push(id.to_string());
                applied.extend(removed.iter().map(|row| row.file_id.clone()));
            }
            None => {
                let orphans = bridge.db().delete_orphaned_children_of_absent_folder(id)?;
                if !orphans.is_empty() {
                    remove_pruned_placeholders(bridge.db(), sync_root, &orphans);
                    applied.extend(orphans.iter().map(|row| row.file_id.clone()));
                }
            }
        },
        "file_restore" => {
            // The restore payload is only `{ id }` (server `routes/files.rs`), so
            // the un-trashed row's full metadata is NOT recoverable from the op.
            // We already DELETED this row on the preceding `file_trash`/
            // `file_delete` op, and there is NO periodic snapshot in steady state
            // (the cursor only advances via ops once bootstrapped) — so a no-op
            // here would make a trash-then-restore on another device leave the
            // file permanently invisible on THIS device. Request a re-bootstrap
            // on the next tick: the authoritative snapshot re-materialises the
            // restored row. Cheap and matches the existing gap-recovery design.
            //
            // Task 1698 (trash ruling): when we still HOLD the row — it stayed
            // in the mirror as `Trashing` (the trash view) instead of being
            // pruned — the restore flips it back OUT of the trash view
            // directly: `CloudOnly` (placeholder re-mints; content
            // re-downloads on open), and the flip is reported so the replica
            // reparents the item out of the trash container.
            if let Some(row) = bridge.db().get_file(id)?
                && row.status == crate::state_db::FileStatus::Trashing
            {
                // PR #100 review (Codex P1): the restore must take the ENTIRE
                // held subtree out of the trash view, not just the root row —
                // the authoritative re-snapshot preserves `Trashing` rows by
                // design, so it can never repair the descendants (they would
                // linger in the Trash view; direct children even become
                // top-level trash entries after the parent leaves).
                let flipped = bridge.db().untrash_subtree(id)?;
                remove_pruned_placeholders(bridge.db(), sync_root, &flipped);
                applied.extend(flipped.iter().map(|r| r.file_id.clone()));
            }
            bridge.db().request_resnapshot()?;
            tracing::info!(
                file_id = %id,
                "sync_tick: file_restore op — scheduling re-snapshot to re-materialise the row"
            );
        }
        "file_rename" | "folder_rename" => {
            // Re-key the leaf name. Build a row carrying the NEW name_encrypted so
            // `resolve_relative_path` re-derives the plaintext leaf. We keep the
            // existing row's parent path by reading its current path's parent.
            let new_name = payload["new_name_encrypted"].as_str();
            let row = synthesize_op_row(bridge, id, op, new_name);
            let parent_rel = existing_parent_rel_path(bridge, id);
            if process_metadata_row(bridge, &row, &parent_rel, now_secs, RowSource::Op, conflicts)?.is_some() {
                applied.push(id.to_string());
            }
        }
        "file_move" | "folder_move" => {
            // Re-parent. The op gives `new_parent_id`; the leaf name is unchanged.
            // The move op carries no name blob, so `synthesize_op_row` supplies the
            // unchanged leaf as a plaintext `"name"` fallback from the existing
            // row's stored path (the `files` table doesn't persist name_encrypted).
            let row = synthesize_op_row(bridge, id, op, None);
            // Resolve the NEW parent's full relative path from the local mirror by
            // `new_parent_id` (the same way `apply_snapshot` resolves nesting), so
            // a move-into-folder lands the row under its folder prefix instead of
            // at the sync root. If the new parent isn't locally known yet, fall
            // back to a re-snapshot rather than mis-placing the row at root.
            // Spec §7.2 (final review): with no `new_parent_id` key the op names no
            // new parent, so the row keeps its path's folders as it keeps its parent
            // (`synthesize_op_row`); a key decides both, `null` meaning the top level.
            let parent_rel = if payload.get("new_parent_id").is_some() {
                parent_rel_path_by_id(bridge, payload["new_parent_id"].as_str())
            } else {
                existing_parent_rel_path(bridge, id)
            };
            if process_metadata_row(bridge, &row, &parent_rel, now_secs, RowSource::Op, conflicts)?.is_some() {
                applied.push(id.to_string());
            }
        }
        "file_create" | "folder_create" | "file_update" => {
            // §6.3.2 (I-3): a content op without `version_number` cannot say which version it
            // made (a legacy chunked replace looks like a create), so the snapshot settles it.
            // A content op on a row this Mac already holds means the row's version is no
            // longer a snapshot fill (§6.1). A file op is a content op when it is a create, or
            // carries a version or a size; the server's thumbnail-flag ops (`has_thumbnail`,
            // `has_large_thumbnail`) carry neither, and are not.
            #[cfg(target_os = "macos")]
            let content = op.op_type == "file_create"
                || (op.op_type == "file_update"
                    && (payload["version_number"].as_i64().is_some() || payload["size_bytes"].as_i64().is_some()));
            #[cfg(target_os = "macos")]
            let versionless = content && payload["version_number"].as_i64().is_none();
            #[cfg(target_os = "macos")]
            let touches_existing_row = content && bridge.db().get_file(id)?.is_some();
            let row = synthesize_op_row(bridge, id, op, payload["name_encrypted"].as_str());
            // For a CREATE there is no existing local row, so the parent path must
            // come from the op's `parent_id` resolved against the local mirror —
            // NOT from the (nonexistent) row's own path, which would yield "" and
            // mis-place a nested create at the sync root. `populate_placeholders`
            // derives the on-disk location purely from the row's `path`, so the
            // path prefix MUST be correct here; there is no periodic snapshot to
            // fix it later. For an UPDATE the row already exists, so keep its
            // current parent prefix (`existing_parent_rel_path`).
            let parent_rel = if op.op_type == "file_update" {
                existing_parent_rel_path(bridge, id)
            } else {
                parent_rel_path_by_id(bridge, payload["parent_id"].as_str())
            };
            if process_metadata_row(bridge, &row, &parent_rel, now_secs, RowSource::Op, conflicts)?.is_some() {
                applied.push(id.to_string());
            }
            #[cfg(target_os = "macos")]
            {
                if touches_existing_row {
                    bridge.db().clear_version_filled(id)?;
                }
                if versionless {
                    bridge.db().request_resnapshot()?;
                }
            }
        }
        other => {
            tracing::debug!(op_type = other, "sync_tick: ignoring unknown op_type");
        }
    }
    // A sparse folder rename/move has no descendant events. Reuse the same
    // metadata write for their new parent paths, then signal those identities
    // with the folder. This preserves content baselines and queued rename holds.
    if matches!(op.op_type.as_str(), "folder_rename" | "folder_move" | "file_rename" | "file_move")
        && applied.iter().any(|changed| changed == id)
        && bridge.db().get_file(id)?.is_some_and(|row| row.is_dir())
    {
        rebase_folder_descendants(bridge, id, now_secs, &mut applied)?;
    }
    Ok(applied)
}

pub(super) fn rebase_folder_descendants(
    bridge: &EngineBridge,
    folder_id: &str,
    now: i64,
    applied: &mut Vec<String>,
) -> Result<(), EngineError> {
    let rows = bridge.db().list_files()?;
    let mut parents = vec![folder_id.to_string()];
    let mut visited = std::collections::HashSet::new();
    while let Some(parent_id) = parents.pop() {
        if !visited.insert(parent_id.clone()) {
            continue;
        }
        let Some(parent) = bridge.db().get_file(&parent_id)? else { continue };
        for child in rows.iter().filter(|row| row.parent_id.as_deref() == Some(&parent_id)) {
            let Some(contract) = bridge.db().get_file_contract_state(&child.file_id)? else { continue };
            let row = serde_json::json!({
                "id": child.file_id,
                "name": child.path.rsplit('/').next().unwrap_or_default(),
                "parent_id": parent_id,
                "is_folder": child.is_dir(),
                "size_bytes": child.size_bytes,
                "updated_at": child.remote_updated_at,
                "version_number": contract.current_version,
                "current_object_version_id": contract.current_object_version_id,
                "content_type": contract.content_type,
            });
            if let Some(updated) = apply_metadata_file_row(
                bridge.db(), &row, contract.namespace, contract.shared_root_id,
                contract.share_id, contract.permission_bits, now, bridge.api().master_key(), &parent.path,
            )? {
                if updated.path != child.path {
                    applied.push(child.file_id.clone());
                }
                if updated.is_dir() {
                    parents.push(child.file_id.clone());
                }
            }
        }
    }
    Ok(())
}

/// Build a `/files`-shaped `serde_json::Value` from a sync op + the existing
/// local row, so `process_metadata_row` ingests it identically to a snapshot
/// node. Carries the op's `id` plus any of `name_encrypted` / `parent_id` /
/// `size_bytes` / `version_number` / `is_folder` it can determine, falling back
/// to the existing row's values where the op is partial (e.g. a `file_update`
/// that only changed `has_thumbnail`).
pub(super) fn synthesize_op_row(
    bridge: &EngineBridge,
    id: &str,
    op: &crate::api_client::SyncOp,
    name_encrypted: Option<&str>,
) -> serde_json::Value {
    let payload = &op.payload;
    let existing = bridge.db().get_file(id).ok().flatten();
    let existing_contract = bridge.db().get_file_contract_state(id).ok().flatten();

    let is_folder = op.op_type.starts_with("folder_")
        || existing
            .as_ref()
            .map(|e| e.item_kind == ItemKind::Folder)
            .unwrap_or(false);

    let mut row = serde_json::json!({ "id": id, "is_folder": is_folder });

    // name_encrypted: op-provided wins (create/rename carry it). A MOVE op carries
    // NO name blob, and the `files` table does NOT persist `name_encrypted`, so it
    // is genuinely unrecoverable from the op for a move. To still compose the
    // correct path, fall back to the EXISTING row's plaintext leaf (the last
    // segment of its already-decrypted stored `path`) as the `"name"` field —
    // `resolve_relative_path` prefers `name_encrypted` but falls through to
    // `"name"`, so the moved row keeps its leaf under the new parent prefix
    // instead of resolving to an empty path (which would mis-seed at the root or
    // skip the row entirely).
    if let Some(name) = name_encrypted.or_else(|| payload["name_encrypted"].as_str()) {
        row["name_encrypted"] = serde_json::json!(name);
    } else if let Some(leaf) = existing.as_ref().and_then(|e| {
        e.path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
    }) {
        row["name"] = serde_json::json!(leaf);
    }
    // parent_id: a move sets new_parent_id; create/update set parent_id; else keep
    // the existing parent. Spec §7.2 (final review): a move's `new_parent_id` of `null` is
    // the top level, never "keep the parent".
    let parent_id = if op.op_type.ends_with("_move") && payload.get("new_parent_id").is_some() {
        payload["new_parent_id"].as_str().map(str::to_string)
    } else {
        payload["new_parent_id"]
            .as_str()
            .or_else(|| payload["parent_id"].as_str())
            .map(str::to_string)
            .or_else(|| existing.as_ref().and_then(|e| e.parent_id.clone()))
    };
    if let Some(pid) = parent_id {
        row["parent_id"] = serde_json::json!(pid);
    }
    // size_bytes: op-provided wins, else existing.
    let size = payload["size_bytes"]
        .as_i64()
        .or_else(|| existing.as_ref().map(|e| e.size_bytes));
    if let Some(sz) = size {
        row["size_bytes"] = serde_json::json!(sz);
    }
    // version_number: op-provided wins, else the existing contract's.
    let version = payload["version_number"]
        .as_i64()
        .or_else(|| existing_contract.as_ref().map(|c| c.current_version));
    if let Some(v) = version {
        row["version_number"] = serde_json::json!(v);
    }
    // current_object_version_id: a `file_update` carries the new version's
    // object id; without it the contract keeps pointing at the old version.
    // `apply_metadata_file_row` keeps the existing id when the op omits it.
    if let Some(ov) = payload["current_object_version_id"].as_str() {
        row["current_object_version_id"] = serde_json::json!(ov);
    }
    // updated_at: the op log carries no file timestamp the client can read, so
    // stamp "now" as the row's `remote_updated_at` / `modified_at` value. This
    // is NOT a freshness token: several ops routinely land in the same second
    // (a version upload emits `file_rename` + `file_update` together), so
    // `process_metadata_row` never gates an op-derived row on it — see
    // [`RowSource::Op`].
    row["updated_at"] = serde_json::json!(now_secs());
    row
}
