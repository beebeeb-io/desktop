//! Whose vault key is this? (task 2447, P0.) Plain functions, so a native UI calls the same rules.
//!
//! A stored vault key and a stored session token each carry an owner tag: the server user id of their account, kept
//! beside them in the credential store. The rules, in one place:
//!
//! 1. A tag is written only with an id the SERVER named for the session the token belongs to (a sign-in answer, the
//!    startup probe), and for a key only after the key was proved to be that account's (the recovery check, a device
//!    approval, a browser handoff). Never from a typed address, never from a profile cached for an earlier session.
//! 2. A tag never outlives what it describes: the store removes the tag BEFORE it writes a new token or key
//!    (`AuthVault::install_session`, `AuthVault::store_wrapped_master_key`), and a removal that fails stops the write.
//!    A crash between the write and the new tag leaves "unknown", never another account's id.
//! 3. Every loader of token + key goes through `load_session_from_vault`, which asks [`stored_key_ownership`] before
//!    it reads the key and refuses anything but [`KeyOwnership::Matches`] or [`KeyOwnership::Unknown`].
//! 4. A token-only sign-in removes a key that is not provably the signing-in account's ([`remove_foreign_key`]), and
//!    is refused while that key cannot be removed.
//!
//! Nothing here reads, copies or logs a key or a token; ids are never logged either.

use crate::keychain::{AuthResult, AuthSecretStore, AuthStoreError, AuthVault};

/// What the loader says when the stored key is not the stored session's (the window then asks for the 12 words).
pub const KEY_NOT_THIS_ACCOUNTS: &str =
    "The key stored on this computer isn\u{2019}t this account\u{2019}s. Enter your recovery phrase to unlock.";

/// What a sign-in says when another account's key is still stored and could not be removed. Nothing was signed in.
pub const FOREIGN_KEY_NOT_REMOVED: &str = "Another account\u{2019}s key is still stored on this computer and Beebeeb couldn\u{2019}t remove it. Nothing was signed in. Restart Beebeeb and try again.";

/// How the stored vault key relates to the stored session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOwnership {
    /// Both tags are known and equal: the key is this session's account's.
    Matches,
    /// The key has no tag (stored before task 2447, or a write that stopped before its tag). Not a match: it is loaded
    /// as it always was, and it is never tagged without a proof.
    Unknown,
    /// Both tags are known and differ: another account's key.
    Foreign,
    /// The key is tagged, the session is not: a token was stored by something that could not say whose it is. The key
    /// cannot be shown to be this session's, so it is refused.
    SessionUnknown,
}

impl KeyOwnership {
    /// Whether a loader may put this key next to the stored token.
    pub fn may_load(self) -> bool {
        matches!(self, Self::Matches | Self::Unknown)
    }
}

/// The one comparison. Blank ids are unknown.
pub fn key_ownership(key_owner: Option<&str>, session_owner: Option<&str>) -> KeyOwnership {
    let known = |id: Option<&str>| id.map(str::trim).filter(|id| !id.is_empty()).map(str::to_owned);
    match (known(key_owner), known(session_owner)) {
        (None, _) => KeyOwnership::Unknown,
        (Some(_), None) => KeyOwnership::SessionUnknown,
        (Some(key), Some(session)) if key == session => KeyOwnership::Matches,
        (Some(_), Some(_)) => KeyOwnership::Foreign,
    }
}

/// Whether a stored vault key is REMOVED when `signing_in_user_id` / `signing_in_email` signs in with a token only
/// (password, 2FA, passkey). A tagged key stays only for its own user id. An untagged key stays only when the email
/// stored beside it is the signing-in account's (the server keeps one account per canonical address); with no stored
/// email nothing says whose it is, and it goes.
///
/// Decision (lane A1b, 2026-11-11; kept by A1c): unknown owner + same email is KEPT, because removing it asks for the
/// 12 words with no sign of another account. Decision (lane A1c, 2026-11-11): unknown owner + NO stored email is
/// REMOVED (A1b kept it), because an untagged key is "unknown", never "matches".
pub fn should_remove_stored_key(
    key_owner: Option<&str>,
    signing_in_user_id: &str,
    signing_in_email: &str,
    stored_email: Option<&str>,
) -> bool {
    match key_owner.map(str::trim).filter(|owner| !owner.is_empty()) {
        Some(owner) => owner != signing_in_user_id.trim(),
        None => !stored_email.is_some_and(|stored| crate::account_binding::same_email(stored, signing_in_email)),
    }
}

/// P2-2: a device approval (or any sign-in that brings token + key) while this PC is already signed in. Allowed only
/// to give the signed-in account its key: the vault is still locked, both ids are known, and they are equal.
pub fn may_add_key_to_signed_in_session(
    signed_in_user_id: Option<&str>,
    approving_user_id: Option<&str>,
    vault_unlocked: bool,
) -> bool {
    !vault_unlocked && key_ownership(approving_user_id, signed_in_user_id) == KeyOwnership::Matches
}

fn is_absent(error: &AuthStoreError) -> bool {
    matches!(error, AuthStoreError::NotFound | AuthStoreError::Unsupported(_))
}

fn key_present<S: AuthSecretStore>(vault: &AuthVault<S>) -> AuthResult<bool> {
    match vault.has_vault_key() {
        Err(error) if is_absent(&error) => Ok(false),
        answer => answer,
    }
}

/// The loader's question, asked BEFORE the key is read: whose key is stored next to this token?
pub fn stored_key_ownership<S: AuthSecretStore>(vault: &AuthVault<S>) -> AuthResult<KeyOwnership> {
    Ok(key_ownership(
        vault.vault_key_owner()?.as_deref(),
        vault.session_owner()?.as_deref(),
    ))
}

/// What the gate shows for a signed-in, locked window. Read from the SAME store the loader reads (the one that holds
/// the session token), with the loader's own ownership rule, so the gate never offers an unlock the loader refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredKey {
    /// A key the loader will accept: the lock screen.
    Usable,
    /// No key: the recovery phrase (or another device).
    Absent,
    /// A key the loader refuses: the recovery phrase replaces it.
    NotThisAccounts,
    /// The store could not answer. Neither "present" nor "absent".
    Unreadable,
}

/// [`StoredKey`] for one store. On Windows the presence check loads the key and drops it at once (`SecretBytes` wipes
/// itself); the macOS Keychain answers from the item's attributes. Blocking: call it off the UI thread.
pub fn stored_key_state<S: AuthSecretStore>(vault: &AuthVault<S>) -> StoredKey {
    match key_present(vault) {
        Ok(false) => StoredKey::Absent,
        Err(_) => StoredKey::Unreadable,
        Ok(true) => match stored_key_ownership(vault) {
            Ok(ownership) if ownership.may_load() => StoredKey::Usable,
            Ok(_) => StoredKey::NotThisAccounts,
            Err(_) => StoredKey::Unreadable,
        },
    }
}

/// A token-only sign-in, BEFORE its token is stored: remove a stored key that is not provably the signing-in
/// account's ([`should_remove_stored_key`]), with its tag. `Ok(true)`: one was removed. `Err`: it is still there, and
/// the caller refuses the sign-in ([`FOREIGN_KEY_NOT_REMOVED`]). A tag left without a key is removed too.
pub fn remove_foreign_key<S: AuthSecretStore>(
    vault: &mut AuthVault<S>,
    signing_in_user_id: &str,
    signing_in_email: &str,
) -> AuthResult<bool> {
    if !key_present(vault)? {
        if vault.vault_key_owner()?.is_some() {
            vault.delete_vault_key_and_owner()?;
        }
        return Ok(false);
    }
    let key_owner = vault.vault_key_owner()?;
    let stored_email = vault.account_email()?;
    if !should_remove_stored_key(
        key_owner.as_deref(),
        signing_in_user_id,
        signing_in_email,
        stored_email.as_deref(),
    ) {
        return Ok(false);
    }
    vault.delete_vault_key_and_owner()?;
    Ok(true)
}

/// After a write of token AND key for one session the server named (`user_id`): bind both. Called in the write's own
/// turn. A failure leaves "unknown" (the store already removed the old tags), so it is the caller's to log, not to
/// fail the sign-in on.
pub fn bind_session_and_key<S: AuthSecretStore>(vault: &AuthVault<S>, user_id: &str) -> AuthResult<()> {
    vault.bind_session_owner(user_id)?;
    vault.bind_vault_key_owner(user_id)
}

/// After the recovery check proved the key just stored is the account's of the stored token: the key takes the
/// session's owner. A session of unknown owner leaves the key untagged (unknown), never guessed.
pub fn bind_key_to_session_owner<S: AuthSecretStore>(vault: &AuthVault<S>) -> AuthResult<bool> {
    match vault.session_owner()? {
        Some(owner) => vault.bind_vault_key_owner(&owner).map(|()| true),
        None => Ok(false),
    }
}

/// The owner tag of the stored session, only if the stored token is `token` (a token rotation carries it over to the
/// token the server issued for the same session). Compares in place; copies nothing.
pub fn session_owner_of<S: AuthSecretStore>(vault: &AuthVault<S>, token: &str) -> Option<String> {
    let stored = vault.session_token().ok().flatten()?;
    if stored.expose_for_request() != token {
        return None;
    }
    vault.session_owner().ok().flatten()
}

/// The startup restore, after the server answered for the stored token with `server_user_id`: the server's word is
/// the session's owner (an unknown one is bound, a different one is corrected), and the key is judged against it.
/// Nothing is written unless the stored token is still `probed_token`.
pub fn settle_owner_after_probe<S: AuthSecretStore>(
    vault: &AuthVault<S>,
    probed_token: &str,
    server_user_id: &str,
) -> AuthResult<KeyOwnership> {
    let still_stored = vault
        .session_token()?
        .is_some_and(|stored| stored.expose_for_request() == probed_token);
    if !still_stored {
        return Ok(KeyOwnership::SessionUnknown);
    }
    if vault.session_owner()?.as_deref() != Some(server_user_id) {
        vault.bind_session_owner(server_user_id)?;
    }
    Ok(key_ownership(vault.vault_key_owner()?.as_deref(), Some(server_user_id)))
}

/// A clear that failed must not stay silent (task 2447 d). On Windows a sign-out and a rejected token clear
/// everything, so a store with NO session token that still holds a vault key or an owner tag is a clear that did not
/// finish: this completes it, at the next start, before a sign-in is offered. (macOS keeps the key after a rejected
/// token on purpose, R9, and proves it at the next sign-in, so it does not call this.) `Ok(true)`: something was
/// removed. `Err`: it is still there; the sign-in's own check refuses while it is.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub fn complete_pending_clear<S: AuthSecretStore>(vault: &mut AuthVault<S>) -> AuthResult<bool> {
    match vault.session_token() {
        Ok(Some(_)) => return Ok(false),
        Ok(None) => {}
        Err(error) if is_absent(&error) => {}
        Err(error) => return Err(error),
    }
    let leftover = key_present(vault)? || vault.vault_key_owner()?.is_some() || vault.session_owner()?.is_some();
    if !leftover {
        return Ok(false);
    }
    vault.clear_session()?;
    Ok(true)
}

#[cfg(test)]
#[path = "key_owner_tests.rs"]
mod tests;
