//! `vault_unlock` items split out of `lib.rs`.

use super::*;

/// Where "unlock" goes on this Mac (task 1747 Part 2): `Keychain` when the vault key is in the Keychain, `RecoveryPhrase` when
/// neither layout holds one (a session without keys: the first password sign-in that was never finished with the phrase). The
/// window shows Set up this Mac for the second and Home's locked card for the first (plan D4). Serialized snake_case, which is
/// what `vaultUnlockRoute` in `src/desktopApi.ts` parses (pinned equal in `tests/vaultUnlockRoute.test.ts`).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum VaultUnlockRoute {
    Keychain,
    RecoveryPhrase,
}

/// `key_present` is [`keychain_vault_key_present`]'s answer, which fails closed: a Keychain that cannot be read routes to the
/// Keychain unlock, so the existing unlock error reaches the person, rather than to a path that would overwrite a key.
#[cfg(target_os = "macos")]
pub(super) fn unlock_route(key_present: bool) -> VaultUnlockRoute {
    if key_present {
        VaultUnlockRoute::Keychain
    } else {
        VaultUnlockRoute::RecoveryPhrase
    }
}

/// The route for one account slot. It reaches the Keychain only through [`keychain_vault_key_present`], which reads the item's
/// attributes and never the key.
#[cfg(target_os = "macos")]
pub(super) fn vault_unlock_route_for(account_id: &str) -> VaultUnlockRoute {
    unlock_route(keychain_vault_key_present(account_id))
}

/// The Beebeeb window asks this once, when it opens signed in and locked, and again at the click on “Unlock vault” (and
/// Settings › Account's Unlock). An answer about the ACTIVE account, so the window keeps its revision guard on it, and it is
/// not part of `sync_status`, which is polled every second. The Keychain is asked off the async workers; a worker that failed
/// to answer is the Keychain unlock, the fail-closed side.
#[cfg(target_os = "macos")]
#[tauri::command]
pub(super) async fn vault_unlock_route(state: State<'_, AppState>) -> Result<VaultUnlockRoute, String> {
    let account_id = state.active_account()?.id.as_str().to_string();
    Ok(tokio::task::spawn_blocking(move || vault_unlock_route_for(&account_id))
        .await
        .unwrap_or(VaultUnlockRoute::Keychain))
}

/// Does the Keychain still hold a session token for this account slot (id-keyed or legacy)? A read error counts as
/// present: an unreadable item is a trace, never an absence. (`keychain_session_present` answers the routing
/// question and reads an error as "no session"; this one answers "is anything left?".)
#[cfg(not(target_os = "windows"))]
pub(super) fn keychain_session_trace(account_id: &str) -> bool {
    keychain::holds_session_token(&platform_keychain_store_for(account_id))
        || keychain::holds_session_token(&keychain::legacy_platform_keychain_store())
}

/// Does the Keychain still hold an account email for this account slot (id-keyed or legacy)? Fails closed like
/// [`keychain_session_trace`].
#[cfg(not(target_os = "windows"))]
pub(super) fn keychain_email_trace(account_id: &str) -> bool {
    keychain::holds_account_email(&platform_keychain_store_for(account_id))
        || keychain::holds_account_email(&keychain::legacy_platform_keychain_store())
}

/// Changes waiting to upload, or staged copies of them, in the local data. An unreadable `state.db` counts as
/// present (fail closed).
#[cfg(not(target_os = "windows"))]
pub(super) fn queued_or_staged_present(sources: &LocalSources) -> bool {
    match sources.state_db() {
        Ok(Some(db)) => db.queued_or_staged_count().map(|n| n > 0).unwrap_or(true),
        Ok(None) => false,
        Err(_) => true,
    }
}

/// Is there local data that no account is recorded as owning? Rows of `state.db` (the queue, staged payloads, file
/// rows, cursors, activity: everything `has_account_data` counts, and the cache they point at) in a database whose owner
/// is not recorded. No database yet is no data. A database or a state directory that cannot be read is an error, not an
/// answer: the caller settles nothing (`reauth::SignInKind::Unconfirmed`). Credentials are not data: this is about what
/// a switch would have to purge.
#[cfg(not(target_os = "windows"))]
pub(super) fn unowned_local_data(sources: &LocalSources) -> Result<bool, String> {
    match sources.state_db()? {
        Some(db) => db
            .has_account_data()
            .map_err(|e| format!("inspect the local data: {e}")),
        None => Ok(false),
    }
}

/// The account this Mac's credentials name, for a Mac with no owner record (a sign-in whose engine has not started):
/// the cached profile's user id and email, else the email of the session in memory, else the Keychain's. A lock that
/// cannot be read names nobody.
#[cfg(not(target_os = "windows"))]
pub(super) fn credentials_identity(acct: &AccountRuntime) -> account_binding::Identity {
    let (user_id, profile_email) = match acct.cached_profile.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(profile) => (Some(profile.user_id.clone()), Some(profile.email.clone())),
            None => (None, None),
        },
        Err(_) => (None, None),
    };
    let session_email = acct
        .session
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(|session| session.email.clone()));
    let email = profile_email
        .filter(|email| !email.trim().is_empty())
        .or(session_email)
        .or_else(|| keychain_account_email(acct.id.as_str()));
    account_binding::Identity::new(user_id.as_deref(), email.as_deref())
}

/// R8 + R10: the same account signed in again (decided by `reauth::sign_in_kind`), so the record names it by its
/// user id and the server's own spelling of its email. Never called for another account.
#[cfg(not(target_os = "windows"))]
pub(super) fn record_local_data_owner(sources: &LocalSources, owner: &account_binding::Identity) -> Result<(), String> {
    match sources.state_db()? {
        Some(db) => db
            .set_owner(owner)
            .map_err(|e| format!("record the local data owner: {e}")),
        None => Ok(()),
    }
}

/// How many changes on this Mac have not uploaded yet: the number in the R8 warning. The queue and the staged copies
/// no queued operation points at, each once (`StateDb::pending_changes_count`). No database yet is none. A count that
/// cannot be read is an error, never 0: the sign-in then settles nothing (`LocalDataUnreadable`), because a warning
/// that says 0 would let the switch delete the only copy of an edit without saying so.
#[cfg(not(target_os = "windows"))]
pub(super) fn pending_changes_count(sources: &LocalSources) -> Result<u64, String> {
    match sources.state_db()? {
        Some(db) => db
            .pending_changes_count()
            .map_err(|e| format!("count the changes not uploaded: {e}")),
        None => Ok(0),
    }
}

/// Swap the token of the session in memory and keep its keys and email. `false` when no session is in memory. The
/// replaced token is wiped, not just dropped.
#[cfg(not(target_os = "windows"))]
pub(super) fn replace_session_token_in_memory(_write: &SessionWrite, acct: &AccountRuntime, token: &str) -> Result<bool, String> {
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    match guard.as_mut() {
        Some(session) => {
            let mut replaced = std::mem::replace(&mut session.token, token.to_string());
            zeroize::Zeroize::zeroize(&mut replaced);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Give the session in memory the server's own spelling of its account's email. A same-account re-sign-in does this
/// so the next engine start compares the identity the owner record holds. `false` when no session is in memory or
/// the email is empty.
#[cfg(not(target_os = "windows"))]
pub(super) fn name_session_in_memory(_write: &SessionWrite, acct: &AccountRuntime, email: &str) -> Result<bool, String> {
    let email = email.trim();
    if email.is_empty() {
        return Ok(false);
    }
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    match guard.as_mut() {
        Some(session) => {
            session.email = Some(email.to_string());
            Ok(true)
        }
        None => Ok(false),
    }
}

/// What a re-sign-in resets: the revoked-session streak ends, and the session is present again.
#[cfg(not(target_os = "windows"))]
pub(super) fn reauth_settle_flags(_write: &SessionWrite, state: &AppState, acct: &AccountRuntime, email: &str) {
    acct.auth_health.note_result(None);
    set_auth_present(state, true);
    let email = email.trim();
    if !email.is_empty() {
        set_auth_email(state, Some(email.to_string()));
    }
}

/// What a sign-in found of an earlier account on this Mac.
#[cfg(not(target_os = "windows"))]
pub(super) struct LocalFacts {
    pub(super) traces: reauth::LocalTraces,
    /// The recorded owner of the local data, when the record names an account.
    pub(super) owner: Option<account_binding::Identity>,
    /// Local data that no account is recorded as owning (credentials are not data).
    pub(super) unowned_data: bool,
    /// The owner record or `state.db` could not be read: nothing can be decided (see `reauth::SignInKind::Unconfirmed`).
    pub(super) unreadable: bool,
    /// The account the credentials name: used when there is no owner record.
    pub(super) credentials: account_binding::Identity,
    /// The number in the switch warning. When it cannot be read the facts are `unreadable`, and this is 0.
    pub(super) pending_changes: u64,
}

/// Every trace of a previous account on this Mac, from its real sources. Reads only: it asks whether a vault key
/// exists and never reads one. An unreadable source (a poisoned lock, an unreadable `state.db`, a Keychain item that
/// answers with an error) counts as a trace, never as an absence: a sign-in is `Fresh` only when NOTHING of an
/// account remains (`reauth::LocalTraces`).
#[cfg(not(target_os = "windows"))]
pub(super) fn gather_local_facts(state: &AppState, acct: &AccountRuntime, sources: &LocalSources) -> LocalFacts {
    let pending_changes = pending_changes_count(sources);
    // R10: the recorded owner of the local data is the account on this computer.
    let owner = local_data_owner(sources);
    let recorded_owner = !matches!(owner, Ok(None));
    // Every retained trace counts, including what a startup 401 keeps on purpose (R9: the Keychain email and the
    // vault key).
    let traces = reauth::LocalTraces {
        session_in_memory: acct.session.lock().map(|guard| guard.is_some()).unwrap_or(true),
        auth_present: state.auth_present.lock().map(|guard| *guard).unwrap_or(true),
        keychain_token: keychain_session_trace(acct.id.as_str()),
        keychain_email: keychain_email_trace(acct.id.as_str()),
        vault_key: keychain_vault_key_present(acct.id.as_str()),
        recorded_owner,
        cached_profile: acct.cached_profile.lock().map(|guard| guard.is_some()).unwrap_or(true),
        queued_or_staged: pending_changes.as_ref().map_or(true, |count| *count > 0)
            || queued_or_staged_present(sources),
    };
    // An owner record that cannot be read is not "no owner record": it is what makes the sign-in `unreadable`.
    let owner_unreadable = owner.is_err();
    let owner = owner.ok().flatten().filter(account_binding::Identity::is_known);
    let data = if owner.is_some() {
        Ok(false)
    } else {
        unowned_local_data(sources)
    };
    LocalFacts {
        traces,
        owner,
        unowned_data: matches!(data, Ok(true)),
        // The switch warning's count that cannot be read settles nothing either (triage 25).
        unreadable: owner_unreadable || data.is_err() || pending_changes.is_err(),
        credentials: credentials_identity(acct),
        pending_changes: pending_changes.unwrap_or(0),
    }
}

/// R8: what a sign-in as `profile` (the server's own record of the account behind the new session) is on this Mac,
/// and how many changes have not uploaded: the pure decision (`reauth::sign_in_kind`) over the account this Mac holds
/// (the recorded owner of the local data, else what its credentials name) and what else is here. May answer
/// `NeedsKeyProof` (see [`decide_sign_in`]). Reads only.
#[cfg(not(target_os = "windows"))]
pub(super) fn classify_sign_in(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
) -> (reauth::SignInKind, u64) {
    classify_sign_in_after(state, acct, sources, profile, None)
}

/// [`classify_sign_in`] with the server's key proof, when one was asked for: the facts are read again (the proof can
/// take a while), and the proof decides only if they still ask for it (`reauth::after_key_proof`).
#[cfg(not(target_os = "windows"))]
pub(super) fn classify_sign_in_after(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
    proof: Option<reauth::KeyProof>,
) -> (reauth::SignInKind, u64) {
    let facts = gather_local_facts(state, acct, sources);
    // The account this Mac holds: the recorded owner of the local data, else what its credentials name.
    let account = facts.owner.as_ref().unwrap_or(&facts.credentials);
    let local = reauth::LocalAccount {
        user_id: account.user_id.as_deref(),
        email: account.email.as_deref(),
        traces: facts.traces.any(),
        key_stored: facts.traces.vault_key,
        // A session in memory always holds its vault key. A lock that cannot be read counts as one: the proof then
        // cannot read it either, and settles nothing.
        key_in_memory: facts.traces.session_in_memory,
        unowned_data: facts.unowned_data,
        unreadable: facts.unreadable,
    };
    let kind = match (reauth::sign_in_kind(&local, &profile.user_id, &profile.email), proof) {
        (reauth::SignInKind::NeedsKeyProof, Some(proof)) => {
            reauth::after_key_proof(proof, &local, &profile.user_id, &profile.email)
        }
        (kind, _) => kind,
    };
    (kind, facts.pending_changes)
}

/// The vault key stored in one Keychain layout, for the server proof, as a `MasterKey`: wiped on drop and not `Clone`.
/// The only plain buffer is a `Zeroizing` one that lives inside this function, because the core builds a `MasterKey`
/// from 32 bytes passed by value (it has no constructor that takes a reference, and it wipes the argument it is
/// given). `Ok(None)`: this layout holds no key.
#[cfg(not(target_os = "windows"))]
pub(super) fn read_vault_key<S: keychain::AuthSecretStore>(
    mut vault: AuthVault<S>,
) -> Result<Option<beebeeb_core::kdf::MasterKey>, String> {
    match vault.unlock() {
        Ok(()) => {}
        Err(keychain::AuthStoreError::NotFound | keychain::AuthStoreError::Unsupported(_)) => return Ok(None),
        Err(error) => return Err(keychain_error("unlock vault key from Keychain", error)),
    }
    let mut raw = zeroize::Zeroizing::new([0u8; 32]);
    raw.copy_from_slice(vault.master_key().map_err(|e| keychain_error("read vault key", e))?);
    Ok(Some(beebeeb_core::kdf::MasterKey::from_bytes(*raw)))
}

/// The vault key this Mac stores (the id-keyed layout, else the legacy one), read for exactly one purpose: the server
/// proof that it belongs to the account that is signing in. A same-account sign-in with no session in memory stores its
/// new token in the id-keyed layout and then loads that layout's key (`reauth_swap_token`, through
/// `load_session_from_keychain`), so the id-keyed key is read first. It is a `MasterKey` from the moment it leaves the
/// Keychain read, so nothing else holds a plain copy.
#[cfg(not(target_os = "windows"))]
pub(super) fn load_stored_vault_key(account_id: &str) -> Result<Option<beebeeb_core::kdf::MasterKey>, String> {
    if let Some(key) = read_vault_key(AuthVault::new(platform_keychain_store_for(account_id)))? {
        return Ok(Some(key));
    }
    read_vault_key(AuthVault::new(keychain::legacy_platform_keychain_store()))
}

/// The vault key of the session in memory, for the server proof (spec §5.6): a same-account sign-in keeps the key in
/// memory (`reauth_swap_token`), so with a session in memory this is the key that is proved, whatever the Keychain
/// holds. Read under the session lock into a wiped buffer and made a `MasterKey` (wiped on drop, not `Clone`) before
/// the lock is released, as [`read_vault_key`] does. `Ok(None)`: no session in memory. A lock that cannot be read is an
/// error, so the proof settles nothing.
#[cfg(not(target_os = "windows"))]
pub(super) fn read_session_vault_key(acct: &AccountRuntime) -> Result<Option<beebeeb_core::kdf::MasterKey>, String> {
    let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    let Some(session) = guard.as_ref() else {
        return Ok(None);
    };
    let mut raw = zeroize::Zeroizing::new([0u8; 32]);
    raw.copy_from_slice(&session.master_key);
    Ok(Some(beebeeb_core::kdf::MasterKey::from_bytes(*raw)))
}

/// Ask the server whether the vault key this Mac kept is the account behind `token`'s (the same check the recovery
/// phrase goes through). The key proved is the key the sign-in keeps: the key of the session in memory when there is
/// one (`reauth_swap_token` keeps it), else the key stored in the Keychain (see [`load_stored_vault_key`]). Only a
/// completed answer decides: no key, an unreadable key, a client that cannot be built, a network or server error, an
/// expired session:
/// all are `Unavailable`, and nothing is decided. A `400` decides only with an exact code: `invalid_recovery_phrase` is
/// `Mismatch`, `recovery_check_missing` is `NoCheckOnFile`, and any other `400` is `Unavailable`.
#[cfg(not(target_os = "windows"))]
pub(super) async fn prove_stored_key(acct: &AccountRuntime, base_url: &str, token: &str) -> reauth::KeyProof {
    let kept = match read_session_vault_key(acct) {
        Ok(None) => load_stored_vault_key(acct.id.as_str()),
        in_memory => in_memory,
    };
    let Ok(Some(master_key)) = kept else {
        return reauth::KeyProof::Unavailable;
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .default_headers(api_client::provenance_headers())
        .build()
    else {
        return reauth::KeyProof::Unavailable;
    };
    match recovery_check_answer(&client, base_url, token, &master_key).await {
        Ok(RecoveryCheckAnswer::Matches) => reauth::KeyProof::Matches,
        Ok(RecoveryCheckAnswer::Mismatch) => reauth::KeyProof::Mismatch,
        Ok(RecoveryCheckAnswer::NoCheckOnFile) => reauth::KeyProof::NoCheckOnFile,
        Err(_) => reauth::KeyProof::Unavailable,
    }
}

/// R8: what the sign-in is, with the server's proof where the rules ask for one (`NeedsKeyProof`: a vault key is here,
/// and either the user ids match or this Mac's account has no user id). Never `NeedsKeyProof`. Reads only, apart from
/// the one request.
#[cfg(not(target_os = "windows"))]
pub(super) async fn decide_sign_in(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
    token: &str,
    base_url: &str,
) -> (reauth::SignInKind, u64) {
    let (kind, pending_changes) = classify_sign_in(state, acct, sources, profile);
    if kind != reauth::SignInKind::NeedsKeyProof {
        return (kind, pending_changes);
    }
    let proof = prove_stored_key(acct, base_url, token).await;
    // The facts are read again after the wait; the caller also refuses when any session transition happened meanwhile.
    classify_sign_in_after(state, acct, sources, profile, Some(proof))
}

/// [`decide_sign_in`] for a sign-in whose turn is `turn`: the decision can wait for the server (the key proof), and if a
/// Lock, a Sign-out or another sign-in completed meanwhile, what it saw is no longer this computer, so nothing is
/// decided (lead ruling 1, Task 12). Nothing is stored yet, so the caller revokes the new session.
#[cfg(not(target_os = "windows"))]
pub(super) async fn decide_sign_in_in_turn(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
    token: &str,
    base_url: &str,
    turn: &account::SessionGeneration,
) -> Result<(reauth::SignInKind, u64), ReauthError> {
    let decided = decide_sign_in(state, acct, sources, profile, token, base_url).await;
    if !acct.session_unchanged_since(*turn) {
        return Err(ReauthError::before_store(SessionChanged::of(acct)));
    }
    Ok(decided)
}

/// Does this Mac hold anything of an account? For a sign-in that could not learn which account it is: it cannot be
/// compared, so on a Mac that holds something it is refused (and nothing changes).
#[cfg(not(target_os = "windows"))]
pub(super) fn sign_in_leaves_account_traces(state: &AppState) -> bool {
    match state.active_account() {
        Ok(acct) => gather_local_facts(state, &acct, &LocalSources::production())
            .traces
            .any(),
        Err(_) => true,
    }
}

/// What a sign-in whose account could not be confirmed says: its record could not be fetched, or the server could not
/// say whether the key stored on this Mac is the signing-in account's. Nothing changed on this computer.
#[cfg(not(target_os = "windows"))]
pub(super) const SIGN_IN_ACCOUNT_UNKNOWN: &str = "Beebeeb couldn’t confirm which account just signed in, so nothing changed on this computer. Connect to the internet and try again.";

/// What a sign-in says when this computer's own record of its account (the owner record, or `state.db` itself) could
/// not be read (lead ruling 12, Task 12). Nothing changed on this computer; trying again is safe.
#[cfg(not(target_os = "windows"))]
pub(super) const SIGN_IN_LOCAL_DATA_UNREADABLE: &str = "Beebeeb couldn’t read this Mac’s account data. Try again.";
