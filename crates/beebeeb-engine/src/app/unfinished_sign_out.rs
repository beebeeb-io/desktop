//! A sign-out that did not finish is never taken for one that did. It can stop short in two ways:
//!
//! - **A stop whose end was never confirmed.** A stop takes the engine out of its slot before it knows whether the
//!   engine's task ends (`EngineRunner::abort` consumes it). When the end is not confirmed, the task may still run with
//!   the keys, and the account's `engine_stop_unconfirmed` is the only record left of it (set by `Engine::stop_runner`
//!   and by a start's step 2). An empty slot then answers from that record: `SyncStop::Unconfirmed`, never
//!   `WasNotRunning`, until Beebeeb restarts (the record lives in memory only). So a sign-out pressed again is refused
//!   with the same words ("Quit and reopen Beebeeb"): it neither clears the credential store nor ends the session at
//!   the server while that task may hold the keys. Every other stop sees the same answer (a vault lock reports it; the
//!   strong unlock of a locked screen and File Explorer's Repair and turn-off refuse on it). 0.8.x kept this gate in
//!   its sign-out (`stop_engine_for_sign_out`).
//! - **A clear cut short.** A sign-out's clear removes the session token first, then the key, the address and the
//!   owner tags (`AuthVault::clear_session`). A crash, a power loss or a removal that failed in between leaves a key or
//!   an owner tag with no token. The next start finishes that clear before it reads whether a session is stored
//!   (`signin::key_owner::complete_pending_clear`), in the account's slot and in a 0.8.x PC's flat layout, as 0.8.x's
//!   Windows start did. Not on macOS: `complete_pending_clear`'s own rule (a Mac keeps a key after the server refused
//!   its token, on purpose).
//!
//! Must not: log a key, a token, an address or an id (the log lines are fixed words).

use std::sync::atomic::Ordering;

use crate::account::AccountRuntime;
#[cfg(not(target_os = "macos"))]
use crate::keychain::AuthSecretStore;
use crate::keychain::AuthVault;
#[cfg(not(target_os = "macos"))]
use crate::signin::key_owner;

use super::{CredentialStores, Engine, SyncStop};

/// What a stop answers when the engine slot is empty: `Unconfirmed` while an earlier stop of this account was never
/// confirmed (its task may still run), else `WasNotRunning`.
pub(super) fn stop_of_an_empty_slot(acct: &AccountRuntime) -> SyncStop {
    if acct.engine_stop_unconfirmed.load(Ordering::SeqCst) {
        SyncStop::Unconfirmed
    } else {
        SyncStop::WasNotRunning
    }
}

impl<C: CredentialStores> Engine<C> {
    /// The account's credential slot at start, after a clear that was cut short is finished (in the flat layout too).
    /// [`Engine::restore`] reads the stored session through it, in its step, so nothing reads a leftover first.
    pub(super) fn vault_at_start(&self) -> AuthVault<C::Store> {
        #[cfg(not(target_os = "macos"))]
        {
            if finish_a_clear_cut_short(&mut self.vault()) {
                tracing::warn!("startup: finished an interrupted credential clear");
            }
            if let Some(flat) = self.stores.legacy_flat(self.identity)
                && finish_a_clear_cut_short(&mut AuthVault::new(flat))
            {
                tracing::warn!("startup: finished an interrupted credential clear of the old layout");
            }
        }
        self.vault()
    }
}

/// [`key_owner::complete_pending_clear`] on one layout: `true` when it removed something. A removal that fails is
/// logged by fixed words and tried again at the next start; a sign-in meanwhile removes a key that is not its account's
/// before it stores its token, or is refused (`key_owner::remove_foreign_key`).
#[cfg(not(target_os = "macos"))]
fn finish_a_clear_cut_short<S: AuthSecretStore>(vault: &mut AuthVault<S>) -> bool {
    match key_owner::complete_pending_clear(vault) {
        Ok(removed) => removed,
        Err(_) => {
            tracing::warn!("startup: an interrupted credential clear could not be finished");
            false
        }
    }
}
