//! `sign_out_commands` items split out of `lib.rs`.

use super::*;

/// Drop any cached session and abort the engine if running.
///
/// Shared by the WebView IPC command and the native menu "Sign out" item so
/// both routes have the exact same security side-effects.
pub(super) async fn clear_session_impl(
    state: &AppState,
    forget_email: bool,
    keep_unsent: Option<&str>,
    unknown_files: UnknownFilesArg,
    kept_folder: Option<std::path::PathBuf>,
) -> Result<SignedOut, SignOutFailure> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    let mut _auth_reopen = None;
    // Spec 2026-10-10 W5: the gate runs first, before anything changes. A refusal changes nothing.
    #[cfg(target_os = "macos")]
    let accepted = gate_sign_out(state, keep_unsent)?;
    #[cfg(not(target_os = "macos"))]
    let _ = keep_unsent;
    let acct = state.active_account()?;
    // An account switch (spec §5.6) leaves no account email behind, or it stops: the prefill is forgotten before
    // anything is torn down, and it is never saved again below.
    if forget_email {
        forget_last_signed_in_email()?;
    }
    // Task 12, lead ruling 1: no session transition that began before this sign-out writes anything from here on (the
    // session clear below ends any that begins while it runs).
    drop(end_sessions_in_flight(&acct, account::SessionTransition::SignOut));

    // Spec 2026-10-06 §5.3 (5): a sign-out by choice removes Beebeeb from Finder. The reconciler
    // cancels any check first and holds, so no check can start an engine after the stop below (plan
    // "Spec issues" 8). A removal it cannot confirm (not running, no answer in time, or it failed) does
    // not stop the sign-out: everything below still runs, and the result says it was unconfirmed
    // (lead rulings T8-⚠lock 1, T7-⚠2 5).
    #[cfg(target_os = "macos")]
    let finder_cleanup = finder_remove_for(state, finder_setup::core::Trigger::SignOut).await;
    // Task 1882 (P0): the removal keeps the files that never reached the server
    // (`NSFileProviderDomainRemovalModePreserveDirtyUserData`), and the folder macOS kept them in rides the
    // result to the alert. A removal that was not confirmed is logged here; one that failed but kept a folder
    // still names it (review M2).
    #[cfg(target_os = "macos")]
    let preserved_location = finder_removal::sign_out_kept_location(finder_cleanup.clone());
    #[cfg(not(target_os = "macos"))]
    let preserved_location: Option<String> = None;
    // Review I1: the Finder location is gone from here on, so every failure below carries the folder.
    let kept_with = |message: String| SignOutFailure {
        message,
        preserved_location: preserved_location.clone(),
        kept_unsent: None,
    };
    // What the result says when the sign-out happens: a removal the reconciler did not confirm (FB-24).
    #[cfg(target_os = "macos")]
    let removal_warning = finder_cleanup
        .is_err()
        .then_some(ActionWarning::FinderRemovalUnconfirmed);
    #[cfg(not(target_os = "macos"))]
    let removal_warning: Option<ActionWarning> = None;
    // Held to the end of the function: a sign-out that stops short with the keys still in memory puts
    // Finder back (lead ruling T3-⚠2).
    #[cfg(target_os = "macos")]
    let _finder_restore = FinderRestoreOnAbort { state, acct: &acct };

    // Bug B guard: already signed out everywhere (no auth flag, no in-memory
    // session, no keychain session). The old flow ran the full teardown anyway and
    // could FAIL (CFAPI/purge preflight) — trapping the user in a
    // "can't sign out because not signed in" loop. On Windows this path skips the
    // engine stop and the Cloud Files teardown entirely. On macOS and Linux it
    // still takes the engine slot and stops any engine it finds (Task 12 fix round
    // 2, item 1), so it also refuses after an unconfirmed engine stop. What it does
    // skip there, against a signed-in sign-out: saving the email for the next
    // sign-in; the `SignedOut` lifecycle event (it returns `NotSignedIn`, not
    // `Completed`); and failing on a Keychain clear that errors (that error is
    // logged and the sign-out goes on). What still runs below, on BOTH paths:
    // the idempotent keychain clear, the local-state purge (the queue purge is a
    // cross-account safety invariant — never skipped), and the auth-flag resets.
    let already_signed_out = {
        let auth_present = state.auth_present.lock().map(|present| *present).unwrap_or(false);
        let session_installed = acct.session.lock().map(|guard| guard.is_some()).unwrap_or(false);
        !auth_present && !session_installed && !keychain_session_present(acct.id.as_str())
    };
    // The engine slot, once taken below, is held until the session is cleared (lead ruling T8-⚠lock 2):
    // every start reads its keys under the slot, so releasing it first would let a start that was waiting
    // for it see keys this sign-out is about to clear. `clear_session_holding_slot` makes that order the
    // only one that compiles. Session installs are ordered against a sign-out separately (Task 12).
    //
    // Task 1886: every session token this sign-out takes off this computer, each ended on the server once the local
    // clear has run and the slot is free (`end_dropped_sessions`, at the end). It begins with the token in memory or the
    // Keychain, read now, before the slot is taken: from the slot on, nothing here touches the session until its clear.
    // The clears below add what they remove, a session written while the sign-out runs included. An already-signed-out
    // sign-out begins with none.
    let mut dropped = DroppedTokens::default();
    if !already_signed_out {
        dropped.add(session_token_for_revoke(&acct));
    }
    let mut held_engine_slot = if already_signed_out {
        #[cfg(target_os = "windows")]
        tracing::info!("sign-out requested while already signed out; skipping engine stop and Cloud Files teardown");
        #[cfg(not(target_os = "windows"))]
        tracing::info!("sign-out requested while already signed out; stopping any engine still in the slot");
        // macOS and Linux (Task 12 fix round 2, item 1): this path holds the engine slot from here too, BEFORE the
        // purge, and stops any engine it finds there (one a sign-in that completed after this sign-out began has
        // started), through the same helper and with the same refusals as the signed-in path. A start that comes during
        // the purge waits for the slot and then finds no keys.
        #[cfg(not(target_os = "windows"))]
        {
            Some(
                take_slot_and_stop_engine_for_sign_out(&acct)
                    .await
                    .map_err(&kept_with)?,
            )
        }
        #[cfg(target_os = "windows")]
        {
            None
        }
    } else {
        // Remember the signed-in email for the sign-in prefill BEFORE any
        // credential erasure: `clear_keychain_session` deletes the keychain
        // account-email credential, so this capture is the last chance to
        // keep the address the sign-in form should open with. Best-effort —
        // never gates sign-out. On the already-signed-out path the capture
        // finds nothing and no write happens. An account switch saves nothing (it forgot the email above).
        if !forget_email {
            persist_last_signed_in_email(signout_email_to_preserve(&acct));
        }
        #[cfg(target_os = "windows")]
        close_session_commands().await?;
        // Stop the engine before dropping memory so the IPC listener cannot accept
        // new File Provider operations with a cloned master key (see `take_slot_and_stop_engine_for_sign_out`).
        let engine_slot = take_slot_and_stop_engine_for_sign_out(&acct)
            .await
            .map_err(&kept_with)?;

        #[cfg(target_os = "windows")]
        {
            windows_cf::revoke_callbacks()
                .await
                .map_err(|e| format!("Could not disconnect Cloud Files: {e}"))?;
            stop_own_data_proofs();
            windows_cf::wait_for_credential_release().await?;
            // Native/credential shutdown is now confirmed. Cleanup may refuse dirty
            // files; allow an explicit unlock so the user can sync and retry.
            {
                SESSION_COMMANDS.finish_close();
                _auth_reopen = Some(auth_attempts::ReopenOnDrop(&AUTH_ATTEMPTS));
            }
        }
        Some(engine_slot)
    };
    // Spec 2026-10-10 §5.2 step 7 (W6): after the confirmed stop of either arm above, before the purge. Nothing between
    // here and the purge deletes a file, writes the queue, clears a session or starts an engine (U10).
    #[cfg(test)]
    sign_out_seam("before_keep").map_err(&kept_with)?;
    #[cfg(target_os = "macos")]
    let kept = keep_for_sign_out(accepted.as_ref()).map_err(&kept_with)?;
    // Spec 2026-10-10 §15 question 3: the keep step left Finder accepts closed for the purge, which clears them with
    // the rows. Any return before the purge has done so reopens them.
    #[cfg(target_os = "macos")]
    let mut reopen_accepts = AcceptsReopenOnAbort { armed: true };
    #[cfg(not(target_os = "macos"))]
    let kept: Option<sign_out::kept_set::KeptSet> = None;
    let kept_unsent = kept.as_ref().and_then(|kept| {
        kept.folder.as_ref().map(|folder| KeptUnsent {
            folder: folder.to_string_lossy().into_owned(),
            files: kept.files,
        })
    });
    // §5.2 step 10: from here on every failure carries our folder too.
    let kept_with = |message: String| SignOutFailure {
        message,
        preserved_location: preserved_location.clone(),
        kept_unsent: kept_unsent.clone(),
    };
    #[cfg(test)]
    sign_out_seam("after_keep").map_err(&kept_with)?;
    // Task 2191 (D11): the folder a Windows sign-out kept unknown bytes in, for the alert.
    #[cfg(target_os = "windows")]
    let mut windows_kept: Option<KeptUnsent> = None;
    #[cfg(not(target_os = "windows"))]
    let _ = (unknown_files, kept_folder);

    #[cfg(target_os = "windows")]
    {
        // Local teardown is authoritative even offline. The server revocation
        // runs after this block, on every OS (task 1886: it used to be here,
        // Windows only), best effort and bounded.
        //
        // The PURGE below runs on BOTH paths (already-signed-out included):
        // it is the cross-account safety invariant and must never be skipped.
        // Only the Cloud Files unregister (and, after this block, the server
        // revocation) is gated on actually having been signed in — when
        // already signed out there is no live registration of ours to remove
        // and no session to revoke.
        let root = if already_signed_out {
            DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root)
        } else {
            DesktopConfig::load()?.sync_root
        };
        let db = state_db_from_app_local_state_dir()?;
        if let Some(db) = db {
            // Task 2191 (D11): files of unknown content are kept as copies or deleted, as the
            // person chose in the confirmation.
            let choice = match (unknown_files, root.as_deref()) {
                (UnknownFilesArg::Keep, Some(root)) => windows_cf::signout::UnknownFiles::Keep {
                    // Review-157 F4: the folder the confirmation named, when it is one of ours and
                    // outside the sync folder; else a fresh one. A sync folder at a drive root with
                    // no home folder has nowhere outside it, so the sign-out stops.
                    dir: kept_folder
                        .filter(|dir| {
                            !dir.starts_with(root)
                                && dir
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .is_some_and(|name| name.starts_with("Beebeeb kept files "))
                        })
                        .or_else(|| kept_unknown_files_dir(root))
                        .ok_or_else(|| {
                            "Sign-out paused: there is no folder outside the sync folder to keep copies in.".to_string()
                        })?,
                },
                (UnknownFilesArg::Keep, None) | (UnknownFilesArg::Delete, _) => {
                    windows_cf::signout::UnknownFiles::Delete
                }
            };
            let kept = windows_cf::signout::purge(crate::APP, &db, root.as_deref(), &choice)
                .map_err(|e| format!("Sign-out paused: {e}"))?;
            windows_kept = kept.dir.map(|dir| KeptUnsent {
                folder: dir.to_string_lossy().into_owned(),
                files: kept.files,
            });
        } else if let Some(root) = root.as_ref() {
            if std::fs::read_dir(root)
                .map_err(|e| format!("Cannot inspect sync folder: {e}"))?
                .next()
                .is_some()
            {
                return Err("Cannot verify sync-folder ownership without its database. Restore the local state before signing out.".into());
            }
        }
        // Task 2191 (review thread on #157): from here on the purge may have kept copies and
        // deleted their originals, so every failure names their folder too.
        let failed_after_purge =
            |message: String| sign_out_failure_after_purge(message, &preserved_location, &kept_unsent, &windows_kept);
        if !already_signed_out {
            if let Some(root) = root {
                // Cloud Files must release the root before WinRT removes its shell registration.
                windows_cf::unregister_sync_root(&root)
                    .map_err(|e| failed_after_purge(format!("Could not unregister Cloud Files: {e}")))?;
                windows_cf::unregister_shell_sync_root(crate::APP, &root)
                    .map_err(|e| failed_after_purge(format!("Could not remove Explorer registration: {e}")))?;
            }
        }
    }
    // Task 2191 (review thread on #157): every later failure carries the folder the purge kept
    // copies in, as the success does.
    #[cfg(target_os = "windows")]
    let kept_with =
        |message: String| sign_out_failure_after_purge(message, &preserved_location, &kept_unsent, &windows_kept);

    // Task 1538 findings 1+2: purge every queued operation (so a later
    // account's engine can never drain THIS account's still-queued uploads —
    // `operation_queue` carries no account scoping, see
    // `state_db::StateDb::purge_all_local_state`) and every decrypted cache
    // file this device holds for this account (so it can't be permanently
    // orphaned on disk once the next account's first sync prunes the DB row
    // that pointed at it). Per file it stays best-effort (the DB rows are
    // cleared regardless). A failure of the purge itself stops the sign-out
    // (R10, spec 2026-10-06 §5.6).
    //
    // Task 1538 Codex P1 (lib.rs:1085 thread): `db` is resolved from the
    // app-local state dir DIRECTLY — `state_db_from_app_local_state_dir`,
    // not the old `DesktopConfig::load().ok().and_then(state_db_for_config)`
    // chain — because `state.db` lives there independently of
    // `desktop.toml`/`sync_root`. A missing, corrupt, or unreadable config
    // (or one whose `sync_root` was rejected on load) must never silently
    // skip this security purge. `sync_root` itself is still read from
    // `DesktopConfig` best-effort, separately — it is used ONLY to resolve
    // Windows placeholder paths below, never to gate whether the purge runs
    // at all.
    #[cfg(not(target_os = "windows"))]
    {
        let cfg_sync_root = DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root);
        // On a Mac a removal the reconciler could not confirm leaves the domain registered: that is a debt no
        // engine may start before.
        #[cfg(target_os = "macos")]
        let owe_removal = finder_cleanup.is_err();
        #[cfg(not(target_os = "macos"))]
        let owe_removal = false;
        // Test builds only: every sign-out test shares the one scratch state directory the purge below works on, so
        // two purges at once can find the database held ("database is locked"). They take turns here (a test that
        // already holds the gate for its whole body says so, and does not wait for itself).
        #[cfg(test)]
        let _one_purge_at_a_time = if PURGE_GATE_HELD.try_with(|_| ()).is_ok() {
            None
        } else {
            Some(SIGN_OUT_TEST_PURGE_GATE.lock().await)
        };
        purge_local_data_for_sign_out(
            state_paths::beebeeb_state_dir(),
            cfg_sync_root.as_deref(),
            owe_removal,
            kept.as_ref(),
        )
        .map_err(&kept_with)?;
        // The purge cleared the rows, and the closed accepts with them.
        #[cfg(target_os = "macos")]
        reopen_accepts.disarm();
        #[cfg(test)]
        sign_out_seam("after_purge").map_err(&kept_with)?;
    }

    // (macOS: the Finder removal is the reconciler's, at the start of this function: spec 2026-10-06
    // §5.3 (5). A logged-out Mac has no File Provider domain pointing at a folder the person is no
    // longer signed into, and no second, direct removal runs here.)
    // Task 1670 round 2: also the account-switch boundary — this codebase's
    // sign-out IS its account-switch mechanism (single active-account slot,
    // see `AppState::active_account`'s own "Phase 0" comment), so there is no
    // separate switch-account hook to add this to.
    purge_macos_hydrate_cache("sign-out");

    // Every path holds the engine slot by now (it took the slot to stop the engine), except the Windows
    // already-signed-out path, which stopped nothing and holds none: it takes the slot here, only to clear under it.
    // Task 1886: the local clear is one step (`cleared`), so the sessions it took are ended on the server only after
    // it, with its result known and the engine slot released.
    let cleared: Result<SignedOut, SignOutFailure> = async {
        let engine_slot = match held_engine_slot.take() {
            Some(slot) => slot,
            None => acct.engine.lock().await,
        };
        let in_memory =
            clear_session_holding_slot(&acct, &engine_slot, account::SessionTransition::SignOut).map_err(&kept_with)?;
        dropped.add(in_memory);
        // Windows: only now may a start waiting for the engine slot get it, and it finds no keys. On macOS and Linux the
        // slot is held through the last turn below (`finish_sign_out_turn`).
        #[cfg(target_os = "windows")]
        drop(engine_slot);
        // Drop the cached account profile so a subsequent `account_profile` IPC
        // can't cache-hit a stale (logged-out) identity. A fresh login repopulates it.
        clear_cached_profile(state);
        // And the storage summary (task 1887's rule: nothing of the account survives sign-out). The session is gone by
        // now, so no new fetch can start with its token; one still running answers into nothing (#139 review P2-3).
        popover_data::clear_storage(&state.popover_storage);
        // Sign out = clean slate: drop any in-flight 2FA challenge so an abandoned
        // partial token can't linger (and is zeroized) past a logout.
        if let Ok(mut guard) = state.pending_2fa.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = acct.engine_state.lock() {
            *guard = "stopped".to_string();
        }
        // Sign out = clean slate for the auth-health streak too (task 1546
        // finding 5): the engine that was feeding it just stopped, and a fresh
        // sign-in must not inherit a stale `auth_expired: true` banner.
        acct.auth_health.note_result(None);
        if already_signed_out {
            // Already signed out: after a startup 401 (R9) the vault key and the email can still be here. The clear is
            // idempotent, but on a store that cannot answer (e.g. the Linux
            // fail-closed stub) an error must not turn an already-signed-out
            // no-op into a failure — log it and go on to the check (macOS/Linux: inside the last turn).
            #[cfg(not(target_os = "windows"))]
            {
                finish_sign_out_turn(&acct, &engine_slot, true, &mut dropped).map_err(&kept_with)?;
                // Only now may a start waiting for the engine slot get it, and it finds no keys.
                drop(engine_slot);
            }
            #[cfg(target_os = "windows")]
            if let Err(error) = clear_keychain_session_into(acct.id.as_str(), &mut dropped) {
                tracing::warn!(
                    %error,
                    "already-signed-out sign-out: keychain session clear failed (nothing should be left); continuing"
                );
            }
            set_auth_present(state, false);
            set_auth_email(state, None);
            state.sign_out.snapshots.clear();
            return Ok(SignedOut {
                outcome: SignOutOutcome::NotSignedIn,
                warning: removal_warning,
                preserved_location: preserved_location.clone(),
                #[cfg(target_os = "windows")]
                kept_unsent: kept_unsent.clone().or_else(|| windows_kept.clone()),
                #[cfg(not(target_os = "windows"))]
                kept_unsent: kept_unsent.clone(),
            });
        }
        // macOS/Linux: the Keychain clear and its check in the sign-out's last turn, which moves the generation on after them.
        #[cfg(not(target_os = "windows"))]
        {
            finish_sign_out_turn(&acct, &engine_slot, false, &mut dropped).map_err(&kept_with)?;
            // Only now may a start waiting for the engine slot get it, and it finds no keys.
            drop(engine_slot);
        }
        #[cfg(target_os = "windows")]
        clear_keychain_session_into(acct.id.as_str(), &mut dropped).map_err(&kept_with)?;
        set_auth_present(state, false);
        set_auth_email(state, None);
        lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedOut);
        // Everything this sign-out can clear is cleared, so it happened; a removal from Finder it could not confirm is a
        // warning on that success, not an error (FB-24; lead rulings T8-⚠lock 1 and T7-⚠2 5).
        state.sign_out.snapshots.clear();
        Ok(SignedOut {
            outcome: SignOutOutcome::Completed,
            warning: removal_warning,
            preserved_location: preserved_location.clone(),
            #[cfg(target_os = "windows")]
            kept_unsent: kept_unsent.clone().or_else(|| windows_kept.clone()),
            #[cfg(not(target_os = "windows"))]
            kept_unsent: kept_unsent.clone(),
        })
    }
    .await;

    // Task 1886 (P0): the sessions this sign-out took off this computer end on the server too, on every OS (it was
    // Windows only, and only for the token it began with, so a Mac's or a Linux computer's signed-out token kept
    // working on the server for up to 30 days). Here: after the local clear, whatever its result (a clear that failed
    // ends only what this computer no longer holds), with the engine slot released. Best effort and bounded: whatever
    // the server answers, or if it does not, the sign-out's result stands and no token is logged.
    end_dropped_sessions(&acct, dropped, cleared.is_ok()).await;
    cleared
}

/// Sign-out's engine stop: take the engine slot and stop whatever engine holds it, through the one stop helper
/// (`stop_engine_in_slot`, called by [`stop_engine_for_sign_out`]). Returns the slot, still held, so the session is
/// cleared before any start can take it. Refuses (with the slot released and nothing cleared) when an earlier stop was
/// never confirmed, when this stop cannot be confirmed, or when the Windows Cloud Files revocation failed. Both
/// sign-out paths use it (Task 12 fix round 2).
pub(super) async fn take_slot_and_stop_engine_for_sign_out(
    acct: &AccountRuntime,
) -> Result<tokio::sync::MutexGuard<'_, Option<EngineRunner>>, String> {
    let mut engine_slot = acct.engine.lock().await;
    #[cfg(all(test, target_os = "macos"))]
    signal_sign_out_holds_the_engine_slot();
    if let Err(refusal) = stop_engine_for_sign_out(
        acct,
        &mut engine_slot,
        upload_staging_dir_for_session_purge().as_deref(),
    )
    .await
    {
        drop(engine_slot);
        return Err(refusal);
    }
    Ok(engine_slot)
}

/// Sign-out's engine stop on the held slot: the gate in front of every purge sign-out runs.
/// Refuses (Bug A / A2) unless the engine task is CONFIRMED terminated, or no
/// engine is running and no earlier attempt left an unconfirmed one behind.
///
/// Then upload staging: every handed-over copy is removed. A request that arrives later reaches no daemon and fails
/// transiently. One Finder write or change may still be running: a save, a rename, a move or a folder create in
/// `spawn_blocking`, or a delete's synchronous enqueue, which an abort cannot stop once it started. The keep step
/// closes accepts before it reads the queue, each of them reads that in the transaction that queues it, and a
/// sign-out that stops reopens them (spec 2026-10-10 §5.2 step 6, §15 question 3). The caller holds the engine slot,
/// so no new engine can start in between: no engine start reopens accepts between the keep step and the purge.
pub(super) async fn stop_engine_for_sign_out(
    acct: &crate::account::AccountRuntime,
    engine_slot: &mut Option<runner::EngineRunner>,
    upload_staging: Option<&std::path::Path>,
) -> Result<(), String> {
    // Stop the engine before dropping memory so the IPC listener cannot accept
    // new File Provider operations with a cloned master key.
    //
    // Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): `abort()` reports
    // whether the engine's task is CONFIRMED terminated, not just "we asked
    // and waited a bit". The purge below is a cross-account data-
    // exfiltration control (findings 1+2) — it is only safe to run once the
    // OLD engine (and everything nested in its single task: the IPC socket
    // server, the Windows upload watcher) is genuinely gone and can no
    // longer drain or enqueue operations behind its back. If we can't
    // confirm that, refuse to complete sign-out rather than purge anyway
    // and hand the next account's engine a false sense of a clean slate.
    //
    // Bug A2: a refused attempt consumes the engine handle, leaving the
    // slot empty — without the `engine_stop_unconfirmed` flag below, a
    // RETRY would see an idle slot, skip this gate entirely, and purge
    // while the old engine may still be running. The flag (per-account,
    // in-memory) keeps the gate closed until the process restarts, which
    // is exactly what the error message tells the user to do.
    if acct.engine_stop_unconfirmed.load(std::sync::atomic::Ordering::SeqCst) {
        tracing::error!(
            "sign-out refused: a previous attempt could not confirm the sync engine \
             stopped; restart Beebeeb before signing in with a different account"
        );
        return Err(UNCONFIRMED_ENGINE_STOP_ERROR.to_string());
    }
    if let Some(prev) = engine_slot.take() {
        match stop_engine_in_slot(acct, prev).await {
            runner::AbortOutcome::Stopped => {
                tracing::info!("engine aborted on logout");
            }
            runner::AbortOutcome::RevokeFailed { stage, source } => {
                // Bug A misattribution fix: the engine task itself IS
                // confirmed stopped here — what failed is the Windows
                // Cloud Files callback revocation. Say so, instead of
                // blaming the sync engine.
                tracing::error!(
                    stage,
                    %source,
                    "sign-out refused: Cloud Files revocation failed after the engine task itself stopped"
                );
                return Err(format!(
                    "Cloud Files revocation failed ({stage}); the sync engine itself stopped. \
                     Please try signing out again; if this keeps happening, restart Beebeeb \
                     before signing in with a different account. ({source})"
                ));
            }
            runner::AbortOutcome::TaskUnconfirmed => {
                // (`stop_engine_in_slot` has set `engine_stop_unconfirmed`, before the slot is released.)
                tracing::error!(
                    "sign-out refused: could not confirm the sync engine stopped; \
                     refusing to purge local state or clear credentials while it may still be running"
                );
                return Err(UNCONFIRMED_ENGINE_STOP_ERROR.to_string());
            }
        }
    }
    // Every unconfirmed or failed stop returned above: no engine, and so no
    // IPC listener, is left.
    purge_upload_staging_after_engine_stop("sign-out", true, upload_staging);
    Ok(())
}

/// The turn-taking for the purge in the sign-out tests (see `clear_session_impl`). Test builds only.
#[cfg(all(test, not(target_os = "windows")))]
pub(super) static SIGN_OUT_TEST_PURGE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Run `body` holding the gate the sign-out purge takes. For a test that works in the shared scratch state directory
/// itself (it seeds the database before it signs out, and checks it after): from its first write to its last check, no
/// other test's purge runs. Its own sign-out sees the marker and does not wait for the gate it holds.
#[cfg(all(test, not(target_os = "windows")))]
pub(super) async fn with_the_shared_state_dir<T>(body: impl std::future::Future<Output = T>) -> T {
    let _gate = SIGN_OUT_TEST_PURGE_GATE.lock().await;
    PURGE_GATE_HELD.scope((), body).await
}

/// The local-data purge of a sign-out on macOS and Linux, with the state dir passed in (a unit test points it at a
/// throwaway one; `clear_session_impl` passes the app's). Purge the queue, the staged payloads and the cache files,
/// then clear EVERY account row and the owner, so the next sign-in finds a clean database instead of leftovers
/// (fix round 1 of Task 10). `owe_finder_removal`: the sign-out's Finder removal was not confirmed, so a removal is
/// owed before any engine starts. Any failure stops the sign-out: a purge that cannot run (state dir unresolved,
/// database unopenable), one that fails (a file that cannot be removed), a failed clear. A state.db that does not
/// exist yet is a fresh install: nothing to purge.
///
/// `kept` (macOS, spec 2026-10-10 §5.2 step 8): what the keep step kept. The purge refuses before it deletes anything
/// unless every waiting change is in it, and that refusal (`unsent_copy_failed: …`) is returned as it is, so the caller
/// can match its code. Once the rows are cleared, the unfinished sign-out marker is cleared too (W13).
#[cfg(not(target_os = "windows"))]
pub(super) fn purge_local_data_for_sign_out(
    state_dir: Result<PathBuf, String>,
    sync_root: Option<&Path>,
    owe_finder_removal: bool,
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<(), String> {
    match state_dir.and_then(|dir| state_db_from_state_dir(&dir)) {
        Ok(Some(db)) => match purge_local_state_files(&db, sync_root, kept) {
            Ok(summary) => {
                tracing::info!(
                    queued_ops_purged = summary.queued_ops_purged,
                    files_removed = summary.files_removed,
                    files_skipped = summary.files_skipped,
                    windows_placeholders_dehydrated = summary.windows_placeholders_dehydrated,
                    "purged operation queue and decrypted cache on sign-out"
                );
                // Spec 2026-10-10 W13: the rows are cleared, so no write can look missing any more. Best effort
                // (§7.3). The warning names the step that failed, `path`, `read` or `write`, and never an error's
                // text: a read error quotes a line of the file, and the list's lines are kept folders' paths (W10).
                #[cfg(target_os = "macos")]
                if let Err(step) = DesktopConfig::path()
                    .map_err(|_| "path".to_string())
                    .and_then(|path| sign_out::kept_record::clear_unfinished_at(&path))
                {
                    tracing::warn!(step = %step, "could not clear the unfinished sign-out marker");
                }
                if let Err(error) = db.clear_account_data(owe_finder_removal) {
                    tracing::error!(error = %error, "sign-out stopped: the account rows could not be cleared");
                    return Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"));
                }
                Ok(())
            }
            // The guard's refusal is E2 with its code, so the caller can match it (spec §11).
            Err(error) if error.starts_with(&format!("{}: ", sign_out::copy::UNSENT_COPY_FAILED_CODE)) => Err(error),
            Err(error) => {
                tracing::error!(error = %error, "sign-out stopped: the local data could not be purged");
                Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"))
            }
        },
        // No state.db yet: a fresh install that never synced anything. Nothing to purge; not an error.
        Ok(None) => Ok(()),
        Err(error) => {
            tracing::error!(error = %error, "sign-out stopped: the local state database could not be opened");
            Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"))
        }
    }
}

/// Spec 2026-10-10 §5.1 step 1 without the send conditions (the send phase adds them): one read; the earlier folders
/// only when the ask shows a missing write. A failed read of the queue is E3, never a 0. A failed read of the
/// unfinished list leaves the earlier folders out and the ask still counts; its warning names the step that failed,
/// never the error: a read error quotes a line of the file, and the list's lines are kept folders' paths (W10).
#[cfg(target_os = "macos")]
pub(super) fn count_ask(owner: &str) -> Result<sign_out::snapshot::AskSnapshot, String> {
    let e3 = || {
        sign_out::copy::coded(
            sign_out::copy::UNSENT_COUNT_FAILED_CODE,
            sign_out::copy::E3_UNSENT_COUNT_FAILED,
        )
    };
    let waiting = read_waiting_changes().map_err(|_| e3())?;
    let earlier = if waiting.counts().missing_files > 0 {
        DesktopConfig::path()
            .map_err(|_| "path")
            .and_then(|path| sign_out::kept_record::unfinished_at(&path).map_err(|_| "read"))
            .unwrap_or_else(|step| {
                tracing::warn!(
                    step = %step,
                    "the ask names no earlier folder: the unfinished sign-out list could not be read"
                );
                Vec::new()
            })
    } else {
        Vec::new()
    };
    Ok(sign_out::snapshot::AskSnapshot::of(&waiting, owner, earlier))
}

/// The queue as the sign-out counts it, from the app's state dir. No `state.db` yet: nothing waits.
#[cfg(target_os = "macos")]
pub(super) fn read_waiting_changes() -> Result<sign_out::waiting::WaitingChanges, String> {
    match state_db_from_state_dir(&state_paths::beebeeb_state_dir()?)? {
        Some(db) => sign_out::waiting::WaitingChanges::read(&db),
        None => Ok(sign_out::waiting::WaitingChanges::default()),
    }
}

/// §5.1, the contract: `sign_out_count`'s answer. Built on a Mac only; elsewhere the command's twin answers an error.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SignOutCount {
    #[serde(flatten)]
    pub(super) counts: sign_out::snapshot::AskCounts,
    pub(super) can_send: bool,
    pub(super) not_sending: Option<sign_out::waiting::NotSending>,
}

/// §6.1 conditions 1-4 for the active account, and the running engine's phase link.
#[cfg(target_os = "macos")]
pub(super) async fn send_conditions(
    state: &AppState,
) -> (sign_out::waiting::SendConditions, Option<Arc<runner::SendPhaseLink>>) {
    let Ok(acct) = state.active_account() else {
        return (
            sign_out::waiting::SendConditions {
                engine_running: false,
                vault_unlocked: false,
                session_expired: true,
                link_failed: false,
                sync_paused: false,
            },
            None,
        );
    };
    // Running, not merely present: a runner whose task ended at startup leaves its handle in the slot, and that must
    // not advertise a send nobody would run (the count then goes straight to the engine-stopped ask).
    let link = acct
        .engine
        .lock()
        .await
        .as_ref()
        .filter(|engine| engine.is_running())
        .map(|engine| engine.send_phase());
    let conditions = sign_out::waiting::SendConditions {
        engine_running: link.is_some(),
        vault_unlocked: acct.session.lock().map(|session| session.is_some()).unwrap_or(false),
        session_expired: acct.auth_health.is_expired(),
        link_failed: link.as_ref().is_some_and(|link| link.link_failed()),
        sync_paused: acct.sync_paused.load(Ordering::Relaxed),
    };
    (conditions, link)
}

/// §5.1 step 1: one read, a snapshot owned by the calling window, and one `info` line with counts only (review I8).
#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) async fn sign_out_count(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<SignOutCount, String> {
    let snapshot = count_ask(window.label())?;
    let (conditions, _) = send_conditions(&state).await;
    let not_sending = sign_out::waiting::readiness(conditions, snapshot.is_empty(), snapshot.any_live);
    let can_send = !snapshot.is_empty() && not_sending.is_none();
    tracing::info!(
        files = snapshot.counts.files,
        copied_files = snapshot.counts.copied_files,
        others = snapshot.counts.others,
        missing_files = snapshot.counts.missing_files,
        can_send,
        not_sending = not_sending.map(|reason| format!("{reason:?}")).unwrap_or_default(),
        earlier_folder = !snapshot.counts.earlier_folders.is_empty(),
        "sign-out flow counted"
    );
    let counts = snapshot.counts.clone();
    state.sign_out.snapshots.record(snapshot);
    Ok(SignOutCount {
        counts,
        can_send,
        not_sending,
    })
}

/// Windows and Linux keep today's sign-out (spec 2026-10-10 W5): the flow's commands are registered everywhere,
/// because `generate_handler!` takes no `cfg`, and say so off a Mac.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) async fn sign_out_count(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<SignOutCount, String> {
    let _ = (window, state);
    Err("This works on a Mac only.".into())
}

/// W11: on macOS the switch step renders a snapshot, owned by the window the step runs in: the Beebeeb window (`main`),
/// which the app destroys on a close, so a close drops it (task 1747 Part 1, review P3-5 on #143; it was owned by the label
/// `onboarding`, a window a Mac no longer builds). Its one caller, `settle_sign_in`, is not built on Windows.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(super) fn switch_snapshot(state: &AppState) -> Result<Option<sign_out::snapshot::AskCounts>, String> {
    #[cfg(target_os = "macos")]
    {
        let snapshot = count_ask(surfaces::policy::main_window_label(
            surfaces::policy::Platform::current(),
        ))?;
        // §6.4 (review I8): the switch's snapshot logs the same line as `sign_out_count`; it never sends (W11).
        tracing::info!(
            files = snapshot.counts.files,
            copied_files = snapshot.counts.copied_files,
            others = snapshot.counts.others,
            missing_files = snapshot.counts.missing_files,
            can_send = false,
            not_sending = "",
            earlier_folder = !snapshot.counts.earlier_folders.is_empty(),
            "sign-out flow counted"
        );
        let counts = snapshot.counts.clone();
        state.sign_out.snapshots.record(snapshot);
        Ok(Some(counts))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Ok(None)
    }
}

/// What the driver needs of a started phase.
#[cfg(target_os = "macos")]
pub(super) struct PhaseRun {
    pub(super) owner: String,
    pub(super) link: Option<Arc<runner::SendPhaseLink>>,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) started: std::time::Instant,
    pub(super) changes_at_start: usize,
}

/// §6.2: a stale ask refuses; a running phase starts nothing; otherwise make live changes due, arm, and register.
#[cfg(target_os = "macos")]
pub(super) fn start_send_phase(
    sign_out: &sign_out::SignOutState,
    ask_id: &str,
    owner: &str,
    link: Option<Arc<runner::SendPhaseLink>>,
) -> Result<Option<PhaseRun>, String> {
    if sign_out.snapshots.newest_ask_id().as_deref() != Some(ask_id) {
        return Err(sign_out::copy::coded(
            sign_out::copy::UNSENT_CHANGES_CODE,
            sign_out::copy::E1_UNSENT_CHANGES,
        ));
    }
    let mut phase = sign_out.phase.lock().unwrap_or_else(|p| p.into_inner());
    if phase.is_some() {
        return Ok(None);
    }
    let changes_at_start = read_waiting_changes().map(|waiting| waiting.changes()).unwrap_or(0);
    if let Ok(Some(db)) = state_paths::beebeeb_state_dir().and_then(|dir| state_db_from_state_dir(&dir)) {
        let _ = db.make_due_for_send_phase(now_unix_seconds());
    }
    if let Some(link) = &link {
        link.arm();
    }
    let (stop, closed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    *phase = Some(sign_out::send_phase::RunningPhase {
        ask_id: ask_id.to_string(),
        owner: owner.to_string(),
        stop: stop.clone(),
        closed: closed.clone(),
    });
    Ok(Some(PhaseRun {
        owner: owner.to_string(),
        link,
        stop,
        closed,
        started: std::time::Instant::now(),
        changes_at_start,
    }))
}

/// The phase is over, for any reason: disarm, so the runner is back on its tick within a second.
#[cfg(target_os = "macos")]
pub(super) fn end_send_phase(sign_out: &sign_out::SignOutState, link: Option<&runner::SendPhaseLink>) {
    if let Some(link) = link {
        link.disarm();
    }
    *sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) async fn sign_out_send_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    ask_id: String,
) -> Result<(), String> {
    let (_, link) = send_conditions(&state).await;
    if let Some(run) = start_send_phase(&state.sign_out, &ask_id, window.label(), link)? {
        tauri::async_runtime::spawn(drive_send_phase(app, run));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) async fn sign_out_send_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    ask_id: String,
) -> Result<(), String> {
    let _ = (app, window, state, ask_id);
    Err("This works on a Mac only.".into())
}

/// "Stop waiting" (and Esc): the phase ends `stopped`. Nothing happens when no phase runs.
#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) fn sign_out_send_stop(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(phase) = state.sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        phase.stop.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(super) fn sign_out_send_stop(state: State<'_, AppState>) -> Result<(), String> {
    let _ = state;
    Err("This works on a Mac only.".into())
}

/// One observation of the world for the phase (§6.4): the queue as read, the account's conditions, and what the
/// phase's attempts met on its link. The driver's own step, split out so a test can run it with no window.
#[cfg(target_os = "macos")]
pub(super) fn observe_phase(
    run: &PhaseRun,
    waiting: &sign_out::waiting::WaitingChanges,
    conditions: sign_out::waiting::SendConditions,
) -> sign_out::send_phase::PhaseObservation {
    sign_out::send_phase::PhaseObservation {
        elapsed: run.started.elapsed(),
        changes_left: waiting.changes(),
        any_live: waiting.any_live(),
        engine_running: run.link.is_some() && conditions.engine_running,
        session_expired: conditions.session_expired,
        link_failed: conditions.link_failed,
        attempt: run.link.as_ref().map(|link| link.signals()).unwrap_or_default(),
        stop_requested: run.stop.load(Ordering::SeqCst),
        window_closed: run.closed.load(Ordering::SeqCst),
    }
}

/// The driver: once a second, wake the runner's queue step, read the world, and end or report progress.
#[cfg(target_os = "macos")]
pub(super) async fn drive_send_phase(app: tauri::AppHandle, run: PhaseRun) {
    let state = app.state::<AppState>();
    let mut phase = sign_out::send_phase::SendPhase::default();
    let mut changes_left = run.changes_at_start;
    let end = loop {
        if let Some(link) = &run.link {
            link.wake();
        }
        // `std::time::Duration`: `lib.rs` imports none at the crate level.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let Ok(waiting) = read_waiting_changes() else {
            break sign_out::send_phase::PhaseEnd::Stopped;
        };
        changes_left = waiting.changes();
        let (conditions, _) = send_conditions(&state).await;
        let observation = observe_phase(&run, &waiting, conditions);
        match phase.observe(&observation) {
            sign_out::send_phase::PhaseStep::Continue => {}
            sign_out::send_phase::PhaseStep::Progress(left) => {
                let _ = app.emit_to(
                    run.owner.as_str(),
                    "sign-out-send-progress",
                    serde_json::json!({ "changesLeft": left }),
                );
            }
            sign_out::send_phase::PhaseStep::End(end) => break end,
        }
    };
    end_send_phase(&state.sign_out, run.link.as_deref());
    use sign_out::send_phase::PhaseEnd;
    let ask = match end {
        PhaseEnd::AllSent | PhaseEnd::Closed => None,
        _ => count_ask(&run.owner).ok().map(|snapshot| {
            let counts = snapshot.counts.clone();
            state.sign_out.snapshots.record(snapshot);
            counts
        }),
    };
    tracing::info!(
        reason = end.as_str(),
        changes_at_start = run.changes_at_start,
        changes_left,
        elapsed_ms = run.started.elapsed().as_millis() as u64,
        "sign-out send phase ended"
    );
    let _ = app.emit_to(
        run.owner.as_str(),
        "sign-out-send-ended",
        serde_json::json!({ "reason": end, "ask": ask }),
    );
}

/// W5, the gate: one read compared with the snapshot `keep_unsent` names.
#[cfg(target_os = "macos")]
pub(super) fn gate_sign_out(
    state: &AppState,
    keep_unsent: Option<&str>,
) -> Result<Option<sign_out::snapshot::AskSnapshot>, SignOutFailure> {
    let waiting = read_waiting_changes().map_err(|_| {
        SignOutFailure::from(sign_out::copy::coded(
            sign_out::copy::UNSENT_COUNT_FAILED_CODE,
            sign_out::copy::E3_UNSENT_COUNT_FAILED,
        ))
    })?;
    state.sign_out.snapshots.gate(&waiting, keep_unsent).map_err(|_| {
        SignOutFailure::from(sign_out::copy::coded(
            sign_out::copy::UNSENT_CHANGES_CODE,
            sign_out::copy::E1_UNSENT_CHANGES,
        ))
    })
}

/// Where the kept folder goes: the container's `Documents/Changes not sent` (§7.1). A test build writes under its
/// per-process sandbox, never the person's folders.
#[cfg(target_os = "macos")]
pub(super) fn sign_out_kept_root() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        crate::test_sandbox::dir("kept")
            .map(|dir| dir.join("Changes not sent"))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(test))]
    {
        sign_out::kept_names::kept_root_for_this_app(crate::APP).ok_or_else(|| "no folder for kept changes".to_string())
    }
}

/// Who runs the keep step. A failed copy's log line names it: an R10 reset is not a sign-out (W12).
#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
pub(super) enum KeepCaller {
    /// §5.2 step 7: a sign-out by choice.
    SignOut,
    /// W12: spec A's R10 reset before an engine starts.
    Reset,
}

/// The keep step on `db` (§5.2 step 7, W12): the kept root, the config, the clone and the durable save are the app's.
/// A failed copy is logged by its category, never a path (W10), in the caller's own line.
#[cfg(target_os = "macos")]
pub(super) fn keep_in(
    db: &state_db::StateDb,
    acknowledge: sign_out::keep::Acknowledge<'_>,
    caller: KeepCaller,
) -> Result<sign_out::kept_set::KeptSet, String> {
    let failed = || {
        sign_out::copy::coded(
            sign_out::copy::UNSENT_COPY_FAILED_CODE,
            sign_out::copy::E2_UNSENT_COPY_FAILED,
        )
    };
    let root = sign_out_kept_root().map_err(|_| failed())?;
    let config = DesktopConfig::path().map_err(|_| failed())?;
    sign_out::keep::keep_unsent(sign_out::keep::KeepInputs {
        db,
        acknowledge,
        root: &root,
        event_name: sign_out::kept_names::event_folder_name(chrono::Local::now()),
        config_path: &config,
        clone: &sign_out::kept_writer::SystemClone,
        config_fs: &config::RealConfigFs,
    })
    .map_err(|failure| {
        if let sign_out::keep::KeepFailure::Copy(reason) = &failure {
            match caller {
                KeepCaller::SignOut => tracing::error!(
                    reason,
                    "sign-out stopped: the waiting changes could not be kept; nothing was purged"
                ),
                KeepCaller::Reset => tracing::error!(
                    reason,
                    "R10: local data reset stopped: the waiting changes could not be kept; nothing was purged"
                ),
            }
        }
        failure.message()
    })
}

/// §5.2 step 7: the keep step on the app's state dir. `None`: no `state.db` yet, nothing to keep. A state dir that does
/// not resolve, or a `state.db` that does not open, means the queue cannot be read: E3 with its code, as a failed read
/// inside the keep step is. Its log names the step, never the error, whose text names the database's path (W10).
#[cfg(target_os = "macos")]
pub(super) fn keep_for_sign_out(
    accepted: Option<&sign_out::snapshot::AskSnapshot>,
) -> Result<Option<sign_out::kept_set::KeptSet>, String> {
    use sign_out::keep::Acknowledge;
    let opened = state_paths::beebeeb_state_dir()
        .map_err(|_| "state_dir")
        .and_then(|dir| state_db_from_state_dir(&dir).map_err(|_| "state_db"));
    let db = match opened {
        Ok(Some(db)) => db,
        Ok(None) => return Ok(None),
        Err(step) => {
            tracing::error!(
                step = %step,
                "sign-out stopped: the waiting changes could not be read; nothing was purged"
            );
            return Err(sign_out::copy::coded(
                sign_out::copy::UNSENT_COUNT_FAILED_CODE,
                sign_out::copy::E3_UNSENT_COUNT_FAILED,
            ));
        }
    };
    let acknowledge = match accepted {
        Some(snapshot) => Acknowledge::Asked {
            others: &snapshot.others,
            missing: &snapshot.missing,
        },
        None => Acknowledge::Nothing,
    };
    keep_in(&db, acknowledge, KeepCaller::SignOut).map(Some)
}

/// Spec 2026-10-10 §15 question 3: reopen Finder accepts in the app's state database. Best effort: an engine start
/// reopens them too (`engine_start_repair`). No `state.db` yet: nothing was closed. One that does not open is logged
/// by its step, never by the error, whose text names the database's path (W10).
#[cfg(target_os = "macos")]
pub(super) fn reopen_finder_accepts_in(state_dir: &Path) {
    let db = match state_db_from_state_dir(state_dir) {
        Ok(Some(db)) => db,
        Ok(None) => return,
        Err(_) => {
            accepts_not_reopened("state_db");
            return;
        }
    };
    if let Err(error) = db.reopen_finder_accepts() {
        tracing::warn!(%error, "could not reopen Finder accepts; the next engine start reopens them");
    }
}

/// A sign-out quit between its keep step and its purge left accepts closed on disk: the launch reopens them first.
#[cfg(target_os = "macos")]
pub(super) fn reopen_finder_accepts_at_startup() {
    match state_paths::beebeeb_state_dir() {
        Ok(state_dir) => reopen_finder_accepts_in(&state_dir),
        Err(_) => accepts_not_reopened("state_dir"),
    }
}

/// A reopen that could not reach the state database: the step only (W10).
#[cfg(target_os = "macos")]
pub(super) fn accepts_not_reopened(step: &'static str) {
    tracing::warn!(
        step = %step,
        "could not reopen Finder accepts; the next engine start reopens them"
    );
}

/// From a completed keep step until the purge has cleared the rows (and the key with them), any return reopens Finder
/// accepts at once: a refused or failed purge, a test seam, a panic.
#[cfg(target_os = "macos")]
pub(super) struct AcceptsReopenOnAbort {
    pub(super) armed: bool,
}

#[cfg(target_os = "macos")]
impl AcceptsReopenOnAbort {
    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(target_os = "macos")]
impl Drop for AcceptsReopenOnAbort {
    fn drop(&mut self) {
        if self.armed
            && let Ok(state_dir) = state_paths::beebeeb_state_dir()
        {
            reopen_finder_accepts_in(&state_dir);
        }
    }
}

/// Test builds only: a hook between the steps of `clear_session_impl`, per test task (`SIGN_OUT_SEAM.scope`). An
/// `Err` stops the sign-out at that point with that message.
#[cfg(test)]
pub(super) type SignOutSeam = Arc<dyn Fn(&'static str) -> Result<(), String> + Send + Sync>;

#[cfg(test)]
pub(super) fn sign_out_seam(point: &'static str) -> Result<(), String> {
    SIGN_OUT_SEAM.try_with(|seam| seam(point)).unwrap_or(Ok(()))
}

/// R10: on macOS and Linux a sign-out whose local-data purge fails stops instead of completing, as
/// Windows already does. The engine is already stopped, so trying again is safe.
#[cfg(not(target_os = "windows"))]
pub(super) const SIGN_OUT_PURGE_FAILED: &str = "Sign-out paused: Beebeeb couldn’t clear this computer’s local data. Try again; if it keeps happening, restart Beebeeb.";
