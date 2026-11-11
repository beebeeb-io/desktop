//! Task 2447: the credential-store side of [`crate::auth_v2::key_owner`] (whose vault key is stored next to whose
//! session token). The rules are there; these functions only choose the store and hold the turn.

use super::*;
use crate::auth_v2::key_owner::{self, KeyOwnership, StoredKey};

type PlatformVault = AuthVault<keychain::PlatformKeychainStore>;

/// The layout that holds the session token: the id-keyed one, else the legacy one (the read fallback
/// `load_session_from_keychain` has). The gate's probe and the restore ask THIS store about the key, so they never
/// see a key the loader would not read. `Ok(None)`: no token in either. `Err`: a store could not answer.
pub(crate) fn vault_holding_session(account_id: &str) -> Result<Option<PlatformVault>, String> {
    for vault in [
        AuthVault::new(platform_keychain_store_for(account_id)),
        AuthVault::new(keychain::legacy_platform_keychain_store()),
    ] {
        match vault.session_token() {
            Ok(Some(_)) => return Ok(Some(vault)),
            Ok(None) | Err(keychain::AuthStoreError::NotFound | keychain::AuthStoreError::Unsupported(_)) => {}
            Err(error) => return Err(keychain_error("load session from Keychain", error)),
        }
    }
    Ok(None)
}

/// What the gate shows for a signed-in, locked window (P2-6): the state of the key the loader would read. A store
/// that cannot answer is `Unreadable`, never "present". Blocking (on Windows it loads the key and drops it at once).
pub(crate) fn stored_key_state_for(account_id: &str) -> StoredKey {
    match vault_holding_session(account_id) {
        Ok(Some(vault)) => key_owner::stored_key_state(&vault),
        // Signed in by the flags, yet no token is stored: nothing can be unlocked from here.
        Ok(None) => StoredKey::Absent,
        Err(_) => StoredKey::Unreadable,
    }
}

/// A token-only sign-in, in its turn and BEFORE its token is stored: a stored vault key that is not provably the
/// signing-in account's is removed (`key_owner::remove_foreign_key`). While it cannot be removed the sign-in is
/// refused with [`key_owner::FOREIGN_KEY_NOT_REMOVED`], and the caller revokes the new session.
pub(crate) fn remove_foreign_vault_key_before_sign_in(
    _write: &SessionWrite,
    account_id: &str,
    signing_in_user_id: &str,
    signing_in_email: &str,
) -> Result<(), String> {
    let mut vault = AuthVault::new(platform_keychain_store_for(account_id));
    match key_owner::remove_foreign_key(&mut vault, signing_in_user_id, signing_in_email) {
        Ok(true) => {
            tracing::info!("sign-in: a vault key that was not this account's was removed before the token was stored");
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(error) => {
            tracing::warn!(%error, "sign-in refused: a stored vault key that is not this account's could not be removed");
            Err(key_owner::FOREIGN_KEY_NOT_REMOVED.to_string())
        }
    }
}

/// The owner of the token just stored, in the same turn: `user_id` is the id the server named for that session. A
/// failure leaves the owner unknown (the store removed the old tag before it wrote the token), which is safe.
pub(crate) fn bind_session_owner_in_keychain(_write: &SessionWrite, account_id: &str, user_id: &str) {
    if let Err(error) = AuthVault::new(platform_keychain_store_for(account_id)).bind_session_owner(user_id) {
        tracing::warn!(%error, "could not record the session's owner in the credential store (it stays unknown)");
    }
}

/// The owner of the token AND the key just stored together (device approval, browser handoff, sign-up), same turn.
pub(crate) fn bind_session_and_key_owner_in_keychain(_write: &SessionWrite, account_id: &str, user_id: &str) {
    if let Err(error) =
        key_owner::bind_session_and_key(&AuthVault::new(platform_keychain_store_for(account_id)), user_id)
    {
        tracing::warn!(%error, "could not record the session's and the key's owner in the credential store (unknown)");
    }
}

/// The recovery phrase proved the key just stored is the stored session's account's: the key takes that session's
/// owner. Same turn as the key's write, after the stored token was checked to be the one the proof was made with.
pub(crate) fn bind_vault_key_to_session_owner_in_keychain(_write: &SessionWrite, account_id: &str) {
    if let Err(error) = key_owner::bind_key_to_session_owner(&AuthVault::new(platform_keychain_store_for(account_id))) {
        tracing::warn!(%error, "could not record the vault key's owner in the credential store (it stays unknown)");
    }
}

/// A token rotation (password change): the owner of the session whose stored token is `old_token`, to carry over to
/// the token the server issued for the same session.
pub(crate) fn session_owner_for_rotation(account_id: &str, old_token: &str) -> Option<String> {
    key_owner::session_owner_of(&AuthVault::new(platform_keychain_store_for(account_id)), old_token)
}

/// The startup restore, in its turn, after the server named the account of the stored token: record that owner, and
/// judge the stored key against the SERVER's id. A store that cannot answer is not a match.
pub(crate) fn settle_owner_after_probe_in_keychain(
    _write: &SessionWrite,
    account_id: &str,
    probed_token: &str,
    server_user_id: &str,
) -> KeyOwnership {
    let settled = vault_holding_session(account_id).and_then(|vault| match vault {
        Some(vault) => key_owner::settle_owner_after_probe(&vault, probed_token, server_user_id)
            .map_err(|e| keychain_error("settle the vault key's owner", e)),
        None => Ok(KeyOwnership::SessionUnknown),
    });
    settled.unwrap_or_else(|error| {
        tracing::warn!(%error, "startup restore: the credential store could not say whose vault key it holds");
        KeyOwnership::SessionUnknown
    })
}

/// Windows, at startup and before any sign-in is offered: finish a clear that failed earlier (a vault key or an owner
/// tag with no session token, in either layout). Not silent: what cannot be removed is logged, and a sign-in is
/// refused while a key that is not its account's is still there.
#[cfg(target_os = "windows")]
pub(crate) fn complete_pending_keychain_clear(_write: &SessionWrite, account_id: &str) {
    for (layout, mut vault) in [
        ("id-keyed", AuthVault::new(platform_keychain_store_for(account_id))),
        ("legacy", AuthVault::new(keychain::legacy_platform_keychain_store())),
    ] {
        match key_owner::complete_pending_clear(&mut vault) {
            Ok(true) => tracing::warn!(
                layout,
                "startup: finished a credential clear that had left a vault key behind"
            ),
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(layout, %error, "startup: a leftover vault key could not be removed; sign-in will try again")
            }
        }
    }
}

/// P2-2: may a session that arrives with its key (a device approval) be installed while this PC is already signed
/// in? Only for the signed-in account itself, while it is still locked: the stored session's recorded owner (the id
/// the server named at its sign-in) equals the id the server named for the arriving session (`profile`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn approval_adds_key_to_signed_in_session(
    account_id: &str,
    profile: Option<&account_dto::AccountProfile>,
    vault_unlocked: bool,
) -> bool {
    let signed_in = vault_holding_session(account_id)
        .ok()
        .flatten()
        .and_then(|vault| vault.session_owner().ok().flatten());
    key_owner::may_add_key_to_signed_in_session(
        signed_in.as_deref(),
        profile.map(|profile| profile.user_id.as_str()),
        vault_unlocked,
    )
}
