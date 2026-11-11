//! Task 2447: the credential-store side of [`crate::key_owner`] (whose vault key is stored next to whose session
//! token). The rules are there; these functions only choose the store and hold the turn. Each has a core over any
//! two layouts (`*_over`), which the tests drive with an in-memory store, and a one-line platform form the commands
//! call.
//! Must not: read, copy or log a key or a token; log a user id.

use super::*;
use crate::key_owner::{self, KeyOwnership};
use keychain::{AuthResult, AuthSecretStore, AuthStoreError};

/// The id-keyed layout, then the legacy one: the order every reader of a stored session asks them in.
fn both_layouts(account_id: &str) -> [AuthVault<keychain::PlatformKeychainStore>; 2] {
    [
        AuthVault::new(platform_keychain_store_for(account_id)),
        AuthVault::new(keychain::legacy_platform_keychain_store()),
    ]
}

/// The layout that holds the session token: the first of `layouts` that has one (the read fallback
/// `load_session_from_keychain` has). The restore asks THIS store about the key, so it never judges a key the loader
/// would not read. `Ok(None)`: no token in either. `Err`: a store could not answer.
pub(crate) fn layout_holding_session<S: AuthSecretStore>(
    layouts: impl IntoIterator<Item = AuthVault<S>>,
) -> AuthResult<Option<AuthVault<S>>> {
    for vault in layouts {
        match vault.session_token() {
            Ok(Some(_)) => return Ok(Some(vault)),
            Ok(None) | Err(AuthStoreError::NotFound | AuthStoreError::Unsupported(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// [`remove_foreign_vault_key_before_sign_in`] over any store: `Err` is the refusal's sentence, and the key is still
/// there.
pub(crate) fn remove_foreign_key_or_refuse<S: AuthSecretStore>(
    vault: &mut AuthVault<S>,
    signing_in_user_id: &str,
    signing_in_email: &str,
) -> Result<(), String> {
    match key_owner::remove_foreign_key(vault, signing_in_user_id, signing_in_email) {
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
    remove_foreign_key_or_refuse(&mut vault, signing_in_user_id, signing_in_email)
}

/// The owner of the token just stored, in the same turn: `user_id` is the id the server named for that session. A
/// failure leaves the owner unknown (the store removed the old tag before it wrote the token), which is safe.
pub(crate) fn bind_session_owner_in_keychain(_write: &SessionWrite, account_id: &str, user_id: &str) {
    if let Err(error) = AuthVault::new(platform_keychain_store_for(account_id)).bind_session_owner(user_id) {
        tracing::warn!(%error, "could not record the session's owner in the credential store (it stays unknown)");
    }
}

/// The owner of the token AND the key just stored together (the browser handoff), same turn.
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

/// [`settle_owner_after_probe_in_keychain`] over any two layouts. A store that cannot answer is not a match.
pub(crate) fn settle_owner_after_probe_over<S: AuthSecretStore>(
    layouts: impl IntoIterator<Item = AuthVault<S>>,
    probed_token: &str,
    server_user_id: &str,
) -> KeyOwnership {
    let settled = layout_holding_session(layouts).and_then(|vault| match vault {
        Some(vault) => key_owner::settle_owner_after_probe(&vault, probed_token, server_user_id),
        None => Ok(KeyOwnership::SessionUnknown),
    });
    settled.unwrap_or_else(|error| {
        tracing::warn!(%error, "startup restore: the credential store could not say whose vault key it holds");
        KeyOwnership::SessionUnknown
    })
}

/// The startup restore, in its turn, after the server named the account of the stored token: record that owner, and
/// judge the stored key against the SERVER's id (in the layout that holds the token, the one the loader read).
pub(crate) fn settle_owner_after_probe_in_keychain(
    _write: &SessionWrite,
    account_id: &str,
    probed_token: &str,
    server_user_id: &str,
) -> KeyOwnership {
    settle_owner_after_probe_over(both_layouts(account_id), probed_token, server_user_id)
}

/// [`complete_pending_keychain_clear`] over any layouts: each is tried, whatever the one before it answered. Returns
/// how many could not be finished (0: nothing is left over).
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub(crate) fn complete_pending_clear_over<S: AuthSecretStore>(
    layouts: impl IntoIterator<Item = AuthVault<S>>,
) -> usize {
    let mut unfinished = 0;
    for (layout, mut vault) in layouts.into_iter().enumerate() {
        match key_owner::complete_pending_clear(&mut vault) {
            Ok(true) => tracing::warn!(
                layout,
                "startup: finished a credential clear that had left a vault key behind"
            ),
            Ok(false) => {}
            Err(error) => {
                unfinished += 1;
                tracing::warn!(layout, %error, "startup: a leftover vault key could not be removed; sign-in will try again")
            }
        }
    }
    unfinished
}

/// Windows, at startup and before any sign-in is offered: finish a clear that failed earlier (a vault key or an owner
/// tag with no session token, in either layout). Not silent: what cannot be removed is logged, and a sign-in is
/// refused while a key that is not its account's is still there. (macOS keeps the key after a rejected token on
/// purpose, R9, and proves it at the next sign-in; Linux stores nothing.)
#[cfg(target_os = "windows")]
pub(crate) fn complete_pending_keychain_clear(_write: &SessionWrite, account_id: &str) {
    complete_pending_clear_over(both_layouts(account_id));
}
