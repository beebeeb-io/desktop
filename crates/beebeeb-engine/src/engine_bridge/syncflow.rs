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

pub fn sync_folder_content(
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
pub(super) fn ensure_session_item_id(
    session: &crate::state_db::UploadResume,
    local_file_id: LocalFileId<'_>,
) -> Result<(), EngineError> {
    let ours = if session.is_create {
        session.self_minted
    } else {
        session.server_file_id == local_file_id.as_str()
    };
    if ours {
        return Ok(());
    }
    Err(
        anyhow::anyhow!("server_changed_item_id: the upload init answered with an id this device did not choose")
            .into(),
    )
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

/// `complete` was refused because the file was renamed while the upload ran
/// (`UploadRefused::file_changed`): the session can never complete with the name it sealed.
pub(super) fn upload_file_changed(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<crate::api_client::UploadRefused>())
        .any(|refused| refused.file_changed)
}

/// The class of an `init` 409 somewhere in `error`'s chain (spec §8.4).
pub fn init_conflict_class(error: &anyhow::Error) -> Option<crate::api_client::InitConflictClass> {
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

pub fn local_file_path_under_sync_root(sync_root: &Path, rel_path: &str) -> Result<PathBuf, EngineError> {
    crate::reject_unsafe_rel_path(rel_path)
        .map_err(|e| anyhow::anyhow!("local path must stay under the sync root: {e}"))?;
    Ok(sync_root.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR)))
}

pub(super) fn record_moved_to_trash_activity(
    db: &StateDb,
    file_id: &str,
    rel_path: &str,
    occurred_at: i64,
) -> Result<(), EngineError> {
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
pub(super) fn after_failed_snapshot(
    bridge: &EngineBridge,
    cursor: i64,
    error: anyhow::Error,
) -> Result<i64, EngineError> {
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

#[cfg(any(test, feature = "test-support"))]
pub async fn sync_tick(
    bridge: &EngineBridge,
    // See `sync_tick_outcome` for the `sync_root` contract.
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
) -> anyhow::Result<Vec<ConflictDetected>> {
    Ok(sync_tick_outcome(bridge, sync_root).await?.conflicts)
}

/// The real tick body — same behavior as [`sync_tick`], plus the applied item
/// ids for the working-set signal. RC2-4: after a pull that reached the server, a rebuilt
/// `state.db`'s check of the files on disk runs, bounded ([`EngineBridge::check_after_rebuild`]);
/// it never fails the tick.
pub async fn sync_tick_outcome(
    bridge: &EngineBridge,
    sync_root: &Path,
) -> anyhow::Result<SyncTickOutcome> {
    let mut outcome = sync_tick_pull(bridge, sync_root).await?;
    if outcome.reached_server {
        // RC3-4: files agreed here leave the "not checked yet" row; the runner counts the tick as a change so the
        // windows read their lists again (not only at the next slow refresh).
        outcome.rebuilt_agreed = bridge.check_after_rebuild(sync_root).await.agreed;
    }
    Ok(outcome)
}

async fn sync_tick_pull(
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

    // Task 2445: a one-time heal for DBs hit by the null-envelope bug. Forces a
    // re-snapshot so every row is re-prepared with the fixed request_envelope.
    // P2-A: do NOT clear blocked_fingerprint (that would lift legitimate blocks
    // from malformed envelopes and lost adoption races). prepare_owned_rows
    // already resets blocked_fingerprint on every pass.
    // P1-B: mark done only AFTER the snapshot succeeds (see below); requesting
    // here and marking after bootstrap prevents a lost heal on network failure.
    let needs_envelope_heal = if bridge.db().envelope_heal_done()? {
        false
    } else if bridge.db().has_blocked_or_nameless_rows()? {
        tracing::info!("task 2445: requesting re-snapshot for the null-envelope heal");
        bridge.db().request_resnapshot()?;
        true
    } else {
        // (b) Healthy DB: mark done so the json_extract scan does not run every tick.
        bridge.db().mark_envelope_heal_done()?;
        false
    };

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
            // P1-B: mark the heal done only after a successful bootstrap.
            if needs_envelope_heal {
                bridge.db().mark_envelope_heal_done()?;
            }
            applied_item_ids.extend(applied);
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: true,
                pull_failure: None,
                rebuilt_agreed: 0,
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
                    // P1-B: mark the heal done only after a successful bootstrap.
                    if needs_envelope_heal {
                        bridge.db().mark_envelope_heal_done()?;
                    }
                    applied_item_ids.extend(applied);
                    return Ok(SyncTickOutcome {
                        conflicts,
                        applied_item_ids: dedupe(applied_item_ids),
                        reached_server: true,
                        pull_failure: None,
                        rebuilt_agreed: 0,
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
            // The pull did not come back: `Ok`, but not a tick that reached the server (spec D11, review P1). It
            // carries why (task 2521): a 401 here is a refusal of the session, which the runner counts.
            return Ok(SyncTickOutcome {
                conflicts,
                applied_item_ids: dedupe(applied_item_ids),
                reached_server: false,
                pull_failure: Some(e),
                rebuilt_agreed: 0,
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
                pull_failure: None,
                rebuilt_agreed: 0,
            });
        }
        // `now_secs` (captured at tick start) is the prune freshness cutoff: it
        // predates this snapshot fetch, so it can't be later than any row a
        // concurrent completion stamps during the fetch.
        let fetched_at = now_secs;
        match bridge.api().sync_snapshot().await {
            Ok(mut snap) if snap.seq_id < cursor => {
                snap.nodes = bridge
                    .prepare_owned_rows(&snap.nodes)
                    .await?
                    .into_iter()
                    .map(|p| p.row)
                    .collect();
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
            pull_failure: None,
            rebuilt_agreed: 0,
        });
    }

    // Apply the delta ops in order, advancing the cursor to the last op that
    // landed. Task 2272: an op that fails to apply holds the cursor just before
    // it (the ops after it wait too, so order is kept) and is tried again after a
    // back-off; one that keeps failing is parked with words and a re-snapshot
    // brings its item level. It is never stepped over silently.
    let mut max_seq = cursor;
    for original in &ops.ops {
        if let Some(retry_at) = op_retry_pending(sync_root, original.seq_id, now_secs) {
            tracing::info!(
                seq_id = original.seq_id,
                retry_in_secs = retry_at - now_secs,
                "sync_tick: an op that failed to apply is waiting for its retry; the cursor stays before it"
            );
            break;
        }
        let prepared = prepare_owned_sync_op(bridge, original).await?;
        bridge.owner_key_scope().check()?;
        let Some(op) = prepared else {
            max_seq = max_seq.max(original.seq_id);
            continue;
        };
        match apply_sync_op_once(bridge, sync_root, &op, now_secs, &mut conflicts) {
            Ok(applied) => {
                // A failed op applied nothing, so only a landed one contributes ids.
                applied_item_ids.extend(applied);
                max_seq = max_seq.max(original.seq_id);
            }
            Err(e) => match note_op_apply_failure(sync_root, &op, &e, now_secs) {
                OpFailure::Retry { attempts, retry_at } => {
                    tracing::warn!(
                        error = %e,
                        op_type = %op.op_type,
                        seq_id = op.seq_id,
                        attempts,
                        retry_in_secs = retry_at - now_secs,
                        "sync_tick: op apply error; the cursor stays before it and it is tried again"
                    );
                    break;
                }
                OpFailure::Parked(parked) => {
                    tracing::error!(
                        op_type = %parked.op_type,
                        seq_id = parked.seq_id,
                        item_id = %parked.item_id,
                        attempts = parked.attempts,
                        "sync_tick: {}",
                        parked.words
                    );
                    // The snapshot is authoritative: it carries the item's state, so the
                    // change the op described still reaches this PC.
                    bridge.db().request_resnapshot()?;
                    max_seq = max_seq.max(original.seq_id);
                }
            },
        }
    }
    if max_seq > cursor {
        bridge.db().set_sync_cursor(max_seq)?;
        forget_op_failures_through(sync_root, max_seq);
    }
    Ok(SyncTickOutcome {
        conflicts,
        applied_item_ids: dedupe(applied_item_ids),
        reached_server: true,
        pull_failure: None,
        rebuilt_agreed: 0,
    })
}

// ── Task 2272: an op that fails to apply ─────────────────────────────────────

/// How many times one op may fail to apply before it is parked.
pub const SYNC_OP_MAX_ATTEMPTS: u32 = 5;

/// The wait after the `attempts`-th failure: 30 s, doubling, at most 10 minutes.
pub(super) fn sync_op_backoff_secs(attempts: u32) -> i64 {
    (30i64 << attempts.saturating_sub(1).min(5)).min(600)
}

/// An op this PC could not apply after [`SYNC_OP_MAX_ATTEMPTS`] tries, for the review list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedSyncOp {
    pub seq_id: i64,
    pub op_type: String,
    pub item_id: String,
    pub attempts: u32,
    /// The last apply error.
    pub error: String,
    /// What the person reads.
    pub words: String,
    pub parked_at: i64,
}

#[derive(Debug, Clone, Copy)]
struct FailingOp {
    attempts: u32,
    retry_at: i64,
}

#[derive(Debug, Default)]
struct OpRetryState {
    failing: HashMap<i64, FailingOp>,
    parked: Vec<ParkedSyncOp>,
}

/// The parked list keeps the newest this many.
const PARKED_SYNC_OPS_KEPT: usize = 100;

/// Per sync root (one per account), the ops that failed to apply. Held in memory: after a restart
/// the cursor still stands before a failing op, so it is simply tried again from its first attempt.
static OP_RETRY: std::sync::LazyLock<std::sync::Mutex<HashMap<PathBuf, OpRetryState>>> =
    std::sync::LazyLock::new(Default::default);

fn op_retry<R>(f: impl FnOnce(&mut HashMap<PathBuf, OpRetryState>) -> R) -> R {
    let mut guard = OP_RETRY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

pub(super) enum OpFailure {
    Retry { attempts: u32, retry_at: i64 },
    Parked(ParkedSyncOp),
}

/// `Some(retry_at)` while the op at `seq_id` failed before and its back-off has not passed.
pub(super) fn op_retry_pending(sync_root: &Path, seq_id: i64, now: i64) -> Option<i64> {
    op_retry(|all| {
        all.get(sync_root)
            .and_then(|state| state.failing.get(&seq_id))
            .map(|f| f.retry_at)
            .filter(|at| *at > now)
    })
}

/// Count one more failure of `op`; at [`SYNC_OP_MAX_ATTEMPTS`] it is parked.
pub(super) fn note_op_apply_failure(
    sync_root: &Path,
    op: &crate::api_client::SyncOp,
    error: &EngineError,
    now: i64,
) -> OpFailure {
    op_retry(|all| {
        let state = all.entry(sync_root.to_path_buf()).or_default();
        let attempts = state.failing.get(&op.seq_id).map_or(0, |f| f.attempts) + 1;
        if attempts < SYNC_OP_MAX_ATTEMPTS {
            let retry_at = now + sync_op_backoff_secs(attempts);
            state.failing.insert(op.seq_id, FailingOp { attempts, retry_at });
            return OpFailure::Retry { attempts, retry_at };
        }
        state.failing.remove(&op.seq_id);
        let item_id = op.payload["id"].as_str().unwrap_or_default().to_string();
        let parked = ParkedSyncOp {
            seq_id: op.seq_id,
            op_type: op.op_type.clone(),
            item_id: item_id.clone(),
            attempts,
            error: error.to_string(),
            words: format!(
                "A change from your vault (a {} of item {item_id}) could not be applied on this PC after {attempts} tries. \
                 It is set aside for review, and this PC reads the full vault list again so the item still ends up right.",
                op.op_type.replace('_', " ")
            ),
            parked_at: now,
        };
        state.parked.push(parked.clone());
        let excess = state.parked.len().saturating_sub(PARKED_SYNC_OPS_KEPT);
        state.parked.drain(..excess);
        OpFailure::Parked(parked)
    })
}

/// The cursor moved to `seq_id`: no op at or before it is waiting for a retry any more.
pub(super) fn forget_op_failures_through(sync_root: &Path, seq_id: i64) {
    op_retry(|all| {
        if let Some(state) = all.get_mut(sync_root) {
            state.failing.retain(|seq, _| *seq > seq_id);
        }
    });
}

/// The ops parked on this sync root, oldest first: the review list's entries for sync.
pub fn parked_sync_ops(sync_root: &Path) -> Vec<ParkedSyncOp> {
    op_retry(|all| all.get(sync_root).map(|state| state.parked.clone()).unwrap_or_default())
}

/// How many times the op at `seq_id` has failed so far (0: not failing).
pub fn sync_op_failed_attempts(sync_root: &Path, seq_id: i64) -> u32 {
    op_retry(|all| {
        all.get(sync_root)
            .and_then(|state| state.failing.get(&seq_id))
            .map_or(0, |f| f.attempts)
    })
}

#[cfg(test)]
thread_local! {
    /// Task 2272: the seq ids whose apply fails, for the cursor tests.
    static FAIL_APPLY_SEQS: std::cell::RefCell<HashSet<i64>> = std::cell::RefCell::new(HashSet::new());
}

/// Task 2272 tests: make the apply of these seq ids fail (empty: none fails).
#[cfg(test)]
pub(super) fn fail_sync_op_apply_for_test(seqs: &[i64]) {
    FAIL_APPLY_SEQS.with(|s| *s.borrow_mut() = seqs.iter().copied().collect());
}

/// Task 2272 tests: let every waiting op on this root be retried now.
#[cfg(test)]
pub(super) fn expire_sync_op_backoff_for_test(sync_root: &Path) {
    op_retry(|all| {
        if let Some(state) = all.get_mut(sync_root) {
            for f in state.failing.values_mut() {
                f.retry_at = 0;
            }
        }
    });
}

fn apply_sync_op_once(
    bridge: &EngineBridge,
    sync_root: &Path,
    op: &crate::api_client::SyncOp,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> Result<Vec<String>, EngineError> {
    #[cfg(test)]
    if FAIL_APPLY_SEQS.with(|s| s.borrow().contains(&op.seq_id)) {
        return Err(anyhow::anyhow!("test: the apply of op {} fails", op.seq_id).into());
    }
    apply_sync_op(bridge, sync_root, op, now_secs, conflicts)
}

/// Enrich actual sparse events before entering the synchronous reconciler.
pub(super) async fn prepare_owned_sync_op(
    bridge: &EngineBridge,
    op: &crate::api_client::SyncOp,
) -> Result<Option<crate::api_client::SyncOp>, EngineError> {
    if !matches!(
        op.op_type.as_str(),
        "file_create" | "folder_create" | "file_update" | "file_rename" | "folder_rename" | "file_move" | "folder_move"
    ) {
        return Ok(Some(op.clone()));
    }
    let Some(id) = op.payload["id"].as_str() else {
        return Ok(Some(op.clone()));
    };
    // L12: the id class decides. A foreign row needs the canonical listing so
    // `prepare_owned_rows` can see its envelope; an owner row never does.
    let known_foreign = beebeeb_core::item_id::is_foreign_item_id(id).unwrap_or(false);
    let mut row = synthesize_op_row(
        bridge,
        id,
        op,
        op.payload["new_name_encrypted"]
            .as_str()
            .or_else(|| op.payload["name_encrypted"].as_str()),
    );
    if known_foreign && op.payload["name_encrypted"].is_null() && op.payload["new_name_encrypted"].is_null() {
        row = serde_json::json!({"id":id});
    }
    let mut prepared = bridge.prepare_owned_rows(&[row]).await?;
    let Some(prepared) = prepared.pop() else {
        return Ok(None);
    };
    let mut result = op.clone();
    result.payload = prepared.row;
    if matches!(op.op_type.as_str(), "file_move" | "folder_move") {
        result.payload["new_parent_id"] = result.payload["parent_id"].clone();
    }
    if matches!(op.op_type.as_str(), "file_rename" | "folder_rename") {
        result.payload["new_name_encrypted"] = result.payload["name_encrypted"].clone();
    }
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
    let started = std::time::Instant::now();
    let snapshot = bridge.api().sync_snapshot().await?;
    tracing::info!(
        rows = snapshot.nodes.len(),
        fetch_ms = started.elapsed().as_millis() as u64,
        "first listing: the server's list arrived"
    );
    // RE-a: the bar counts from here (it said "list on its way" through the whole decrypt pass), and the log says
    // how far it is every LISTING_LOG_EVERY.
    let mut progress = ListingProgress::start(bridge.transfers().clone(), snapshot.nodes.len() as u64);
    // RE3-e: a request made while this list was on its way (64-71 s for 42k rows on Guus's line: the first
    // reconcile pass's name retry, an accepted share) is served by it: its decrypt pass runs now, with the keys
    // in memory now, and a server change after the list's `seq_id` comes through the ops path. It is given back
    // when the decrypt or the apply fails. macOS keeps its own counted request (spec §6.3.2).
    #[cfg(not(target_os = "macos"))]
    let asked_meanwhile = bridge.db().take_needs_resnapshot()?;
    #[cfg(not(target_os = "macos"))]
    if asked_meanwhile {
        tracing::info!("a re-snapshot asked for while the list was on its way is served by this list");
    }
    let ingested = ingest_snapshot(bridge, sync_root, snapshot, now_secs, conflicts, &mut progress).await;
    #[cfg(not(target_os = "macos"))]
    if asked_meanwhile && ingested.is_err() {
        bridge.db().request_resnapshot()?;
    }
    ingested
}

/// RE-a: `bootstrap_from_snapshot` after its fetch: the prepare (decrypt) pass, then the ingest, reporting to
/// `progress` all along. Split out so the speed test runs it without HTTP.
pub(crate) async fn ingest_snapshot(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    mut snapshot: crate::api_client::SyncSnapshot,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
    progress: &mut ListingProgress,
) -> Result<Vec<String>, EngineError> {
    let fetched_at = now_secs;
    let prepared = bridge
        .prepare_owned_rows_reporting(&snapshot.nodes, &mut |done| progress.decrypted(done))
        .await;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            // RE-a: one row's canonical fetch failing (a 401) failed the whole list, which the next tick fetched
            // again, with nothing in the log. Its status and category only, never its text.
            tracing::warn!(
                status = crate::engine_bridge::error_http_status(&error),
                reason = error_category(error.as_ref()),
                "first listing: the decrypt pass failed; the list is fetched again at the next check"
            );
            return Err(error.into());
        }
    };
    snapshot.nodes = prepared.into_iter().map(|p| p.row).collect();
    progress.phase("ingest");
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut applied = apply_snapshot_reporting(bridge, sync_root, &snapshot, now_secs, fetched_at, conflicts, progress)?;
    // §6.3.3: an upload still waiting for its base after this successful snapshot counts
    // one more; at the 10th it parks with its bytes. The park changes how its item is
    // presented (spec §9.1), and the snapshot did not list the file, so its id joins the
    // tick's applied ids, which the tick's one working-set signal carries.
    #[cfg(target_os = "macos")]
    for (op_id, file_id) in bridge.db().note_snapshot_for_base_pending(now_secs)? {
        log_parked(&op_id, file_id.as_deref(), ParkReason::BaseUnknown);
        applied.extend(file_id);
    }
    progress.whole();
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
pub fn apply_snapshot(
    bridge: &EngineBridge,
    sync_root: &Path,
    snapshot: &crate::api_client::SyncSnapshot,
    now_secs: i64,
    snapshot_fetched_at: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> Result<Vec<String>, EngineError> {
    let mut progress = ListingProgress::start(bridge.transfers().clone(), snapshot.nodes.len() as u64);
    progress.phase("ingest");
    let applied = apply_snapshot_reporting(bridge, sync_root, snapshot, now_secs, snapshot_fetched_at, conflicts, &mut progress)?;
    progress.whole();
    Ok(applied)
}

/// [`apply_snapshot`], counting its rows on `progress` (in the `ingest` phase) and timing its prune.
fn apply_snapshot_reporting(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    snapshot: &crate::api_client::SyncSnapshot,
    now_secs: i64,
    snapshot_fetched_at: i64,
    conflicts: &mut Vec<ConflictDetected>,
    progress: &mut ListingProgress,
) -> Result<Vec<String>, EngineError> {
    // RE3-e: a re-snapshot this apply asks for itself is served by this very list.
    StateDb::during_snapshot_apply(|| {
        apply_snapshot_rows(bridge, sync_root, snapshot, now_secs, snapshot_fetched_at, conflicts, progress)
    })
}

/// The body of [`apply_snapshot_reporting`].
fn apply_snapshot_rows(
    bridge: &EngineBridge,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    snapshot: &crate::api_client::SyncSnapshot,
    now_secs: i64,
    snapshot_fetched_at: i64,
    conflicts: &mut Vec<ConflictDetected>,
    progress: &mut ListingProgress,
) -> Result<Vec<String>, EngineError> {
    // every ingested and pruned item id flows back
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
    // P2-2 (task 2445, restored from main): nodes not placed in the mirror.
    // Their children are skipped instead of being placed at the root.
    let mut unplaced: HashSet<String> = HashSet::new();

    // dc-163: the rows commit PULL_BATCH_ROWS at a time (they committed one by one: 2.1 M write calls
    // at 41,510 rows); dc-170: or every BATCH_MAX_TIME (250 ms), whichever comes first. Each batch holds whole rows only; a failure or a crash mid-pull leaves the
    // batches before it, no cursor, and the next tick pulls again over them (every write here is
    // an upsert). The count is reported between batches (DW-0802b, RE-a), so the board's wake never
    // runs under the database lock; the roots come first, so the first batch shows the Files root.
    let mut batch = bridge.db().write_batch(crate::state_db::PULL_BATCH_ROWS)?;
    for (listed, f) in order.into_iter().enumerate() {
        if listed > 0 {
            batch.row_done(|listed| progress.ingested(listed))?;
        }
        #[cfg(test)]
        first_pull_batch_tests::crash_point(listed);
        let file_id = f["id"].as_str().unwrap_or_default();
        if file_id.is_empty() {
            continue;
        }
        if !seen.insert(file_id.to_string()) {
            continue;
        }
        // P2-2: children of an unplaced folder are skipped until the folder is placed.
        if f["parent_id"].as_str().is_some_and(|pid| unplaced.contains(pid)) {
            tracing::debug!(
                file_id = %file_id,
                "sync_tick: snapshot node under a folder not placed; not placed either"
            );
            unplaced.insert(file_id.to_string());
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
            store_has_thumbnail(bridge.db(), file_id, f)?;
            if !rel_path.is_empty() {
                resolved_paths.insert(file_id.to_string(), rel_path);
                applied.push(file_id.to_string());
            } else {
                // P2-B: a blocked folder returns ("", kind). Its children must
                // not be placed at the root; treat it as unplaced.
                applied.push(file_id.to_string());
                unplaced.insert(file_id.to_string());
            }
        } else if let Some(stored) = bridge.db().get_file(file_id)?.filter(|row| !row.path.is_empty()) {
            // A stored row the store refused now keeps its place; the rows below it stay under it.
            resolved_paths.insert(file_id.to_string(), stored.path);
        } else {
            unplaced.insert(file_id.to_string());
        }
    }
    batch.finish()?;
    progress.phase("prune");

    // Deletion reconciliation: anything in the local own-tree the snapshot did
    // NOT mention was deleted/trashed server-side. `prune_absent` excludes shared
    // rows, rows with a pending upload/op, rows touched at/after the snapshot
    // fetch (a just-completed upload the snapshot predates), and refuses to prune
    // the whole tree on a suspicious EMPTY snapshot (see its doc) — so neither an
    // in-flight local create nor a just-uploaded file is ever collateral.
    let pruned = crate::drive_v2::model::drop_absent_rows(bridge.db(), &seen, snapshot_fetched_at)?;
    if !pruned.is_empty() {
        tracing::info!(count = pruned.len(), "sync_tick: pruned rows absent from snapshot");
        // task 0806: removing only the DB row left the on-disk Cloud Files
        // placeholder ghosting in Explorer. Now ALSO remove each pruned row's
        // placeholder. `prune_absent` returns rows CHILDREN-BEFORE-PARENTS (orphan
        // descendants of a pruned folder precede the folder), which is exactly the
        // order placeholder removal needs (leaf files before their directory).
        // Never the entry of a row still here whose name differs only by case: on
        // Windows that entry is reached by the pruned row's path too (`case_place`).
        // Nor, for two server items with one name, an entry whose placeholder names
        // the other row (or nothing). Only Windows removes entries here; elsewhere
        // nothing is read for it.
        #[cfg(target_os = "windows")]
        let removable = crate::drive_v2::case_place::removable_by_identity(
            bridge.db(),
            sync_root,
            crate::drive_v2::case_place::removable_places(
                bridge.db(),
                sync_root,
                &pruned,
                crate::drive_v2::case_place::CaseRule::host(),
            ),
            &crate::windows_cf::identity_at::identity_at,
        );
        #[cfg(not(target_os = "windows"))]
        let removable = pruned.clone();
        remove_pruned_placeholders(bridge.db(), sync_root, &removable);
        // A pruned row is a REMOTE deletion Finder must see.
        applied.extend(pruned.iter().map(|row| row.file_id.clone()));
    }

    bridge.db().set_sync_cursor(snapshot.seq_id)?;
    Ok(applied)
}

/// RE-a: the least time between two log lines of the first listing's progress.
pub const LISTING_LOG_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// RE-a: how far the first listing is. The board's count (DW-0802b) at most every `LISTING_REPORT_EVERY`, a log
/// line every [`LISTING_LOG_EVERY`], and each phase's time in one line at the end (decrypt, ingest, prune). The
/// decrypt pass counts for the first quarter of the bar and the ingest for the rest (41,510 rows on the test rig:
/// about 0.7 s and 2.5 s), so the bar moves from the moment the list arrives. Dropped, whole or not, it ends the
/// board's report; not whole, it says in the log where it stopped.
pub(crate) struct ListingProgress {
    board: Arc<crate::transfer_progress::TransferBoard>,
    total: u64,
    listed: u64,
    /// Where the ingest's count starts on the bar (after the decrypt pass's share).
    ingest_from: u64,
    started: std::time::Instant,
    phase: &'static str,
    phase_started: std::time::Instant,
    /// (phase, milliseconds) of every phase that ended.
    phases: Vec<(&'static str, u64)>,
    reported: std::time::Instant,
    logged: std::time::Instant,
    whole: bool,
}

impl ListingProgress {
    /// A list of `total` rows arrived: the board says "0 of `total`" at once.
    pub(crate) fn start(board: Arc<crate::transfer_progress::TransferBoard>, total: u64) -> Self {
        board.set_first_listing(0, total);
        let now = std::time::Instant::now();
        Self {
            board,
            total,
            listed: 0,
            ingest_from: 0,
            started: now,
            phase: "decrypt",
            phase_started: now,
            phases: Vec::new(),
            reported: now,
            logged: now,
            whole: false,
        }
    }

    /// `done` rows of the decrypt pass are done.
    fn decrypted(&mut self, done: usize) {
        self.at((done as u64).min(self.total) / 4);
    }

    /// `done` rows are ingested (called between write batches, with the database lock free).
    fn ingested(&mut self, done: usize) {
        let left = self.total - self.ingest_from;
        let part = (done as u64).min(self.total).saturating_mul(left).checked_div(self.total).unwrap_or(0);
        self.at(self.ingest_from + part);
    }

    fn at(&mut self, listed: u64) {
        self.listed = self.listed.max(listed);
        if self.reported.elapsed() >= crate::transfer_progress::LISTING_REPORT_EVERY {
            self.board.set_first_listing(self.listed, self.total);
            self.reported = std::time::Instant::now();
        }
        if self.logged.elapsed() >= LISTING_LOG_EVERY {
            tracing::info!(
                listed = self.listed,
                total = self.total,
                phase = self.phase,
                elapsed_ms = self.started.elapsed().as_millis() as u64,
                "first listing: progress"
            );
            self.logged = std::time::Instant::now();
        }
    }

    /// The current phase ended; `next` begins. Entering `ingest` puts the bar at the decrypt pass's share.
    fn phase(&mut self, next: &'static str) {
        self.phases.push((self.phase, self.phase_started.elapsed().as_millis() as u64));
        self.phase = next;
        self.phase_started = std::time::Instant::now();
        if next == "ingest" {
            self.ingest_from = self.total / 4;
            self.listed = self.listed.max(self.ingest_from);
            self.board.set_first_listing(self.listed, self.total);
            self.reported = std::time::Instant::now();
        }
    }

    /// The list is whole: one line with every phase's time.
    fn whole(&mut self) {
        self.phases.push((self.phase, self.phase_started.elapsed().as_millis() as u64));
        let ms = |name: &str| self.phases.iter().filter(|(phase, _)| *phase == name).map(|(_, ms)| ms).sum::<u64>();
        tracing::info!(
            rows = self.total,
            decrypt_ms = ms("decrypt"),
            ingest_ms = ms("ingest"),
            prune_ms = ms("prune"),
            total_ms = self.started.elapsed().as_millis() as u64,
            "first listing: whole"
        );
        self.whole = true;
    }

    /// The time each phase took so far, in order (the speed test).
    #[cfg(test)]
    pub(crate) fn phases(&self) -> &[(&'static str, u64)] {
        &self.phases
    }
}

impl Drop for ListingProgress {
    fn drop(&mut self) {
        if !self.whole {
            tracing::warn!(
                listed = self.listed,
                total = self.total,
                phase = self.phase,
                elapsed_ms = self.started.elapsed().as_millis() as u64,
                "first listing: stopped before it was whole"
            );
        }
        self.board.end_first_listing();
    }
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
        if let Some(id) = n["id"].as_str()
            && emitted.insert(id) {
                ordered.push(n);
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
    // the ids of items this op actually changed on
    // the local mirror — what the runner's working-set signal must carry.
    let mut applied: Vec<String> = Vec::new();
    let payload = &op.payload;
    let id = payload["id"].as_str().unwrap_or_default();
    if id.is_empty() {
        return Ok(applied);
    }

    // Trash, delete and a move change what the shared-folder index of the roots involved must list: collect
    // the roots before the op rewrites the rows, mark them dirty after it applied.
    let mut dirty_roots: Vec<String> = Vec::new();
    if matches!(
        op.op_type.as_str(),
        "file_trash" | "file_delete" | "file_move" | "folder_move"
    ) {
        dirty_roots = bridge.db().shared_roots_in_subtree(id)?;
        if let Some(new_parent) = payload["new_parent_id"].as_str() {
            dirty_roots.extend(bridge.db().shared_roots_in_subtree_root_only(new_parent)?);
        }
    }

    match op.op_type.as_str() {
        // the two server-side deletion
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
                // the root may already be parked
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
                    // The orphaned children left the mirror; the folder
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
            // when we still HOLD the row — it stayed
            // in the mirror as `Trashing` (the trash view) instead of being
            // pruned — the restore flips it back OUT of the trash view
            // directly: `CloudOnly` (placeholder re-mints; content
            // re-downloads on open), and the flip is reported so the replica
            // reparents the item out of the trash container.
            if let Some(row) = bridge.db().get_file(id)?
                && row.status == crate::state_db::FileStatus::Trashing
            {
                // the restore must take the ENTIRE
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
            // dc-168: after the row's version is written, so the flag is stored at the version it speaks of. A
            // thumbnail-flag op (`{id, has_thumbnail}`) changes no row field and is stored all the same.
            store_has_thumbnail(bridge.db(), id, payload)?;
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
        // LD-0113 (spec §5.4): the server flagged (or an admin cleared) a folder's system role. The
        // op carries `{id, system_role}` only, so the row's other fields stay as they are; a row this
        // device has not listed yet takes the role from its listing's own field.
        "folder_system_role" => {
            let role = payload["system_role"]
                .as_str()
                .and_then(beebeeb_types::backup::SystemRole::parse);
            if bridge.db().set_system_role(id, role)? {
                applied.push(id.to_string());
                tracing::info!(file_id = %id, role = role.map_or("none", |r| r.as_str()), "system role of a folder changed");
                crate::backup_protect::system_role_changed(bridge.db(), id);
            }
        }
        other => {
            tracing::debug!(op_type = other, "sync_tick: ignoring unknown op_type");
        }
    }
    if matches!(op.op_type.as_str(), "file_move" | "folder_move") {
        dirty_roots.extend(bridge.db().shared_roots_in_subtree(id)?);
    }
    for root in dirty_roots {
        bridge.db().mark_shared_index_dirty(&root)?;
    }
    // A sparse folder rename/move has no descendant events. Reuse the same
    // metadata write for their new parent paths, then signal those identities
    // with the folder. This preserves content baselines and queued rename holds.
    if matches!(
        op.op_type.as_str(),
        "folder_rename" | "folder_move" | "file_rename" | "file_move"
    ) && applied.iter().any(|changed| changed == id)
        && bridge.db().get_file(id)?.is_some_and(|row| row.is_dir())
    {
        rebase_folder_descendants(bridge, id, now_secs, &mut applied)?;
    }
    Ok(applied)
}

/// dc-168: a server row's or op's `has_thumbnail` into `files.has_thumbnail`, at the row's current version. A row or
/// op without the field (an older server, a rename, a move) leaves the stored flag as it is.
fn store_has_thumbnail(db: &StateDb, file_id: &str, f: &serde_json::Value) -> Result<(), EngineError> {
    if let Some(has) = f.get("has_thumbnail").and_then(serde_json::Value::as_bool) {
        db.set_server_has_thumbnail(file_id, has)?;
    }
    Ok(())
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
        let Some(parent) = bridge.db().get_file(&parent_id)? else {
            continue;
        };
        for child in rows.iter().filter(|row| row.parent_id.as_deref() == Some(&parent_id)) {
            let Some(contract) = bridge.db().get_file_contract_state(&child.file_id)? else {
                continue;
            };
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
                bridge.db(),
                &row,
                contract.namespace,
                contract.shared_root_id,
                contract.share_id,
                contract.permission_bits,
                now,
                bridge.api().master_key(),
                &parent.path,
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
    // updated_at: the op log carries no file timestamp the client can read. An
    // op that names the version it made (and a create) stamps "now" as the
    // row's `remote_updated_at` / `modified_at` value. An op that names no
    // version (a rename, a move, a thumbnail flag) made no version, so the row
    // keeps the server time it already holds: it never stamps this PC's clock
    // as the row's server time, and a later snapshot is still compared against
    // the server's own clock (task 2538). A row with no time yet takes "now" as
    // before. Neither is a freshness token: several ops routinely land in the
    // same second (a version upload emits `file_rename` + `file_update`
    // together), so `process_metadata_row` never gates an op-derived row on it
    // — see [`RowSource::Op`].
    let versionless = payload["version_number"].as_i64().is_none() && !op.op_type.ends_with("_create");
    let kept_server_time = if versionless {
        existing.as_ref().map(|e| e.remote_updated_at).filter(|secs| *secs > 0)
    } else {
        None
    };
    row["updated_at"] = serde_json::json!(kept_server_time.unwrap_or_else(now_secs));
    row
}

/// Trash, delete and a move between shared roots mark the shared-folder index of each root involved dirty.
#[cfg(test)]
mod shared_folder_index_dirty_tests {
    use super::*;

    const KEY: [u8; 32] = [39u8; 32];
    const ROOT_A: &str = "06600000-0000-4000-8000-0000000000a0";
    const ROOT_B: &str = "06600000-0000-4000-8000-0000000000b0";
    const FILE: &str = "06600000-0000-4000-8000-0000000000f1";

    fn bridge_in(dir: &Path) -> EngineBridge {
        let db = Arc::new(StateDb::open(dir.join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new("http://127.0.0.1:9".into(), "token".into(), KEY));
        EngineBridge::new(&crate::identity::TEST, db, api)
    }

    fn seed(bridge: &EngineBridge, id: &str, parent: Option<&str>, folder: bool, root: &str) {
        #[allow(deprecated)]
        let name = {
            let master = beebeeb_core::kdf::MasterKey::from_bytes(KEY);
            beebeeb_core::encrypt::encrypt_name(&master, id, "n", None).unwrap()
        };
        let row = serde_json::json!({
            "id": id, "parent_id": parent, "name_encrypted": name, "size_bytes": 1,
            "is_folder": folder, "content_hash": null, "version_number": 1,
            "is_trashed": false, "is_starred": false, "updated_at": 1_000,
        });
        apply_metadata_file_row(
            bridge.db(),
            &row,
            Namespace::SharedWithMe,
            Some(root.into()),
            None,
            PERMISSION_READ | PERMISSION_WRITE,
            1_000,
            &KEY,
            "",
        )
        .unwrap()
        .expect("row written");
    }

    fn setup() -> (tempfile::TempDir, std::path::PathBuf, EngineBridge) {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("Beebeeb");
        std::fs::create_dir_all(&sync_root).unwrap();
        let bridge = bridge_in(dir.path());
        seed(&bridge, ROOT_A, None, true, ROOT_A);
        seed(&bridge, ROOT_B, None, true, ROOT_B);
        seed(&bridge, FILE, Some(ROOT_A), false, ROOT_A);
        for root in [ROOT_A, ROOT_B] {
            bridge.db().clear_shared_index_dirty(root).unwrap();
        }
        (dir, sync_root, bridge)
    }

    fn run(bridge: &EngineBridge, sync_root: &Path, op_type: &str, payload: serde_json::Value) {
        let op = crate::api_client::SyncOp { seq_id: 1, op_type: op_type.into(), payload };
        apply_sync_op(bridge, sync_root, &op, now_secs(), &mut Vec::new()).unwrap();
    }

    /// dc-168: a thumbnail-flag op and a content op carrying the field store it at the row's version; an op
    /// without it leaves it.
    #[test]
    fn has_thumbnail_is_stored_from_file_update_ops() {
        let (_dir, sync_root, bridge) = setup();
        assert_eq!(bridge.db().server_has_thumbnail(FILE, 1).unwrap(), None);
        run(&bridge, &sync_root, "file_update", serde_json::json!({ "id": FILE, "has_thumbnail": true }));
        assert_eq!(bridge.db().server_has_thumbnail(FILE, 1).unwrap(), Some(true));
        run(&bridge, &sync_root, "file_update", serde_json::json!({ "id": FILE, "has_large_thumbnail": true }));
        assert_eq!(bridge.db().server_has_thumbnail(FILE, 1).unwrap(), Some(true), "no field, kept");
        run(&bridge, &sync_root, "file_update", serde_json::json!({ "id": FILE, "has_thumbnail": false }));
        assert_eq!(bridge.db().server_has_thumbnail(FILE, 1).unwrap(), Some(false));
    }

    #[test]
    fn shared_folder_index_is_dirty_after_a_trash() {
        let (_d, root, bridge) = setup();
        run(&bridge, &root, "file_trash", serde_json::json!({ "id": FILE }));
        assert_eq!(bridge.db().dirty_shared_index_roots().unwrap(), vec![ROOT_A.to_string()]);
    }

    #[test]
    fn shared_folder_index_is_dirty_after_a_delete() {
        let (_d, root, bridge) = setup();
        run(&bridge, &root, "file_delete", serde_json::json!({ "id": FILE }));
        assert_eq!(bridge.db().dirty_shared_index_roots().unwrap(), vec![ROOT_A.to_string()]);
    }

    #[test]
    fn shared_folder_index_is_dirty_for_both_roots_after_a_move() {
        let (_d, root, bridge) = setup();
        run(&bridge, &root, "file_move", serde_json::json!({ "id": FILE, "new_parent_id": ROOT_B }));
        let mut dirty = bridge.db().dirty_shared_index_roots().unwrap();
        dirty.sort();
        assert_eq!(dirty, vec![ROOT_A.to_string(), ROOT_B.to_string()]);
    }
}

/// dc-163: the first pull commits in batches, not row by row, and a crash mid-pull leaves a
/// database the next pull finishes.
#[cfg(test)]
mod first_pull_batch_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const KEY: [u8; 32] = [63u8; 32];
    /// LargeAccount::REFERENCE's size (dc-086): 41,510 items, 2,739 of them folders.
    const ITEMS: usize = 41_510;
    const FOLDERS: usize = 2_739;
    const CHILD: &str = "BB_DC163_FIRST_PULL_CHILD";

    static CRASH_AT: AtomicUsize = AtomicUsize::new(usize::MAX);

    /// The pull dies here (no unwinding, no rollback, as a killed process) at the row the test chose.
    pub(super) fn crash_point(listed: usize) {
        if listed == CRASH_AT.load(Ordering::Relaxed) {
            std::process::abort();
        }
    }

    fn id(i: usize) -> String {
        format!("{:08x}-0000-4000-8000-{:012x}", 0x0dc1_6300 + (i >> 24), i)
    }

    /// Folders under each other ten wide, files spread over the folders and the root.
    fn nodes() -> Vec<serde_json::Value> {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(KEY);
        (0..ITEMS)
            .map(|i| {
                let folder = i < FOLDERS;
                let parent = if folder {
                    (i >= 10).then(|| id(i / 10))
                } else {
                    (i % 7 != 0).then(|| id(i % FOLDERS))
                };
                let item = id(i);
                #[allow(deprecated)]
                let name = beebeeb_core::encrypt::encrypt_name(&master, &item, &format!("n{i}"), None).unwrap();
                serde_json::json!({
                    "id": item, "parent_id": parent, "name_encrypted": name, "size_bytes": 10,
                    "is_folder": folder, "content_hash": null, "version_number": 1,
                    "is_trashed": false, "is_starred": false, "updated_at": 1_000,
                })
            })
            .collect()
    }

    fn bridge_at(db: &Path) -> EngineBridge {
        let db = Arc::new(StateDb::open(db).unwrap());
        let api = Arc::new(ApiClient::new("http://127.0.0.1:9".into(), "token".into(), KEY));
        EngineBridge::new(&crate::identity::TEST, db, api)
    }

    /// `bootstrap_from_snapshot` after its fetch: the prepare pass, then the ingest.
    async fn first_pull(bridge: &EngineBridge, sync_root: &Path, nodes: &[serde_json::Value]) {
        let nodes = bridge
            .prepare_owned_rows(nodes)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.row)
            .collect();
        let snapshot = crate::api_client::SyncSnapshot { seq_id: 41, nodes };
        apply_snapshot(bridge, sync_root, &snapshot, 2_000, 2_000, &mut Vec::new()).unwrap();
    }

    fn rows(db: &Path, sql: &str) -> i64 {
        rusqlite::Connection::open(db).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[tokio::test]
    async fn first_pull_of_41k_items_commits_in_batches() {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("Beebeeb");
        std::fs::create_dir_all(&sync_root).unwrap();
        let db = dir.path().join("state.db");
        let bridge = bridge_at(&db);
        bridge.db().keep_wal_for_test();
        let before = bridge.db().wal_commits_for_test();
        let nodes = nodes();
        let started = std::time::Instant::now();
        first_pull(&bridge, &sync_root, &nodes).await;
        let commits = bridge.db().wal_commits_for_test() - before;
        println!("first pull of {ITEMS} items: {commits} commits, {:?}", started.elapsed());
        // dc-170: batches also commit every BATCH_MAX_TIME, so a slow (debug) build commits more often than
        // ITEMS / PULL_BATCH_ROWS; one commit per row (the dc-086 bug) is still 100x over this bound.
        assert!(commits <= ITEMS / 100, "{commits} commits for {ITEMS} items");
        assert_eq!(rows(&db, "SELECT COUNT(*) FROM files"), ITEMS as i64);
        assert_eq!(rows(&db, "SELECT COUNT(*) FROM owned_key_records"), ITEMS as i64);
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(41));
    }

    /// The child run of `first_pull_killed_halfway_leaves_a_db_the_next_pull_finishes`: dies at half.
    async fn child_pull(db: &Path) {
        let sync_root = db.parent().unwrap().join("Beebeeb");
        std::fs::create_dir_all(&sync_root).unwrap();
        CRASH_AT.store(ITEMS / 2, Ordering::Relaxed);
        first_pull(&bridge_at(db), &sync_root, &nodes()).await;
        unreachable!("the pull did not reach its crash point");
    }

    #[tokio::test]
    async fn first_pull_killed_halfway_leaves_a_db_the_next_pull_finishes() {
        if let Some(db) = std::env::var_os(CHILD) {
            child_pull(Path::new(&db)).await;
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "engine_bridge::syncflow::first_pull_batch_tests::first_pull_killed_halfway_leaves_a_db_the_next_pull_finishes",
                "--test-threads=1",
                "--nocapture",
            ])
            .env(CHILD, &db)
            .output()
            .unwrap();
        assert!(!out.status.success(), "the child pull was not killed");
        assert!(db.exists(), "the child never opened the database");

        // What the crash left: a sound database, whole batches of rows only, no cursor.
        let check: String = rusqlite::Connection::open(&db)
            .unwrap()
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(check, "ok");
        let landed = rows(&db, "SELECT COUNT(*) FROM files");
        assert!(landed > 0 && landed < (ITEMS / 2) as i64, "{landed} rows after a crash at half");
        // dc-170: a batch also commits by time (BATCH_MAX_TIME), so the count need not be a multiple of
        // PULL_BATCH_ROWS; whole rows only is what the folder check below proves.
        assert_eq!(
            rows(&db, "SELECT COUNT(*) FROM files f WHERE f.parent_id IS NOT NULL
                        AND NOT EXISTS(SELECT 1 FROM files p WHERE p.file_id = f.parent_id)"),
            0,
            "a row landed without its folder"
        );
        let bridge = bridge_at(&db);
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), None, "a cursor after half a pull");

        // The next pull goes over it and finishes.
        let sync_root = dir.path().join("Beebeeb");
        first_pull(&bridge, &sync_root, &nodes()).await;
        assert_eq!(rows(&db, "SELECT COUNT(*) FROM files"), ITEMS as i64);
        assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(41));
    }
}
