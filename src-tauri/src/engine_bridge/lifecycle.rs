//! `lifecycle` — the `EngineBridge` impl for construction, accessors, the pass loop and its operation dispatch, split out of `engine_bridge.rs`.

use super::*;

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
    pub(super) fn set_thumbnails_pending(&self, file_id: &str, wait: ThumbnailWait) {
        let mut set = self
            .thumbnails_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if wait.is_waiting() {
            set.insert(file_id.to_string());
        } else {
            set.remove(file_id);
        }
    }

    /// Whether `file_id`'s landed write still waits for its new thumbnails (spec §5.6).
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub(super) fn thumbnails_pending(&self, file_id: &str) -> bool {
        self.thumbnails_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(file_id)
    }

    /// A named point for the concurrency tests; nothing outside tests.
    pub(super) fn seam(&self, name: &'static str) {
        #[cfg(test)]
        self.seams.fire(name);
        #[cfg(not(test))]
        let _ = name;
    }

    /// Where this bridge stages a Finder write's plaintext copy ([`default_finder_staging_root`]). A test may
    /// point one bridge at bases of its own.
    pub(super) fn finder_staging_root(&self) -> Result<PathBuf, FinderStagingUnavailable> {
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
    pub(super) fn record_transfer_done(&self, direction: crate::transfer_progress::Direction, file_id: &str, bytes: u64) {
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
    fn auth_ops_open_under_this_key(&self) -> Result<bool, EngineError> {
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
                .await
                .map_err(land_error_to_anyhow);
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
    ) -> Result<OpDone, EngineError> {
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
                    return Err(anyhow::anyhow!("hydrate operation target missing from state").into());
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
}
