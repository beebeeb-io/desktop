//! `session_reauth` items split out of `lib.rs`.

use super::*;

/// What a successful authentication became on this Mac (R8).
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SignInSettlement {
    Fresh,
    Reauthenticated {
        vault_unlocked: bool,
        /// The vault key this Mac kept was no longer the account's (spec §5.6): it was removed, and the recovery phrase
        /// step follows (`vault_unlocked` is false).
        key_replaced: bool,
    },
    AccountMismatch {
        pending_changes: u64,
        /// On a Mac, the ask snapshot the switch step renders (spec 2026-10-10 W11); elsewhere `None`.
        unsent: Option<sign_out::snapshot::AskCounts>,
        /// On a Mac, the coded E3 when the switch's count could not be read (§5.3 row 1).
        unsent_error: Option<String>,
    },
    /// The server could not say whose key this is: nothing changed, and the new session is revoked.
    Unconfirmed,
}

/// A same-account re-sign-in that failed, and whether the new token had already been written to the Keychain by
/// then. Before that nothing of the new session is stored, so the caller revokes it (as it does for a first
/// sign-in whose token could not be stored); after that the new token is the live one and is kept.
#[cfg(not(target_os = "windows"))]
#[derive(Debug)]
pub(super) struct ReauthError {
    pub(super) message: String,
    pub(super) token_stored: bool,
}

#[cfg(not(target_os = "windows"))]
impl ReauthError {
    pub(super) fn before_store(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            token_stored: false,
        }
    }

    pub(super) fn after_store(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            token_stored: true,
        }
    }
}

/// R8: before anything is stored, decide whether this sign-in is fresh, the same account again, or a different
/// account. The same account gets only a new token (`reauth_in_place`), and keeps the vault key this Mac kept only once
/// the server has confirmed it; a key the server no longer accepts is removed instead (`reauth_without_the_kept_key`,
/// spec §5.6). A different account changes nothing here: the caller revokes the new session and returns the warning. A
/// sign-in the server could not place is `Unconfirmed`: nothing changes either. Never reads key material to decide (the
/// one proof reads the kept key into a wiped value). `Fresh` requires that nothing of a previous account remains
/// (`gather_local_facts`). `handoff_key`: the vault key a browser sign-in handed over (`None` for a password sign-in).
/// Not on Windows, where sign-in refuses while a session exists.
#[cfg(not(target_os = "windows"))]
pub(super) async fn settle_sign_in(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    token: &str,
    profile: &account_dto::AccountProfile,
    handoff_key: Option<&[u8; 32]>,
    turn: &mut account::SessionGeneration,
) -> Result<SignInSettlement, ReauthError> {
    let acct = state.active_account().map_err(ReauthError::before_store)?;
    let sources = LocalSources::production();
    let (kind, pending_changes) =
        decide_sign_in_in_turn(state, &acct, &sources, profile, token, &runner::api_base_url(), turn).await?;
    match kind {
        reauth::SignInKind::Fresh => Ok(SignInSettlement::Fresh),
        reauth::SignInKind::DifferentAccount => Ok(switch_settlement(pending_changes, switch_snapshot(state))),
        reauth::SignInKind::NeedsKeyProof | reauth::SignInKind::Unconfirmed => Ok(SignInSettlement::Unconfirmed),
        // Lead ruling 12: this computer's own record of its account could not be read. Its own sentence, not the
        // "connect to the internet" one; nothing is stored, so the caller revokes the new session.
        reauth::SignInKind::LocalDataUnreadable => Err(ReauthError::before_store(SIGN_IN_LOCAL_DATA_UNREADABLE)),
        reauth::SignInKind::SameAccount => {
            let vault_unlocked =
                reauth_in_place(app, state, &acct, &sources, token, profile, handoff_key, turn).await?;
            Ok(SignInSettlement::Reauthenticated {
                vault_unlocked,
                key_replaced: false,
            })
        }
        reauth::SignInKind::KeyOutdated => {
            reauth_without_the_kept_key(state, &acct, &sources, token, profile, turn).await
        }
    }
}

/// W11 and §5.3 row 1 (spec 2026-10-10): the switch settles with what its count found: a snapshot (macOS), none
/// (elsewhere), or E3 for the switch step to show as a line. A count that cannot be read never fails the sign-in:
/// nothing changed, and "Sign out and switch" counts again through the gate.
#[cfg(not(target_os = "windows"))]
pub(super) fn switch_settlement(
    pending_changes: u64,
    counted: Result<Option<sign_out::snapshot::AskCounts>, String>,
) -> SignInSettlement {
    match counted {
        Ok(unsent) => SignInSettlement::AccountMismatch {
            pending_changes,
            unsent,
            unsent_error: None,
        },
        Err(error) => SignInSettlement::AccountMismatch {
            pending_changes,
            unsent: None,
            unsent_error: Some(error),
        },
    }
}

/// R8, same account: replace ONLY the session token. The queue, the cache, the Finder domain and the vault key are
/// never touched (`reauth_tests` pins it). Keys already in memory stay. Otherwise the key this Mac still holds in the
/// Keychain is loaded (a relaunch after a startup 401). The owner record is rewritten first, with the account's user
/// id and the server's spelling of its email (a legacy record gets both here); if that fails nothing else has
/// changed. Returns whether the vault is unlocked afterwards; `false` sends the person to the recovery phrase.
/// `handoff_key`: the vault key a browser sign-in handed over (`None` for a password sign-in). On a Mac, a sign-in
/// that unlocks asks to resume the edits paused for `auth`, unless the handed-over key replaces the key now in memory:
/// that is decided here, before the ask, because the edits are then held (`key_replaced`) and never sent (F9 re-review
/// N-1).
#[cfg(not(target_os = "windows"))]
pub(super) fn reauth_swap_token(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    handoff_key: Option<&[u8; 32]>,
    turn: &mut account::SessionGeneration,
) -> Result<bool, ReauthError> {
    let email = session_email(Some(profile));
    // The re-sign-in's turn (lead ruling 1, Task 12): a Lock, a Sign-out or another sign-in that completed since this
    // sign-in began stops it here, before anything is written; everything below is written in this one turn.
    let write = claim_session_write(acct, turn).map_err(ReauthError::before_store)?;
    // R10: the same account (decided by the caller) owns the local data.
    record_local_data_owner(
        sources,
        &account_binding::Identity::new(Some(&profile.user_id), email.as_deref()),
    )
    .map_err(ReauthError::before_store)?;
    persist_session_token_to_keychain(&write, acct.id.as_str(), token, email.as_deref())
        .map_err(ReauthError::before_store)?;
    // Task 2447: the new token's owner, as the server named it for this sign-in.
    bind_session_owner_in_keychain(&write, acct.id.as_str(), &profile.user_id);
    // From here the new token is the stored one: a failure below is not a sign-in to revoke.
    let mut vault_unlocked = replace_session_token_in_memory(&write, acct, token).map_err(ReauthError::after_store)?;
    if vault_unlocked {
        if let Some(email) = email.as_deref() {
            name_session_in_memory(&write, acct, email).map_err(ReauthError::after_store)?;
        }
    } else {
        match load_session_from_keychain(acct.id.as_str(), email.clone()) {
            Ok(Some(session)) => {
                install_unlocked_session(&write, acct, session).map_err(ReauthError::after_store)?;
                vault_unlocked = true;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::info!(%error, "re-sign-in: no vault key on this Mac; the recovery phrase step follows")
            }
        }
    }
    if let Ok(mut cached) = acct.cached_profile.lock() {
        *cached = Some(profile.clone());
    }
    reauth_settle_flags(&write, state, acct, email.as_deref().unwrap_or_default());
    // Ruling F9: unlocked, so sync resumes; on Linux too since spec D11 (task 1888; Windows does not build this).
    // Without keys here the recovery-phrase unlock asks instead. F9 re-review N-1: a browser sign-in whose handed-over
    // key replaces the kept one is decided here, before the ask, and does not ask: `reauth_in_place` holds what is
    // queued under the kept key as `key_replaced`, and a hold that fails must not leave an open request for a later
    // start to serve with that key. The handoff's own install (`apply_session`) asks once the new key is in memory.
    if vault_unlocked && !handoff_replaces_kept_key(acct, handoff_key) {
        ask_to_resume_auth_paused(&write, acct);
    }
    drop(write);
    Ok(vault_unlocked)
}

/// An engine that did not start after a successful re-sign-in is not a failed sign-in: the session is replaced and the
/// person is signed in. The failure goes where every engine failure goes: the engine status (`sync_status` reads
/// "error") and the Finder reconciler's check that `keys_arrived` asks for, which retries the start and reports it.
#[cfg(not(target_os = "windows"))]
pub(super) fn note_engine_start_failure(acct: &AccountRuntime, error: &str) {
    tracing::warn!(%error, "re-sign-in: signed in, but the sync engine did not start");
    if let Ok(mut guard) = acct.engine_state.lock() {
        *guard = "error".to_string();
    }
}

/// R8, same account: the token swap above, then, with the keys in memory, a new engine with the new token (the queue in
/// `state.db` is kept and drains) and the reconciler's one "keys are here" trigger. The engine's own failure does not
/// fail the sign-in (see [`note_engine_start_failure`]), so a retry never mints a second server session for an account
/// that is already signed in. A browser sign-in hands over the account's current key (`handoff_key`): when the key in
/// memory differs from it (compared in constant time), no engine starts here and this returns `false`, so the
/// handoff installs its own key (`apply_session`), which starts the engine. On a Mac a browser sign-in swaps the token
/// while holding the engine slot, and when its key replaces the kept one, the outgoing engine stops and the edits queued
/// under the kept key are held (`key_replaced`) before the slot is released (F9 review M2, lead ruling I-1). A hold that
/// cannot be written fails the sign-in, with the new token already stored. The swap of such a sign-in opened no resume
/// request (F9 re-review N-1), so a failed hold leaves none open for a start in this process to serve.
#[cfg(not(target_os = "windows"))]
#[allow(clippy::too_many_arguments)]
pub(super) async fn reauth_in_place(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    handoff_key: Option<&[u8; 32]>,
    turn: &mut account::SessionGeneration,
) -> Result<bool, ReauthError> {
    // F9 review M2 (lead ruling I-1): on a Mac, a browser sign-in swaps the token while holding the engine slot, so no
    // start serves the swap's ask before the handed-over key is compared with the kept one below.
    #[cfg(target_os = "macos")]
    let mut engine_slot = if handoff_key.is_some() {
        Some(acct.engine.lock().await)
    } else {
        None
    };
    let vault_unlocked = reauth_swap_token(state, acct, sources, token, profile, handoff_key, turn)?;
    if vault_unlocked && handoff_replaces_kept_key(acct, handoff_key) {
        tracing::info!("browser sign-in: the vault key this Mac kept is not the one handed over; installing that one");
        // Ruling I-1: nothing queued under the kept key is sent. Held before the slot is released.
        #[cfg(target_os = "macos")]
        if let Some(slot) = engine_slot.as_mut() {
            stop_the_outgoing_engine_and_hold_auth_paused(acct, sources, slot)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "browser sign-in: the operations queued under the kept key could not be held");
                    ReauthError::after_store(SIGN_IN_LOCAL_DATA_UNREADABLE)
                })?;
        }
        return Ok(false);
    }
    #[cfg(target_os = "macos")]
    drop(engine_slot);
    if vault_unlocked {
        let started = start_engine_if_possible(app, state).await;
        keys_arrived(state, KeysFrom::SignIn);
        if let Err(error) = started {
            note_engine_start_failure(acct, &error);
        }
    }
    tracing::info!(vault_unlocked, "signed in again in place (R8)");
    Ok(vault_unlocked)
}

/// Constant-time equality of two vault keys: every byte is compared, wherever the first difference is.
#[cfg(not(target_os = "windows"))]
pub(super) fn vault_keys_equal(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let difference = a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(difference) == 0
}

/// Does the session in memory hold exactly `key`? Compared in constant time, under the session lock. No session, or a
/// lock that cannot be read: `false`.
#[cfg(not(target_os = "windows"))]
pub(super) fn session_holds_vault_key(acct: &AccountRuntime, key: &[u8; 32]) -> bool {
    acct.session
        .lock()
        .map(|guard| {
            guard
                .as_ref()
                .is_some_and(|session| vault_keys_equal(&session.master_key, key))
        })
        .unwrap_or(false)
}

/// A browser sign-in handed over `handoff_key`, the account's current key (spec §5.6). Does it replace what the session
/// in memory holds? Yes unless memory holds exactly that key. `None` (a password sign-in hands over no key): no.
#[cfg(not(target_os = "windows"))]
pub(super) fn handoff_replaces_kept_key(acct: &AccountRuntime, handoff_key: Option<&[u8; 32]>) -> bool {
    handoff_key.is_some_and(|handoff| !session_holds_vault_key(acct, handoff))
}

/// What a re-sign-in says when the outdated vault key could not be removed from this Mac. The new session is revoked
/// and nothing is installed; trying again is safe.
#[cfg(not(target_os = "windows"))]
pub(super) const SIGN_IN_KEY_NOT_REMOVED: &str =
    "Beebeeb couldn’t remove this account’s old vault key from this computer, so nothing changed. Try again.";

/// Remove the vault key, and only the key, from both Keychain layouts (the id-keyed and the legacy one), then check
/// that neither still holds one. Only inside a checked turn (`_write`). A key that is not there, or a store that cannot
/// hold secrets, is no failure; a key that cannot be removed, or still reads as present, is.
#[cfg(not(target_os = "windows"))]
pub(super) fn delete_vault_key_from_keychain(_write: &SessionWrite, account_id: &str) -> Result<(), String> {
    use keychain::AuthSecretStore as _;
    for removed in [
        platform_keychain_store_for(account_id).delete_wrapped_master_key(),
        keychain::legacy_platform_keychain_store().delete_wrapped_master_key(),
    ] {
        match removed {
            Ok(()) | Err(keychain::AuthStoreError::NotFound | keychain::AuthStoreError::Unsupported(_)) => {}
            Err(error) => {
                tracing::warn!(error = %error, "re-sign-in: the outdated vault key could not be removed");
                return Err(SIGN_IN_KEY_NOT_REMOVED.to_string());
            }
        }
    }
    if keychain_vault_key_present(account_id) {
        return Err(SIGN_IN_KEY_NOT_REMOVED.to_string());
    }
    Ok(())
}

/// [`reauth_without_the_kept_key`]'s writes, in the sign-in's turn and under the engine slot (`_engine_slot`), so no
/// engine start can read keys in between (every start reads its keys under the slot). The session in memory is cleared
/// first, as Lock clears it; then the owner is recorded, the outdated key is removed from both Keychain layouts, and
/// the new token is stored with the account's email and no key. A failure up to and including the token write is
/// `before_store` (the caller revokes the new session); the memory clear stands either way.
#[cfg(not(target_os = "windows"))]
pub(super) fn reauth_swap_token_without_key(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    turn: &mut account::SessionGeneration,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
) -> Result<(), ReauthError> {
    let email = session_email(Some(profile));
    let write = claim_session_write(acct, turn).map_err(ReauthError::before_store)?;
    match acct.session.lock() {
        Ok(mut guard) => drop(guard.take()),
        Err(poisoned) => {
            drop(poisoned.into_inner().take());
            acct.session.clear_poison();
        }
    }
    record_local_data_owner(
        sources,
        &account_binding::Identity::new(Some(&profile.user_id), email.as_deref()),
    )
    .map_err(ReauthError::before_store)?;
    // Lead ruling, F9 review I-1: in this turn, under this slot, and before the key is removed, the edits queued under
    // that key are held (kept, never sent). If they cannot be, nothing more changes and the sign-in can be tried again.
    #[cfg(target_os = "macos")]
    if let Err(error) = hold_auth_paused_for_the_replaced_key(sources, _engine_slot) {
        tracing::warn!(%error, "re-sign-in: the operations queued under the replaced key could not be held");
        return Err(ReauthError::before_store(SIGN_IN_LOCAL_DATA_UNREADABLE));
    }
    delete_vault_key_from_keychain(&write, acct.id.as_str()).map_err(ReauthError::before_store)?;
    persist_session_token_to_keychain(&write, acct.id.as_str(), token, email.as_deref())
        .map_err(ReauthError::before_store)?;
    // Task 2447: the new token's owner, as the server named it for this sign-in.
    bind_session_owner_in_keychain(&write, acct.id.as_str(), &profile.user_id);
    if let Ok(mut cached) = acct.cached_profile.lock() {
        *cached = Some(profile.clone());
    }
    reauth_settle_flags(&write, state, acct, email.as_deref().unwrap_or_default());
    drop(write);
    Ok(())
}

/// R8, same account, but the server says the vault key this Mac kept is no longer the account's: it was changed on
/// another device (spec §5.6). That key must never reach an engine again. The sign-in's turn is taken first, so every
/// transition that began before it is refused from now on, as a Lock's is. A session in memory is then ended the way
/// Lock ends it: on a Mac the reconciler cancels any check and holds (it never removes the domain), and the engine
/// stops under its slot. Still under the slot, [`reauth_swap_token_without_key`] clears memory, removes the key and
/// stores the new token; on a Mac it first holds the edits queued under that key (`key_replaced`: kept, never sent; lead
/// ruling, F9 review I-1). The queue, the cache and Finder stay; the recovery phrase step follows (`vault_unlocked` is
/// false and `key_replaced` says why), and its server check stores the current key.
#[cfg(not(target_os = "windows"))]
pub(super) async fn reauth_without_the_kept_key(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    turn: &mut account::SessionGeneration,
) -> Result<SignInSettlement, ReauthError> {
    drop(claim_session_write(acct, turn).map_err(ReauthError::before_store)?);
    let keys_in_memory = acct.session.lock().map(|guard| guard.is_some()).unwrap_or(true);
    #[cfg(target_os = "macos")]
    if keys_in_memory && let Err(error) = finder_lock_for(state).await {
        tracing::warn!(%error, "the Finder reconciler did not acknowledge the hold; removing the outdated key anyway");
    }
    let mut engine_slot = acct.engine.lock().await;
    if let Some(engine) = engine_slot.take()
        && !stop_engine_in_slot(acct, engine).await.is_stopped()
    {
        tracing::error!("the sync engine did not confirm it stopped before the outdated vault key was removed");
    }
    if keys_in_memory {
        purge_macos_hydrate_cache("key-outdated");
    }
    let swapped = reauth_swap_token_without_key(state, acct, sources, token, profile, turn, &engine_slot);
    drop(engine_slot);
    if let Ok(mut engine_state) = acct.engine_state.lock() {
        *engine_state = "stopped".to_string();
    }
    if keys_in_memory {
        bump_vault_epoch();
    }
    tracing::info!(
        stored = swapped.is_ok(),
        "signed in again in place without the vault key this Mac kept (spec §5.6)"
    );
    swapped.map(|()| SignInSettlement::Reauthenticated {
        vault_unlocked: false,
        key_replaced: true,
    })
}

/// How long Lock waits for the reconciler to acknowledge it (spec §5.3 (3); lead ruling T8-⚠lock).
///
/// The reconciler handles an event only between operations, so the wait must outlast the longest one
/// it can be inside. That is a `StartEngine` whose socket never comes up: it waits for the engine slot
/// (up to 15 s), then the socket (3 s), then stops the engine it started: the slot again (15 s) and
/// `abort()`'s own 3 s + 2 s windows. 15 + 3 + 15 + 5 = 38 s. A shorter wait would make a slow but
/// working check look like a lock the reconciler never acknowledged.
/// `the_acknowledgement_timeouts_outlast_the_longest_operation_they_can_wait_behind` pins the sum.
#[cfg(target_os = "macos")]
pub(super) const FINDER_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// How long Sign-out and Repair wait for the reconciler's removal: the longest operation it can be
/// inside (38 s, see [`FINDER_LOCK_TIMEOUT`]) and then its own `RemoveDomain` (15 s) = 53 s.
#[cfg(target_os = "macos")]
pub(super) const FINDER_REMOVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(all(test, target_os = "macos"))]
pub(super) fn signal_lock_holds_the_engine_slot() {
    let _ = LOCK_HOLDS_THE_ENGINE_SLOT.try_with(|signal| signal.notify_one());
}

#[cfg(all(test, target_os = "macos"))]
pub(super) fn signal_sign_out_holds_the_engine_slot() {
    let _ = SIGN_OUT_HOLDS_THE_ENGINE_SLOT.try_with(|signal| signal.send(()));
}

#[cfg(target_os = "macos")]
pub(super) fn finder_wait(production: std::time::Duration) -> std::time::Duration {
    #[cfg(test)]
    if let Ok(short) = FINDER_WAIT_OVERRIDE.try_with(|wait| *wait) {
        return short;
    }
    production
}

/// Shown when Lock could not confirm the reconciler stopped (it did not acknowledge in time, or it has
/// stopped). The keys are already cleared and the engine stopped; the sentence says only what is
/// unconfirmed. No OS text, no path (lead ruling T1-4). A warning on a Lock that happened (FB-24).
pub(super) const FINDER_LOCK_UNCONFIRMED_WARNING: &str =
    "The vault is locked, but Beebeeb could not confirm that its Finder setup stopped. Restart Beebeeb to be sure.";

/// The same for Sign-out: signed out, keys cleared, but the removal from Finder is unconfirmed.
pub(super) const FINDER_SIGN_OUT_UNCONFIRMED_WARNING: &str = "You are signed out, but Beebeeb could not confirm that it was removed from Finder. If it still shows there, restart Beebeeb and sign out again.";

/// Lock when the sync engine's stop is unconfirmed (an earlier stop never was, or this one is not):
/// its task may still hold the vault key, so this is never a plain success (lead ruling T8-lockflag).
pub(super) const LOCK_ENGINE_UNCONFIRMED_WARNING: &str =
    "The vault is locked, but the sync engine did not confirm it stopped. Restart Beebeeb before unlocking again.";

pub(super) const ENGINE_START_REFUSED_ERROR: &str =
    "Could not start the sync engine: a previous one did not confirm it stopped. Restart Beebeeb.";

/// Lead ruling T8-lockflag: every place that puts an engine in the slot calls this with the slot
/// held, so two engines can never run at once. The flag is set when an engine's stop could not be
/// confirmed (its task may still be running with the keys) and stays set until a restart.
/// `finder_setup_wiring_tests` keeps the list of starts complete.
pub(super) fn engine_start_refusal(acct: &account::AccountRuntime) -> Result<(), String> {
    if acct.engine_stop_unconfirmed.load(Ordering::SeqCst) {
        tracing::error!("engine start refused: an earlier stop was never confirmed; restart Beebeeb");
        // Spec §5.6: the refusal has its own code and sentence on `sync_status.engine_refusal`, so the surfaces say to
        // quit and reopen instead of offering a "Try again" that cannot work. Cleared like the binding refusals, by
        // every session transition (no start runs while the flag is set).
        *acct
            .engine_refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(account_binding::Refusal::EngineStopUnconfirmed);
        return Err(ENGINE_START_REFUSED_ERROR.to_string());
    }
    Ok(())
}

/// The one way the in-memory session is cleared for a Lock or a Sign-out (lead ruling T10-lockorder). It takes
/// the engine-slot guard as a parameter, so the clear can only be written while the caller still holds the
/// slot: "release the slot, then clear the session" would be a use of a moved guard and does not compile. That
/// order is what keeps a start that was waiting for the slot from finding keys the clear was about to remove
/// (every start reads its keys under the slot). A poisoned mutex still guards the master key: it is recovered
/// and cleared, never skipped (and cleared of its poison, so the next unlock can install a session). Returns the token
/// of the session it took (task 1886: a sign-out ends it on the server; Lock, which keeps the session, drops it).
pub(super) fn clear_session_holding_slot(
    acct: &account::AccountRuntime,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    by: account::SessionTransition,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    // Every session transition that began before this point is refused from now on (Task 12, lead ruling 1): one that
    // put its session in memory before it is cleared just below, and one that has not yet written never will. The
    // session-write lock is not held across the clear (`bump_vault_epoch` must never wait under it).
    drop(end_sessions_in_flight(acct, by));
    let taken = match acct.session.lock() {
        Ok(mut guard) => {
            let taken = guard.take();
            bump_vault_epoch();
            tracing::info!("runtime session cleared");
            taken
        }
        #[cfg(target_os = "windows")]
        Err(_) => return Err("Could not clear the runtime session. The vault is not locked; restart Beebeeb.".into()),
        #[cfg(not(target_os = "windows"))]
        Err(poisoned) => {
            let taken = poisoned.into_inner().take();
            acct.session.clear_poison();
            bump_vault_epoch();
            tracing::warn!("the session mutex was poisoned; the runtime session was cleared anyway");
            taken
        }
    };
    // `Session` wipes itself on drop; its token moves out first, into a value that wipes itself too.
    Ok(taken.map(|mut session| zeroize::Zeroizing::new(std::mem::take(&mut session.token))))
}

/// The one way an engine leaves its slot to be stopped (Task 9 fix round 1, item A; lead ruling
/// T8-lockflag). The caller holds the engine slot and has already taken the handle out of it, so an
/// unconfirmed stop leaves an EMPTY slot while the task may still run with the keys. This sets
/// `engine_stop_unconfirmed` before it returns, so before the caller can release the slot, and every
/// start refuses from then on. Repair and the respawn inside a start used to abort without recording it,
/// and the next start (on a Mac, Repair's own follow-up check) spawned a second engine beside the first.
/// `every_engine_stop_goes_through_the_one_helper_and_every_respawn_refuses_after_it` keeps it the only
/// abort in the app.
pub(super) async fn stop_engine_in_slot(acct: &account::AccountRuntime, engine: EngineRunner) -> runner::AbortOutcome {
    let outcome = engine.abort().await;
    if !outcome.task_confirmed() {
        acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
        tracing::error!(
            "the sync engine's stop could not be confirmed; every engine start refuses until Beebeeb restarts"
        );
    }
    outcome
}

/// Repair's warning when its engine stop is unconfirmed (the same advice as Lock's error). Windows and Linux only: a
/// Mac's surfaces render `engine_stop_unconfirmed`'s fixed sentence instead.
#[cfg(not(target_os = "macos"))]
pub(super) const REPAIR_ENGINE_UNCONFIRMED_WARNING: &str =
    "The sync engine did not confirm it stopped. Restart Beebeeb before syncing again.";

/// What Repair's engine stop did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RepairEngineStop {
    /// The stale IPC socket was there and is gone.
    pub(super) removed_socket: bool,
    /// An engine stop on this account is unconfirmed after this one (see [`stop_engine_for_repair`]).
    pub(super) engine_stop_unconfirmed: bool,
}

/// Repair's engine stop (spec §5.3 (6)): stops the engine and, on Windows and Linux, says so in `warnings` when the
/// stop cannot be confirmed (a Mac's surfaces render the reported flag's fixed sentence instead, so its `warnings`
/// do not repeat it). On a Mac the reconciler then runs its follow-up check, which starts an engine again
/// with the keys still in memory; the flag the helper sets is what makes that start refuse instead of
/// running a second engine beside one that may still run. Reports that flag as it is AFTER this stop, so it
/// covers this stop and an earlier one (a Lock, a Sign-out or a check) that left the slot empty: Repair
/// reports it as `engine_stop_unconfirmed` (spec §5.6). The stale IPC socket is removed (`remove_socket`, which is
/// `remove_stale_ipc_socket` in production) while the slot is still held: an engine that a waiting check starts as
/// soon as the slot is free binds a fresh socket, and Repair must never delete that one.
pub(super) async fn stop_engine_for_repair(
    acct: &account::AccountRuntime,
    warnings: &mut Vec<String>,
    remove_socket: impl FnOnce() -> Result<bool, String>,
) -> RepairEngineStop {
    let mut engine_slot = acct.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        // Task 1538 Codex P1 — see `start_engine_if_possible`'s identical respawn guard.
        let outcome = stop_engine_in_slot(acct, prev).await;
        if outcome.is_stopped() {
            tracing::info!("engine aborted for macOS integration reset");
        } else {
            tracing::warn!("engine did not confirm termination before macOS integration reset");
            #[cfg(not(target_os = "macos"))]
            if !outcome.task_confirmed() {
                warnings.push(REPAIR_ENGINE_UNCONFIRMED_WARNING.to_string());
            }
        }
    }
    let removed_socket = match remove_socket() {
        Ok(removed) => removed,
        Err(error) => {
            warnings.push(error);
            false
        }
    };
    drop(engine_slot);
    if let Ok(mut guard) = acct.engine_state.lock() {
        *guard = "stopped".to_string();
    }
    RepairEngineStop {
        removed_socket,
        engine_stop_unconfirmed: acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
    }
}

/// Why an engine start has no keys to start with.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum EngineStartBlocked {
    /// An engine stop was never confirmed (the sentence says to restart), or the session mutex is unusable.
    Refused(String),
    /// Nothing is signed in with keys in memory: locked, signed out, or a Lock that won the slot first.
    NoSession,
}

/// Shown when a start finds the vault locked at the moment it holds the engine slot.
pub(super) const ENGINE_START_NO_SESSION: &str = "The vault is locked, so sync didn’t start.";

impl std::fmt::Display for EngineStartBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(sentence) => f.write_str(sentence),
            Self::NoSession => f.write_str(ENGINE_START_NO_SESSION),
        }
    }
}

/// The keys every engine start uses, and the identity of the session they belong to, read together UNDER
/// the engine slot (lead rulings T8-⚠lock, T9-starts and T10-identity). Lock and Sign-out clear the session
/// before they release the slot, so a start that was waiting for the slot finds nothing here instead of
/// starting an engine with keys that were just cleared. No start receives keys from its caller, so there
/// is nothing to read them from but this. The identity comes from the SAME `Session` (its own email, and
/// the cached profile only when that profile has that exact email), never from separate fields that a
/// sign-in updates at another moment. Refuses first while an earlier engine stop is unconfirmed. The
/// copies are `Zeroizing`, so none outlives the engine start that takes them.
pub(super) fn keys_for_engine_start(
    acct: &account::AccountRuntime,
) -> Result<(EngineKeys, account_binding::Identity), EngineStartBlocked> {
    engine_start_refusal(acct).map_err(EngineStartBlocked::Refused)?;
    let (keys, email) = acct
        .session
        .lock()
        .map_err(|_| EngineStartBlocked::Refused("session mutex poisoned".to_string()))?
        .as_ref()
        .map(|session| {
            (
                (
                    zeroize::Zeroizing::new(session.token.clone()),
                    zeroize::Zeroizing::new(session.master_key),
                ),
                session.email.clone(),
            )
        })
        .ok_or(EngineStartBlocked::NoSession)?;
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    Ok((keys, identity_of_session(email.as_deref(), profile.as_ref())))
}

/// The session token and master key an engine start hands to the engine, wiped when dropped.
pub(super) type EngineKeys = (zeroize::Zeroizing<String>, zeroize::Zeroizing<[u8; 32]>);

/// What a bound engine start came to (see [`spawn_bound_engine`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EngineStart {
    Started,
    /// The vault was locked (or signed out) by the time this start held the engine slot, so no engine
    /// was started and nothing was bound.
    NoSession,
    /// A Finder domain removal is owed (macOS): the previous account's domain is still registered, and no engine
    /// may start until a removal is confirmed. Nothing was started; the reconciler removes the domain and its next
    /// check starts the engine.
    FinderRemovalOwed,
}

/// What [`authorize_engine_start`] decided. Only `Go` carries the keys.
pub(super) enum StartPermit {
    Go(EngineKeys),
    NoSession,
    FinderRemovalOwed,
}

/// Never prints the keys (a failing assertion formats its operands).
impl std::fmt::Debug for StartPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Go(_) => f.write_str("Go(<keys>)"),
            Self::NoSession => f.write_str("NoSession"),
            Self::FinderRemovalOwed => f.write_str("FinderRemovalOwed"),
        }
    }
}

pub(super) const ENGINE_SLOT_OCCUPIED_ERROR: &str = "Could not start the sync engine: one is already running.";

/// The reconciler's start found no keys under the slot (a Lock won it, or nothing is signed in).
#[cfg(target_os = "macos")]
pub(super) const ENGINE_START_NO_SESSION_FOR_FINDER: &str = "Unlock the vault before adding Beebeeb to Finder.";

/// A start that finds a Finder removal owed: shown to the reconciler's checks (which retry) and never to a person
/// as a failure of their sign-in (`start_engine_if_possible` treats it as "deferred").
#[cfg(target_os = "macos")]
pub(super) const FINDER_REMOVAL_OWED_ERROR: &str =
    "Beebeeb is still removing the previous account from Finder. Sync starts when that is done.";

/// R10 (spec 2026-10-06 §5.6): the binding's view of this computer's local data.
pub(super) struct StateDbLocalData {
    pub(super) db: state_db::StateDb,
    pub(super) sync_root: Option<PathBuf>,
    /// Directories whose plaintext copies the reset sweeps as a whole (see `purge_local_state_files_in`).
    pub(super) staging_dirs: Vec<PathBuf>,
    /// Spec 2026-10-10 W12: what the reset's keep step kept (macOS), from the keep until the binding hands its folder
    /// on (`hand_on_reset_kept`), whether or not the purge after it finished.
    pub(super) kept: std::sync::Mutex<Option<sign_out::kept_set::KeptSet>>,
}

/// Where an engine start looks: the app-local state dir (`state.db`) and the sync root the engine will use.
/// Production resolves the state dir from the process-wide setting; a test passes its own throwaway one.
pub(super) struct LocalDataPaths {
    pub(super) state_dir: PathBuf,
    pub(super) sync_root: PathBuf,
    pub(super) staging_dirs: Vec<PathBuf>,
}

impl LocalDataPaths {
    pub(super) fn production(sync_root: &Path) -> Result<Self, String> {
        Ok(Self {
            state_dir: state_paths::beebeeb_state_dir()?,
            sync_root: sync_root.to_path_buf(),
            staging_dirs: production_staging_dirs(),
        })
    }

    #[cfg(test)]
    pub(super) fn for_test(state_dir: &Path, sync_root: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
            sync_root: sync_root.to_path_buf(),
            staging_dirs: Vec::new(),
        }
    }
}

impl StateDbLocalData {
    /// Opens `state.db` the way the engine will, and creates it if needed, so the owner of a first engine
    /// start is recorded before that engine writes anything. On a first start the state-dir migration runs
    /// here first; the runner's own call then finds `state.db` and skips it.
    pub(super) fn for_engine_start(paths: &LocalDataPaths) -> Result<Self, String> {
        let path = state_paths::state_db_path_from_state_dir(&paths.state_dir);
        if !path.exists() {
            state_paths::prepare_beebeeb_state_dir_at(&paths.sync_root, &paths.state_dir)?;
        }
        let db = state_db::StateDb::open(&path).map_err(|e| format!("open state.db: {e}"))?;
        // An engine start shuts the upgrade's adoption window on every database it opens: one it creates (or
        // migrates in from an older layout) was never the upgrade's, and an existing one has had its startup pass.
        // Nothing is adopted into either afterwards.
        db.close_adoption_window().map_err(|e| format!("mark state.db: {e}"))?;
        Ok(Self {
            db,
            sync_root: Some(paths.sync_root.clone()),
            staging_dirs: paths.staging_dirs.clone(),
            kept: Default::default(),
        })
    }
}

impl account_binding::LocalData for StateDbLocalData {
    fn owner(&self) -> Result<Option<account_binding::Identity>, String> {
        self.db.owner().map_err(|e| format!("read the local data owner: {e}"))
    }

    fn has_account_data(&self) -> Result<bool, String> {
        self.db
            .has_account_data()
            .map_err(|e| format!("inspect the local data: {e}"))
    }

    /// Spec 2026-10-10 W12: keep (no send, no ask: every missing write and other change is acknowledged), then the
    /// sign-out purge with the guard (its one deletion gate, `is_disposable_cache_path`, never reaches the lifecycle
    /// log directory), then every remaining row. A failed keep fails the reset, and nothing is deleted. An error from
    /// any step means nothing may start.
    fn reset(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let kept = Some(keep_in(
            &self.db,
            sign_out::keep::Acknowledge::Everything,
            KeepCaller::Reset,
        )?);
        #[cfg(not(target_os = "macos"))]
        let kept: Option<sign_out::kept_set::KeptSet> = None;
        // W12: the folder is recorded now, so it is handed on to be surfaced whether or not the purge below then
        // finishes.
        *self.kept.lock().unwrap_or_else(|p| p.into_inner()) = kept.clone();
        // Spec 2026-10-10 §15 question 3: a reset that stops without purging reopens Finder accepts at once (the keep
        // step already reopens them on its own failure).
        let reopen = || {
            #[cfg(target_os = "macos")]
            if let Err(error) = self.db.reopen_finder_accepts() {
                tracing::warn!(%error, "could not reopen Finder accepts; the next engine start reopens them");
            }
        };
        let summary =
            match purge_local_state_files_in(&self.db, self.sync_root.as_deref(), &self.staging_dirs, kept.as_ref()) {
                Ok(summary) => summary,
                Err(error) => {
                    reopen();
                    return Err(error);
                }
            };
        tracing::info!(
            queued_ops_purged = summary.queued_ops_purged,
            files_removed = summary.files_removed,
            files_skipped = summary.files_skipped,
            others_dropped = kept.as_ref().map_or(0, |kept| kept.others_acknowledged.len()),
            "R10: local data reset before sync started"
        );
        // W13: the purge cleared the queue's rows, so no write can look missing any more. Best effort (§7.3), as the
        // sign-out's: the warning names the step that failed, `path`, `read` or `write`, never an error's text (W10).
        #[cfg(target_os = "macos")]
        if let Err(step) = DesktopConfig::path()
            .map_err(|_| "path".to_string())
            .and_then(|path| sign_out::kept_record::clear_unfinished_at(&path))
        {
            tracing::warn!(step = %step, "could not clear the unfinished sign-out marker");
        }
        // B05e (lead ruling 2026-11-09): a reset of local data removes the share marks and sealed keys, which a
        // sign-out keeps. First, so a failed removal fails the reset before the rows go.
        self.db.forget_share_trust_rows().map_err(|e| {
            reopen();
            format!("clear the local data: {e}")
        })?;
        // On a Mac the previous account's Finder domain is still registered: the debt is recorded in the same
        // transaction that clears the rows, so no crash can leave the rows gone and the debt forgotten.
        self.db.clear_account_data(cfg!(target_os = "macos")).map_err(|e| {
            reopen();
            format!("clear the local data: {e}")
        })
    }

    fn record_owner(&self, owner: &account_binding::Identity) -> Result<(), String> {
        self.db
            .set_owner(owner)
            .map_err(|e| format!("record the local data owner: {e}"))
    }

    fn adopt_once(&self, candidate: Option<&account_binding::Identity>) -> Result<bool, String> {
        self.db
            .adopt_owner_once(candidate)
            .map_err(|e| format!("adopt the local data owner: {e}"))
    }
}
