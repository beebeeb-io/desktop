//! What every engine start reads before an engine exists: the keys and the identity of their session, together,
//! under the engine slot. Moved out of the Tauri shell (`session_reauth.rs`, `bound_engine.rs`) as text.
//!
//! Must not: hand keys to a start from anywhere but the session in memory; return them as anything but
//! `Zeroizing` copies.

use std::sync::atomic::Ordering;

use crate::{account, account_binding, account_dto};

pub const ENGINE_START_REFUSED_ERROR: &str =
    "Could not start the sync engine: a previous one did not confirm it stopped. Restart Beebeeb.";

/// Why an engine start has no keys to start with.
#[derive(Debug, PartialEq, Eq)]
pub enum EngineStartBlocked {
    /// An engine stop was never confirmed (the sentence says to restart), or the session mutex is unusable.
    Refused(String),
    /// Nothing is signed in with keys in memory: locked, signed out, or a Lock that won the slot first.
    NoSession,
}

/// Shown when a start finds the vault locked at the moment it holds the engine slot.
pub const ENGINE_START_NO_SESSION: &str = "The vault is locked, so sync didn’t start.";

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
pub fn keys_for_engine_start(
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
pub type EngineKeys = (zeroize::Zeroizing<String>, zeroize::Zeroizing<[u8; 32]>);

/// Lead ruling T8-lockflag: every place that puts an engine in the slot calls this with the slot
/// held, so two engines can never run at once. The flag is set when an engine's stop could not be
/// confirmed (its task may still be running with the keys) and stays set until a restart.
/// `finder_setup_wiring_tests` keeps the list of starts complete.
pub fn engine_start_refusal(acct: &account::AccountRuntime) -> Result<(), String> {
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

/// R10: who a session is, from the session's OWN email and, when the cached profile is that account's, its
/// user id. The profile counts only when its email is exactly the session's, so a profile left over from
/// another account is never taken for this one; a session with no email is unidentified (and an owner that
/// cannot be compared with it refuses the start).
pub fn identity_of_session(
    email: Option<&str>,
    profile: Option<&account_dto::AccountProfile>,
) -> account_binding::Identity {
    let email = email.map(str::trim).filter(|email| !email.is_empty());
    let profile = profile.filter(|profile| email.is_some_and(|email| profile.email.trim() == email));
    account_binding::Identity::new(profile.map(|profile| profile.user_id.as_str()), email)
}
