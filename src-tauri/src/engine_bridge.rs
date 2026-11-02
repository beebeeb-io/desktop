//! Bridge between the Tauri runner, the SQLite state DB, and the
//! Beebeeb API client. Everything that "does sync work" goes through
//! this module:
//!
//! - **State machine** ([`FileSM`]) — explicit transition table for
//!   per-file lifecycle. The plan's value here is preventing nonsense
//!   transitions like Uploading → Downloading mid-flight.
//! - **`EngineBridge`** — owns the [`StateDb`] + [`ApiClient`] handles
//!   and exposes async operations: `hydrate_file` (cloud-only → local
//!   on demand) and the periodic `sync_tick` (metadata sweep, called
//!   from [`crate::runner`]).
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 1 Task 3.
//!
//! Content frames are opened only by core's pin-aware chunk_stream API. Transport
//! normalization accepts the existing raw and JSON envelopes without decrypting them.
//! ChunkDecryptor is the public facade over core StreamOpener; finish gates whole-file
//! publication. Indexed range reads authenticate frame zero before calling open_chunk.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zeroize::{Zeroize, Zeroizing};

use base64::Engine;
use beebeeb_types::EncryptedBlob;
use serde::{Deserialize, Serialize};

use crate::api_client::{ApiClient, DesktopUploadInitRequest};
use crate::conflict::is_text_file;
use crate::state_db::{
    ClaimOutcome, ClaimedOp, FileContractState, FileEntry, FileStatus, ItemKind, KeptReason, LocalActivityEventInput,
    LocalActivityKind, Namespace, OperationKind, OperationPauseReason, PERMISSION_OWNER, PERMISSION_READ,
    PERMISSION_SHARE, PERMISSION_WRITE, ParkReason, PendingOperation, QueueDiagnostics, StateDb, TookOver,
    UploadResume,
};

pub struct EngineBridge {
    db: Arc<StateDb>,
    api: Arc<ApiClient>,
    /// Wire-byte counters shared with the heartbeat producer. Both are
    /// drained (swapped to 0) once per beat; incremented by the chunk loops.
    pub wire: Arc<WireCounters>,
    /// Bytes done and total for every transfer in flight (task 1683 slice 2). The
    /// runner hands in the board the popover snapshot reads; a one-shot bridge
    /// keeps a private one nobody reads.
    transfers: Arc<crate::transfer_progress::TransferBoard>,
    /// Cooperative stop signal (task 1538 Codex P1, PR #49 lib.rs:1087
    /// thread). `false` for the lifetime of a normal bridge. Flipped `true`
    /// by [`crate::runner::EngineRunner::abort`] BEFORE it even sends the
    /// tick-loop's cancel oneshot, so [`Self::process_due_operations`]
    /// (checked before every queued operation) and
    /// [`Self::queue_finder_create`]/[`Self::queue_finder_modify`]/
    /// [`Self::queue_finder_delete`] (checked before enqueuing) can refuse
    /// to start/queue further work immediately — instead of relying solely
    /// on `abort`'s tick-boundary cancel, which could otherwise let an
    /// entire in-progress due-operations batch, or a fresh watcher/File-
    /// Provider write landing mid-teardown, through unchecked.
    stopping: Arc<AtomicBool>,
    /// Files whose landed Finder write still waits for its new thumbnails (spec §5.6). The
    /// landing takes the op off the queue before they are uploaded, and until they are, the
    /// server holds the previous version's (one per file): [`Self::finder_thumbnail`] refuses.
    thumbnails_pending: std::sync::Mutex<HashSet<String>>,
    /// Re-snapshots that failed in a row (spec §6.3.2, macOS): from
    /// [`FAILED_SNAPSHOTS_BEFORE_THE_QUEUE_RUNS`] on, a failed one no longer ends the tick.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    failed_snapshots: std::sync::atomic::AtomicU32,
    /// Task 2244: edits newly held since the scan last logged its count ([`Self::take_newly_held`]), by
    /// reason: content unknown, then base unknown. A hold made between passes (a file's close) is counted
    /// in the next pass's line.
    newly_held: [std::sync::atomic::AtomicUsize; 2],
    /// Spec D11 (task 1888, review P2): the account's resume and pass requests, shared with the runner. A one-shot
    /// bridge keeps a private one nobody moves. Off macOS, [`Self::process_due_operations`] sends nothing while a
    /// release waits for a pass ([`Self::a_pass_is_owed`]).
    resume: crate::runner::ResumeTicket,
    #[cfg(test)]
    pub(crate) seams: Seams,
}

impl EngineBridge {
    /// Build a bridge with its own private, never-flipped stop flag. Correct
    /// for the one-shot bridges Tauri IPC commands build over the app-local
    /// state DB (restore version, set pin, resolve conflict, …) — those are
    /// each a fresh, independent unit of work, not the long-running engine
    /// loop `EngineRunner` owns, so there is nothing external that should
    /// ever ask THIS instance to stop mid-call.
    pub fn new(db: Arc<StateDb>, api: Arc<ApiClient>) -> Self {
        Self::new_with_stop_flag(db, api, Arc::new(AtomicBool::new(false)))
    }

    /// Like [`Self::new`], but shares an externally-owned stop flag —
    /// [`crate::runner::run`] passes the SAME `Arc<AtomicBool>` its
    /// `EngineRunner` flips on `abort()`, so this bridge (and every clone of
    /// it handed to the IPC socket server / Windows upload watcher) observes
    /// the stop request the instant it's set, not just at the next tick
    /// boundary (task 1538 Codex P1).
    pub fn new_with_stop_flag(db: Arc<StateDb>, api: Arc<ApiClient>, stopping: Arc<AtomicBool>) -> Self {
        Self {
            db,
            api,
            wire: WireCounters::new(),
            transfers: crate::transfer_progress::TransferBoard::new(),
            stopping,
            thumbnails_pending: std::sync::Mutex::new(HashSet::new()),
            failed_snapshots: std::sync::atomic::AtomicU32::new(0),
            newly_held: Default::default(),
            resume: crate::runner::ResumeTicket::default(),
            #[cfg(test)]
            seams: Seams::default(),
        }
    }

    pub(crate) fn owner_key_scope(&self) -> crate::owner_keys::AccountScope<'_> {
        crate::owner_keys::AccountScope::new(&self.db, &self.api, &self.stopping)
    }

    /// Mark or clear `file_id`'s landed write as waiting for its new thumbnails (spec §5.6).
    fn set_thumbnails_pending(&self, file_id: &str, pending: bool) {
        let mut set = self
            .thumbnails_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending {
            set.insert(file_id.to_string());
        } else {
            set.remove(file_id);
        }
    }

    /// Whether `file_id`'s landed write still waits for its new thumbnails (spec §5.6).
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    fn thumbnails_pending(&self, file_id: &str) -> bool {
        self.thumbnails_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(file_id)
    }

    /// A named point for the concurrency tests; nothing outside tests.
    fn seam(&self, name: &'static str) {
        #[cfg(test)]
        self.seams.fire(name);
        #[cfg(not(test))]
        let _ = name;
    }

    /// Where this bridge stages a Finder write's plaintext copy ([`default_finder_staging_root`]). A test may
    /// point one bridge at bases of its own.
    fn finder_staging_root(&self) -> Result<PathBuf, FinderStagingUnavailable> {
        #[cfg(test)]
        if let Some(bases) = self.seams.staging_bases() {
            return finder_staging_root_from(&bases);
        }
        default_finder_staging_root()
    }

    /// Report transfer progress to `board` (task 1683 slice 2).
    pub fn with_transfers(mut self, board: Arc<crate::transfer_progress::TransferBoard>) -> Self {
        self.transfers = board;
        self
    }

    /// Share the account's resume and pass requests (spec D11; review P2). [`crate::runner::run`] passes the same
    /// ticket it serves in [`Self::reconcile_then_resume`].
    pub fn with_resume(mut self, resume: crate::runner::ResumeTicket) -> Self {
        self.resume = resume;
        self
    }

    /// Review P2 (task 1888): a release asked for a pass that no committed pass has served yet. A released change may
    /// be a ghost folder's create the pass has not seen; sent now, it would make a folder the server already has a
    /// second time (anti-goal 1). The next successful tick runs the pass.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn a_pass_is_owed(&self) -> bool {
        self.resume.pass_wanted.load(Ordering::SeqCst) > self.resume.pass_served.load(Ordering::SeqCst)
    }

    pub fn transfers(&self) -> &Arc<crate::transfer_progress::TransferBoard> {
        &self.transfers
    }

    /// Remember that a transfer finished, for the popover's recent-activity arrow.
    /// Best-effort: a failed write costs a row in a list, never the transfer.
    fn record_transfer_done(&self, direction: crate::transfer_progress::Direction, file_id: &str, bytes: u64) {
        let (name, rel_path) = match self.db.get_file(file_id) {
            Ok(Some(entry)) => (crate::transfer_progress::display_name(&entry.path), Some(entry.path)),
            _ => return,
        };
        let input = crate::state_db::TransferActivityInput {
            direction: direction.as_str(),
            file_id: Some(file_id.to_string()),
            file_name: name,
            rel_path,
            bytes: i64::try_from(bytes).unwrap_or(i64::MAX),
            occurred_at: now_secs() as i64,
        };
        if let Err(e) = self.db.record_transfer_activity(input) {
            tracing::debug!(error = %e, "could not record transfer activity (best-effort)");
        }
    }

    /// `true` once a caller has asked this engine to stop (task 1538).
    /// `SeqCst` — this flag is the FIRST thing `EngineRunner::abort` sets,
    /// before it even sends the tick loop's cancel oneshot, specifically so
    /// this read is guaranteed to observe it promptly from any thread.
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Borrow the underlying state DB so callers (e.g. [`sync_tick`])
    /// can do their own writes without piping every operation through
    /// the bridge.
    pub fn db(&self) -> &StateDb {
        &self.db
    }

    /// Borrow the API client for the same reason.
    pub fn api(&self) -> &ApiClient {
        &self.api
    }

    /// Spec D11, D12 and I6 (task 1888): Windows and Linux resume the operations paused for `auth` here, in the engine,
    /// after a successful pull proved that the server accepts this session's token. It acts at the run's first
    /// successful tick (`*reconciled` still false; the first-tick rule) and at any later one where a sign-in asked
    /// (`asked > done`). Each time:
    /// - the reconcile pass (D1-D10), then the removal of the scaffolding directories it dropped (D5);
    /// - the key check (D12): every name that can be checked opens under this session's key, or every `auth` op is held
    ///   as `key_replaced`;
    /// - the resume (or that hold), then `done = asked`.
    ///
    /// A release that asked for a pass (`pass_wanted > pass_served`; review P2) gets the pass alone: no key check, no
    /// resume. Any committed pass serves the release ask it read before it planned.
    ///
    /// `*reconciled` turns true only when all of it committed, so a step that fails is tried again at the next tick,
    /// and nothing is resumed before a pass of this run (I6). Local only, counts only in the log (I7).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub async fn reconcile_then_resume(
        &self,
        sync_root: &Path,
        resume: &crate::runner::ResumeTicket,
        reconciled: &mut bool,
    ) -> anyhow::Result<()> {
        let asked = resume.asked.load(Ordering::SeqCst);
        // Review P2: a release's ask, read before the pass plans, so the pass that serves it has seen the release.
        let wanted = resume.pass_wanted.load(Ordering::SeqCst);
        let first_pass = !*reconciled;
        let resume_owed = first_pass || asked > resume.done.load(Ordering::SeqCst);
        if !resume_owed && wanted <= resume.pass_served.load(Ordering::SeqCst) {
            return Ok(());
        }
        let opts = crate::held_queue::Options {
            device_segment: crate::runner::device_name()
                .map(|name| crate::known_folder::sanitize_device_segment(&name)),
            case_insensitive: cfg!(windows),
        };
        let now = now_secs();
        let counts = self
            .db
            .reconcile_held_queue(&opts, now, &|rel| crate::state_db::tree_holds_a_file(&sync_root.join(rel)))?;
        remove_scaffold_dirs(sync_root, &counts.scaffold_dirs);
        tracing::info!(
            landed = counts.landed,
            remapped = counts.remapped,
            replay = counts.replay,
            held = counts.held_ambiguous
                + counts.held_acts_on_ghost
                + counts.held_parents_disagree
                + counts.held_twin_busy,
            released = counts.released,
            rearmed = counts.rearmed + counts.rearmed_paused,
            parent_resolved = counts.parent_resolved,
            scaffold_dropped = counts.scaffold_dropped,
            backup_unknown_kept = counts.backup_unknown_kept,
            file_twin = counts.file_twin,
            "reconcile pass committed"
        );
        // A pass that only a release asked for resumes nothing: a release is not a sign-in (spec D11).
        let (resumed, held_key_replaced) = if !resume_owed {
            (0, 0)
        } else if self.auth_ops_open_under_this_key()? {
            (self.db.resume_operations_paused_for_auth(now)?, 0)
        } else {
            (0, self.db.hold_operations_paused_for_auth_as_key_replaced(now)?)
        };
        if resume_owed {
            resume.done.fetch_max(asked, Ordering::SeqCst);
        }
        resume.pass_served.fetch_max(wanted, Ordering::SeqCst);
        *reconciled = true;
        tracing::info!(
            resumed,
            held_key_replaced,
            first_pass,
            "after the reconcile pass: the operations paused for auth were resumed, or held under a replaced key"
        );
        Ok(())
    }

    /// Spec D12 (task 1888): whether the operations paused for `auth` were queued under this session's vault key. An op
    /// is checked when its metadata carries a sealed name (an envelope, as the 2267 proof reads it) and no shared
    /// context (`shared_root_id`, `share_id`), so its name is sealed by its own `file_id`: it must open under the
    /// master key in memory ([`name_opens`]). Ops with no name, or with a shared context, are not checked and follow
    /// the verdict; with none checked, nothing contradicts this key and the verdict is that they open. The key never
    /// leaves memory, and nothing is logged.
    fn auth_ops_open_under_this_key(&self) -> anyhow::Result<bool> {
        let master_key = self.api.master_key();
        for op in self.db.operations_paused_for_auth()? {
            let Some(file_id) = op.file_id.as_deref() else {
                continue;
            };
            let Some(metadata) = op
                .metadata_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            else {
                continue;
            };
            let shared = ["shared_root_id", "share_id"]
                .iter()
                .any(|field| metadata.get(*field).is_some_and(|value| !value.is_null() && value.as_str() != Some("")));
            let sealed = metadata
                .get("name_encrypted")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|sealed| sealed.starts_with('{'));
            if let Some(sealed) = sealed
                && !shared
                && !name_opens(master_key, file_id, sealed)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn queue_diagnostics(&self, now: i64) -> anyhow::Result<QueueDiagnostics> {
        Ok(self.db.queue_diagnostics(now)?)
    }

    pub async fn process_due_operations(&self, sync_root: &Path, now: i64) -> anyhow::Result<TransferLoopOutcome> {
        let mut outcome = TransferLoopOutcome {
            completed_op_ids: Vec::new(),
            retried_op_ids: Vec::new(),
            paused_op_ids: Vec::new(),
            invalidated_item_ids: Vec::new(),
            post_complete_errors: Vec::new(),
            link_failed: false,
            session_refused: false,
            account_refused: false,
        };
        #[cfg(target_os = "windows")]
        if !self.is_stopping() {
            crate::windows_cf::upload_finalization::retry(&self.db, sync_root);
        }
        let mut due = DueOperations::new(self.db.list_due_operations(now)?);

        while let Some(op) = due.next(&self.db, now)? {
            // Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): stop draining
            // the queue the instant a caller asks this engine to stop,
            // rather than finishing every due operation first. `abort()`'s
            // graceful window is bounded (3s) before it force-terminates the
            // whole task — a caller waiting on that to purge the queue on
            // sign-out needs this loop to actually stop promptly on its own,
            // not "eventually, once the batch happens to finish".
            if self.is_stopping() {
                break;
            }
            // Review P2 (task 1888): a release that asked for a pass after this list was read stops the queue here, so
            // a released ghost create is never sent before a pass has seen it; the next successful tick runs the pass.
            #[cfg(not(target_os = "macos"))]
            if self.a_pass_is_owed() {
                break;
            }
            // The list is read once per pass, and an upload that lands moves
            // the later ops of its file along (`StateDb::apply_landing`),
            // so run each op as it is NOW: the claim re-reads it, enforces its
            // file's content order and marks this attempt, in one transaction.
            // Every write that records the attempt's outcome names the claim.
            // Every outcome below that changes how an item is presented (spec §9.1) puts the
            // item's id in `invalidated_item_ids`, once its transaction committed, so the tick's
            // one working-set signal tells the system to read it again. Without it a park
            // reached Finder only at the next unrelated signal, and the person could keep
            // editing a file whose saves would not upload.
            self.seam("claim:before_tx");
            let claimed = match self.db.claim_operation(&op.op_id, now)? {
                // Waiting for an earlier content op of its file is not an attempt.
                ClaimOutcome::Gone | ClaimOutcome::Wait => continue,
                // The hand-over committed (spec §8.4); the successor itself still waits.
                ClaimOutcome::WaitAfterHandOver(took_over) => {
                    self.after_hand_over(&op, &took_over);
                    outcome.invalidated_item_ids.extend(op.file_id.clone());
                    continue;
                }
                ClaimOutcome::Claimed(claimed) => {
                    if let Some(took_over) = &claimed.took_over {
                        self.after_hand_over(&claimed.op, took_over);
                        outcome.invalidated_item_ids.extend(claimed.op.file_id.clone());
                    }
                    *claimed
                }
                // The park committed in the claim's transaction, with any hand-over: the
                // successor is the op that parked, so its id covers both.
                ClaimOutcome::Parked {
                    op_id,
                    file_id,
                    reason,
                    took_over,
                } => {
                    if let Some(took_over) = &took_over {
                        self.after_hand_over(&op, took_over);
                    }
                    log_parked(&op_id, file_id.as_deref(), reason);
                    outcome.invalidated_item_ids.extend(file_id);
                    outcome.retried_op_ids.push(op_id);
                    continue;
                }
            };
            let op = claimed.op.clone();
            let result = self
                .execute_operation(&claimed, sync_root, now, &mut outcome.post_complete_errors)
                .await;
            self.seam("outcome:before_tx");
            // §8.6 rule 6: an upload whose completion is recorded never parks, is never paused
            // and is never abandoned: whatever failed, its retry repeats the landing (rule 4).
            // Write-keyed: only a File Provider write records a completion.
            let completed = matches!(&result, Err(error) if error.downcast_ref::<QueueStateMoved>().is_none())
                && claimed.write.is_some()
                && matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile)
                && self
                    .db
                    .get_upload_resume(&op.op_id)?
                    .and_then(|resume| resume.completed_version)
                    .is_some();
            match result {
                Ok(done) => {
                    let release = match done {
                        OpDone::Remove { release } => {
                            if !self
                                .db
                                .finish_claimed(&op.op_id, &claimed.claim_id, release.as_deref())?
                            {
                                log_queue_state_moved(&op.op_id, "landing");
                                continue;
                            }
                            release
                        }
                        // The landing transaction removed the op.
                        OpDone::Removed { release } => release,
                    };
                    if let Some(path) = release.as_deref() {
                        crate::staged_payload::remove_or_log(&self.db, Path::new(path));
                    }
                    if let Some(file_id) = &op.file_id {
                        outcome.invalidated_item_ids.push(file_id.clone());
                    }
                    if op.kind == OperationKind::CreateFolder {
                        due.folder_landed();
                    }
                    outcome.completed_op_ids.push(op.op_id);
                }
                // The attempt already logged the step that found the op gone.
                Err(error) if error.downcast_ref::<QueueStateMoved>().is_some() => continue,
                Err(error) if !completed && error.downcast_ref::<ParkNow>().is_some() => {
                    let reason = error
                        .downcast_ref::<ParkNow>()
                        .map(|park| park.0)
                        .unwrap_or(ParkReason::BaseUnknown);
                    if !self.db.park_claimed(&op.op_id, &claimed.claim_id, reason, now)? {
                        log_queue_state_moved(&op.op_id, "park");
                        continue;
                    }
                    log_parked(&op.op_id, op.file_id.as_deref(), reason);
                    outcome.invalidated_item_ids.extend(op.file_id.clone());
                    outcome.retried_op_ids.push(op.op_id);
                }
                // A stale base never becomes valid again: the server's version only grows
                // (spec §8.4). A File Provider write parks at once, and so does every other
                // upload sent with a base (a sync-folder edit, task 1640), its bytes kept; an
                // upload without a base keeps today's retries.
                Err(error)
                    if !completed
                        && init_conflict_class(&error) == Some(crate::api_client::InitConflictClass::StaleBase)
                        && (claimed.write.is_some() || op.base_version.is_some()) =>
                {
                    if !self
                        .db
                        .park_claimed(&op.op_id, &claimed.claim_id, ParkReason::StaleBase, now)?
                    {
                        log_queue_state_moved(&op.op_id, "park");
                        continue;
                    }
                    log_refused_upload(
                        &op,
                        op.attempts.saturating_add(1),
                        crate::api_client::InitConflictClass::StaleBase,
                        true,
                    );
                    outcome.invalidated_item_ids.extend(op.file_id.clone());
                    outcome.retried_op_ids.push(op.op_id);
                }
                Err(error) => {
                    let finder_write = claimed.write.is_some()
                        && matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile);
                    // §8.4: for a File Provider write, a refusal the person cures is a quota refusal.
                    let class = if finder_write && is_curable_refusal(&error) {
                        OperationFailureClass::Quota
                    } else {
                        classify_operation_error(&error.to_string())
                    };
                    outcome.link_failed |= crate::link_health::classify_error(&error).is_some();
                    outcome.session_refused |= class == OperationFailureClass::Auth;
                    outcome.account_refused |= finder_write && is_curable_refusal(&error);
                    // m-11: a recorded completion is never paused (a busy database reads as
                    // `Locked`), because a paused op is not listed again.
                    let transient = finder_write && finder_write_failure_is_transient(class, &error);
                    match failed_attempt_rule(class, finder_write, completed, transient) {
                        FailedAttemptRule::Pause(reason) => {
                            if !self.db.record_pause_claimed(
                                &op.op_id,
                                &claimed.claim_id,
                                reason,
                                Some(&error.to_string()),
                                now,
                            )? {
                                log_queue_state_moved(&op.op_id, "pause");
                                continue;
                            }
                            outcome.paused_op_ids.push(op.op_id);
                        }
                        // The park settled the row in its transaction (spec §9.2).
                        FailedAttemptRule::Park(reason) => {
                            if !self.db.park_claimed(&op.op_id, &claimed.claim_id, reason, now)? {
                                log_queue_state_moved(&op.op_id, "park");
                                continue;
                            }
                            log_parked(&op.op_id, op.file_id.as_deref(), reason);
                            outcome.invalidated_item_ids.extend(op.file_id.clone());
                            outcome.retried_op_ids.push(op.op_id);
                        }
                        FailedAttemptRule::Retry(count) => {
                            // §8.6 rule 6 and §8.4: a recorded completion, and a File Provider
                            // write whose failure waiting cures, keep retrying on the normal
                            // backoff and never use up their attempts.
                            let uses_up_attempts = count == AttemptCount::UsesUp;
                            let attempts = next_attempt_count(op.attempts, op.max_attempts, count);
                            if completed {
                                log_completed_landing_retried(&op.op_id, op.file_id.as_deref(), attempts);
                            }
                            if matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile)
                                && error_http_status(&error) == Some(409)
                            {
                                let class =
                                    init_conflict_class(&error).unwrap_or(crate::api_client::InitConflictClass::Other);
                                log_refused_upload(&op, attempts, class, false);
                            }
                            let next_retry_at = now.saturating_add(retry_delay_seconds(attempts));
                            if !self.db.record_attempt_claimed(
                                &op.op_id,
                                &claimed.claim_id,
                                attempts,
                                next_retry_at,
                                Some(&error.to_string()),
                            )? {
                                log_queue_state_moved(&op.op_id, "attempt");
                                continue;
                            }
                            if uses_up_attempts && attempts >= op.max_attempts {
                                // A File Provider write parked by the queue's rule: its line, once,
                                // after the record committed. A 409 says so on its own line above.
                                if finder_write && error_http_status(&error) != Some(409) {
                                    log_parked_attempts_used_up(&op.op_id, op.file_id.as_deref());
                                }
                                self.abandon_upload_after_give_up(&op).await;
                                // Its attempts are used up, so it has parked by the queue's rule,
                                // and the attempt's transaction settled the row (spec §9.2).
                                outcome.invalidated_item_ids.extend(op.file_id.clone());
                            }
                            outcome.retried_op_ids.push(op.op_id);
                        }
                    }
                }
            }
        }

        Ok(outcome)
    }

    /// The hand-over committed (spec §8.4): one line, then the retired copy is unlinked,
    /// after the commit (S6).
    fn after_hand_over(&self, successor: &PendingOperation, took_over: &TookOver) {
        log_took_over(&successor.op_id, successor.file_id.as_deref(), &took_over.parked_op_id);
        if let Some(path) = took_over.released_payload.as_deref() {
            crate::staged_payload::remove_or_log(&self.db, Path::new(path));
        }
    }

    /// Run one claimed op, and say what the runner still does after it (`OpDone`).
    async fn execute_operation(
        &self,
        claimed: &ClaimedOp,
        sync_root: &Path,
        now: i64,
        post_complete_errors: &mut Vec<String>,
    ) -> anyhow::Result<OpDone> {
        let op = &claimed.op;
        match op.kind {
            OperationKind::PinTree => Ok(OpDone::Remove { release: None }),
            OperationKind::HydrateFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("hydrate operation missing file_id"))?;
                let dest = if let Some(target_path) = op.target_path.as_deref() {
                    local_file_path_under_sync_root(sync_root, target_path)?
                } else if let Some(entry) = self.db.get_file(file_id)? {
                    local_file_path_under_sync_root(sync_root, &entry.path)?
                } else {
                    return Err(anyhow::anyhow!("hydrate operation target missing from state"));
                };
                self.hydrate_file(file_id, &dest, &[sync_root]).await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::CreateFolder => {
                let metadata = operation_metadata(op)?;
                let name = metadata["name_encrypted"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("create folder operation missing encrypted name"))?;
                self.api
                    .create_folder(name, op.parent_id.as_deref(), op.file_id.as_deref())
                    .await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::MoveFile | OperationKind::RenameFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("metadata operation missing file_id"))?;
                let metadata = operation_metadata(op)?;
                let name = metadata["name_encrypted"].as_str();
                // Spec §7.2 (final review): a rename leaves the parent alone; a move names its
                // target, the top level (no parent) included.
                let parent = match op.kind {
                    OperationKind::MoveFile => crate::api_client::ParentChange::to(op.parent_id.as_deref()),
                    _ => crate::api_client::ParentChange::Unchanged,
                };
                self.api.update_metadata(file_id, name, parent).await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::TrashFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("trash operation missing file_id"))?;
                let activity_entry = self.db.get_file(file_id)?;
                // Server-authoritative deletion (task 0802). `trash_file` calls
                // `.error_for_status()`, so an `Ok` here means the server returned
                // HTTP 2xx for `DELETE /files/{id}` (it set `is_trashed=TRUE` and
                // emitted a `file_trash` sync op).
                //
                // We deliberately DO NOT delete the local row here. The row was
                // parked in `Trashing` by `watcher::handle_delete`; we KEEP it in
                // `Trashing` as the durable hidden-locally marker. Returning `Ok`
                // makes `process_due_operations` REMOVE this op — so the
                // `Trashing` status (not the op) is now what protects the row.
                //
                // Why not delete now: deleting + a same-tick / replica-lagged
                // `/sync/snapshot` that still lists the (just-trashed) file would
                // re-insert a FRESH `CloudOnly` row and re-mint the placeholder —
                // the "deleted file comes back" race. Instead we let convergence
                // happen authoritatively: while the snapshot still lists the file
                // the `Trashing` guard in `process_metadata_row` preserves it; once
                // the trash propagates the file is ABSENT from the snapshot and
                // `prune_absent` removes the (now op-less) `Trashing` row. If the
                // `file_trash` op echo arrives first, `apply_sync_op` deletes the
                // row directly — either path converges.
                self.api.trash_file(file_id).await?;
                if let Some(entry) = activity_entry {
                    record_moved_to_trash_activity(self.db.as_ref(), file_id, &entry.path, now)?;
                }
                // Trace logs keep only the opaque id; plaintext names stay in the
                // local state DB, where file paths already live.
                tracing::info!(
                    file_id = %file_id,
                    "trash op: server DELETE /files/{{id}} returned 2xx — keeping row Trashing, dropping op"
                );
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::RestoreFile => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("restore file operation missing file_id"))?;
                self.api.restore_file(file_id).await?;
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::RestoreVersion => {
                let file_id = op
                    .file_id
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("restore operation missing file_id"))?;
                let metadata = operation_metadata(op)?;
                let version_id = metadata["version_id"]
                    .as_str()
                    .or(op.base_object_version_id.as_deref())
                    .ok_or_else(|| anyhow::anyhow!("restore operation missing version id"))?;
                // Spec §5.4 row 10: the restore's version becomes current, so the
                // system re-downloads the restored bytes instead of keeping a token.
                #[cfg(target_os = "macos")]
                {
                    let response = self.api.restore_version(file_id, version_id).await?;
                    self.seam("restore:after_server");
                    // The server has restored, and each restore mints a version: a
                    // retry would make another. A local failure is left to a snapshot
                    // to repair, and the op completes. A fixed category only: the
                    // error can carry a path (spec §11).
                    if self
                        .db
                        .apply_restore_response(
                            file_id,
                            response["version_number"].as_i64(),
                            response["current_object_version_id"].as_str(),
                        )
                        .is_err()
                    {
                        tracing::warn!(
                            file_id = %file_id,
                            category = "restore_bookkeeping_failed",
                            "restore done on the server; its local record failed, a snapshot will repair it"
                        );
                        if self.db.request_resnapshot().is_err() {
                            tracing::warn!(
                                file_id = %file_id,
                                category = "resnapshot_request_failed",
                                "restore done on the server; a snapshot could not be requested"
                            );
                        }
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.api.restore_version(file_id, version_id).await?;
                }
                Ok(OpDone::Remove { release: None })
            }
            OperationKind::UploadVersion | OperationKind::UploadFile => {
                self.upload_version(op, Some(claimed), sync_root, post_complete_errors)
                    .await
            }
        }
    }

    /// Upload one op's staged bytes. `claim` is the runner's claim of the op; every
    /// queue write the upload makes is guarded by it (spec §8.7 S4). `None` is Keep
    /// Mine's inline run, outside the queue: its writes stay unguarded. The `release` of
    /// the returned `OpDone` is, on macOS, the staged payload, which the caller unlinks
    /// once the op is gone.
    async fn upload_version(
        &self,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        post_complete_errors: &mut Vec<String>,
    ) -> anyhow::Result<OpDone> {
        let local_file_id = op
            .file_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing file_id"))?;
        let previous_status = self.db.get_file(local_file_id)?.map(|entry| entry.status);
        // Write-keyed: a File Provider write (spec §9.1–§9.2). Keep Mine (`claim: None`) and
        // Windows and watcher uploads keep today's status handling.
        let finder_write = claim.and_then(|c| c.write.as_ref()).is_some();
        // §9.1 (m-8): an item in the trash never looks writable. A Finder write's attempt
        // leaves a `Trashing` row in the trash; its `init` meets the server's refusal and retries.
        if finder_write {
            self.db.set_uploading_unless_trashing(local_file_id)?;
        } else {
            self.db.set_status(local_file_id, FileStatus::Uploading)?;
        }

        struct Rollback<'a> {
            db: &'a StateDb,
            id: &'a str,
            previous: Option<FileStatus>,
            finder_write: bool,
        }
        impl Drop for Rollback<'_> {
            fn drop(&mut self) {
                // §9.2: a Finder write's row stays `Uploading` while its op will retry; a park
                // sets `Error` in the park's own transaction (`settle_status_after_park_conn`).
                if self.finder_write {
                    return;
                }
                // A server-completed upload has already committed Local; don't
                // undo that if cancellation happens during thumbnail work.
                if self
                    .db
                    .get_file(self.id)
                    .ok()
                    .flatten()
                    .is_some_and(|r| r.status == FileStatus::Uploading)
                {
                    let status = if self.previous == Some(FileStatus::Conflict) {
                        FileStatus::Conflict
                    } else {
                        FileStatus::Error
                    };
                    let _ = self.db.set_status(self.id, status);
                }
            }
        }
        let _rollback = Rollback {
            db: &self.db,
            id: local_file_id,
            previous: previous_status,
            finder_write,
        };
        self.do_upload_version(local_file_id, op, claim, sync_root, post_complete_errors)
            .await
    }

    async fn do_upload_version(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        post_complete_errors: &mut Vec<String>,
    ) -> anyhow::Result<OpDone> {
        // §8.6 rules 4 and 6: a recorded completion lands without the network, before anything
        // else is checked, because it never parks, not even on a missing payload. Write-keyed:
        // a completion is recorded only for a File Provider write, so only one can be found
        // here; Windows and watcher uploads never take this shortcut.
        if claim.is_some_and(|claim| claim.write.is_some())
            && let Some(previous) = self.db.get_upload_resume(&op.op_id)?
            && let (Some(version), Some(object)) =
                (previous.completed_version, previous.completed_object_version_id.clone())
        {
            // The save's own content type, as the first attempt had it, and the server's
            // `mime_type` recorded with the completion, its fallback.
            let content_type = op
                .metadata_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|metadata| metadata["content_type"].as_str().map(str::to_string));
            return self
                .land(
                    local_file_id,
                    op,
                    claim,
                    &previous.server_file_id,
                    version,
                    &object,
                    previous.payload_size as u64,
                    content_type,
                    previous.completed_mime_type.clone(),
                    post_complete_errors,
                    sync_root,
                )
                .await;
        }
        let payload_path = op
            .payload_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing staged payload"))?;
        let payload_path = Path::new(payload_path);
        match std::fs::metadata(payload_path) {
            Ok(meta) if meta.is_file() => {}
            // The staged copy lives in the app's own container and cannot come back
            // (spec §8.4). Only a File Provider write parks on it.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && claim.and_then(|c| c.write.as_ref()).is_some() => {
                return Err(anyhow::Error::new(ParkNow(ParkReason::PayloadMissing)));
            }
            // An op without a write id keeps today's retry and today's text
            // (`test_process_due_operations_records_retry_for_upload_worker_handoff`), without
            // the path: `classify_operation_error` matches bare substrings such as `403`, so a
            // path would pause the op instead.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!("staged upload payload is missing"));
            }
            // Every other read error (permission, I/O, not a regular file) retries as
            // today (spec §8.4), with a fixed text. Neither the path nor the I/O error
            // reaches `classify_operation_error`: it matches bare substrings such as
            // `403` and `permission`, and would pause the op instead.
            _ => return Err(anyhow::anyhow!("staged upload payload could not be read")),
        }

        let metadata = operation_metadata(op)?;
        let name_encrypted = metadata["name_encrypted"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("upload operation missing encrypted name"))?;
        let content_type = metadata["content_type"].as_str().map(str::to_string);
        // An empty (0-byte) file is a normal upload: the canonical plan for it is
        // ONE chunk carrying the AEAD of zero bytes (`plan_chunks(0, _)` →
        // `chunk_count == 1`), which the chunk loop below sends as a 28-byte
        // nonce + tag. Requires a server whose `/uploads/init` accepts
        // `file_size_bytes: 0`.
        let plaintext_size = std::fs::metadata(payload_path)?.len();

        let is_create = is_create_file_operation(&metadata);
        let payload_path_str = payload_path.to_string_lossy().into_owned();
        let payload_mtime_ns = payload_mtime_ns(payload_path);

        // Flow 7: resume the persisted upload session when the retry is for the
        // SAME staged bytes. Re-running `init` would mint a second server file
        // row (the first left behind as a broken `is_uploading` duplicate) and
        // re-send every chunk from zero.
        let mut session: Option<UploadResume> = None;
        if let Some(previous) = self.db.get_upload_resume(&op.op_id)? {
            if previous.is_create && !previous.self_minted {
                // SF-0001 (L13): an older version let the server mint a create's id.
                // Such a session can never be ours again, whatever its payload: it
                // is abandoned and this attempt starts a fresh, self-minted upload.
                tracing::info!(
                    op_id = %op.op_id,
                    file_id = %previous.server_file_id,
                    "upload: persisted create session was not self-minted — abandoning it and starting a fresh upload"
                );
                self.abandon_upload_session(&previous).await?;
            } else {
                let same_payload = previous.payload_path == payload_path_str
                    && previous.payload_size == plaintext_size as i64
                    && previous.payload_mtime_ns == payload_mtime_ns
                    && previous.chunk_size_bytes > 0
                    && previous.chunk_count > 0
                    && previous.acked_chunks >= 0
                    && previous.acked_chunks <= previous.chunk_count;
                let v2 = owned_key_and_pin(&self.db, self.api.master_key(), &previous.server_file_id)?.1
                    == beebeeb_core::chunk_stream::ExpectedChunkFormat::V2;
                if same_payload && v2 {
                    // A fresh core writer has a fresh stream identity. Never append it to an old v2 stream.
                    // Fresh init can refuse a live lease; preserve the old resume record until it succeeds.
                } else if same_payload {
                    tracing::info!(
                        op_id = %op.op_id,
                        file_id = %previous.server_file_id,
                        acked_chunks = previous.acked_chunks,
                        chunk_count = previous.chunk_count,
                        "upload: resuming persisted upload session"
                    );
                    session = Some(previous);
                } else {
                    tracing::info!(
                        op_id = %op.op_id,
                        file_id = %previous.server_file_id,
                        "upload: staged payload changed since the persisted session — abandoning it"
                    );
                    self.abandon_upload_session(&previous).await?;
                }
            }
        }

        // Only a session resumed from its persisted row can have been reaped by the server (each
        // chunk renews a session's lease, so one opened in this attempt cannot be).
        let resumed = session.is_some();
        let session = match session {
            Some(session) => session,
            None => {
                guard_init_base(
                    op.base_version,
                    is_create,
                    claim.and_then(|claim| claim.write.as_ref()).is_some(),
                )?;
                // SF-0001 (L13): this device chooses the item id. A create proposes
                // its own id when that id is canonical; otherwise (a local-only id)
                // it mints a fresh one for this init. A replace keeps the row's id.
                let proposed_id = if is_create && !is_canonical_item_id(local_file_id) {
                    uuid::Uuid::new_v4().to_string()
                } else {
                    local_file_id.to_string()
                };
                let init_request = upload_init_request_for_operation(
                    &proposed_id,
                    name_encrypted,
                    content_type.clone(),
                    op.parent_id.clone(),
                    plaintext_size,
                    op.base_version,
                );
                let upload = self.api.init_upload(&init_request).await?;
                if upload.chunk_size_bytes <= 0 || upload.chunk_count <= 0 {
                    return Err(anyhow::anyhow!("upload init returned invalid chunk plan"));
                }
                // Before anything is persisted or sealed: an init that answers with
                // another id is not this device's item. Exact equality — a re-spelled
                // echo (upper case, trimming) is a different id.
                if upload.file_id != proposed_id {
                    return Err(anyhow::anyhow!(
                        "server_changed_item_id: the upload init answered with an id this device did not choose"
                    ));
                }
                let session = UploadResume {
                    op_id: op.op_id.clone(),
                    payload_path: payload_path_str.clone(),
                    payload_size: plaintext_size as i64,
                    payload_mtime_ns,
                    upload_session_id: upload.upload_session_id,
                    server_file_id: upload.file_id,
                    object_version_id: upload.object_version_id,
                    chunk_size_bytes: upload.chunk_size_bytes,
                    chunk_count: upload.chunk_count,
                    acked_chunks: 0,
                    metadata_applied: false,
                    // The op kind, not whether init carried an id: every init now does.
                    is_create,
                    // Every session this code opens sends an id it chose itself.
                    self_minted: true,
                    completed_version: None,
                    completed_object_version_id: None,
                    completed_mime_type: None,
                };
                // Persist BEFORE the next await: a cut anywhere after init must
                // leave the session discoverable by the retry.
                match claim {
                    Some(claim) => {
                        if !self.db.put_upload_resume_claimed(&session, &claim.claim_id)? {
                            log_queue_state_moved(&op.op_id, "resume");
                            return Err(anyhow::Error::new(QueueStateMoved));
                        }
                    }
                    // Keep Mine runs inline, outside the queue: no claim guards it.
                    None => self.db.put_upload_resume(&session)?,
                }
                session
            }
        };
        // SF-0001 (L13): whatever the session's origin (resumed or fresh), it must
        // target the item this op writes before any byte or name goes to the server.
        ensure_session_item_id(&session, local_file_id)?;

        let body = self
            .upload_session_body(
                local_file_id,
                op,
                &metadata,
                name_encrypted,
                content_type.clone(),
                payload_path,
                &session,
            )
            .await;
        match body {
            Ok(completed) => {
                // §8.6.1: the version this upload produced, from the server's reply. An
                // idempotent repeat answers `already_completed` with neither field; its version
                // is then base + 1 (1 for a create), its object id the session's. An upload
                // without a base (Windows and the watcher) keeps the stored version + 1.
                let produced_version = match completed["version_number"].as_i64() {
                    Some(version) => version,
                    None if is_create => 1,
                    None => match op.base_version {
                        Some(base) => base.saturating_add(1),
                        None => self
                            .db
                            .get_file_contract_state(local_file_id)?
                            .map_or(0, |contract| contract.current_version)
                            .saturating_add(1),
                    },
                };
                let produced_object = completed["current_object_version_id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| session.object_version_id.clone());
                let mime_type = completed["mime_type"].as_str().map(str::to_string);
                if let Some(claim) = claim
                    && claim.write.is_some() // write-keyed
                    && !self.db.record_completion_claimed(
                        &op.op_id,
                        &claim.claim_id,
                        produced_version,
                        &produced_object,
                        mime_type.as_deref(),
                    )?
                {
                    log_queue_state_moved(&op.op_id, "completion");
                    return Err(anyhow::Error::new(QueueStateMoved));
                }
                let size = completed["size_bytes"].as_u64().unwrap_or(plaintext_size);
                ensure_session_item_id(&session, local_file_id)?;
                self.land(
                    local_file_id,
                    op,
                    claim,
                    &session.server_file_id,
                    produced_version,
                    &produced_object,
                    size,
                    content_type,
                    mime_type,
                    post_complete_errors,
                    sync_root,
                )
                .await
            }
            Err(error) => {
                if upload_session_is_gone(&error) {
                    let status = error_http_status(&error);
                    // The status and a category only: the error's text carries the request URL
                    // with the session id (spec §11).
                    tracing::warn!(
                        op_id = %op.op_id,
                        file_id = %session.server_file_id,
                        status,
                        resumed,
                        reason = error_category(error.as_ref()),
                        "upload: server no longer accepts the upload session — abandoning it; the next attempt starts a fresh upload"
                    );
                    self.abandon_upload_session(&session).await?;
                    // §8.4: a resumed session the server reaped (404, 410) is not a counted
                    // failure: the next attempt starts a fresh upload. A 400, 404 or 410 at a
                    // session opened in this attempt, and a 400 at a resumed one, is counted, so a
                    // server that refuses every new session still parks the write.
                    if resumed && matches!(status, Some(404 | 410)) {
                        return Err(anyhow::Error::new(SessionAbandoned(error)));
                    }
                }
                Err(error)
            }
        }
    }

    /// The landing (spec §8.6.2): one transaction (`StateDb::apply_landing`), then the work
    /// that follows its commit. Keep Mine passes `claim: None`: the landing removes nothing
    /// it does not own, and the caller unlinks the released copy.
    #[allow(clippy::too_many_arguments)]
    async fn land(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        claim: Option<&ClaimedOp>,
        server_file_id: &str,
        produced_version: i64,
        produced_object: &str,
        size: u64,
        content_type: Option<String>,
        mime_type: Option<String>,
        post_complete_errors: &mut Vec<String>,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> anyhow::Result<OpDone> {
        self.seam("landing:after_complete");
        let payload_path = op.payload_path.clone();
        // macOS only: `release` says only what the caller unlinks after the commit (S6).
        // Elsewhere `finish_completed_upload` unlinks the copy, as before.
        let release = if cfg!(target_os = "macos") {
            payload_path.clone()
        } else {
            None
        };
        // Windows: a create's finalization journal row, read from local state only, and
        // written in the landing's own transaction (the journal then owns the retry).
        #[cfg(target_os = "windows")]
        let finalization = match payload_path.as_deref() {
            Some(path) => self.defer_local_upload_finalization(op, server_file_id, sync_root, Path::new(path))?,
            None => None,
        };
        #[cfg(not(target_os = "windows"))]
        let finalization: Option<crate::state_db::UploadFinalization> = None;
        // §5.6: a Finder write's thumbnails are uploaded after the landing commits, and the
        // commit takes the op off the queue. From just before it to the end of that upload the
        // server holds only the previous version's thumbnail, which the system would cache under
        // the token: `finder_thumbnail` refuses meanwhile. Keyed on a minted write (macOS).
        let marks_thumbnails = claim.and_then(|c| c.write.as_ref()).is_some() && payload_path.is_some();
        if marks_thumbnails {
            self.set_thumbnails_pending(server_file_id, true);
        }
        // Task 1640: a sync-folder upload (no write id) records what it sent as the content this
        // device now agrees with the server on. A Finder write's copy is not read again.
        let completed_payload_hash = match (claim.and_then(|c| c.write.as_ref()), payload_path.as_deref()) {
            (None, Some(path)) => match sha256_file_hex(Path::new(path)) {
                Ok(hash) => Some(hash),
                Err(e) => {
                    tracing::warn!(
                        op_id = %op.op_id,
                        error = %e,
                        "landing: could not hash the copy that was sent; the file keeps its previous agreed content"
                    );
                    None
                }
            },
            _ => None,
        };
        let master_key = self.api.master_key();
        let landing_material = owned_key_and_pin(&self.db, master_key, server_file_id)?;
        let landed = self.db.apply_landing(
            &crate::state_db::LandingInput {
                op_id: &op.op_id,
                claim_id: claim.map(|c| c.claim_id.as_str()),
                write_id: claim.and_then(|c| c.write.as_ref()).map(|w| w.write_id.as_str()),
                local_file_id,
                server_file_id,
                target_path: op.target_path.as_deref(),
                parent_id: op.parent_id.as_deref(),
                landed_base: op.base_version,
                produced_version,
                produced_object_version_id: produced_object,
                size_bytes: i64::try_from(size).unwrap_or(i64::MAX),
                content_type: content_type.as_deref(),
                mime_type: mime_type.as_deref(),
                // Always the op's payload path, whatever `release` is.
                completed_payload: payload_path.as_deref(),
                completed_payload_hash: completed_payload_hash.as_deref(),
                finalization: finalization.as_ref(),
                now: now_secs(),
            },
            &|queued, server_id| metadata_rekeyed_with_material(master_key, queued, server_id,
                (server_id == server_file_id).then_some((&landing_material.0, landing_material.1))),
        );
        // Nothing landed: the op is still queued, and the refusal is the queue's again.
        let landed = match landed {
            Ok(Some(landed)) => landed,
            Ok(None) => {
                if marks_thumbnails {
                    self.set_thumbnails_pending(server_file_id, false);
                }
                log_queue_state_moved(&op.op_id, "landing");
                return Err(anyhow::Error::new(QueueStateMoved));
            }
            Err(e) => {
                if marks_thumbnails {
                    self.set_thumbnails_pending(server_file_id, false);
                }
                return Err(e.into());
            }
        };
        for (successor, reason) in &landed.parked_successors {
            log_parked(successor, Some(server_file_id), *reason);
        }
        self.record_transfer_done(crate::transfer_progress::Direction::Up, server_file_id, size);
        if let Some(path) = payload_path.as_deref() {
            // Task 1700: post-complete thumbnail work never fails the upload, but its
            // failure is surfaced on the outcome so a red run names the real error.
            let file_key = owned_key_and_pin(&self.db, self.api.master_key(), server_file_id)?.0;
            match self
                .finish_completed_upload(op, server_file_id, Path::new(path), content_type, &file_key, sync_root)
                .await
            {
                Ok(()) => {
                    if marks_thumbnails {
                        self.set_thumbnails_pending(server_file_id, false);
                    }
                }
                // The refusal stays: the server still holds the previous version's thumbnail.
                Err(e) => {
                    // A category only: the error's text can carry the request URL (spec §11).
                    tracing::warn!(
                        file_id = %server_file_id,
                        reason = error_category(e.as_ref()),
                        "upload-time thumbnail generation/upload skipped"
                    );
                    post_complete_errors.push(format!("{}: {e}", op.op_id));
                }
            }
        }
        Ok(if claim.is_some() {
            OpDone::Removed { release }
        } else {
            OpDone::Remove { release }
        })
    }

    /// Everything between init and a successful `complete`: the post-init
    /// metadata PATCH and the chunk PUTs from the acknowledged watermark on.
    /// Every step's success is persisted before the next await so a cut
    /// resumes exactly where the server's acknowledgements stop.
    #[allow(clippy::too_many_arguments)]
    async fn upload_session_body(
        &self,
        local_file_id: &str,
        op: &PendingOperation,
        metadata: &serde_json::Value,
        name_encrypted: &str,
        content_type: Option<String>,
        payload_path: &Path,
        session: &UploadResume,
    ) -> anyhow::Result<serde_json::Value> {
        // SF-0001 (L13): never seal a name or send a chunk under an id this device
        // did not choose, even when the session was read back from disk.
        ensure_session_item_id(session, local_file_id)?;
        let server_file_id = session.server_file_id.clone();
        if !session.metadata_applied {
            let effective_name_encrypted = if server_file_id != local_file_id {
                owned_metadata_for_name(
                    &self.db, self.api.master_key(),
                    &server_file_id,
                    metadata_display_name(metadata, op)
                        .as_deref()
                        .unwrap_or(&server_file_id),
                    content_type.as_deref(),
                )?
            } else {
                name_encrypted.to_string()
            };
            self.api
                .update_metadata(
                    &server_file_id,
                    Some(&effective_name_encrypted),
                    upload_parent_change(op, metadata),
                )
                .await?;
            self.db.set_upload_resume_metadata_applied(&op.op_id)?;
        }

        let (file_key, pin) = owned_key_and_pin(&self.db, self.api.master_key(), &server_file_id)?;

        let file = std::fs::File::open(payload_path)?;
        let chunk_size = session.chunk_size_bytes as usize;
        let chunk_count = session.chunk_count as u64;
        if chunk_size == 0 || chunk_count == 0 {
            return Err(anyhow::anyhow!("upload init returned invalid chunk plan"));
        }
        let first_chunk = session.acked_chunks.max(0) as u64;
        anyhow::ensure!(first_chunk == 0 || pin != beebeeb_core::chunk_stream::ExpectedChunkFormat::V2,
            "a v2 upload must restart in a fresh session");
        let mut writer = beebeeb_core::chunk_stream::ChunkEncryptor::from_reader_with_file_key(
            file_key, &server_file_id, session.payload_size as u64, chunk_size as u64, file)?
            .with_recorded_pin(pin)?;
        // Task 1683 slice 2: per-file bytes for the popover. Keyed by the LOCAL file
        // id (the `files` row the activity list reads); a resumed session starts at
        // its acknowledged watermark, not at 0.
        let payload_total = u64::try_from(session.payload_size).unwrap_or(0);
        let progress = self
            .transfers
            .begin(local_file_id, crate::transfer_progress::Direction::Up, payload_total);
        progress.update(crate::transfer_progress::uploaded_bytes(
            first_chunk,
            chunk_size as u64,
            payload_total,
        ));
        // Rate-limit ceiling: read once per file, not per chunk (config is on
        // disk but the file is fast to parse; at most one read per upload op).
        let upload_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.upload_kbps_limit)
            .unwrap_or(0);

        for chunk_index in 0..chunk_count {
            let encrypted = writer.next_chunk()?.ok_or_else(|| anyhow::anyhow!("staged upload ended early"))?;
            if chunk_index < first_chunk { continue; }
            let read = encrypted.data.len().saturating_sub(28);
            let chunk_start = std::time::Instant::now();
            self.api
                .upload_session_chunk(&session.upload_session_id, chunk_index as u32, &encrypted.data)
                .await?;
            self.db.set_upload_resume_acked(&op.op_id, chunk_index as i64 + 1)?;
            progress.update(crate::transfer_progress::uploaded_bytes(
                chunk_index + 1,
                chunk_size as u64,
                payload_total,
            ));

            // P1 — wire-byte counter: count plaintext bytes (the user-data rate).
            self.wire.upload_bytes.fetch_add(read as u64, Ordering::Relaxed);

            // E — token-bucket pacing: if a limit is set and the chunk transferred
            // faster than the budget allows, sleep the remainder.
            if upload_kbps_limit > 0 {
                let budget_secs = read as f64 / (upload_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }

        writer.finish()?;
        let completed = self.api.complete_upload_session(&session.upload_session_id).await;
        if completed.is_ok() {
            progress.finish();
        }
        completed
    }

    /// Post-`complete` best-effort work: thumbnails, staged-payload cleanup and
    /// (Windows) placeholder conversion. Never fails the upload — but the
    /// thumbnail outcome is RETURNED (task 1700) instead of being swallowed
    /// into a `tracing::warn!` that no test subscriber captures, so callers
    /// can surface it for diagnostics. Cleanup and Windows finalization stay
    /// warn-only inside.
    async fn finish_completed_upload(
        &self,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] op: &PendingOperation,
        server_file_id: &str,
        payload_path: &Path,
        thumbnail_content_type: Option<String>,
        file_key: &beebeeb_core::kdf::FileKey,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
    ) -> anyhow::Result<()> {
        let server_file_id = server_file_id.to_string();
        let thumbnails = self
            .upload_thumbnails_for_plaintext_media(
                &server_file_id,
                payload_path,
                thumbnail_content_type.as_deref(),
                file_key,
            )
            .await;

        #[cfg(target_os = "windows")]
        {
            crate::windows_cf::upload_finalization::retry(&self.db, sync_root);
            // A failed stamp or unlink retains its durable completed proof.
            match self.db.upload_finalizations() {
                Ok(rows) if rows.iter().any(|row| row.op_id == op.op_id) => return thumbnails,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot inspect finalization journal");
                    return thumbnails;
                }
                _ => {}
            }
        }
        // On macOS the caller releases the payload once the op's removal has
        // committed (spec §8.7 S6).
        #[cfg(not(target_os = "macos"))]
        crate::staged_payload::remove_or_log(&self.db, payload_path);
        thumbnails
    }

    /// Give up on a persisted upload session (payload changed, session gone,
    /// not self-minted, or the op exhausted its retries). For a CREATE the
    /// server row holds no completed content and is only a broken
    /// `is_uploading` duplicate, so its upload is abandoned — never trashed:
    /// it is this client's own row, not something the user deleted
    /// (best-effort: the server's stale-upload sweep still reaps it after 7
    /// days if this fails). A REPLACE targets the user's existing file and its
    /// session is only dropped locally. The resume row is always dropped so the
    /// next attempt starts a fresh session.
    async fn abandon_upload_session(&self, session: &UploadResume) -> anyhow::Result<()> {
        if session.is_create {
            match self.api.abandon_upload(&session.server_file_id).await {
                Ok(()) => tracing::info!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    "upload: abandoned the in-progress server row of an abandoned create"
                ),
                Err(e) => tracing::warn!(
                    op_id = %session.op_id,
                    file_id = %session.server_file_id,
                    error = %e,
                    "upload: could not abandon the in-progress server row; the server's stale-upload sweep reaps it"
                ),
            }
        }
        self.db.clear_upload_resume(&session.op_id)?;
        Ok(())
    }

    /// Called when an upload op has exhausted its retries: nothing will ever
    /// resume its session, so abandon it now instead of leaving the orphan
    /// visible for the server's 7-day stale-upload window.
    async fn abandon_upload_after_give_up(&self, op: &PendingOperation) {
        if !matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile) {
            return;
        }
        match self.db.get_upload_resume(&op.op_id) {
            Ok(Some(session)) => {
                if let Err(e) = self.abandon_upload_session(&session).await {
                    tracing::warn!(op_id = %op.op_id, error = %e, "upload: failed to abandon given-up session");
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(op_id = %op.op_id, error = %e, "upload: failed to read resume state"),
        }
    }

    async fn upload_thumbnails_for_plaintext_media(
        &self,
        server_file_id: &str,
        payload_path: &Path,
        mime_type: Option<&str>,
        file_key: &beebeeb_core::kdf::FileKey,
    ) -> anyhow::Result<()> {
        let pin = owned_key_and_pin(&self.db, self.api.master_key(), server_file_id)?.1;
        let uploads = prepare_thumbnail_uploads_with_pin(payload_path, mime_type, file_key, pin)?;
        for upload in uploads {
            self.api
                .upload_thumbnail(
                    server_file_id,
                    upload.variant.label(),
                    &upload.encrypted,
                    upload.blurhash.as_deref(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("upload {} thumbnail: {e}", upload.variant.label()))?;
        }
        Ok(())
    }

    /// The journal row that defers a create's native stamping past the landing, or `None`
    /// when there is nothing on disk to stamp. The landing writes it in its own transaction,
    /// before any cancellable post-completion work, so the op is never gone without it.
    #[cfg(target_os = "windows")]
    fn defer_local_upload_finalization(
        &self,
        op: &PendingOperation,
        server_file_id: &str,
        sync_root: &Path,
        payload_path: &Path,
    ) -> anyhow::Result<Option<crate::state_db::UploadFinalization>> {
        let is_create = op
            .metadata_json
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .is_some_and(|m| m["operation"].as_str() == Some("create_file"));
        if !is_create {
            return Ok(None);
        }
        let Some(queued_path) = op.target_path.as_deref() else {
            return Ok(None);
        };
        // Task 2204 S0.2 (review-128 F4): renamed since it was queued, the file is at its row's
        // path; the landing writes the journal there too.
        let renamed_path = match op.file_id.as_deref() {
            Some(id) if self.db.has_queued_local_rename(id)? => self.db.get_file(id)?.map(|row| row.path),
            _ => None,
        };
        let target_path = renamed_path.as_deref().unwrap_or(queued_path);
        let on_disk = local_file_path_under_sync_root(sync_root, target_path)?;
        if !on_disk.is_file() {
            return Ok(None);
        }
        Ok(Some(crate::state_db::UploadFinalization {
            op_id: op.op_id.clone(),
            local_file_id: op.file_id.as_deref().unwrap_or(server_file_id).to_string(),
            server_file_id: server_file_id.to_string(),
            target_path: target_path.to_string(),
            payload_path: payload_path.to_string_lossy().into_owned(),
            stamped: false,
            review: None,
        }))
    }

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

    async fn shared_folder_material(
        &self,
        root: &SharedRootMapping,
    ) -> anyhow::Result<(Zeroizing<[u8; 32]>, serde_json::Value)> {
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
                anyhow::ensure!(row["folder_id"].as_str() == Some(&root.file_id)
                    && row["key_version"].as_u64().is_some_and(|v| v > 0), "invalid folder custody record");
                // The server returns the caller's copy: L6 for owners, the
                // existing authenticated invite envelope for members. A member
                // cannot open another account's owner/L6 envelope.
                let custody_key = if row["role"] == "member" {
                    let wrapped = row["wrapped_folder_key"].as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing member custody key"))?;
                    unwrap_folder_share_key(self.api.master_key(), sender_public_key, &root.file_id, wrapped)?
                } else {
                    anyhow::ensure!(row["role"] == "owner", "invalid custody role");
                    let wrap = crate::owner_keys::normalize_wrap(&row["owner_wrapped_folder_key"])?
                        .ok_or_else(|| anyhow::anyhow!("missing owner custody key"))?;
                    let key = beebeeb_sync::key_resolution::unwrap_folder_custody(
                        &beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key()), &root.file_id, &wrap)?;
                    Zeroizing::new(*key.as_bytes())
                };
                anyhow::ensure!(*custody_key == *folder_key, "custody and invite keys differ");
                scope.remember_custody(&root.file_id)?;
                true
            }
            Err(error) if crate::owner_keys::custody_absent(&error) => {
                anyhow::ensure!(!self.db.has_seen_folder_custody(&root.file_id)?, "This folder's keys are unavailable");
                false
            }
            Err(error) => return Err(error),
        };
        let mut folder_keys_response = self.api.get_folder_keys(&root.invite_id).await?;
        scope.check()?;
        folder_keys_response["desktop_has_custody"] = serde_json::json!(custody);
        Ok((folder_key, folder_keys_response))
    }

    fn shared_folder_child_key(
        &self,
        folder: &[u8; 32],
        root: &str,
        id: &str,
        kind: u8,
        wrap: &str,
        custody: bool,
    ) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
        if custody {
            return unwrap_folder_child_for_format(folder, root, id, kind, wrap, true);
        }
        self.db.read_without_folder_custody(root, || {
            unwrap_folder_child_for_format(folder, root, id, kind, wrap, false)
        })?.ok_or_else(|| anyhow::anyhow!("This folder's keys are unavailable"))?
    }

    fn decrypted_folder_root_name(
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
    ) -> anyhow::Result<()> {
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

    pub fn queue_finder_create(&self, target: FinderWriteTarget) -> anyhow::Result<FinderWriteOutcome> {
        self.queue_finder_create_from(target, None)
    }

    /// [`Self::queue_finder_create`], reading a file's contents from
    /// `contents` when given: a file the caller already opened and checked
    /// (the macOS handed-over copy, see `ipc_socket::open_staged_contents`).
    /// `target.contents_path` is then only the journal's label for it, and is
    /// never opened.
    pub fn queue_finder_create_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        let Some(start) = self.start_finder_create(&target)? else {
            return Ok(ignored_finder_item(&target.filename));
        };
        match target.kind {
            FinderWriteItemKind::Folder => {
                let FinderCreateStart {
                    parent_contract,
                    file_id,
                    rel_path,
                } = start;
                let metadata = owned_metadata_for_name(&self.db, self.api.master_key(), &file_id, &target.filename, None)?;
                // Spec §7.2 (final review): an extension's create names its folder by parent
                // only, so the row takes the full relative path under that parent, as every
                // Finder writer does. A caller's own relative path (the watcher's) is kept.
                let row_path = match &target.rel_path {
                    Some(_) => rel_path.clone(),
                    None => self
                        .db
                        .relative_path_under(target.parent_id.as_deref(), &target.filename)?,
                };
                // Write the local folder row + contract synchronously, the same
                // way the File branch writes its `Uploading` row, so a nested
                // child dispatched later in the SAME scan resolves this folder as
                // its parent (`resolve_parent_id_for` reads `files.item_kind`)
                // without waiting for a server `/sync` round-trip. `Local` =
                // present on disk, server CreateFolder op pending in the queue.
                // Spec §7.2 (final review): the row and its parent in one transaction, so no
                // listing sees the new folder without its parent.
                let entry = FileEntry {
                    file_id: file_id.clone(),
                    path: row_path,
                    status: FileStatus::Local,
                    size_bytes: 0,
                    modified_at: now_secs(),
                    content_hash: None,
                    remote_updated_at: 0,
                    // Not persisted by upsert_file; the contract update owns these.
                    parent_id: None,
                    item_kind: ItemKind::Folder,
                };
                let mut payload = serde_json::json!({
                    "operation": "create_folder",
                    "name_encrypted": metadata,
                });
                apply_shared_context(&mut payload, parent_contract.as_ref());
                self.enqueue_finder_operation(
                    OperationKind::CreateFolder,
                    Some(file_id),
                    target.parent_id.clone(),
                    Some(rel_path),
                    payload,
                    None,
                    None,
                    None,
                    // The row, its contract (`Folder`, under its parent) and the op in one transaction, so a
                    // nested child dispatched later still resolves this folder as its parent, and a refused
                    // create leaves no row.
                    Some(crate::state_db::FinderFolderRow {
                        entry: &entry,
                        parent_id: target.parent_id.as_deref(),
                    }),
                )
            }
            FinderWriteItemKind::File => {
                let PreparedFinderCreate {
                    file_id,
                    rel_path,
                    staged,
                    staged_path,
                    row,
                    payload,
                    ..
                } = self.prepare_finder_create(&target, contents, start)?;
                // Spec §7.2 (final review): the row and its parent in one transaction, as the
                // landing will write them, so no listing sees a new nested file without its
                // parent while it is queued.
                self.db.upsert_file_with_contract(&row, |contract| {
                    contract.item_kind = ItemKind::File;
                    contract.parent_id = target.parent_id.clone();
                })?;
                let outcome = self.enqueue_finder_operation(
                    OperationKind::UploadVersion,
                    Some(file_id),
                    target.parent_id,
                    // target_path = the FULL relative key, so
                    // defer_local_upload_finalization joins sync_root + this
                    // and finds the nested file on disk to re-stamp it in-sync.
                    Some(rel_path),
                    payload,
                    Some(staged_path),
                    None,
                    None,
                    None,
                )?;
                staged.retain();
                Ok(outcome)
            }
        }
    }

    /// What every Finder create checks first, and the ids it gets. `None`: the name is
    /// one Finder writes for itself, which is ignored.
    fn start_finder_create(&self, target: &FinderWriteTarget) -> anyhow::Result<Option<FinderCreateStart>> {
        // Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): refuse a brand-new
        // enqueue once this engine has been asked to stop. Both the Windows
        // upload watcher (`watcher::spawn`'s debounce/scan loops) and the
        // macOS/Linux File Provider extension (via `ipc_socket::handle_connection`'s
        // `QueueFinderCreate` dispatch) call this directly, so this single
        // check covers "the watcher" on every platform without needing a
        // separate flag check duplicated in each caller.
        if self.is_stopping() {
            anyhow::bail!("engine is stopping; refusing to enqueue a new local write");
        }
        if is_ignored_finder_name(&target.filename) {
            return Ok(None);
        }

        let parent_contract = self.ensure_shared_parent_allows_write(target.parent_id.as_deref())?;
        // Honour a caller-supplied id (task 0811 folder scaffolding writes the
        // local row under a known id and needs the server CreateFolder to reuse
        // it — the server accepts a client `folder_id`). The file path and the
        // legacy IPC always pass `None`, getting a fresh uuid as before.
        let file_id = target
            .file_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // Full server-relative key for this item: the FULL nested path when the
        // caller supplied one (`docs/a.txt`), else the leaf filename (top-level /
        // legacy IPC). This is stored as the row's `path` AND threaded as the
        // upload's `target_path`, so filter-3, finalize, and delete all key off
        // the same path. `clone` because `filename` is still needed for the
        // server `name` (always the leaf).
        let rel_path = target.rel_path.clone().unwrap_or_else(|| target.filename.clone());
        Ok(Some(FinderCreateStart {
            parent_contract,
            file_id,
            rel_path,
        }))
    }

    /// A Finder file create, staged and described: the daemon's own copy of the bytes,
    /// the row it inserts and the upload's metadata. Shared by both create entry points.
    fn prepare_finder_create(
        &self,
        target: &FinderWriteTarget,
        contents: Option<&std::fs::File>,
        start: FinderCreateStart,
    ) -> anyhow::Result<PreparedFinderCreate> {
        let FinderCreateStart {
            parent_contract,
            file_id,
            rel_path,
        } = start;
        let contents_path = target
            .contents_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Finder create file callback did not include contents"))?;
        let staged = stage_finder_contents(
            &self.db,
            self.finder_staging_root()?,
            Path::new(contents_path),
            contents,
        )?;
        let staged_path = staged.path().to_string();
        let size_bytes = std::fs::metadata(&staged_path).map(|m| m.len() as i64).unwrap_or(0);
        let mime = target
            .content_type
            .as_deref()
            .or_else(|| beebeeb_core::media::guess_mime_type(&target.filename));
        let name_encrypted = owned_metadata_for_name(&self.db, self.api.master_key(), &file_id, &target.filename, mime)?;
        let row = FileEntry {
            file_id: file_id.clone(),
            // FULL relative key (e.g. `docs/a.txt`), so a nested file's
            // row is found by `get_file_by_path(full_key)` immediately —
            // not keyed by the leaf, which classify_local_path's filter-3
            // would miss for a nested file → spurious re-upload.
            path: rel_path.clone(),
            status: FileStatus::Uploading,
            size_bytes,
            modified_at: now_secs(),
            content_hash: None,
            remote_updated_at: 0,
            // Not persisted by upsert_file; contract owns these.
            parent_id: None,
            item_kind: ItemKind::File,
        };
        let mut payload = serde_json::json!({
            "operation": "create_file",
            "name_encrypted": name_encrypted,
            "content_type": target.content_type,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut payload, parent_contract.as_ref());
        Ok(PreparedFinderCreate {
            file_id,
            rel_path,
            staged,
            staged_path,
            size_bytes,
            row,
            payload,
        })
    }

    /// The File Provider create (spec §8.8: minting is keyed on the IPC entries, never on
    /// the shared `queue_finder_*`, which the watcher and the upload driver call too).
    /// One accept transaction inserts the row, mints the write token and queues the
    /// upload (spec §8.7 S1.1). A folder carries no content and gets no token.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_file_provider_create_from(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        if target.kind == FinderWriteItemKind::Folder {
            return self.queue_finder_create_from(target, contents).map(FpWrite::plain);
        }
        self.queue_file_provider_create_retiring(target, contents, None)?
            .map(|(write, _)| write)
            .ok_or_else(|| anyhow::anyhow!("a create that retires no parked write is never refused as moved"))
    }

    /// Task 2190 ("Keep both", review-169 #3): [`Self::queue_file_provider_create_from`], taking the
    /// place of the parked write `retire` names in the SAME transaction. `Ok(None)`: that write is no
    /// longer decidable at the version shown, nothing was queued, and the copy staged for the create
    /// is removed with its guard. With `retire`, the second value is the parked write's staged copy,
    /// released in the journal, for the caller to unlink. A file only: a folder create goes through
    /// [`Self::queue_file_provider_create_from`], and "Keep both" copies a file.
    fn queue_file_provider_create_retiring(
        &self,
        target: FinderWriteTarget,
        contents: Option<&std::fs::File>,
        retire: Option<crate::state_db::RetireParked<'_>>,
    ) -> anyhow::Result<Option<(FpWrite, Option<String>)>> {
        let Some(start) = self.start_finder_create(&target)? else {
            return Ok(Some((FpWrite::plain(ignored_finder_item(&target.filename)), None)));
        };
        let prepared = self.prepare_finder_create(&target, contents, start)?;
        let op_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = serde_json::to_string(&prepared.payload)?;
        let backup_source_key = crate::known_folder::backup_source_key_for_this_device(&prepared.rel_path);
        self.seam("accept:before_tx");
        let accepted = self.db.accept_finder_write(
            &crate::state_db::FinderAccept {
                retire_parked: retire,
                op_id: &op_id,
                file_id: &prepared.file_id,
                kind: crate::state_db::FinderAcceptKind::Create { row: &prepared.row },
                parent_id: target.parent_id.as_deref(),
                // The FULL relative key, as the Finder create queues it.
                target_path: Some(&prepared.rel_path),
                metadata_json: &metadata_json,
                payload_path: &prepared.staged_path,
                size_bytes: prepared.size_bytes,
                modified_at: prepared.row.modified_at,
                backup_source_key: backup_source_key.as_deref(),
                now: now_secs(),
            },
            &|entry, contract| legacy_item_identifiers(entry, contract).to_vec(),
        )?;
        let (token, released) = match accepted {
            crate::state_db::AcceptOutcome::Queued { token, .. } => (token, None),
            crate::state_db::AcceptOutcome::QueuedRetiring { token, released } => (token, released),
            crate::state_db::AcceptOutcome::ParkedAtOnce { token, reason } => {
                log_parked(&op_id, Some(&prepared.file_id), reason);
                (token, None)
            }
            crate::state_db::AcceptOutcome::ParkedMoved => return Ok(None),
            crate::state_db::AcceptOutcome::UnknownItem => anyhow::bail!("a create always inserts its row"),
            crate::state_db::AcceptOutcome::InTrash => anyhow::bail!("a create is never refused as in the trash"),
            crate::state_db::AcceptOutcome::CopyGone => return Err(anyhow::Error::new(StagedCopyGone)),
            crate::state_db::AcceptOutcome::AcceptsClosed => return Err(anyhow::Error::new(AcceptsClosed)),
        };
        prepared.staged.retain();
        Ok(Some((
            FpWrite {
                outcome: FinderWriteOutcome::Queued {
                    op_id,
                    file_id: Some(prepared.file_id),
                    kind: OperationKind::UploadVersion,
                    ignored: false,
                    message: "queued for encrypted sync".to_string(),
                },
                token: Some(token),
                present_as: None,
            },
            released,
        )))
    }

    /// A create with `.deletionConflicted` (spec §7.2, I-1): the system could not apply our
    /// delete of the template's item because the person edited it. Resolved through the alias
    /// (P→S), or a live row with the template's id, it is a content modify of that row,
    /// replied under the provider's identifier, S (`REPL.h:438-443`), so the system's reuse
    /// rule replaces item S on disk with the edited file. Otherwise an ordinary create.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn queue_file_provider_deletion_conflicted_create(
        &self,
        target: FinderWriteTarget,
        template_identifier: Option<String>,
        template_content_version: Option<String>,
        contents: Option<&std::fs::File>,
    ) -> anyhow::Result<FpWrite> {
        let resolved = match template_identifier.as_deref() {
            Some(id) => match self.db.get_file(id)? {
                Some(row) => Some(row),
                None => match self.db.resolve_alias(id)? {
                    Some(server_id) => {
                        log_alias_resolved("deletion_conflicted_create", id, &server_id);
                        self.db.get_file(&server_id)?
                    }
                    None => None,
                },
            },
            None => None,
        };
        match resolved {
            // A content modify of S. Only a file has content to modify.
            Some(row)
                if row.status != FileStatus::Trashing
                    && row.item_kind == ItemKind::File
                    && target.kind == FinderWriteItemKind::File =>
            {
                self.queue_file_provider_modify_from(
                    FinderWriteTarget {
                        file_id: Some(row.file_id),
                        parent_id: target.parent_id,
                        filename: target.filename,
                        rel_path: None,
                        kind: FinderWriteItemKind::File,
                        contents_path: target.contents_path,
                        content_type: target.content_type,
                        base_version_identifier: template_content_version,
                    },
                    contents,
                )
            }
            // No alias and no row (a file deleted elsewhere: the edit becomes a new file), or a
            // row going to the trash (the edit must not follow it there): an ordinary create.
            _ => self.queue_file_provider_create_from(target, contents),
        }
    }

    /// §7.2: the id a request under `id` acts on, and the id its reply is presented under. A
    /// live row with `id` is acted on as is; otherwise an alias sends a provisional id to the
    /// server id its create landed as, presented under `id`. An id with neither is returned
    /// as is, for the caller to refuse or to let fail.
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub fn resolve_provisional(&self, id: &str, request: &'static str) -> anyhow::Result<(String, Option<String>)> {
        if self.db.get_file(id)?.is_some() {
            return Ok((id.to_string(), None));
        }
        match self.db.resolve_alias(id)? {
            Some(server_id) => {
                log_alias_resolved(request, id, &server_id);
                Ok((server_id, Some(id.to_string())))
            }
            None => Ok((id.to_string(), None)),
        }
    }

    /// §7.4: a fetch of an item whose held write is queued is served from that write's
    /// staged bytes, the bytes its token names, and never reaches the server; otherwise
    /// from the server, as before. A write that has parked with its copy gone is the one
    /// exception: nothing local can serve it, so the server's version is fetched.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub async fn serve_hydrate(
        &self,
        id: &str,
        dest: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<HydrateSource> {
        if !hydrate_dest_is_allowed(dest, allowed_roots) {
            anyhow::bail!("hydrate destination is not within an allowed root");
        }
        let (file_id, _) = self.resolve_provisional(id, "hydrate")?;
        // A copy that is gone at the open is decided once more. If the landing unlinked it, the
        // write is no longer queued and the server has its bytes (§7.4, 4b). A write that is
        // still queued is never fetched from the server, which holds only the previous version:
        // with its copy missing, the fetch fails (`QueuedCopyMissing`), and the system retries.
        // A write that has parked never leaves the queue by itself and
        // its copy cannot come back, so that retry would never end. Its fetch goes to the
        // server for the current version, and the item keeps the parked status (§9.1).
        let mut copy_missing = false;
        for _ in 0..2 {
            let Some((path, parked)) = self.db.queue_fetch_source(&file_id)? else {
                copy_missing = false;
                break;
            };
            self.seam("hydrate:before_open");
            match std::fs::File::open(&path) {
                Ok(mut staged) => {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = write_hydrated_from_reader(dest, allowed_roots, &mut staged)?;
                    // Recorded as the server path records it, so the sign-out sweep finds a
                    // copy written outside the hydrate cache.
                    self.record_hydrated_destination(&file_id, dest)?;
                    if let Some(progress) = progress {
                        progress(bytes, bytes);
                    }
                    log_hydrate_served(&file_id, HydrateSource::Queue);
                    return Ok(HydrateSource::Queue);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && parked => {
                    copy_missing = false;
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    copy_missing = true;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
        if copy_missing {
            return Err(anyhow::Error::new(QueuedCopyMissing));
        }
        self.hydrate_file_with_progress(&file_id, dest, allowed_roots, progress)
            .await?;
        log_hydrate_served(&file_id, HydrateSource::Server);
        Ok(HydrateSource::Server)
    }

    /// A Finder thumbnail (spec §5.6, §7.2): through the alias, and never while the item's
    /// held write is queued, nor after its landing until that version's thumbnails are
    /// uploaded (the safe default: a per-item error, and nothing asked of the server).
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub async fn finder_thumbnail(&self, id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let (file_id, _) = self.resolve_provisional(id, "thumbnail")?;
        // The queue first, then the mark. A landing sets the mark before it commits the op's
        // removal and clears it only after the upload, so a request that finds the write no
        // longer queued finds the mark. Read the other way round, a landing between the two
        // reads served the previous version's thumbnail under the new token.
        let queued = self
            .db
            .item_presentation(&file_id)?
            .is_some_and(|presentation| presentation.held_write_queued);
        self.seam("thumbnail:between_reads");
        let marked = self.thumbnails_pending(&file_id);
        if queued || marked {
            return Err(anyhow::Error::new(ThumbnailNotYet));
        }
        self.fetch_thumbnail_to_memory(&file_id, variant).await
    }

    /// [`Self::queue_file_provider_create_from`] reading the contents by path.
    #[cfg(test)]
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
        let Some((file_id, item_contract)) = self.start_finder_modify(&target)? else {
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
            let outcome = self.enqueue_finder_operation(
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
            )?;
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
        }
    }

    /// A metadata-only modify's kind and payload: a move is a change of parent, read against
    /// the row BEFORE the op changes it.
    fn metadata_update_payload(
        &self,
        file_id: &str,
        target: &FinderWriteTarget,
        item_contract: Option<&crate::state_db::FileContractState>,
    ) -> anyhow::Result<(OperationKind, serde_json::Value)> {
        let name_encrypted = owned_metadata_for_name(
            &self.db, self.api.master_key(),
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
            "operation": "metadata_update",
            "name_encrypted": name_encrypted,
            "base_version_identifier": target.base_version_identifier,
        });
        apply_shared_context(&mut payload, item_contract);
        Ok((kind, payload))
    }

    /// Task 2204 S0.1: a Windows rename or move of a tracked FILE, queued with its row moved to
    /// `new_path` in the same transaction (`StateDb::enqueue_local_rename`). The op is marked
    /// `local_rename`, which holds the row against stale landings and pulls until it is sent.
    pub fn queue_local_rename(&self, target: FinderWriteTarget, new_path: &str) -> anyhow::Result<FinderWriteOutcome> {
        let Some((file_id, item_contract)) = self.start_finder_modify(&target)? else {
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
        match self.db.enqueue_local_rename(&op, &file_id, new_path, target.parent_id.as_deref())? {
            crate::state_db::FinderEnqueue::AcceptsClosed => Err(anyhow::Error::new(AcceptsClosed)),
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
    ) -> anyhow::Result<Option<(String, Option<FileContractState>)>> {
        // Task 1538 Codex P1 — see `queue_finder_create`'s identical guard.
        if self.is_stopping() {
            anyhow::bail!("engine is stopping; refusing to enqueue a new local write");
        }
        if is_ignored_finder_name(&target.filename) {
            return Ok(None);
        }

        let file_id = target
            .file_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Finder modify callback did not include a file id"))?;
        let item_contract = self.ensure_item_allows_shared_write(&file_id, "modify")?;
        Ok(Some((file_id, item_contract)))
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
    ) -> anyhow::Result<PreparedFinderModify> {
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
            "operation": "upload_version",
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
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
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
                return Err(anyhow::Error::new(UnknownItem { requested_id }));
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
        let Some((file_id, item_contract)) = self.start_finder_modify(&target)? else {
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
                }));
            }
            crate::state_db::AcceptOutcome::CopyGone => return Err(anyhow::Error::new(StagedCopyGone)),
            crate::state_db::AcceptOutcome::AcceptsClosed => return Err(anyhow::Error::new(AcceptsClosed)),
            // §7.2: refused; the staged copy is dropped with `prepared`, and nothing was queued.
            crate::state_db::AcceptOutcome::InTrash => return Err(anyhow::Error::new(ItemInTrash)),
            // A modify retires no parked write (task 2190's "Keep both" is a create).
            crate::state_db::AcceptOutcome::ParkedMoved | crate::state_db::AcceptOutcome::QueuedRetiring { .. } => {
                anyhow::bail!("a modify retires no parked write")
            }
        };
        prepared.staged.retain();
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
    #[cfg(test)]
    pub fn queue_file_provider_modify(&self, target: FinderWriteTarget) -> anyhow::Result<FpWrite> {
        self.queue_file_provider_modify_from(target, None)
    }

    pub fn queue_finder_delete(
        &self,
        file_id: &str,
        base_version_identifier: Option<String>,
    ) -> anyhow::Result<FinderWriteOutcome> {
        // Task 1538 Codex P1 — see `queue_finder_create`'s identical guard.
        if self.is_stopping() {
            anyhow::bail!("engine is stopping; refusing to enqueue a new local write");
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
                return self.queue_finder_delete(&server_id, base_version_identifier);
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
        let mut payload = serde_json::json!({
            "operation": "trash",
            "base_version_identifier": base_version_identifier,
        });
        apply_shared_context(&mut payload, item_contract.as_ref());
        let outcome = self.enqueue_finder_operation(
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
        )?;
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

    /// Single, parent-aware classifier shared by every local-write trigger
    /// (the retired `notify` watcher *and* the Windows Cloud Files NOTIFY
    /// callbacks). Given an absolute on-disk `path` under `sync_root`, it
    /// decides whether `path` is a *genuinely-new* user file that should be
    /// uploaded, and if so returns a ready-to-queue [`FinderWriteTarget`] with
    /// `parent_id` resolved from the path's parent directory.
    ///
    /// It runs the three feedback-loop filters, in order, so a download / a
    /// hydration / an engine-internal write is NEVER mistaken for a new upload:
    ///
    /// 1. **Engine-internal paths + OS junk** — anything under `<root>/.beebeeb/`,
    ///    the `<root>/.beebeeb-sync.lock`, any `.beebeeb` path component, and the
    ///    ignored-name set ([`is_ignored_finder_name`]: `.tmp`, `~$…`, `.DS_Store`,
    ///    …). These are writes the engine itself (or the OS) makes constantly.
    /// 2. **Cloud Files placeholders** (Windows only) — a reparse point under the
    ///    sync root is something WE minted (placeholder seed or post-upload
    ///    convert). Hydration fills a placeholder's data stream WITHOUT clearing
    ///    its reparse-point attribute, so a hydration-write still trips this guard
    ///    and never re-uploads. Checked via
    ///    [`crate::windows_cf::placeholders::is_cloud_placeholder`].
    /// 3. **Already a known server file** (authoritative) — look the path up in
    ///    the state DB. A row means the file already lives on the server
    ///    (cloud-only / downloading / local / uploading), so a write to it is a
    ///    hydration or a re-download, NOT a new creation. Only a path with NO DB
    ///    row survives.
    ///
    /// `parent_id` resolution (nested uploads): for a survivor at
    /// `<parent_dir>/<name>`, the parent directory's server folder id is looked
    /// up by its server-relative path. A root-level file resolves to `None`; a
    /// nested file resolves to `Some(<parent folder file_id>)` so the upload
    /// lands in the right server folder. If the parent directory has no DB row
    /// yet (its folder placeholder hasn't reconciled), `parent_id` falls back to
    /// `None` rather than guessing — the file uploads to the root and a later
    /// tick can reconcile, which is strictly safer than attaching it to the
    /// wrong parent.
    ///
    /// Returns `None` when `path` is not a regular file, is filtered out, or the
    /// DB lookup fails (fail-closed: a lookup error skips the upload rather than
    /// risk a spurious one). Cross-platform so it type-checks everywhere; only
    /// the Windows triggers call it.
    pub fn classify_local_path(&self, sync_root: &Path, path: &Path) -> Option<FinderWriteTarget> {
        match self.classify_local_path_verdict(sync_root, path) {
            LocalPathVerdict::Write(target) => Some(target),
            LocalPathVerdict::Settled | LocalPathVerdict::Skip => None,
        }
    }

    /// Task 2229: [`Self::classify_local_path`], telling a tracked file that needs nothing while its row
    /// and its bytes stay as they are ([`LocalPathVerdict::Settled`]) from one to ask about again.
    pub(crate) fn classify_local_path_verdict(&self, sync_root: &Path, path: &Path) -> LocalPathVerdict {
        // The file may have been deleted/renamed during a debounce window — if
        // it is no longer a regular file there is nothing to upload.
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => {}
            Ok(m) => {
                let file_type = m.file_type();
                tracing::debug!(
                    path = %path.display(),
                    is_dir = file_type.is_dir(),
                    is_symlink = file_type.is_symlink(),
                    "classify_local_path: rejected non-regular file"
                );
                return LocalPathVerdict::Skip;
            }
            Err(e) => {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "classify_local_path: stat failed; skipping"
                );
                return LocalPathVerdict::Skip;
            }
        }

        // Filter 1 — engine-internal paths + OS junk.
        if path_is_engine_internal(sync_root, path) {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected engine-internal path"
            );
            return LocalPathVerdict::Skip;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected path with non-utf8 filename"
            );
            return LocalPathVerdict::Skip;
        };
        if is_ignored_finder_name(&file_name) {
            tracing::debug!(
                path = %path.display(),
                filename = %file_name,
                "classify_local_path: rejected ignored/temp filename"
            );
            return LocalPathVerdict::Skip;
        }

        // Filter 3 (authoritative) — already a known server file? Then it is not a new
        // upload. Task 1640: it may be an edit of that file; `classify_tracked_edit` decides
        // from the content this device last agreed with the server on, before filter 2, which
        // rejects every placeholder (a synced file is one).
        let Some(rel) = relative_db_path(sync_root, path) else {
            tracing::debug!(
                path = %path.display(),
                sync_root = %sync_root.display(),
                "classify_local_path: rejected path outside sync root"
            );
            return LocalPathVerdict::Skip;
        };
        // Task 1640 (review N13): an original whose kept copy is made but which is not removed
        // yet (it was in use) is never sent: a later tick removes it.
        if self.db.is_kept_original_pending(&rel).unwrap_or(false) {
            tracing::debug!(rel_path = %rel, "classify_local_path: rejected an original whose copy is kept");
            return LocalPathVerdict::Skip;
        }
        // Task 2204 S0.4: a file at the path a Windows rename vacated waits until that rename is
        // sent: it may be a safe save's new bytes, which fold back onto the original.
        if self.db.is_vacated_by_pending_rename(&rel).unwrap_or(true) {
            tracing::debug!(rel_path = %rel, "classify_local_path: the path a pending rename vacated; skipped this pass");
            return LocalPathVerdict::Skip;
        }
        match self.row_judging_bytes_at(&rel, path) {
            Ok(RowAtPath::One(existing)) => return self.classify_tracked_edit(path, existing, file_name, rel),
            Ok(RowAtPath::Ambiguous { file_ids, rows }) => {
                // Lead ruling P2 (2271 contract §7): file ids and the row count, never the path.
                tracing::warn!(
                    ?file_ids,
                    rows,
                    "sync folder: several synced files share one path and its bytes are none of them; kept on this PC, not sent"
                );
                return LocalPathVerdict::Settled;
            }
            Ok(RowAtPath::NotWhollyOnDisk) => {
                tracing::debug!(rel_path = %rel, "classify_local_path: several rows at a path not wholly on disk; asked again later");
                return LocalPathVerdict::Skip;
            }
            Ok(RowAtPath::None) => { /* not tracked — filter 2, then new */ }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    rel_path = %rel,
                    error = %e,
                    "classify_local_path: could not tell which row the path's bytes belong to (a state DB lookup or a read of the bytes failed); skipping to be safe"
                );
                return LocalPathVerdict::Skip;
            }
        }

        // Filter 2 — Cloud Files placeholders are engine-owned (Windows only). An
        // untracked reparse point under the sync root is a placeholder we minted or
        // converted whose row is not written yet; the hydration write that fills it
        // does NOT turn it back into a plain file, so we must never treat a
        // placeholder write as a new upload.
        #[cfg(target_os = "windows")]
        if crate::windows_cf::placeholders::is_cloud_placeholder(path) {
            // Task 2191 (D12): an untracked placeholder may be one a remote rename left behind,
            // holding unknown bytes; the tick's retry checks its identity.
            if let Err(e) = self.db.put_unknown_orphan(&rel, "", true) {
                tracing::debug!(error = %e, "classify_local_path: could not record an untracked placeholder");
            }
            tracing::debug!(
                path = %path.display(),
                "classify_local_path: rejected Cloud Files placeholder"
            );
            return LocalPathVerdict::Skip;
        }

        // parent_id resolution: a survivor at `<parent_dir>/<name>`. If the parent
        // directory maps to a known server FOLDER row, attach the new file to it;
        // otherwise upload at the root (None). We never attach to a row that isn't
        // a folder.
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        tracing::debug!(
            path = %path.display(),
            rel_path = %rel,
            nested = parent_id.is_some(),
            "classify_local_path: accepted new local file"
        );

        LocalPathVerdict::Write(FinderWriteTarget {
            file_id: None,
            parent_id,
            filename: file_name.clone(),
            // The FULL '/'-joined relative key (e.g. `docs/a.txt`), so the row is
            // stored + the upload's target_path is threaded under the same key
            // that filter-3, finalize, and delete all query. `rel` was computed
            // above for the filter-3 lookup; reuse it verbatim.
            rel_path: Some(rel),
            kind: FinderWriteItemKind::File,
            contents_path: Some(path.to_string_lossy().into_owned()),
            content_type: beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string),
            base_version_identifier: None,
        })
    }

    /// Task 2190 (the lead's condition on E3; glegion's constraints from #167's scan memo): the ONE
    /// call each held branch of [`Self::classify_tracked_edit`] makes. Records that `file_id`'s edit
    /// is held, not sent, with the file's content fingerprint, so the review feed shows it. Nothing
    /// is written while neither the reason nor the fingerprint changed, and the `files` row is never
    /// touched (the memo skips a settled file while that row is unchanged). A failure is logged: the
    /// edit stays held either way.
    /// Task 2244: a hold that writes is logged at DEBUG and counted; the scan logs the count once a
    /// pass, not a line per file (28,224 on every start of 0.8.17 on the maintainer's PC).
    fn hold_unsent_edit(&self, file_id: &str, path: &Path, reason: &str) {
        let fingerprint = ContentFingerprint::of(path).map(|fp| format!("{fp:?}")).unwrap_or_default();
        match self.db.hold_unsent_edit(file_id, reason, &fingerprint, now_secs()) {
            Ok(true) => {
                tracing::debug!(file_id = %file_id, reason, "sync folder: an edit of a synced file is kept on this PC, not sent");
                self.newly_held[usize::from(reason == HELD_BASE_UNKNOWN)].fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(file_id = %file_id, error = %e, "sync folder: could not record a held edit"),
        }
    }

    /// Task 2244: the edits newly held since the last call, for the scan's one line a pass:
    /// (content unknown, base unknown).
    pub(crate) fn take_newly_held(&self) -> (usize, usize) {
        let [content_unknown, base_unknown] = &self.newly_held;
        (
            content_unknown.swap(0, Ordering::Relaxed),
            base_unknown.swap(0, Ordering::Relaxed),
        )
    }

    /// Task 2190: the one call that ends a held edit's record, when the file reads as agreed, is
    /// queued, or can be sent on a known base.
    fn release_unsent_edit(&self, file_id: &str) {
        if let Err(e) = self.db.release_unsent_edit(file_id) {
            tracing::warn!(file_id = %file_id, error = %e, "sync folder: could not clear a held edit");
        }
    }

    /// Task 2271: the row the scan judges the bytes at `rel` by. Normally the one row there. When
    /// several share the path (0.8.18's reconcile pass left the server's file of that name and the
    /// copy this PC sent at one path), the row that records content this PC agreed on decides: the
    /// only such row, or the one whose agreed content the bytes are. With no such row it is the
    /// path's first, as before. With several and bytes that are none of them, no row is guessed.
    /// Choosing among several reads the bytes once, and only when they are wholly on this PC.
    fn row_judging_bytes_at(&self, rel: &str, path: &Path) -> anyhow::Result<RowAtPath> {
        let mut rows = self.db.file_rows_at_path(rel)?;
        if rows.len() < 2 {
            return Ok(rows.pop().map_or(RowAtPath::None, RowAtPath::One));
        }
        let mut recorded = Vec::new();
        for (at, row) in rows.iter().enumerate() {
            if row.is_dir() {
                continue;
            }
            let agreed = match self.db.get_file_contract_state(&row.file_id)?.and_then(|c| c.local_hash) {
                Some(hash) => Some(Some(hash)),
                None => self.db.fetched_content(&row.file_id)?.map(|(_, hash)| hash),
            };
            if let Some(agreed) = agreed {
                recorded.push((at, agreed));
            }
        }
        let at = match recorded[..] {
            [] => 0,
            [(at, _)] => at,
            [(first, _), ..] => {
                // Windows: reading a placeholder that is not wholly on disk would fetch the rest of
                // it (as in `classify_tracked_edit`); the scan asks again once it is.
                #[cfg(target_os = "windows")]
                if !crate::windows_cf::placeholders::fully_on_disk(path) {
                    return Ok(RowAtPath::NotWhollyOnDisk);
                }
                let bytes = cached_sha256_file_hex(&rows[first].file_id, path)?;
                // A mixed fetch (no single version) is never agreed.
                let matching: Vec<usize> = recorded
                    .iter()
                    .filter(|(_, agreed)| agreed.as_deref() == Some(bytes.as_str()))
                    .map(|(at, _)| *at)
                    .collect();
                match matching[..] {
                    [at] => at,
                    _ => {
                        let file_ids = recorded.iter().map(|(at, _)| rows[*at].file_id.clone()).collect();
                        return Ok(RowAtPath::Ambiguous { file_ids, rows: rows.len() });
                    }
                }
            }
        };
        Ok(RowAtPath::One(rows.swap_remove(at)))
    }

    /// Task 1640: a close or scan of a tracked sync-folder file is an edit when the file is a
    /// synced (`Local`) file and its bytes differ from the content this device last agreed with
    /// the server on. The edit is a content modify of that file, sent with the version its bytes
    /// are based on, so the server refuses it as stale when the file changed elsewhere meanwhile
    /// (it then parks with its bytes). Anything else is not an upload: a folder, a row in
    /// transfer or in conflict, agreed bytes, an upload already queued, no agreed content
    /// recorded, or an edit whose base is not known (never sent without one: it would replace
    /// the server's version unseen).
    fn classify_tracked_edit(
        &self,
        path: &Path,
        existing: FileEntry,
        file_name: String,
        rel: String,
    ) -> LocalPathVerdict {
        if existing.is_dir() || existing.status != FileStatus::Local {
            tracing::debug!(
                path = %path.display(),
                rel_path = %rel,
                "classify_local_path: rejected already-tracked server file"
            );
            return LocalPathVerdict::Settled;
        }
        // Windows: reading a placeholder that is not wholly on disk would fetch the rest of it.
        #[cfg(target_os = "windows")]
        let os_in_sync = {
            if !crate::windows_cf::placeholders::fully_on_disk(path) {
                return LocalPathVerdict::Skip;
            }
            crate::windows_cf::placeholders::placeholder_in_sync(path)
        };
        #[cfg(not(target_os = "windows"))]
        let os_in_sync = None;
        let base = match sync_folder_content(&self.db, &existing.file_id, path, os_in_sync) {
            Ok(SyncFolderContent::Changed { base: Some(base) }) => {
                self.release_unsent_edit(&existing.file_id);
                base
            }
            Ok(SyncFolderContent::Changed { base: None }) => {
                self.hold_unsent_edit(&existing.file_id, path, HELD_BASE_UNKNOWN);
                // Task 2229: held until its row records a base; no change of its bytes sends it.
                return LocalPathVerdict::Settled;
            }
            // A file hydrated before this build has no agreed content: its change is kept on
            // disk (out of sync, so never stamped or freed) but not sent (task 2191).
            // Task 2190: not reported in sync (Windows), or not a placeholder (an atomic save left a
            // plain file; and every build without Cloud Files) may hold an edit.
            Ok(SyncFolderContent::Unknown) if os_in_sync != Some(true) => {
                self.hold_unsent_edit(&existing.file_id, path, HELD_CONTENT_UNKNOWN);
                return LocalPathVerdict::Settled;
            }
            // Task 2229: unknown content is held until its row records the agreed content, whatever
            // its bytes and its in-sync state; the scan remembers it like agreed bytes.
            Ok(SyncFolderContent::Agreed | SyncFolderContent::Unknown) => {
                self.release_unsent_edit(&existing.file_id);
                return LocalPathVerdict::Settled;
            }
            Ok(SyncFolderContent::Queued) => {
                self.release_unsent_edit(&existing.file_id);
                return LocalPathVerdict::Skip;
            }
            Err(e) => {
                tracing::warn!(file_id = %existing.file_id, error = %e, "sync folder: could not compare a synced file with its agreed content");
                return LocalPathVerdict::Skip;
            }
        };
        let contract = self.db.get_file_contract_state(&existing.file_id).ok().flatten();
        tracing::debug!(
            file_id = %existing.file_id,
            base_version = base,
            "classify_local_path: accepted an edit of a synced file"
        );
        LocalPathVerdict::Write(FinderWriteTarget {
            file_id: Some(existing.file_id),
            parent_id: contract.as_ref().and_then(|contract| contract.parent_id.clone()),
            content_type: contract
                .and_then(|contract| contract.content_type)
                .or_else(|| beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string)),
            filename: file_name,
            rel_path: Some(rel),
            kind: FinderWriteItemKind::File,
            contents_path: Some(path.to_string_lossy().into_owned()),
            base_version_identifier: Some(base.to_string()),
        })
    }

    /// Task 1640 (review N1b): the row's version (`None` when unknown), the version a legacy
    /// download records. Accepted for legacy files only: it can say less than the bytes are
    /// (a later change then parks as stale), never more.
    fn legacy_download_version(&self, file_id: &str) -> anyhow::Result<Option<i64>> {
        Ok(self
            .db
            .get_file_contract_state(file_id)?
            .map(|contract| contract.current_version)
            .filter(|version| *version > 0))
    }

    /// Task 1640: whether `dest` is `file_id`'s own place in the sync folder, under one of
    /// `allowed_roots`, rather than a staging copy (a Finder fetch, a Linux FUSE temp file).
    fn dest_is_files_sync_folder_place(&self, file_id: &str, dest: &Path, allowed_roots: &[&Path]) -> anyhow::Result<bool> {
        let Some(entry) = self.db.get_file(file_id)? else {
            return Ok(false);
        };
        Ok(allowed_roots.iter().any(|root| {
            local_file_path_under_sync_root(root, &entry.path).is_ok_and(|place| place.as_path() == dest)
        }))
    }

    /// Resolve the server folder id of `path`'s immediate parent directory, for
    /// nested uploads. Returns `None` for a root-level file (parent == sync root)
    /// or when the parent directory has no known server FOLDER row yet. The
    /// parent's server-relative key is built in the same '/'-joined shape the
    /// state DB stores, then looked up via [`StateDb::get_file_by_path`]; a hit
    /// that is a folder yields its `file_id`.
    ///
    /// `pub(crate)` so the upload driver's rename handler can resolve the NEW
    /// parent of a moved file through the exact same logic.
    pub(crate) fn resolve_parent_id_for(&self, sync_root: &Path, path: &Path) -> Option<String> {
        let parent_dir = path.parent()?;
        // Root-level file: parent IS the sync root → no server parent.
        if parent_dir == sync_root {
            return None;
        }
        let parent_rel = relative_db_path(sync_root, parent_dir)?;
        match self.db.get_file_by_path(&parent_rel) {
            Ok(Some(entry)) if entry.is_dir() => Some(entry.file_id),
            // No row, or a row that is somehow a file (shouldn't happen for a
            // directory path) → upload at the root rather than mis-parent.
            _ => None,
        }
    }

    /// Ensure the on-disk DIRECTORY at `path` (under `sync_root`) has a server
    /// vault folder, returning its `file_id`. This is the folder-hierarchy half
    /// of the local-create pipeline (task 0811): the enumeration scan only
    /// uploaded *files*, so a freshly-mirrored nested tree
    /// (`Backup/<device>/<folder>/<sub>/file`) had no folder rows — every nested
    /// file's `resolve_parent_id_for` missed and the file uploaded FLAT to the
    /// vault root. Scaffolding the folder first (parents before children, which
    /// the top-down walk already guarantees) gives each child a real parent to
    /// attach to, so the server vault mirrors the on-disk hierarchy.
    ///
    /// Idempotent + parent-aware:
    /// - If a DB row already exists for this directory's relative key, returns its
    ///   `file_id` (a folder row, or `None` if it is somehow a file row).
    /// - Otherwise mints a client folder id, writes a local `Folder` row + contract
    ///   IMMEDIATELY (so a child file dispatched later in the SAME scan resolves
    ///   this parent without waiting for a server round-trip), and enqueues a
    ///   `CreateFolder` op carrying that same id — the server honours the
    ///   client-supplied `folder_id`, so the local row and the server folder share
    ///   one id and the parent linkage is consistent.
    ///
    /// The op's `target_path` is the folder's relative key, so a backup folder is
    /// auto-tagged with its origin key (Part C) and purged on disable. Returns
    /// `None` for the sync root itself, a non-directory, or on a DB error
    /// (fail-closed: the file then uploads at the root rather than mis-parenting).
    pub fn ensure_local_folder(&self, sync_root: &Path, path: &Path) -> Option<String> {
        // Root itself has no server folder.
        if path == sync_root {
            return None;
        }
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => {}
            _ => return None,
        }
        if path_is_engine_internal(sync_root, path) {
            return None;
        }
        let name = path.file_name().and_then(|n| n.to_str())?.to_string();
        if is_ignored_finder_name(&name) {
            return None;
        }
        let rel = relative_db_path(sync_root, path)?;

        // Already known? Return the existing folder id (idempotent across scans).
        match self.db.get_file_by_path(&rel) {
            Ok(Some(entry)) if entry.is_dir() => return Some(entry.file_id),
            Ok(Some(_)) => return None, // a file row at a dir path — never mis-parent
            Ok(None) => { /* mint below */ }
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: DB lookup failed; skipping");
                return None;
            }
        }

        // Task 1640 (review F3): a folder deleted elsewhere that stays on disk only for what this PC
        // keeps (an original still open, a file partly on disk) is never created again on the
        // server; one the person put something new into is wanted again.
        match self.db.outermost_removed_folder(&rel) {
            Ok(Some(outermost)) => {
                let held = Self::placeholder_path_under(sync_root, &outermost)
                    .is_some_and(|dir| held_only_for_kept(&self.db, sync_root, &dir, 0));
                if held {
                    tracing::debug!("ensure_local_folder: a folder deleted elsewhere, kept for what it holds; not created again");
                    return None;
                }
                let _ = self.db.forget_removed_folder(&outermost);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: kept-folder lookup failed; skipping");
                return None;
            }
        }

        // Parent linkage: resolve THIS directory's parent dir to its folder id
        // (None at the Backup root level — a top-level folder under the vault).
        let parent_id = self.resolve_parent_id_for(sync_root, path);
        let folder_id = uuid::Uuid::new_v4().to_string();

        // Delegate the row write + contract + CreateFolder enqueue to the one
        // shared create path, carrying our pre-minted id (which the server
        // honours) so the local row and the server folder share an id. The
        // `rel`-path `target_path` auto-tags a backup folder with its origin key.
        let target = FinderWriteTarget {
            file_id: Some(folder_id.clone()),
            parent_id,
            filename: name,
            rel_path: Some(rel),
            kind: FinderWriteItemKind::Folder,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        };
        match self.queue_finder_create(target) {
            Ok(FinderWriteOutcome::Queued { op_id, .. }) => {
                tracing::info!(op_id = %op_id, folder_id = %folder_id, "upload driver: scaffolded vault folder for nested upload");
                Some(folder_id)
            }
            Ok(FinderWriteOutcome::Ignored { .. }) => None,
            Err(e) => {
                tracing::warn!(error = %e, "ensure_local_folder: failed to scaffold vault folder");
                None
            }
        }
    }

    /// Apply a pin ("available offline") toggle to `file_id` and its subtree.
    ///
    /// `sync_root` is the on-disk root of the vault — needed on Windows to
    /// resolve each affected DB row to its placeholder path and set the OS-level
    /// pin state. It is unused on macOS/Linux (the `#[cfg]` below silences the
    /// warning) but threaded in unconditionally to keep one signature.
    ///
    /// Three things happen on a pin change:
    /// 1. **DB** — `db.set_recursive_pin` flips the `pin_state` column for the
    ///    whole subtree and returns the ids whose state actually changed.
    /// 2. **Server** — `enqueue_pin_tree_operation` propagates the pin so OTHER
    ///    devices learn about it. Always runs (every platform).
    /// 3. **OS hydration** —
    ///    - **Windows**: `CfSetPinState` is the OS contract. A pinned placeholder
    ///      is hydrated by Windows via the normal FETCH_DATA path AND is exempt
    ///      from auto-dehydration, so it stays resident. We therefore do NOT
    ///      enqueue our own out-of-band hydrate ops (the old DB-only model did,
    ///      because Windows was never told about the pin — that loop is dropped on
    ///      Windows). New children inherit the parent pin via CF_PIN_STATE_INHERIT.
    ///    - **macOS/Linux**: there is no CF pin primitive, so we keep enqueuing an
    ///      explicit hydrate op per newly-pinned cloud-only file to materialise it.
    pub fn set_recursive_pin(
        &self,
        #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] sync_root: &Path,
        file_id: &str,
        pinned: bool,
    ) -> anyhow::Result<PinUpdateOutcome> {
        let now = now_secs();
        let changed_item_ids = self.db.set_recursive_pin(file_id, pinned, now)?;
        // Mutated only on the non-Windows hydrate-enqueue path below; on Windows
        // the OS (CfSetPinState) drives hydration, so this stays 0.
        #[cfg_attr(target_os = "windows", allow(unused_mut))]
        let mut hydrate_operations = 0usize;

        self.enqueue_pin_tree_operation(file_id, pinned, now)?;

        #[cfg(target_os = "windows")]
        {
            // OS-level pin: tell Windows the actual "available offline" state so a
            // pinned file is kept resident (no auto-dehydration) and an unpinned
            // one becomes reclaim-eligible again. We pin/unpin only the TOP item
            // the user toggled, with RECURSE when it is a directory — the
            // recursive flag stamps existing descendants in one call and
            // CF_PIN_STATE_INHERIT covers descendants created later — rather than
            // issuing a per-row CfSetPinState for every changed id.
            if let Some(top) = self.db.get_file(file_id)? {
                if let Some(path) = Self::placeholder_path_under(sync_root, &top.path) {
                    let recurse = top.is_dir();
                    if let Err(e) = crate::windows_cf::placeholders::set_pin_state(&path, pinned, recurse) {
                        // Zero-knowledge: never log the path. A pin-state failure
                        // must not abort the DB+server pin that already succeeded;
                        // surface it as a log line and continue.
                        tracing::warn!(file_id = %file_id, pinned, recurse, error = %e, "CfSetPinState failed for pin toggle");
                    }
                }

                // Proactive hydrate. CfSetPinState(PINNED) marks the subtree
                // pinned but Windows does NOT eagerly download a not-yet-opened
                // placeholder — a pinned-but-unopened file stays cloud-only
                // (RECALL_ON_DATA_ACCESS) until something reads it. So "available
                // offline" is only real once we force the download. Collect the
                // cloud-only file descendants (the toggled item itself if it is a
                // cloud-only file, or every cloud-only file under it if a folder)
                // and CfHydratePlaceholder each. We only ever hydrate on pin, not
                // unpin (unpinning just makes files reclaim-eligible again).
                if pinned {
                    match self.db.cloud_only_file_descendants(file_id) {
                        Ok(entries) if !entries.is_empty() => {
                            // Resolve placeholder paths the SAME way the pin does,
                            // dropping any that fail the containment guard.
                            let paths: Vec<PathBuf> = entries
                                .iter()
                                .filter_map(|e| Self::placeholder_path_under(sync_root, &e.path))
                                .collect();
                            // CfHydratePlaceholder BLOCKS until each file's bytes
                            // are on disk, and a folder may hold many/large files,
                            // so run the whole sweep OFF the IPC thread — set_recursive_pin
                            // returns immediately while hydration proceeds in the
                            // background. A per-file failure is logged and never
                            // aborts the rest of the sweep.
                            let total = paths.len();
                            std::thread::spawn(move || {
                                tracing::debug!(count = total, "proactive pin hydrate: starting");
                                let mut ok = 0usize;
                                for path in &paths {
                                    match crate::windows_cf::placeholders::hydrate_placeholder(path) {
                                        Ok(()) => ok += 1,
                                        // Zero-knowledge: never log the path.
                                        Err(e) => tracing::warn!(error = %e, "proactive pin hydrate: file failed"),
                                    }
                                }
                                tracing::debug!(hydrated = ok, total, "proactive pin hydrate: finished");
                            });
                        }
                        Ok(_) => {} // nothing cloud-only to hydrate
                        Err(e) => {
                            // A query failure must not abort the DB+server pin that
                            // already succeeded; log and continue.
                            tracing::warn!(file_id = %file_id, error = %e, "proactive pin hydrate: descendant query failed");
                        }
                    }
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        if pinned {
            for changed_id in &changed_item_ids {
                let Some(entry) = self.db.get_file(changed_id)? else {
                    continue;
                };
                let Some(contract) = self.db.get_file_contract_state(changed_id)? else {
                    continue;
                };
                if contract.effective_pin_state() == crate::state_db::PinState::Pinned
                    && entry.status == FileStatus::CloudOnly
                    && entry.size_bytes > 0
                {
                    self.enqueue_hydrate_operation(&entry, now)?;
                    hydrate_operations += 1;
                }
            }
        }

        Ok(PinUpdateOutcome {
            changed_item_ids,
            hydrate_operations,
        })
    }

    /// Map a server-relative, '/'-joined DB key (`FileEntry::path`) to its
    /// on-disk placeholder path under `sync_root`, rejecting empty/traversal
    /// keys. Mirrors `windows_cf::callbacks::safe_join_under_root` (kept private
    /// there) so the pin path resolution uses the same containment guard as the
    /// fetch fallback. Returns `None` for an empty key or any `.`/`..`/empty
    /// segment, or if the join escapes the root.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub(crate) fn placeholder_path_under(sync_root: &Path, rel_path: &str) -> Option<PathBuf> {
        let rel = rel_path.trim_matches('/');
        if rel.is_empty() {
            return None;
        }
        if rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
            return None;
        }
        let native_rel = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
        let candidate = sync_root.join(&native_rel);
        if !candidate.starts_with(sync_root) {
            return None;
        }
        Some(candidate)
    }

    pub fn record_smart_cache_open(
        &self,
        file_id: &str,
        cache_path: &Path,
        cache_bytes: i64,
    ) -> anyhow::Result<CacheCleanupOutcome> {
        self.db
            .mark_cached(file_id, &cache_path.to_string_lossy(), cache_bytes, now_secs())?;
        self.enforce_configured_cache_limit()
    }

    pub fn enforce_smart_cache(&self, policy: CachePolicy) -> anyhow::Result<CacheCleanupOutcome> {
        let evicted = self
            .db
            .evict_unpinned_cache_until_under(policy.max_unpinned_cache_bytes, now_secs())?;
        #[cfg(target_os = "linux")]
        self.remove_linux_freedesktop_thumbnails_for_file_ids(&evicted);
        Ok(CacheCleanupOutcome {
            evicted_file_ids: evicted,
        })
    }

    pub fn enforce_local_cache_limit(&self, max_local_cache_bytes: Option<i64>) -> anyhow::Result<CacheCleanupOutcome> {
        self.enforce_local_cache_limit_after_guard(max_local_cache_bytes, &crate::unknown_check::STARTUP_GUARD)
    }

    /// Desktop 0.8.15: [`Self::enforce_local_cache_limit`], which evicts nothing until `guard`'s
    /// start-up pass has walked every file of unknown content (the pass runs beside the engine).
    pub(crate) fn enforce_local_cache_limit_after_guard(
        &self,
        max_local_cache_bytes: Option<i64>,
        guard: &crate::unknown_check::StartupGuard,
    ) -> anyhow::Result<CacheCleanupOutcome> {
        if !guard.allows_freeing() {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        }
        let Some(max_local_cache_bytes) = max_local_cache_bytes else {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        };
        if max_local_cache_bytes <= 0 {
            return Ok(CacheCleanupOutcome {
                evicted_file_ids: Vec::new(),
            });
        }

        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let max_unpinned_cache_bytes = max_local_cache_bytes.saturating_sub(pinned_bytes);
        self.enforce_smart_cache(CachePolicy {
            max_unpinned_cache_bytes,
            ..CachePolicy::default()
        })
    }

    pub fn local_cache_usage_bytes(&self) -> anyhow::Result<i64> {
        let pinned_bytes = self.db.cache_bytes_by_effective_pin(true)?.max(0);
        let unpinned_bytes = self.db.cache_bytes_by_effective_pin(false)?.max(0);
        Ok(pinned_bytes.saturating_add(unpinned_bytes))
    }

    pub fn enforce_configured_cache_limit(&self) -> anyhow::Result<CacheCleanupOutcome> {
        let cfg = crate::config::DesktopConfig::load().unwrap_or_default();
        self.enforce_local_cache_limit(cfg.local_cache_limit_for_eviction())
    }

    #[cfg(target_os = "linux")]
    async fn write_linux_freedesktop_thumbnails(&self, file_id: &str, dest_path: &Path) {
        let entry = match self.db.get_file(file_id) {
            Ok(Some(entry)) if !entry.is_dir() => entry,
            Ok(_) => return,
            Err(error) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: state row unavailable");
                return;
            }
        };

        let source_path = linux_thumbnail_source_path_for_entry(&entry).unwrap_or_else(|| dest_path.to_path_buf());
        let thumbnail = match tokio::time::timeout(
            Duration::from_secs(8),
            self.fetch_thumbnail_to_memory(file_id, "medium"),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail skipped: server thumbnail unavailable");
                return;
            }
            Err(_) => {
                tracing::debug!(file_id = %file_id, "linux thumbnail skipped: server thumbnail fetch timed out");
                return;
            }
        };

        match crate::linux_thumbnail::write_freedesktop_thumbnails(&source_path, entry.modified_at, &thumbnail) {
            Ok(paths) => {
                tracing::debug!(
                    file_id = %file_id,
                    count = paths.len(),
                    "linux freedesktop thumbnails written"
                );
            }
            Err(error) => {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail write failed");
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn remove_linux_freedesktop_thumbnails_for_file_ids(&self, file_ids: &[String]) {
        for file_id in file_ids {
            let entry = match self.db.get_file(file_id) {
                Ok(Some(entry)) if !entry.is_dir() => entry,
                Ok(_) => continue,
                Err(error) => {
                    tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail cleanup skipped: state row unavailable");
                    continue;
                }
            };
            let Some(source_path) = linux_thumbnail_source_path_for_entry(&entry) else {
                continue;
            };
            if let Err(error) = crate::linux_thumbnail::remove_freedesktop_thumbnails(&source_path) {
                tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail cleanup failed");
            }
        }
    }

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
                    "operation": "restore_version",
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
    fn enqueue_finder_operation(
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
    ) -> anyhow::Result<FinderWriteOutcome> {
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
            crate::state_db::FinderEnqueue::AcceptsClosed => Err(anyhow::Error::new(AcceptsClosed)),
            crate::state_db::FinderEnqueue::Queued => Ok(FinderWriteOutcome::Queued {
                op_id,
                file_id,
                kind,
                ignored: false,
                message: "queued for encrypted sync".to_string(),
            }),
        }
    }

    /// The op a Finder or sync-folder write queues (`enqueue_finder_operation`'s first half).
    #[allow(clippy::too_many_arguments)]
    fn finder_operation(
        &self,
        kind: OperationKind,
        file_id: Option<String>,
        parent_id: Option<String>,
        target_path: Option<String>,
        metadata: serde_json::Value,
        payload_path: Option<String>,
        base_version: Option<i64>,
        base_object_version_id: Option<String>,
    ) -> anyhow::Result<PendingOperation> {
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

    fn enqueue_pin_tree_operation(&self, file_id: &str, pinned: bool, now: i64) -> anyhow::Result<()> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::PinTree,
            file_id: Some(file_id.to_string()),
            parent_id: None,
            target_path: None,
            metadata_json: Some(
                serde_json::json!({
                    "operation": "pin_tree",
                    "pinned": pinned,
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

    fn enqueue_hydrate_operation(&self, entry: &FileEntry, now: i64) -> anyhow::Result<()> {
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::HydrateFile,
            file_id: Some(entry.file_id.clone()),
            parent_id: None,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(
                serde_json::json!({
                    "operation": "hydrate_file",
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

    fn ensure_shared_parent_allows_write(&self, parent_id: Option<&str>) -> anyhow::Result<Option<FileContractState>> {
        let Some(parent_id) = parent_id else {
            return Ok(None);
        };
        if parent_id == "namespace:shared_with_me" {
            return Err(anyhow::anyhow!(
                "Shared with me is read-only at the namespace root; open an editable shared folder first"
            ));
        }
        let Some(contract) = self.db.get_file_contract_state(parent_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(anyhow::anyhow!(
                "read-only shared folder cannot accept new Finder items"
            ));
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    fn ensure_item_allows_shared_write(
        &self,
        file_id: &str,
        operation: &str,
    ) -> anyhow::Result<Option<FileContractState>> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(None);
        };
        if contract.is_shared() && !contract.can_write() {
            return Err(anyhow::anyhow!(
                "read-only shared item cannot be changed from Finder during {operation}"
            ));
        }
        Ok(Some(contract).filter(|contract| contract.is_shared()))
    }

    /// Download `file_id` from the vault, decrypt, write to `dest_path`.
    /// Called by the Linux FUSE driver, the IPC socket, and conflict-resolution
    /// paths where the decrypted file must land on disk (sync root or a caller-
    /// chosen path). Status transitions: `Downloading` → `Local` on success,
    /// `Error` on failure.
    ///
    /// Steps mirror `repos/cli/src/commands/pull.rs::pull_single_file`:
    ///
    /// 1. Flip status to `Downloading` so the Finder/Explorer overlay
    ///    shows a spinner immediately.
    /// 2. Fetch fresh metadata to learn `chunk_count` (the local
    ///    placeholder may not have it).
    /// 3. Resolve the per-file key. Owned files derive it from the master key;
    ///    shared-with-me files unwrap the sender-provided key envelope using
    ///    the share invite's X25519/HKDF contract.
    /// 4. For each chunk index: GET the bytes, parse as
    ///    `EncryptedBlob` JSON, decrypt with `decrypt_chunk`,
    ///    accumulate plaintext.
    /// 5. Write the whole plaintext to `dest_path` (caller chose the
    ///    layout — `dest_path` is usually `<sync_root>/<decrypted_name>`
    ///    or a per-extension cache path).
    /// 6. Flip status to `Local`.
    ///
    /// On any error the status is flipped to `Error` so the overlay can
    /// render a problem indicator, then the error is bubbled.
    ///
    /// **Windows Cloud Files callers must use [`Self::hydrate_file_to_memory`]
    /// instead** — that variant never writes plaintext to disk, satisfying the
    /// zero-knowledge requirement for on-demand CF hydration.
    pub async fn hydrate_file(&self, file_id: &str, dest_path: &Path, allowed_roots: &[&Path]) -> anyhow::Result<()> {
        self.hydrate_file_with_progress(file_id, dest_path, allowed_roots, None)
            .await
    }

    /// [`Self::hydrate_file`] plus an optional `progress(done_bytes,
    /// total_bytes)` callback, invoked synchronously from the download loop
    /// (task 1670 issue 3 — the macOS IPC handler forwards it to Finder).
    ///
    /// Cancel-safe: if this future is dropped while the download is in
    /// flight (the IPC client hung up), the row's status is restored instead
    /// of being left on `Downloading` forever.
    pub async fn hydrate_file_with_progress(
        &self,
        file_id: &str,
        dest_path: &Path,
        allowed_roots: &[&Path],
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<()> {
        // Task 1247 self-defense: validate `dest_path` against the caller's
        // trusted root(s) BEFORE any directory creation or plaintext write.
        // `dest_path` reaches the one untrusted caller (the IPC socket handler)
        // straight off the wire, and the `file_id`-keyed guard below
        // (`ensure_shared_hydrate_path_safe`) validates an entirely SEPARATE
        // value (`entry.path`), so it can never vouch for `dest_path`. Without
        // this check any local process could turn the daemon into a
        // decrypt-oracle + arbitrary-write primitive (e.g. writing decrypted
        // vault plaintext over ~/.ssh/authorized_keys).
        if !hydrate_dest_is_allowed(dest_path, allowed_roots) {
            return Err(anyhow::anyhow!("hydrate destination is not within an allowed root"));
        }
        self.ensure_shared_hydrate_path_safe(file_id)?;
        // Task 1640: a hydrate into the file's own place in the sync folder (on Windows, only
        // conflict resolution reaches it) never writes over an unsent local change: bytes that differ from the content this
        // device last agreed with the server on, or whose upload is queued, are the person's
        // only copy. It refuses, and the file keeps its bytes. A Finder fetch (macOS) writes
        // to the extension's own staging copy, never there.
        let in_sync_folder = self.dest_is_files_sync_folder_place(file_id, dest_path, allowed_roots)?;
        // Task 1640 (review N1b): the version a legacy download (no `download_version`) records,
        // read BEFORE the download: at most the version the bytes are, so a later change can only
        // be refused as stale, never accepted over a newer one.
        let legacy_version = self.legacy_download_version(file_id)?;
        if in_sync_folder && dest_path.exists() {
            let content = sync_folder_content(&self.db, file_id, dest_path, None)?;
            // Task 2191 (G-engine): a file of unknown content (in the population, conflicts and
            // errors excepted: their resolution is the person's) is never written over while its
            // place holds bytes; they may be the person's only copy of a change.
            // Review-157 F3: Windows only (elsewhere nothing marks a row empty, and a hydrate never
            // writes in place).
            let unknown_with_bytes = cfg!(any(target_os = "windows", test))
                && matches!(content, SyncFolderContent::Unknown)
                && self.db.is_unknown_population(file_id)?
                && place_holds_bytes(dest_path);
            if unknown_with_bytes || matches!(content, SyncFolderContent::Changed { .. } | SyncFolderContent::Queued) {
                tracing::warn!(file_id = %file_id, "hydrate refused: the local file holds a change that is not sent");
                return Err(anyhow::Error::new(UnsentLocalChange));
            }
        }
        // Spec §7.4.2: a hydrate never changes the status of a row with a live upload of its
        // own (a Finder upload that has not parked): no `Downloading`, `Local` or `Error`.
        // That status is the upload's, and `Error` would make the item read-only. Nor
        // of a row whose Finder upload has parked. Its status is the
        // parked one (§9.1), and `Local` would make it writable and evictable.
        let touch_status = !self
            .db
            .item_presentation(file_id)?
            .is_some_and(|presentation| presentation.unparked_finder_upload || presentation.parked_finder_upload);
        // Every status the hydrate writes after its `Downloading` is written only over that
        // `Downloading`: a save accepted meanwhile has made the row `Uploading`, and its status
        // stays (spec §9.2). Before, the hydrate's end wrote over it.
        let set_failed = || {
            if touch_status {
                // Best-effort status flip; if the DB is broken we still
                // return the original error.
                let _ = self
                    .db
                    .set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
            }
        };
        // RAII-style: any early return below the status flip should
        // leave the file in `Error`, not `Downloading`. We do that by
        // wrapping the body in an inner async fn whose Err branch we
        // catch.
        let mut downloading_guard = touch_status.then(|| DownloadingStatusGuard::arm(&self.db, file_id));
        if touch_status {
            self.db.set_status(file_id, FileStatus::Downloading)?;
        }
        // Task 1683 slice 2: the popover's per-file bytes. The caller's own progress
        // callback (the IPC handler forwarding to Finder) still gets every report.
        let transfer = self
            .transfers
            .begin(file_id, crate::transfer_progress::Direction::Down, 0);
        let hydrated = self.do_hydrate_versioned(file_id, progress).await;
        // Past the only cancellation point (the download await): every path
        // below sets the final status itself.
        if let Some(guard) = downloading_guard.as_mut() {
            guard.disarm();
        }
        match hydrated {
            Ok((mut buf, downloaded_version)) => {
                // Write the decrypted bytes to disk (this is the intentional
                // disk-writing path — sync root / conflict resolution / FUSE).
                // Make the destination directory if needed.
                if let Some(parent) = dest_path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    buf.zeroize();
                    // Task 1670 round 2: this used to `?` straight out of the
                    // match arm below, which skipped the `Err(e) =>` arm's
                    // `FileStatus::Error` flip entirely — a directory-create
                    // failure left the row stuck on `Downloading` forever.
                    // Route it through the same status flip as every other
                    // failure in this function.
                    set_failed();
                    return Err(anyhow::anyhow!("create dest dir {}: {e}", parent.display()));
                }
                // Write via an O_NOFOLLOW handle (task 1247): the containment
                // guard at the top of this fn is a check-then-use, and there is
                // a real time window here (`do_hydrate` did a network download +
                // decrypt). A same-UID attacker could plant a symlink at
                // `dest_path` during that window pointing at, say,
                // ~/.ssh/authorized_keys; a plain `fs::write` would follow it
                // and overwrite the real target with decrypted plaintext.
                // `write_hydrated_plaintext` fails closed if the final component
                // is (or race-becomes) a symlink, closing the race atomically at
                // open() time rather than re-checking-then-hoping. Since task
                // 1670 round 2 it also stages the write under a temp name and
                // publishes with one atomic rename, so a failure anywhere in
                // that sequence is guaranteed to leave NOTHING new at
                // `dest_path` — there is no separate "delete the half-written
                // file" step needed here because the primitive itself never
                // makes one externally visible.
                let write_result = write_hydrated_plaintext(dest_path, allowed_roots, &buf);
                let hydrated_hash = in_sync_folder.then(|| sha256_hex(&buf));
                // Zeroize the in-memory copy now that it is on disk (or on
                // error) so the allocation does not linger with plaintext.
                buf.zeroize();
                if let Err(e) = write_result {
                    set_failed();
                    return Err(anyhow::anyhow!("write {}: {e}", dest_path.display()));
                }
                // Task 1640: the bytes just written (one atomic rename: the whole file) are the
                // content this device agrees with the server on, at the version the download
                // metadata named, else (a legacy download, review N1b) the row's version read
                // before the download. Recorded before the row leaves `Downloading`, so no
                // classification sees the new bytes against the old baseline. With no version
                // either, the content is not known (review F1), rather than left naming other bytes.
                if let Some(hash) = hydrated_hash.as_deref() {
                    let recorded = match downloaded_version.or(legacy_version) {
                        Some(version) => self.db.record_local_baseline(file_id, version, hash),
                        None => self.db.mark_content_unknown(file_id),
                    };
                    if let Err(e) = recorded {
                        set_failed();
                        return Err(e.into());
                    }
                }

                // Task 1698: a Trashing row keeps its marker (viewing a file
                // from the macOS Trash view must not un-trash it in the
                // replica); everything else completes as `Local`.
                // PR #100 review (Codex P1): the pre-hydrate status is the
                // row's status BEFORE the flip to `Downloading` — re-reading
                // the DB here always saw `Downloading`, so a Trashing row
                // completed as `Local` and reparented itself out of the trash
                // view. `DownloadingStatusGuard::arm` captured exactly that
                // pre-flip status when it armed above — reuse it.
                self.seam("hydrate:before_final_status");
                if let Some(guard) = &downloading_guard {
                    self.db.set_status_if(
                        file_id,
                        FileStatus::Downloading,
                        hydrate_final_status(guard.restore.clone()),
                    )?;
                }
                let downloaded_bytes = self.transfers.get(file_id).map(|t| t.total).unwrap_or(0);
                transfer.finish();
                self.record_transfer_done(crate::transfer_progress::Direction::Down, file_id, downloaded_bytes);

                // Task 1670 round 4 (Codex P2 on PR #75, review thread on
                // `runner.rs:1044`): see [`record_hydration_cache_state`]'s
                // doc comment for the full "why" — in short, a macOS Finder
                // `fetchContents` hydration writes here as a HANDOFF, not a
                // durable local cache copy, and registering it via
                // `mark_cached` anyway left a phantom cache-usage entry in
                // `desktop_storage_summary` / smart-cache eviction after the
                // system (not us) took over the materialized content.
                self.record_hydrated_destination(file_id, dest_path)?;
                #[cfg(target_os = "linux")]
                self.write_linux_freedesktop_thumbnails(file_id, dest_path).await;
                let _ = self.enforce_configured_cache_limit();
                Ok(())
            }
            Err(e) => {
                let e = self.owned_open_failure(file_id, e).await;
                set_failed(); // only writes over Downloading; the recoverable item already left it.
                Err(e)
            }
        }
    }

    /// Record where a hydrate wrote `file_id`'s plaintext, for the cache accounting and the
    /// sign-out sweep. Both hydrate paths, the server's and the queue's, call it. On macOS a
    /// Finder hand-off into the hydrate cache is not a cached copy ([`record_hydration_cache_state`]).
    fn record_hydrated_destination(&self, file_id: &str, dest_path: &Path) -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        record_hydration_cache_state(
            &self.db,
            file_id,
            dest_path,
            &crate::ipc_socket::macos_hydrate_cache_dir(),
        )?;
        #[cfg(not(target_os = "macos"))]
        {
            let cache_bytes = std::fs::metadata(dest_path).map(|m| m.len() as i64).unwrap_or(0);
            self.db
                .mark_cached(file_id, &dest_path.to_string_lossy(), cache_bytes, now_secs())?;
        }
        Ok(())
    }

    fn ensure_shared_hydrate_path_safe(&self, file_id: &str) -> anyhow::Result<()> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return Ok(());
        };
        if contract.namespace != Namespace::SharedWithMe {
            return Ok(());
        }

        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("shared hydrate target missing state row for {file_id}"))?;
        crate::reject_unsafe_rel_path(&entry.path)
            .map_err(|e| anyhow::anyhow!("unsafe shared hydrate path for {file_id}: {e}"))?;
        Ok(())
    }

    /// Download `file_id` from the vault, decrypt, and return the plaintext
    /// **in memory only** — it is NEVER written to disk.
    ///
    /// This is the Windows Cloud Files hydration path. The Cloud Files runtime
    /// calls `fetch_data_callback` on a filter-driver thread and expects us to
    /// deliver bytes via `CfExecute(TRANSFER_DATA)`; there is no requirement to
    /// materialise the plaintext as a file. Writing to `%TEMP%` would expose
    /// decrypted user data on disk, violating the zero-knowledge contract.
    ///
    /// The returned [`Zeroizing`] wrapper overwrites the buffer with zeros on
    /// drop, so plaintext is wiped from memory as soon as the caller is done
    /// with it — even if an early `return` or `?` is taken. Intermediate
    /// per-chunk decrypted buffers are also explicitly zeroized after being
    /// copied into the accumulator (see [`Self::do_hydrate`]).
    ///
    /// Status transitions: `Downloading` → `Local` on success, `Error` on
    /// failure. The `cache_path` recorded in state_db is `""` (empty) because
    /// the bytes live inside the CF placeholder in the sync root, not at a
    /// separate cache file — `unpinned_local_files_for_dehydration` reconstructs
    /// the real on-disk path from `path` + sync root (see state_db comment).
    #[cfg(target_os = "windows")]
    pub async fn hydrate_file_to_memory(&self, file_id: &str) -> anyhow::Result<FetchedBytes> {
        // Self-review #1: counted from before anything is recorded until the caller's transfer ended.
        let ticket = self.begin_fetch(file_id);
        let legacy_version = self.legacy_download_version(file_id)?;
        self.db.set_status(file_id, FileStatus::Downloading)?;
        match self.do_hydrate_versioned(file_id, None).await {
            Ok((buf, downloaded_version)) => {
                // Task 1640 (review N1, F1): every fetch, of a range or of the whole file, is
                // recorded BEFORE its bytes are transferred: the version it serves and the hash of
                // all of that version's bytes join the file's candidate (`record_fetched_version`),
                // so a transfer that fails or is cut off midway still counts. Only once the whole
                // file is transferred does the caller make it the agreed content
                // ([`Self::record_whole_fetch`]); a range never does: the placeholder's other
                // ranges may hold other versions, or writes that were not sent. The stamp pass
                // promotes a candidate once the whole file is on disk, in sync, with exactly those
                // bytes. A download that names no version leaves the content not known. A failure
                // to record fails the fetch, as every other failure here does.
                let hash = sha256_hex(&buf);
                let fetched = downloaded_version.or(legacy_version).map(|version| FetchedVersion { version, hash });
                let recorded = match &fetched {
                    Some(fetched) => self.db.record_fetched_version(file_id, fetched.version, &fetched.hash),
                    None => self.db.mark_content_unknown(file_id),
                };
                if let Err(e) = recorded {
                    let _ = self.db.set_status(file_id, FileStatus::Error);
                    return Err(e.into());
                }
                self.db.set_status(file_id, FileStatus::Local)?;
                // cache_path is empty: on Windows CF the hydrated bytes live
                // INSIDE the placeholder (the CF runtime writes them there after
                // our CfExecute(TRANSFER_DATA) calls). There is no separate temp
                // file. Byte count is still recorded for smart-cache accounting.
                self.db.mark_cached(file_id, "", buf.len() as i64, now_secs())?;
                let _ = self.enforce_configured_cache_limit();
                Ok(FetchedBytes { bytes: buf, version: fetched, ticket })
            }
            Err(e) => {
                let e = self.owned_open_failure(file_id, e).await;
                let _ = self.db.set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
                Err(e)
            }
        }
    }

    /// Task 1640 (review F1, design T5): a fetch transferred the whole file (`fetched`, from
    /// [`Self::hydrate_file_to_memory`]): every byte on disk is that version's now, so they are the
    /// content this device agrees with the server on, and nothing else fetched is left. Only when no
    /// other fetch of the file overlapped this one (`ticket`, self-review #1): another fetch's bytes,
    /// of another version, could land around this transfer. Then the candidate this fetch merged
    /// stays (lower or mixed). `Ok(true)`: recorded.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub fn record_whole_fetch(&self, file_id: &str, fetched: &FetchedVersion, ticket: &FetchTicket) -> anyhow::Result<bool> {
        // Review-151d P3-4: the check and the write under the tracker's lock, so no fetch can begin
        // (and record) between them. Lock order: the tracker, then the database; nothing takes the
        // tracker while holding the database.
        let fetches = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !ticket.alone_in(&fetches) {
            tracing::debug!(file_id = %file_id, "whole fetch overlapped by another fetch of the file; its candidate stays");
            return Ok(false);
        }
        self.db.record_local_baseline(file_id, fetched.version, &fetched.hash)?;
        drop(fetches);
        Ok(true)
    }

    /// Task 1640 (self-review #1): a fetch of `file_id` starts ([`FetchTicket`]).
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    fn begin_fetch(&self, file_id: &str) -> FetchTicket {
        let mut fetches = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = fetches.entry(file_id.to_string()).or_default();
        activity.in_flight += 1;
        activity.events += 1;
        let alone_since = (activity.in_flight == 1).then_some(activity.events);
        FetchTicket {
            file_id: file_id.to_string(),
            alone_since,
        }
    }

    /// Core download + decrypt loop. Returns the full plaintext in a
    /// [`Zeroizing`] wrapper so the allocation is wiped on drop regardless of
    /// which exit path is taken (normal return, early `?`, or a panic unwind).
    ///
    /// Intermediate per-chunk buffers from core are
    /// explicitly zeroized after being copied into the accumulator, so at most
    /// one extra chunk's worth of plaintext is live in memory at any time.
    ///
    /// This function deliberately does NOT write anything to disk. Callers that
    /// need the bytes on disk (`hydrate_file`) or in memory (`hydrate_file_to_memory`)
    /// handle that themselves so the disk-write decision stays at the call site,
    /// not buried inside the crypto loop.
    pub async fn encrypt_owned_name(&self, file_id: &str, name: &str, pin: beebeeb_core::chunk_stream::ExpectedChunkFormat) -> anyhow::Result<String> {
        let key = self.resolved_owned_key(file_id).await?;
        Ok(beebeeb_core::encrypt::encrypt_name_for(&key, name, None, pin)?)
    }

    async fn owned_open_failure(&self, file_id: &str, error: anyhow::Error) -> anyhow::Error {
        if let Ok(Some(record)) = self.db.get_owned_key_record(file_id)
            && record.foreign_added {
            let _ = self.db.set_status_if(file_id, FileStatus::Downloading, FileStatus::CloudOnly);
            let label = if let (Some(root), Some(encrypted)) = (record.shared_root_id.as_deref(), record.origin_label_encrypted.as_deref()) {
                // Display only, read-only custody lookup. Never re-seed custody during an open failure.
                let scope = self.owner_key_scope();
                let custody = self.api.get_shared_folder(root).await.ok();
                if let Err(error) = scope.check() { return error; }
                let key = custody.and_then(|row| crate::owner_keys::normalize_wrap(&row["owner_wrapped_folder_key"]).ok().flatten())
                    .and_then(|wrap| beebeeb_sync::key_resolution::unwrap_folder_custody(
                        &beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key()), root, &wrap).ok());
                if let Some(key) = key {
                    if let Err(error) = scope.remember_custody(root) { return error; }
                    beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, encrypted).ok().map(|metadata| metadata.name)
                } else { None }
            } else { None };
            return anyhow::anyhow!(crate::owner_keys::cannot_open_message(label.as_deref(), record.member_added));
        }
        error
    }

    async fn file_key_for_download(&self, file_id: &str) -> anyhow::Result<beebeeb_core::kdf::FileKey> {
        let Some(contract) = self.db.get_file_contract_state(file_id)? else {
            return self.resolved_owned_key(file_id).await.map(|key| beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()));
        };
        if contract.namespace != Namespace::SharedWithMe {
            return self.resolved_owned_key(file_id).await.map(|key| beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()));
        }
        if !contract.can_read() {
            anyhow::bail!("shared item is not readable");
        }

        let share_id = contract
            .share_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared item is missing invite id"))?;
        let body = self.api.list_shared_roots().await?;
        let invite = body
            .get("invites")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .find(|invite| shared_invite_id_matches(invite, share_id))
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} was not returned by the server"))?;
        let mapping = shared_root_from_invite(invite)
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is not approved or is malformed"))?;
        let sender_public_key = mapping
            .sender_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("shared invite {share_id} is missing sender_public_key"))?;

        if mapping.is_folder {
            let (folder_key, folder_keys) = self.shared_folder_material(&mapping).await?;
            let wrap = encrypted_file_key_from_folder_keys_response(&folder_keys, file_id)
                .ok_or_else(|| anyhow::anyhow!("folder share is missing the child key"))?;
            self.shared_folder_child_key(&folder_key, &mapping.file_id, file_id, 1, wrap,
                folder_keys["desktop_has_custody"].as_bool() == Some(true))
        } else {
            let encrypted_file_key = mapping
                .encrypted_file_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("share invite {share_id} is missing encrypted_file_key"))?;
            unwrap_direct_shared_file_key(self.api.master_key(), sender_public_key, file_id, encrypted_file_key)
        }
    }

    async fn do_hydrate(
        &self,
        file_id: &str,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        self.do_hydrate_versioned(file_id, progress).await.map(|(plaintext, _)| plaintext)
    }

    /// [`Self::do_hydrate`], also saying which server version the bytes are: the download
    /// metadata's `download_version.version_number` (task 1640), `None` for a legacy file whose
    /// metadata names no version.
    async fn do_hydrate_versioned(
        &self,
        file_id: &str,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<(Zeroizing<Vec<u8>>, Option<i64>)> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Per-file metadata. We trust the server's chunk_count rather
        // than the local entry's because a file we know about as
        // `cloud_only` may have been re-uploaded with a new chunk
        // layout since we last saw it.
        let meta = self.api.get_file(file_id).await?;
        self.with_one_restart(file_id, meta, |meta| async move {
            let version = meta["download_version"]["version_number"].as_i64();
            self.do_hydrate_with_meta(file_id, &meta, progress)
                .await
                .map(|plaintext| (plaintext, version))
        })
        .await
    }

    /// Tasks 1643 + 2173: run one download `attempt` on `meta`, and when the version it was bound to went away
    /// ([`crate::download_pin::VersionGone`]) run it ONCE more on fresh metadata; nothing of the first attempt is kept,
    /// and versions are never mixed. The restart keeps the binding: when the first attempt was pinned and the fresh
    /// metadata names no version (`download_version: null`), it is [`crate::download_pin::PinLost`], never an unpinned
    /// download. The one copy of this rule (whole-file hydrate, range read, conflict preview).
    async fn with_one_restart<T, F, Fut>(&self, file_id: &str, meta: serde_json::Value, attempt: F) -> anyhow::Result<T>
    where
        F: Fn(serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let first_was_pinned = crate::download_pin::PinnedVersion::from_metadata(&meta)?.is_some();
        match attempt(meta).await {
            Err(e) if e.downcast_ref::<crate::download_pin::VersionGone>().is_some() => {
                tracing::debug!("download: the file changed while it was downloading; restarting once");
                let fresh = self.api.get_file(file_id).await?;
                if first_was_pinned && crate::download_pin::PinnedVersion::from_metadata(&fresh)?.is_none() {
                    return Err(anyhow::Error::new(crate::download_pin::PinLost));
                }
                attempt(fresh).await
            }
            other => other,
        }
    }

    /// Shared core of [`Self::do_hydrate`] and [`Self::remote_content_preview`]
    /// (task 1546 Codex round 2, finding 1): downloads + decrypts every chunk
    /// for `file_id` given ALREADY-FETCHED metadata. `remote_content_preview`
    /// needs the metadata anyway to check the remote size before deciding
    /// whether to download at all — routing through this shared helper
    /// instead of `do_hydrate` means that check doesn't cost a second
    /// `GET /files/{id}` round trip for files it does end up downloading.
    ///
    /// Internal helper only — callers are responsible for their own
    /// `file_id` UUID validation (both current callers already do theirs
    /// before this is reached).
    async fn do_hydrate_with_meta(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        self.hydrate_whole_file(file_id, meta, progress)
            .await
            .map(|(plaintext, _)| plaintext)
    }

    /// [`Self::do_hydrate_with_meta`], also saying which transport carried the file. Task 2173:
    /// straight from object storage when the server offers it and the handshake succeeds
    /// ([`crate::direct_download`]); `UseApi` means nothing of that attempt was kept, and the file
    /// goes through the API's per-chunk route below, as it always did.
    ///
    /// Task 1643: when the metadata names a `download_version`, both paths are bound to it: every
    /// API chunk request carries `?object_version_id=`, the direct path must land on the same
    /// version, each chunk's wire size and the final plaintext size are checked against its
    /// manifest, and a version that goes away is [`crate::download_pin::VersionGone`] (the caller
    /// starts again from fresh metadata). Without one (a legacy file), today's unpinned download.
    async fn hydrate_whole_file(
        &self,
        file_id: &str,
        meta: &serde_json::Value,
        progress: Option<&HydrateProgressFn>,
    ) -> anyhow::Result<(Zeroizing<Vec<u8>>, crate::direct_download::FileTransport)> {
        let pin = crate::download_pin::PinnedVersion::from_metadata(meta)?;
        let chunk_count = match &pin {
            Some(pin) => pin.chunk_count,
            None => meta
                .get("chunk_count")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("server response missing chunk_count"))? as u32,
        };

        let file_key = self.file_key_for_download(file_id).await?;
        let expected_format = recorded_content_format(&file_key, meta)?;

        // Rate-limit ceiling for downloads, and the transport (task 2173).
        let config = crate::config::DesktopConfig::load().ok();
        let download_kbps_limit = config.as_ref().map(|c| c.download_kbps_limit).unwrap_or(0);
        let transport = self.download_transport(config.as_ref().and_then(|c| c.transport.as_deref()));

        // Walk chunks. Pre-allocate roughly the file size if known,
        // but fall back to defaults — chunks are encrypted so the
        // ciphertext is always larger than plaintext anyway.
        // Use Zeroizing so that if we bail mid-loop (network error,
        // decrypt error) the partial plaintext is still wiped.
        let approx_size = match &pin {
            Some(pin) => usize::try_from(pin.plaintext_size_bytes).unwrap_or(0),
            None => meta.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        };

        let report = HydrateReport {
            bridge: self,
            file_id,
            total: approx_size as u64,
            progress,
        };
        let direct = crate::direct_download::download(
            &self.api,
            crate::direct_download::Request {
                file_id,
                file_key: &file_key,
                chunk_count,
                expected_format,
                download_kbps_limit,
                approx_size,
                pin: pin.as_ref(),
            },
            transport,
            self.storage_override(),
            &report,
        )
        .await
        .map_err(|e| match e {
            crate::direct_download::DirectError::VersionChanged => anyhow::Error::new(crate::download_pin::VersionGone),
            other => anyhow::Error::new(other),
        })?;
        let (plaintext, transport) = match direct {
            crate::direct_download::Outcome::Done { plaintext, transport } => (plaintext, transport),
            crate::direct_download::Outcome::UseApi => (
                self.download_and_decrypt_chunk_range(
                    file_id,
                    &file_key,
                    0..chunk_count,
                    download_kbps_limit,
                    approx_size,
                    progress,
                    pin.as_ref(),
                    expected_format,
                    chunk_count,
                )
                .await?,
                crate::direct_download::FileTransport::Api,
            ),
        };
        // Task 1643: the whole file is exactly the pinned version's plaintext, or nothing.
        if let Some(pin) = &pin
            && plaintext.len() as u64 != pin.plaintext_size_bytes
        {
            anyhow::bail!(
                "the download is {} bytes, the pinned version's manifest says {}",
                plaintext.len(),
                pin.plaintext_size_bytes
            );
        }
        tracing::debug!(transport = transport.as_str(), "download finished");
        Ok((plaintext, transport))
    }

    /// The download transport: `BB_TRANSPORT`, else `desktop.toml`'s `transport`, else `auto`
    /// (task 2173). A test bridge takes the one its test set, else `auto`.
    fn download_transport(&self, configured: Option<&str>) -> crate::direct_download::Transport {
        #[cfg(test)]
        {
            let _ = configured;
            self.seams
                .transport
                .lock()
                .unwrap()
                .unwrap_or(crate::direct_download::Transport::Auto)
        }
        #[cfg(not(test))]
        crate::direct_download::Transport::selected(configured)
    }

    /// A test's storage stand-in (task 2173); production always fetches.
    fn storage_override(&self) -> Option<crate::direct_download::Storage<'static>> {
        #[cfg(test)]
        if let Some(status) = *self.seams.storage_fail.lock().unwrap() {
            return Some(crate::direct_download::Storage::Fail(status));
        }
        None
    }

    /// Download + decrypt chunk indices `[range.start, range.end)` for
    /// `file_id`, concatenating the plaintext into one [`Zeroizing`] buffer.
    /// Shared by [`Self::do_hydrate`] (the whole-file range `0..chunk_count`)
    /// and [`Self::hydrate_file_range`] (a covering sub-range) so the
    /// pacing/wire-counting/zeroize discipline lives in exactly one place.
    ///
    /// `approx_size_hint` only sizes the initial allocation (`Vec::with_capacity`)
    /// — it never bounds or truncates the actual read, so a wrong hint costs at
    /// most a reallocation, never a silent short read.
    ///
    /// Task 1643: with a `pin`, every chunk request carries the pinned version's id and every chunk's
    /// wire size must be the manifest's; a 409 comes back as [`crate::download_pin::VersionGone`].
    /// Without one (a legacy file) the requests are today's unpinned ones.
    async fn download_content_frame(&self, file_id: &str, i: u32,
        pin: Option<&crate::download_pin::PinnedVersion>) -> anyhow::Result<Vec<u8>> {
        Ok(match pin {
                // Task 1643: the pinned route (shared with the direct path's fallback): exactly the manifest's
                // length, a transient failure tried again, a 409 = the version is gone, no URL in an error.
                Some(pin) => {
                    let expected = pin
                        .wire_size(i)
                        .ok_or_else(|| anyhow::anyhow!("chunk {i} is outside the pinned version"))?;
                    self.api
                        .download_chunk_pinned(file_id, i, &pin.object_version_id, expected)
                        .await
                        .map_err(|e| match e {
                            crate::api_client::ChunkApiError::Restart => {
                                anyhow::Error::new(crate::download_pin::VersionGone)
                            }
                            crate::api_client::ChunkApiError::Terminal(status) => {
                                anyhow::Error::new(crate::direct_download::DirectError::Refused(status))
                            }
                            crate::api_client::ChunkApiError::Failed(message) => anyhow::anyhow!(message),
                        })?
                }
                None => self.api.download_chunk(file_id, i).await?,
        })

    }

    #[allow(clippy::too_many_arguments)]
    async fn download_and_decrypt_chunk_range(
        &self,
        file_id: &str,
        file_key: &beebeeb_core::kdf::FileKey,
        range: std::ops::Range<u32>,
        download_kbps_limit: u64,
        approx_size_hint: usize,
        progress: Option<&HydrateProgressFn>,
        pin: Option<&crate::download_pin::PinnedVersion>,
        expected: beebeeb_core::chunk_stream::ExpectedChunkFormat,
        chunk_count: u32,
    ) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let mut acc: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(approx_size_hint));
        // Task 1670 issue 3: tell the caller (the macOS IPC handler, which
        // forwards it to Finder's progress bar) how far along we are. `total`
        // is the size hint (server's plaintext size); the initial 0/total
        // report lets the UI switch from indeterminate to determinate before
        // the first chunk lands.
        let total = approx_size_hint as u64;
        // Task 1683 slice 2: the board first, so a caller that reads it from inside its
        // own callback sees the same bytes it was just told about. A no-op for a file
        // that is not registered (a partial range read).
        self.transfers.report(file_id, 0, total);
        if let Some(report) = progress {
            report(0, total);
        }

        // Public ChunkDecryptor delegates to core's StreamOpener; desktop owns no crypto.
        let whole = range.start == 0 && range.end == chunk_count;
        let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(
            beebeeb_core::kdf::FileKey::from_bytes(*file_key.as_bytes()), file_id, expected);
        let context = if whole { None } else {
            let first = self.download_content_frame(file_id, 0, pin).await?;
            let mut opened = reader.push_frame(&content_frame(&first)?)?;
            opened.data.zeroize();
            Some((reader.format().ok_or_else(|| anyhow::anyhow!("missing chunk format"))?, reader.version_id()))
        };
        for i in range {
            let chunk_start = std::time::Instant::now();
            let chunk_bytes = self.download_content_frame(file_id, i, pin).await?;
            let wire_len = chunk_bytes.len() as u64;
            // Decrypt into a temporary buffer, copy into the accumulator,
            // then zeroize the temporary. This limits live plaintext to
            // the accumulator + one chunk at any moment.
            let frame = content_frame(&chunk_bytes)?;
            let mut decrypted = if let Some((format, version)) = context {
                beebeeb_core::chunk_stream::open_chunk(file_key, file_id,
                    version.as_ref().map(|v| v.as_slice()).unwrap_or(&[]), i,
                    i + 1 == chunk_count, format, &frame)?
            } else { reader.push_frame(&frame)?.data };
            acc.extend_from_slice(&decrypted);
            decrypted.zeroize();
            self.transfers.report(file_id, acc.len() as u64, total);
            if let Some(report) = progress {
                report(acc.len() as u64, total);
            }

            // P1 — wire-byte counter: count raw wire bytes received.
            self.wire.download_bytes.fetch_add(wire_len, Ordering::Relaxed);

            // E — token-bucket pacing for downloads.
            if download_kbps_limit > 0 {
                let budget_secs = wire_len as f64 / (download_kbps_limit as f64 * 1024.0);
                let elapsed_secs = chunk_start.elapsed().as_secs_f64();
                if budget_secs > elapsed_secs {
                    let sleep_ms = ((budget_secs - elapsed_secs) * 1000.0) as u64;
                    if sleep_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    }
                }
            }
        }

        if whole { reader.finish()?; }
        Ok(acc)
    }

    /// Download `file_id` from the vault, decrypt, and return ONLY the plaintext
    /// bytes covering `[required_offset, required_offset + required_length)` —
    /// **in memory only**, never written to disk. This is the range-targeted
    /// counterpart to [`Self::hydrate_file_to_memory`] (task 1024, follow-up to
    /// 0769): instead of decrypting the whole file on every CF fetch callback, it
    /// downloads + decrypts only the chunks that cover the requested range, so
    /// peak memory is bounded by one covering-range buffer (typically a handful
    /// of chunks) rather than the entire file.
    ///
    /// Requires an authoritative, uniform `chunk_size_bytes` for the file. This
    /// value must come from the server's stored `object_versions` row; it cannot
    /// be recovered from `size_bytes / chunk_count`, because real files use a
    /// fixed chunk size with only the final chunk shortened.
    ///
    /// Returns `Ok(None)` (never an error) when the covering-chunk math can't be
    /// established from the metadata (`chunk_count` is `0`, `size_bytes` is
    /// missing, or `chunk_size_bytes` is missing/invalid) — the caller falls
    /// back to
    /// [`Self::hydrate_file_to_memory`] for the whole-file path in that case.
    ///
    /// Status bookkeeping: unlike [`Self::hydrate_file_to_memory`], this method
    /// does **not** flip the row to `FileStatus::Local` — a single range fetch is
    /// not evidence the whole file is now cached, so marking it fully `Local`
    /// here would be a stronger claim than the bytes we actually have. See the
    /// caller (`windows_cf::callbacks::fetch_data_callback`) for how "fully
    /// in-sync" is decided instead.
    #[cfg(any(target_os = "windows", test))]
    pub async fn hydrate_file_range(
        &self,
        file_id: &str,
        required_offset: u64,
        required_length: u64,
    ) -> anyhow::Result<Option<HydratedRange>> {
        let _file_uuid: uuid::Uuid = file_id
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid file_id (not a UUID): {e}"))?;

        // Task 1643: the range is planned on, and read from, one pinned version; a version that goes away
        // mid-read starts the read again, once, from fresh metadata (`with_one_restart`).
        let meta = self.api.get_file(file_id).await?;
        let read = self
            .with_one_restart(file_id, meta, |meta| async move {
                self.range_attempt(file_id, required_offset, required_length, &meta).await
            })
            .await;
        let read = match read { Ok(value) => Ok(value), Err(e) => Err(self.owned_open_failure(file_id, e).await) };
        if read.is_err() {
            // Every failing exit (a second pass's own error too) ends with the row in Error, never left on
            // Downloading, and the placeholder never in sync.
            let _ = self.db.set_status_if(file_id, FileStatus::Downloading, FileStatus::Error);
        }
        read
    }

    /// One pass of [`Self::hydrate_file_range`] on `meta`.
    #[cfg(any(target_os = "windows", test))]
    async fn range_attempt(
        &self,
        file_id: &str,
        required_offset: u64,
        required_length: u64,
        meta: &serde_json::Value,
    ) -> anyhow::Result<Option<HydratedRange>> {
        let pin = crate::download_pin::PinnedVersion::from_metadata(meta)?;
        let (chunk_count, size_bytes, chunk_size_bytes) = match &pin {
            Some(pin) => (
                u64::from(pin.chunk_count),
                Some(pin.plaintext_size_bytes),
                Some(pin.chunk_size_bytes),
            ),
            None => (
                meta.get("chunk_count").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
                meta.get("size_bytes").and_then(|v| v.as_u64()),
                meta.get("chunk_size_bytes").and_then(|v| v.as_u64()),
            ),
        };

        let (Some(size_bytes), Some(chunk_size_bytes), true) = (size_bytes, chunk_size_bytes, chunk_count > 0)
        else {
            // Missing metadata — caller falls back to the whole-file path.
            return Ok(None);
        };
        if chunk_size_bytes == 0 {
            return Ok(None);
        }

        // Clamp the requested range to the file's real extent before computing
        // covering chunks, so a stale/over-wide CF request never derives an
        // out-of-bounds chunk index.
        let range_end = required_offset.saturating_add(required_length).min(size_bytes);
        if range_end <= required_offset {
            // Degenerate/empty range — nothing to hydrate.
            return Ok(Some(HydratedRange {
                data: Zeroizing::new(Vec::new()),
                range_start_bytes: required_offset,
                covers_whole_file: size_bytes == 0,
            }));
        }

        let Some(plan) = plan_hydration_chunk_range(
            size_bytes,
            chunk_count,
            chunk_size_bytes,
            required_offset,
            required_length,
        )?
        else {
            return Ok(None);
        };

        self.db.set_status(file_id, FileStatus::Downloading)?;

        let file_key = self.file_key_for_download(file_id).await?;
        let expected_format = recorded_content_format(&file_key, meta)?;
        let download_kbps_limit = crate::config::DesktopConfig::load()
            .map(|c| c.download_kbps_limit)
            .unwrap_or(0);

        let chunks = plan.first_chunk..plan.last_chunk_exclusive;
        let read = self
            .download_and_decrypt_chunk_range(
                file_id,
                &file_key,
                chunks.clone(),
                download_kbps_limit,
                plan.covering_span_hint,
                None,
                pin.as_ref(),
                expected_format,
                chunk_count as u32,
            )
            .await
            .and_then(|data| {
                // Task 1643: exactly the pinned version's plaintext for those chunks.
                match pin.as_ref().map(|pin| pin.plaintext_size_of(chunks.clone())) {
                    Some(want) if want != Some(data.len() as u64) => anyhow::bail!(
                        "the range read is {} bytes, the pinned version's manifest says {want:?}",
                        data.len()
                    ),
                    _ => Ok(data),
                }
            });
        let data = read?;
        // Only a whole-file covering range is evidence the file is fully
        // cached; a partial range leaves the row `Downloading` for the
        // caller (the CF callback) to resolve based on what CF itself
        // reports (see `windows_cf::callbacks`).
        if plan.covers_whole_file {
            self.db.set_status(file_id, FileStatus::Local)?;
            self.db.mark_cached(file_id, "", data.len() as i64, now_secs())?;
            let _ = self.enforce_configured_cache_limit();
        }
        Ok(Some(HydratedRange {
            data,
            range_start_bytes: plan.range_start_bytes,
            covers_whole_file: plan.covers_whole_file,
        }))
    }

    /// Fetch the encrypted server thumbnail for `file_id`, decrypt it **in
    /// memory only**, and return the decoded image bytes (WebP / JPEG / PNG)
    /// inside a [`Zeroizing`] wrapper.
    ///
    /// This is the shared read path for OS thumbnail consumers. It mirrors
    /// [`Self::do_hydrate`]'s per-file-key derivation but hits
    /// `GET /files/:id/thumbnail/{variant}` instead of the chunk range and reuses
    /// the **existing** encrypted thumbnail the original uploading client
    /// generated (a downscaled still for images, a poster frame for video). OS
    /// integrations never regenerate a thumbnail and never decode the original
    /// media just to preview it — they decrypt this small blob.
    ///
    /// Core's purpose-specific thumbnail reader handles legacy raw-key blobs and
    /// subkey thumbnails. File content always uses the separate stream reader.
    ///
    /// ## Zero-knowledge
    ///
    /// The decrypted bytes live ONLY in the returned `Zeroizing<Vec<u8>>`, which
    /// is wiped on drop (normal return, `?`, or panic unwind). Callers decide how
    /// to consume the decoded image bytes: Windows decodes straight to an
    /// in-memory `HBITMAP`; Linux writes the freedesktop.org cache PNG derivative
    /// required by file managers.
    ///
    /// `variant` is `small` / `medium` / `large`. Any error (no thumbnail for
    /// this file, network failure, decrypt failure) bubbles as `Err` so the
    /// caller can fall back to the file-type icon — it is never reported as a
    /// success with empty bytes.
    pub async fn fetch_thumbnail_to_memory(&self, file_id: &str, variant: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let _: uuid::Uuid = file_id.parse()?;
        let blob = self.api.download_thumbnail(file_id, variant).await?;
        let key = self.file_key_for_download(file_id).await?;
        Ok(Zeroizing::new(beebeeb_core::encrypt::decrypt_thumbnail(&key, &blob)?))
    }

    /// Apply a "Keep Mine" resolution: stage the current local bytes and upload
    /// them as a new server version using the same chunked upload path as normal
    /// Finder writes. On success, [`Self::upload_version`] applies the regular
    /// upload completion bookkeeping (status, version, object version, size, and
    /// remote timestamp). On failure, the row is left conflicted and the error is
    /// returned to the caller so the UI does not report a false resolution.
    ///
    /// `sync_root` is supplied by the caller because the bridge itself doesn't
    /// know it (the runner owns that).
    pub async fn resolve_keep_mine(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        if entry.is_dir() {
            return Err(anyhow::anyhow!("Keep Mine upload requires a file row: {file_id}"));
        }

        let shared_contract = self.ensure_item_allows_shared_write(file_id, "keep mine")?;
        let contract = self.db.get_file_contract_state(file_id)?;
        let parent_id = contract.as_ref().and_then(|contract| contract.parent_id.clone());
        let file_name = display_name_for_path(&entry.path);
        let content_type = contract
            .as_ref()
            .and_then(|contract| contract.content_type.clone())
            .or_else(|| beebeeb_core::media::guess_mime_type(&file_name).map(str::to_string));
        let name_encrypted =
            owned_metadata_for_name(&self.db, self.api.master_key(), file_id, &file_name, content_type.as_deref())?;

        let local_path = local_file_path_under_sync_root(sync_root, &entry.path)?;
        let staged =
            crate::staged_payload::StagedPayload::copy(self.db.clone(), &local_path, self.finder_staging_root()?)?;
        let staged_path = staged.path().to_string();
        let staged_size = std::fs::metadata(&staged_path).map(|m| m.len()).unwrap_or(0);

        let mut metadata = serde_json::json!({
            "operation": "upload_version",
            "name_encrypted": name_encrypted,
            "content_type": content_type,
            "size_bytes": staged_size,
            "uploaded_by": "authenticated_desktop_user",
        });
        apply_shared_context(&mut metadata, shared_contract.as_ref());

        let now = now_secs();
        let op = PendingOperation {
            op_id: uuid::Uuid::new_v4().to_string(),
            kind: OperationKind::UploadVersion,
            file_id: Some(file_id.to_string()),
            parent_id,
            target_path: Some(entry.path.clone()),
            metadata_json: Some(serde_json::to_string(&metadata)?),
            payload_path: Some(staged_path.clone()),
            // Keep Mine is an explicit conflict override. Passing the stale base
            // that caused the conflict would make the server reject the upload.
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 1,
            next_retry_at: now,
            last_error: None,
            backup_source_key: None,
            created_at: now,
            updated_at: now,
        };

        let mut post_complete_errors = Vec::new();
        let released = match self
            .upload_version(&op, None, sync_root, &mut post_complete_errors)
            .await
        {
            Ok(OpDone::Remove { release } | OpDone::Removed { release }) => release,
            Err(e) => {
                // One-shot op (never queued): nothing will resume its session.
                self.abandon_upload_after_give_up(&op).await;
                return Err(anyhow::anyhow!("Keep Mine upload failed: {e}"));
            }
        };
        // macOS: the upload leaves its staged copy for the caller to release. This op
        // was never queued, so there is no removal to wait for.
        if let Some(path) = released.as_deref() {
            crate::staged_payload::remove_or_log(&self.db, Path::new(path));
        }
        // Task 1700: thumbnail failures never fail the resolution, but they
        // must not vanish silently either.
        for error in &post_complete_errors {
            tracing::warn!(file_id = %file_id, error = %error, "conflict keep-mine upload: post-complete thumbnail work skipped");
        }

        tracing::info!(file_id = %file_id, "conflict resolved: keep mine uploaded local version");
        Ok(())
    }

    /// Apply a "Keep Theirs" resolution: download the remote version
    /// and overwrite the local file at `<sync_root>/<entry.path>`.
    /// Status ends in `Local`; `remote_updated_at` is anchored to
    /// "now" so the next tick treats the file as freshly synced.
    ///
    /// Reuses [`Self::hydrate_file`], which already handles status
    /// transitions and Error-on-failure rollback. The extra
    /// `remote_updated_at` bump after a successful hydrate prevents
    /// the next tick from re-flagging a conflict if the user's old
    /// `content_hash` is still on the row (it isn't anymore — the row
    /// was already in Conflict — but we belt-and-brace anchor anyway).
    pub async fn resolve_keep_theirs(&self, file_id: &str, sync_root: &Path) -> anyhow::Result<()> {
        let mut entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let dest = local_file_path_under_sync_root(sync_root, &entry.path)?;
        // hydrate_file flips Conflict → Downloading → Local on success
        // (or Error on failure). We don't care about the intermediate
        // state for this path.
        self.hydrate_file(file_id, &dest, &[sync_root]).await?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        entry.status = FileStatus::Local;
        entry.remote_updated_at = now_secs;
        entry.modified_at = now_secs;
        self.db.upsert_file(&entry)?;
        tracing::info!(file_id = %file_id, dest = %dest.display(), "conflict resolved: keep theirs");
        Ok(())
    }

    /// Apply a "Keep Both" resolution: rename the local copy to a
    /// device-suffixed conflict filename, hydrate the remote into the
    /// original path, flip status back to `Local`. Called by Task 13's
    /// auto-resolution timer in [`crate::runner`] and (eventually) by
    /// the user-driven `resolve_conflict` IPC when the user picks
    /// "Keep Both" from the conflict window.
    ///
    /// Filesystem ops are done in this order so a partial failure
    /// leaves recoverable state:
    ///
    ///   1. Rename local → conflict copy (cheap, instantaneous)
    ///   2. Hydrate remote into the original path (network — may
    ///      fail; if it does, the user still has both their original
    ///      *and* the conflict copy on disk, so no data is lost; we
    ///      flip status to `Error` so the next tick retries).
    ///
    /// `sync_root` is supplied by the caller because the bridge
    /// itself doesn't know it (the runner owns that).
    pub async fn auto_resolve_keep_both(&self, sync_root: &Path, entry: &FileEntry) -> anyhow::Result<String> {
        let original = local_file_path_under_sync_root(sync_root, &entry.path)?;

        // If the local file no longer exists on disk (user deleted it
        // outside the daemon), Keep Both collapses to Keep Remote.
        if !original.exists() {
            self.hydrate_file(&entry.file_id, &original, &[sync_root]).await?;
            self.db.set_status(&entry.file_id, FileStatus::Local)?;
            return Ok(entry.path.clone());
        }

        let host = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "device".into());
        let date = chrono::Utc::now().format("%Y-%m-%d");
        let stem = original.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let conflict_name = format!("{stem} (conflict - {host} - {date}){ext}");
        let conflict_path = original.with_file_name(&conflict_name);

        std::fs::rename(&original, &conflict_path)
            .map_err(|e| anyhow::anyhow!("rename {} -> {}: {e}", original.display(), conflict_path.display()))?;

        // Hydrate remote into the now-vacant original path. On failure
        // we mark Error rather than Conflict — the conflict copy is
        // safe on disk, the remote download just needs a retry.
        if let Err(e) = self.hydrate_file(&entry.file_id, &original, &[sync_root]).await {
            self.db.set_status(&entry.file_id, FileStatus::Error)?;
            return Err(e);
        }
        self.db.set_status(&entry.file_id, FileStatus::Local)?;
        Ok(conflict_name)
    }

    /// Read-only content preview for the conflict-resolution window (task
    /// 1546 finding 2): reads the LOCAL file straight off disk and
    /// downloads+decrypts the CURRENT REMOTE version, without touching
    /// state.db — unlike [`Self::hydrate_file`] / [`Self::hydrate_file_to_memory`],
    /// which both flip the row's status to `Downloading`/`Local` and record a
    /// cache entry. The row is mid-conflict and neither side has been chosen
    /// yet, so daemon bookkeeping must not move just because the user opened
    /// the window — that's why this calls the private `do_hydrate` directly
    /// instead of either public hydrate wrapper.
    ///
    /// Replaces `ConflictWindow.tsx`'s previous hardcoded placeholder text
    /// ("Content from this device…" / "Content from other device…"), which
    /// its own doc-comment admitted was fake for every conflict.
    ///
    /// Task 1546 Codex round 2, finding 3: textness is decided HERE, from
    /// the file's own path via [`is_text_file`], never taken from a
    /// caller-supplied flag — the VersionCenter-initiated open always passed
    /// a hardcoded `isText: false` (only the daemon's auto-open path derived
    /// it correctly from the filename), which made every manually-reviewed
    /// text conflict render as binary and silently discard both text bodies.
    pub async fn conflict_content_preview(
        &self,
        file_id: &str,
        sync_root: &Path,
    ) -> anyhow::Result<ConflictContentPreview> {
        let entry = self
            .db
            .get_file(file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let is_text = is_text_file(&entry.path);

        let local = self.local_content_preview(sync_root, &entry.path, is_text);
        let (remote, _) = self.remote_content_preview(file_id, is_text).await;

        Ok(ConflictContentPreview {
            is_text,
            local,
            remote,
            version: None,
        })
    }

    /// Task 2190 (plan §3.4): both sides of a parked conflict. The Mac's side is the parked write's
    /// staged copy, read with the same bounds as [`Self::local_content_preview`]; the other side is
    /// the server's current version. Read-only.
    pub async fn parked_conflict_preview(&self, op_id: &str) -> anyhow::Result<ConflictContentPreview> {
        let op = self
            .db
            .get_operation(op_id)?
            .ok_or_else(|| anyhow::anyhow!("no queued write {op_id}"))?;
        let file_id = op.file_id.clone().ok_or_else(|| anyhow::anyhow!("the write names no file"))?;
        let entry = self
            .db
            .get_file(&file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let is_text = is_text_file(&entry.path);
        let payload = PathBuf::from(op.payload_path.ok_or_else(|| anyhow::anyhow!("the write has no staged copy"))?);
        let local = match (payload.parent(), payload.file_name().and_then(|name| name.to_str())) {
            (Some(dir), Some(name)) => self.local_content_preview(dir, name, is_text),
            _ => ConflictContentSide {
                size_bytes: None,
                text: None,
                unavailable_reason: Some("Couldn't locate this Mac's copy.".to_string()),
            },
        };
        // Review-169 #1 and F1: the version the person decides against is the version of the bytes
        // shown, from the metadata the download ran on (after a restart, the fresh one). Never the
        // mirror read after the download: the pull can raise it to a version nobody has seen while
        // the download runs. A file whose metadata names no version (legacy, or the fetch failed)
        // gets the mirror as it was BEFORE the fetch, which is never newer than the bytes: at worst
        // the decision is `Moved`, or the write is sent on an older base and the server refuses it.
        let before = self.db.get_file_contract_state(&file_id)?.map(|contract| contract.current_version);
        let (remote, shown) = self.remote_content_preview(&file_id, is_text).await;
        let version = shown.or(before);
        Ok(ConflictContentPreview { is_text, local, remote, version })
    }

    /// Task 2190 (plan §3.2): the person's decision on a parked conflict, against `shown_version`,
    /// the version the conflict window showed. Every choice re-checks the op inside the transaction
    /// that acts on it (decidable, and its file still at that version), and returns `Moved`, with
    /// nothing changed, when it moved since.
    /// - `Local`: the same op is rebased on the server's version and sent again.
    /// - `Remote` (D1): `keep_copy` copies the Mac's bytes into the "Changes not sent" folder FIRST.
    ///   If it fails, nothing changed. Then the op goes and the system reads the server's version.
    ///   The staged copy is released only after that commit.
    /// - `Both` (D4): the Mac's bytes are queued as a new file named [`keep_both_name`] beside the
    ///   original, and the op is retired, in ONE transaction (review-169 #3): if it moved, nothing
    ///   is queued.
    pub fn resolve_parked(
        &self,
        op_id: &str,
        choice: ParkedChoice,
        shown_version: Option<i64>,
        keep_copy: &mut dyn FnMut(&Path, &str) -> anyhow::Result<PathBuf>,
        device: &str,
        hhmm: &str,
    ) -> anyhow::Result<ParkedResolution> {
        // Review-169 #1: the person decides against the version they were shown. Without it, or
        // with the file past it, nothing changes.
        let Some(shown) = shown_version else {
            return Ok(ParkedResolution::Moved);
        };
        if choice == ParkedChoice::Local {
            return Ok(match self.db.rebase_parked(op_id, shown, now_secs())? {
                Some(version) => ParkedResolution::Rebased { version },
                None => ParkedResolution::Moved,
            });
        }
        if !self.db.is_decidable_parked_at(op_id, shown)? {
            return Ok(ParkedResolution::Moved);
        }
        let op = self
            .db
            .get_operation(op_id)?
            .ok_or_else(|| anyhow::anyhow!("no queued write {op_id}"))?;
        let file_id = op.file_id.clone().ok_or_else(|| anyhow::anyhow!("the write names no file"))?;
        let row = self
            .db
            .get_file(&file_id)?
            .ok_or_else(|| anyhow::anyhow!("no state.db row for {file_id}"))?;
        let payload = op
            .payload_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the parked write has no staged copy"))?;
        let (done, released) = match choice {
            ParkedChoice::Remote => {
                // D1: the copy first; a failure changes nothing.
                let kept = keep_copy(Path::new(&payload), &row.path)?;
                match self.db.drop_parked(op_id, shown)? {
                    Some(released) => (
                        ParkedResolution::KeptOther {
                            kept_copy: kept.to_string_lossy().into_owned(),
                        },
                        released,
                    ),
                    // The kept copy stays (nothing is lost) and stays listed.
                    None => return Ok(ParkedResolution::Moved),
                }
            }
            ParkedChoice::Both => {
                let contract = self
                    .db
                    .get_file_contract_state(&file_id)?
                    .ok_or_else(|| anyhow::anyhow!("no contract for {file_id}"))?;
                let (parent_rel, leaf) = match row.path.rsplit_once('/') {
                    Some((parent, leaf)) => (Some(parent), leaf),
                    None => (None, row.path.as_str()),
                };
                let taken = self.db.child_names_lowercase(contract.parent_id.as_deref())?;
                let name = keep_both_name(leaf, device, hhmm, &taken);
                let target = FinderWriteTarget {
                    file_id: None,
                    parent_id: contract.parent_id.clone(),
                    filename: name.clone(),
                    rel_path: Some(match parent_rel {
                        Some(parent) => format!("{parent}/{name}"),
                        None => name.clone(),
                    }),
                    kind: FinderWriteItemKind::File,
                    contents_path: Some(payload.clone()),
                    content_type: contract.content_type.clone(),
                    base_version_identifier: None,
                };
                // Review-169 #3: the copy is queued and the parked write retired in ONE transaction;
                // a write that moved queues nothing.
                let retire = crate::state_db::RetireParked { op_id, shown_version: shown };
                let Some((written, released)) =
                    self.queue_file_provider_create_retiring(target, Some(&std::fs::File::open(&payload)?), Some(retire))?
                else {
                    return Ok(ParkedResolution::Moved);
                };
                match written.outcome {
                    FinderWriteOutcome::Queued { file_id: Some(new_id), .. } => {
                        (ParkedResolution::KeptBoth { file_id: new_id, name }, released)
                    }
                    other => anyhow::bail!("the copy could not be queued as a new file: {other:?}"),
                }
            }
            ParkedChoice::Local => unreachable!("handled above"),
        };
        // The staged copy was released in the decision's transaction; unlink it now (S6).
        if let Some(released) = released {
            crate::staged_payload::remove_or_log(&self.db, Path::new(&released));
        }
        Ok(done)
    }

    /// Local half of [`Self::conflict_content_preview`] (task 1546 Codex
    /// round 2, finding 1): stats the file to learn its size WITHOUT reading
    /// it, and only reads bytes at all when a text preview applies AND the
    /// file is within [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`] — bounded to one
    /// byte past the cap via [`std::io::Read::take`] so a race where the
    /// file grows between the stat and the read can only ever push the
    /// result to "too large," never load an oversized buffer. Conflict
    /// windows open automatically, so a multi-gigabyte local file must never
    /// be pulled fully into memory just to report its size.
    fn local_content_preview(&self, sync_root: &Path, entry_path: &str, is_text: bool) -> ConflictContentSide {
        let local_path = match local_file_path_under_sync_root(sync_root, entry_path) {
            Ok(p) => p,
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: None,
                    text: None,
                    unavailable_reason: Some(format!("Couldn't locate the local file: {e}")),
                };
            }
        };

        let size_bytes = match std::fs::metadata(&local_path) {
            Ok(m) => m.len(),
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: None,
                    text: None,
                    unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
                };
            }
        };

        if !is_text {
            // Binary preview only ever shows size — the bytes are never read.
            return ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: None,
            };
        }
        if size_bytes > CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64 {
            return ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: Some(format!("File is too large to preview, {size_bytes} bytes")),
            };
        }

        let file = match std::fs::File::open(&local_path) {
            Ok(f) => f,
            Err(e) => {
                return ConflictContentSide {
                    size_bytes: Some(size_bytes),
                    text: None,
                    unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
                };
            }
        };
        let mut bytes = Vec::with_capacity((size_bytes as usize).min(CONFLICT_PREVIEW_TEXT_MAX_BYTES) + 1);
        // `take(LIMIT + 1)` bounds the read itself — never `std::fs::read`
        // (unbounded) — so even a TOCTOU race where the file grows after the
        // `metadata()` call above can produce at most LIMIT+1 bytes.
        match file
            .take(CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
        {
            Ok(_) => content_side_from_bytes(bytes, is_text),
            Err(e) => ConflictContentSide {
                size_bytes: Some(size_bytes),
                text: None,
                unavailable_reason: Some(format!("Couldn't read the local file: {e}")),
            },
        }
    }

    /// Remote half of [`Self::conflict_content_preview`] (task 1546 Codex
    /// round 2, finding 1): fetches file metadata — one small JSON response —
    /// to learn the remote size BEFORE deciding whether to download
    /// anything. [`Self::do_hydrate_with_meta`] (which downloads and
    /// decrypts every chunk) is called only when BOTH a text preview applies
    /// AND the metadata size is within [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`] —
    /// a multi-gigabyte conflict, text or binary, therefore never costs
    /// bandwidth or a full decrypt just to be previewed. Reuses the SAME
    /// metadata fetch `do_hydrate_with_meta` needs instead of letting
    /// `do_hydrate` re-fetch it, so the small-file path costs exactly the
    /// metadata GET + the chunk GETs it always cost.
    ///
    /// Also returns the server version the side shows: the `download_version.version_number` of the
    /// metadata the attempt ran on (task 2190, review-169 F1), `None` when no metadata named one.
    async fn remote_content_preview(&self, file_id: &str, is_text: bool) -> (ConflictContentSide, Option<i64>) {
        let unavailable = |reason: String| ConflictContentSide {
            size_bytes: None,
            text: None,
            unavailable_reason: Some(reason),
        };
        if let Err(e) = file_id.parse::<uuid::Uuid>() {
            return (unavailable(format!("invalid file_id (not a UUID): {e}")), None);
        }
        let meta = match self.api.get_file(file_id).await {
            Ok(m) => m,
            Err(e) => return (unavailable(format!("Couldn't download the other device's version: {e}")), None),
        };
        // Tasks 1643 + 2173: a version that goes away mid-download restarts once from fresh metadata, as
        // `do_hydrate` does, and the size is checked again on the new version.
        self.with_one_restart(file_id, meta, |meta| async move {
            let version = meta["download_version"]["version_number"].as_i64();
            self.preview_attempt(file_id, is_text, &meta).await.map(|side| (side, version))
        })
        .await
        .unwrap_or_else(|e| (unavailable(format!("Couldn't download the other device's version: {e}")), None))
    }

    /// One pass of [`Self::remote_content_preview`] on `meta`. `Err` only for what the restart handles (the version
    /// went away) or what ends it; every other failure is the side's own "unavailable" line.
    async fn preview_attempt(&self, file_id: &str, is_text: bool, meta: &serde_json::Value) -> anyhow::Result<ConflictContentSide> {
        // Task 1643: the size of the version that would be downloaded. While a replace runs, the top-level
        // `size_bytes` can be another version's.
        let size_bytes = match crate::download_pin::PinnedVersion::from_metadata(meta)? {
            Some(pin) => Some(pin.plaintext_size_bytes),
            None => meta.get("size_bytes").and_then(|v| v.as_u64()),
        };

        if !is_text {
            // Binary preview only ever shows size — never download the bytes.
            return Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: None,
            });
        }
        let within_limit = matches!(size_bytes, Some(s) if s <= CONFLICT_PREVIEW_TEXT_MAX_BYTES as u64);
        if !within_limit {
            return Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(match size_bytes {
                    Some(s) => format!("File is too large to preview, {s} bytes"),
                    None => "Couldn't determine the other device's file size".to_string(),
                }),
            });
        }

        // Between the metadata and the chunks: where another device's save and the pull's mirror of it
        // can land while the preview downloads (review-169 F1's test).
        self.seam("preview:before_download");
        match self.do_hydrate_with_meta(file_id, meta, None).await {
            Ok(mut bytes) => {
                // Move the plaintext out instead of `bytes.to_vec()` — a
                // clone would briefly hold two live copies of the remote
                // plaintext in memory. `content_side_from_bytes` consumes
                // the Vec by value (no further copy), and the now-empty
                // `bytes` is zeroized below for defense in depth.
                let side = content_side_from_bytes(std::mem::take(&mut *bytes), is_text);
                bytes.zeroize();
                Ok(side)
            }
            Err(e) if e.downcast_ref::<crate::download_pin::VersionGone>().is_some() => Err(e),
            Err(e) => Ok(ConflictContentSide {
                size_bytes,
                text: None,
                unavailable_reason: Some(format!("Couldn't download the other device's version: {e}")),
            }),
        }
    }
}


// ── Wire-byte counters (P1 — live throughput) ────────────────────────────────
//
// Two `AtomicU64` counters track bytes actually sent/received on the wire in
// the chunk loops (upload + download).  The heartbeat producer drains them with
// `swap(0)` each beat so it gets the delta over the beat interval — the true
// wire speed, not a file-completion delta.
//
// Both are incremented from async task context (no blocking), so `Relaxed`
// ordering is sufficient: each counter is touched only from the engine thread,
// and the heartbeat producer races on a separate task.  The only requirement is
// that the swap and the add are individually atomic — a consistent view across
// both counters simultaneously is NOT required (the heartbeat is a telemetry
// estimate, not an accounting figure).








// ── Per-file state machine ────────────────────────────────────────────────────




// ── EngineBridge ──────────────────────────────────────────────────────────────










































































































/// Files [`cached_sha256_file_hex`] read on this thread (tests count them).
#[cfg(test)]
thread_local! {
    static HASH_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
































































































#[cfg(test)]
thread_local! {
    /// Task 2190: the op whose decision check fails, for the feed's per-item test.
    static FAIL_DECIDABLE_READ: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}



















#[cfg(test)]
thread_local! { static LEGACY_CHILD_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }




















// ── Periodic sync tick ────────────────────────────────────────────────────────




































#[cfg(test)]
mod tests {
    fn test_open_content_frame(key: &beebeeb_core::kdf::FileKey, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(
            beebeeb_core::kdf::FileKey::from_bytes(*key.as_bytes()), "legacy", beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded);
        let data = reader.push_frame(&content_frame(bytes)?)?.data;
        reader.finish()?;
        Ok(data)
    }
    mod sf2237d {
        use super::*;
        include!("engine_bridge/2237d_tests.rs");
    }
    use super::*;
    use base64::Engine;
    use serde_json::json;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::thread;

mod basics;
mod uploads;
mod hydration;
mod trash_ops;
mod shared;
mod metadata;
mod finder_queue;
mod uploads_2;
mod conflicts;
mod uploads_3;
mod uploads_4;
mod shared_2;
mod sync;
mod sync_2;
mod ipc;
mod hydrate_files;
mod trash_markers;
mod versions;
mod uploads_5;
mod finder_queue_2;
mod uploads_6;
mod refusals;
mod sync_3;
mod uploads_7;
mod versions_2;
mod finder_queue_3;
mod trash_ops_2;
mod metadata_2;
mod finder_queue_4;
mod versions_3;
mod sync_4;
mod uploads_8;
mod finder_queue_5;
mod refusals_2;
mod finder_queue_6;
mod thumbnails;
mod sync_5;
mod conflicts_2;
mod thumbnails_2;
mod uploads_9;
mod refusals_3;
mod logging;
mod downloads;
mod reg_1640;
mod reg_1640_2;
mod scan;
mod review_2190;
mod review_2190_2;

use basics::{RecordedRequest, UploadMockServer, read_http_request_or_hangup, test_bridge, test_bridge_with_api, write_test_png};
use finder_queue_2::{capture_logs_async, fp_create};
use finder_queue_5::{Refused, refuse, stop_refusing};
use ipc::{IpcHydrationMock};
use metadata_2::{patched_parents};
use refusals_2::{reqwest_status_error, transient};
use refusals_3::{PLANTED, assert_clean_line};
use reg_1640::{regression_1640_chunk, regression_1640_edited_file_in_a_folder, regression_1640_edited_file_in_a_folder_in, regression_1640_synced_file, regression_1640_web_saves};
use reg_1640_2::{regression_1640_new_placeholder, regression_1640_uploads_queued_for};
use scan::{regression_2229_set_modified, task_2190_files_holding, task_2190_fu1_tick_after, task_2190_server_chunk, task_2190_web_saves};
use shared_2::{enqueue_test_operation, seed_bridge_entry, seed_bridge_row};
use sync::{SyncMockServer, enc_name, snap_node};
use sync_5::{landed_create, rule3_setup, thumbnail_blob};
use trash_markers::{MockServerFile, VersionedServerMock, VersionedServerState};
use uploads::{HydrationMockServer, MockResponse, TEST_FILE_ID, encrypt_chunk_wire, http_bytes, http_json, hydration_mock_response, hydration_test_key, read_http_request, upload_mock_response};
use uploads_3::{ResumableUploadMock, ResumeMockMode, count_requests, init_item_id, resumable_create_op};
use uploads_5::{fp_save, queue_save, seed_uploaded_db_row, seed_uploaded_row};
use uploads_7::{land_one_save, remote_update};
use uploads_8::{assert_kept_uploading, failing_passes};
use versions::{drain_upload_queue, finder_file_target, held_content_version, run_competing, versioned_server_answer};
use versions_2::{FP_ROOT, fp_create_in, fp_save_moved, listed_under, name_parent_status, reparented_changes, seed_folder_row, seed_row_in_folder};
use versions_3::*;
}

/// The attempt resumed its persisted upload session, found it reaped (the server answered 404 or
/// 410) and abandoned it: the next attempt starts a fresh upload, so for a File Provider write the
/// failure is transient, never a counted one (spec §8.4). A 400 at any session, and a 404 or 410 at a
/// session opened in the same attempt, abandon the session too, but stay counted. It shows, and has
/// as its source, the failure it wraps, so every reading of the error's text and status stays as it
/// was.
#[derive(Debug)]
struct SessionAbandoned(anyhow::Error);

impl std::fmt::Display for SessionAbandoned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SessionAbandoned {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let inner: &(dyn std::error::Error + 'static) = self.0.as_ref();
        Some(inner)
    }
}

/// A Finder write could not be staged: the staging folder cannot be created or written to, and
/// on macOS nothing falls back to a folder the system may purge (spec §8.4). The reply is
/// `WriteRetryLater`, which the extension reports as a transient error, so the system keeps the
/// change and retries. Path-free on purpose: the text reaches the extension and the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct FinderStagingUnavailable(Option<std::io::ErrorKind>);

impl std::fmt::Display for FinderStagingUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(kind) => write!(f, "the staging folder is unavailable: {kind}"),
            None => f.write_str("the staging folder is unavailable: no data directory"),
        }
    }
}

impl std::error::Error for FinderStagingUnavailable {}

/// The result of [`EngineBridge::hydrate_file_range`]: the decrypted plaintext
/// covering the requested range, plus enough context for the caller to splice
/// out the exact `[required_offset, required_offset + required_length)` span.
#[cfg(any(target_os = "windows", test))]
pub struct HydratedRange {
    /// Decrypted plaintext for chunks `[first_chunk, last_chunk]` — i.e. the
    /// smallest chunk-aligned span that covers the requested byte range. This
    /// is NOT necessarily aligned to the requested range itself; the caller
    /// must slice `data[required_offset - range_start_bytes ..]`.
    pub data: Zeroizing<Vec<u8>>,
    /// Absolute byte offset (into the full file) of `data[0]` — i.e.
    /// `first_chunk * chunk_size_bytes`.
    pub range_start_bytes: u64,
    /// `true` when `data` happens to cover the ENTIRE file (the covering-chunk
    /// range was `[0, chunk_count)`). Only then is a range hydration equivalent
    /// to a whole-file hydration for status-bookkeeping / resolve-or-error
    /// purposes.
    pub covers_whole_file: bool,
}

/// Test-only hooks at named points where the concurrency tests force an
/// interleaving (spec §13, "Concurrency tests"). Each hook fires once.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Seams {
    hooks: std::sync::Mutex<HashMap<&'static str, SeamHook>>,
    /// Staging bases this bridge uses instead of the process's (`stage_under`, macOS tests).
    staging_bases: std::sync::Mutex<Option<FinderStagingBases>>,
    /// The download transport this bridge uses (task 2173). Unset: `Auto`, so a test never reads
    /// the developer's `BB_TRANSPORT` or `desktop.toml`.
    transport: std::sync::Mutex<Option<crate::direct_download::Transport>>,
    /// Every direct storage fetch fails with this status, without a request (task 2173).
    storage_fail: std::sync::Mutex<Option<u16>>,
}

#[cfg(test)]
type SeamHook = Box<dyn FnOnce() + Send>;

#[cfg(test)]
impl Seams {
    pub(crate) fn arm(&self, name: &'static str, hook: impl FnOnce() + Send + 'static) {
        self.hooks.lock().unwrap().insert(name, Box::new(hook));
    }

    fn fire(&self, name: &'static str) {
        // Taken out first: the hook runs without the map's lock held.
        let hook = self.hooks.lock().unwrap().remove(name);
        if let Some(hook) = hook {
            hook();
        }
    }

    /// From now on this bridge stages Finder writes under `bases`, not the test sandbox.
    /// macOS only: its one caller is the macOS staging-folder test.
    #[cfg(target_os = "macos")]
    pub(crate) fn stage_under(&self, bases: FinderStagingBases) {
        *self.staging_bases.lock().unwrap() = Some(bases);
    }

    fn staging_bases(&self) -> Option<FinderStagingBases> {
        self.staging_bases.lock().unwrap().clone()
    }

    /// From now on this bridge downloads with `transport` (task 2173).
    pub(crate) fn use_transport(&self, transport: crate::direct_download::Transport) {
        *self.transport.lock().unwrap() = Some(transport);
    }

    /// From now on every direct storage fetch of this bridge fails with `status` (task 2173).
    pub(crate) fn fail_storage(&self, status: u16) {
        *self.storage_fail.lock().unwrap() = Some(status);
    }
}

mod types;
use types::*;
pub use types::{WireCounters, FileEvent, FileSM, FpWrite, CachePolicy, PinUpdateOutcome, CacheCleanupOutcome, TransferLoopOutcome, ConflictContentSide, ParkedChoice, ParkedResolution, ConflictContentPreview, FetchedVersion, FetchedBytes, FetchTicket, name_opens, ConflictDetected, SyncTickOutcome};
pub(crate) use types::{NON_UUID, log_alias_resolved, SyncFolderContent, RENAME_SETTLE_SECS, OpDone, LocalEntry, PrunedEntry};

mod errors;
use errors::*;
pub use errors::{OperationFailureClass, classify_operation_error, retry_delay_seconds};
pub(crate) use errors::{UnknownItem, QueuedCopyMissing, UnsentLocalChange, ThumbnailNotYet, loggable_id, error_category, StagedCopyGone, AcceptsClosed, ItemInTrash, error_http_status, QueueStateMoved, ParkNow};

mod content;
use content::*;
pub(crate) use content::{sha256_hex, ContentFingerprint, cached_sha256_file_hex, sha256_file_hex, content_frame};

mod media;
use media::*;

mod fs;
use fs::*;
pub(crate) use fs::{path_is_engine_internal, relative_db_path, LocalPathVerdict, move_no_replace};

mod meta;
use meta::*;
pub use meta::{HydrateSource, SharedRootRefreshOutcome};
pub(crate) use meta::{wire_id_fields, promote_fetched_content, HydrateProgressFn, hydrate_final_status, hydrate_dest_is_allowed, write_hydrated_plaintext, write_hydrated_from_reader, item_content_version, item_version_identifier};

mod meta_2;
use meta_2::*;
pub(crate) use meta_2::{parse_base_version_number, decrypt_shared_name_with_key};

mod landing;
use landing::*;
pub use landing::{FinderWriteItemKind, FinderWriteTarget, FinderWriteOutcome, VersionConflictEntry, is_ignored_finder_name, version_conflict_feed_from_db};
pub(crate) use landing::{keep_both_name, FinderStagingBases, finder_staging_candidates, remove_pruned_entry, copy_as_this_pc, retry_kept_originals};

mod syncflow;
use syncflow::*;
pub use syncflow::{sync_tick, sync_tick_outcome};
pub(crate) use syncflow::{sync_folder_content, init_conflict_class, local_file_path_under_sync_root};