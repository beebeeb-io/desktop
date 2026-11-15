//! Reading a stored session out of a credential store, with the key-owner guard (task 2447). Moved out of the
//! Tauri shell (`session_state.rs`) as text; the shell and the engine's own handle both load through here.
//!
//! Must not: log or return a token or a key outside the `Session` it builds; read the vault key before the
//! owner check passed.

use std::fmt;

use crate::keychain::{self, AuthVault};
use crate::session::Session;

pub fn keychain_error(context: &str, error: impl fmt::Display) -> String {
    format!("{context}: {error}")
}

/// Shared tail of `load_session_from_keychain` for whichever store (id-keyed or
/// legacy) actually holds the session. Loads the token, unlocks the vault
/// (validating the 32-byte key), and recovers the email. Returns `Ok(None)` only
/// if the token vanished between the presence check and here (a race the caller
/// already filtered out); any unlock failure (incl. genuine `NotFound`) is an
/// `Err` so the startup restore can distinguish "no key on this PC" from success.
pub fn load_session_from_vault<S: keychain::AuthSecretStore>(
    mut vault: AuthVault<S>,
    email: Option<String>,
) -> Result<Option<Session>, String> {
    let Some(token) = vault
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?
    else {
        return Ok(None);
    };
    // Task 2447: whose key is this? Asked before the key is read, here, where every loader of token + key passes
    // (startup restore, unlock, re-sign-in; both layouts). Another account's key, or a tagged key next to a session
    // nobody named, is refused; an untagged key (from before the tags) loads as it always did.
    if !crate::signin::key_owner::stored_key_ownership(&vault)
        .map_err(|e| keychain_error("read the vault key's owner", e))?
        .may_load()
    {
        return Err(crate::signin::key_owner::KEY_NOT_THIS_ACCOUNTS.to_string());
    }
    vault
        .unlock()
        .map_err(|e| keychain_error("unlock vault key from Keychain", e))?;
    // Wiped when this function returns (Task 12 fix round 1, item 10): the one copy that outlives it is the session's.
    let mut master_key = zeroize::Zeroizing::new([0u8; 32]);
    master_key.copy_from_slice(vault.master_key().map_err(|e| keychain_error("read vault key", e))?);
    // Prefer an email already known in memory (e.g. carried through from a live
    // unlock). Otherwise recover the one persisted in the credential vault so the
    // Account page can show it after an auto-unlock on relaunch. A session stored
    // before email persistence existed simply yields `None` here (no error) —
    // backward-compatible by construction.
    let email = match email {
        Some(email) => Some(email),
        None => vault
            .account_email()
            .map_err(|e| keychain_error("read account email", e))?,
    };
    Ok(Some(Session {
        token: token.expose_for_request().to_string(),
        master_key: *master_key,
        email,
    }))
}
