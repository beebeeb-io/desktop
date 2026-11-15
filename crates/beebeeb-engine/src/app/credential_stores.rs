//! Where the handle keeps an account's session token and vault key, and the one older layout it still reads: the
//! flat layout of a 0.8.x PC from before the per-account slots (`io.beebeeb.app/<leaf>`, no account id in the name).
//!
//! At open ([`adopt_legacy_flat`]) a session that still sits in the flat layout moves into the account's slot
//! (`keychain::migrate_legacy_keychain_between`: written, read back, and only then removed from the flat layout).
//! At every clear of the stored session ([`clear_legacy_flat`]) the flat layout is cleared as well: after a move whose
//! read-back failed, the flat items are kept AND the new token may be written, and a clear that skipped them would
//! let the next open's move put the old session back.
//!
//! No read fallback (a contract choice; reverse it only through the lead). The Tauri shell read the flat store when
//! the migration failed; this engine does not. The engine signs in, locks, re-authenticates and signs out on the
//! account's slot only: a session that lived only in a store the engine never writes could not be cleared or replaced
//! correctly. What a failed migration leaves: the person opens signed out (or, when the token was written and its key
//! was not, asked for the twelve words), signs in once, and their local data (kept on Windows) is bound again by R10
//! for the same account. The next open's migration short-circuits (new token and key present) and sweeps the flat
//! items.
//!
//! Must not: log a token, a key, an email or a store error's text (a platform error can name the item it touched);
//! fail an open, or change what a sign-out returns; write a session into the flat layout.

use crate::identity::EngineIdentity;
use crate::keychain::{self, AuthSecretStore, AuthStoreError};

/// Where the handle keeps an account's session token and vault key. The handle names the app on every call
/// (`identity` is the one [`Engine::open`](super::Engine::open) was given): a store has no identity of its own to
/// disagree with it.
pub trait CredentialStores: Send + Sync + 'static {
    type Store: AuthSecretStore;
    fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store;
    /// The flat layout of a 0.8.x PC: read only to move its session into an account's slot, and cleared at every
    /// clear. `None`: these stores never had that layout.
    fn legacy_flat(&self, _identity: &'static EngineIdentity) -> Option<Self::Store> {
        None
    }
}

/// The platform's credential store (Windows Credential Manager, the macOS Keychain), per app and account id.
/// It carries no identity: which app's items it reads is the handle's identity, nothing else (review of #309,
/// P2-1: a test handle given the real app's stores was accepted).
pub struct PlatformCredentials;

impl CredentialStores for PlatformCredentials {
    type Store = crate::keychain::PlatformKeychainStore;
    fn for_account(&self, identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        crate::keychain::platform_keychain_store_for(identity, account_id)
    }
    /// Windows and macOS had the flat layout. Elsewhere the platform store is a fail-closed stub: asking it would log
    /// a warning at every open for nothing.
    fn legacy_flat(&self, identity: &'static EngineIdentity) -> Option<Self::Store> {
        if cfg!(any(target_os = "windows", target_os = "macos")) {
            Some(crate::keychain::legacy_platform_keychain_store(identity))
        } else {
            None
        }
    }
}

/// The test stores' flat layout: a slot no account id can be. Account ids are minted by
/// `DesktopConfig::ensure_account_id_at` as a v4 UUID in its hyphenated form (hex digits and `-`); this one has
/// spaces and angle brackets.
#[cfg(any(test, feature = "test-support"))]
pub const LEGACY_FLAT_SLOT: &str = "<flat layout>";

#[cfg(any(test, feature = "test-support"))]
impl CredentialStores for crate::signin::memory_store::MemoryCredentials {
    type Store = crate::signin::memory_store::MemoryStore;
    fn for_account(&self, _identity: &'static EngineIdentity, account_id: &str) -> Self::Store {
        self.store_for(account_id)
    }
    fn legacy_flat(&self, _identity: &'static EngineIdentity) -> Option<Self::Store> {
        Some(self.store_for(LEGACY_FLAT_SLOT))
    }
}

/// At open, before anything reads the account's slot: a session the flat layout still holds moves into it. Only a
/// flat layout that holds a session token is moved; without one there is nothing to move, and the account's slot is
/// not read (a closed vault's key entry is read by nothing at a start). A refusal of the store is logged by its kind
/// and never fails the open: the flat items then stay where they are, and the next open tries again.
pub(crate) fn adopt_legacy_flat<C: CredentialStores>(identity: &'static EngineIdentity, account_id: &str, stores: &C) {
    let Some(legacy) = stores.legacy_flat(identity) else {
        return;
    };
    if matches!(legacy.holds_session_token(), Ok(false)) {
        return;
    }
    if let Err(error) = keychain::migrate_legacy_keychain_between(&stores.for_account(identity, account_id), &legacy) {
        refused(Step::Move, &error);
    }
}

/// At every clear of the stored session, before the account's slot is cleared: the flat layout's items go too, so no
/// later open can move an old session back in. A refusal is logged by its kind and never changes what the caller
/// returns.
pub(crate) fn clear_legacy_flat<C: CredentialStores>(identity: &'static EngineIdentity, stores: &C) {
    if let Some(legacy) = stores.legacy_flat(identity)
        && let Err(error) = keychain::sweep_legacy(&legacy)
    {
        refused(Step::Clear, &error);
    }
}

#[derive(Clone, Copy)]
enum Step {
    Move,
    Clear,
}

/// A refusal of a step on the flat layout, by its kind only: a platform error's text can name the item it touched.
/// The kinds are words of the log call itself, so the log file writes them as they are.
fn refused(step: Step, error: &AuthStoreError) {
    tracing::warn!(
        step = match step {
            Step::Move => "move",
            Step::Clear => "clear",
        },
        kind = match error {
            AuthStoreError::Unsupported(_) => "unsupported",
            AuthStoreError::NotFound => "not_found",
            AuthStoreError::InvalidSecret(_) => "invalid_secret",
            AuthStoreError::Backend(_) => "backend",
        },
        "the old credential layout was not moved or cleared"
    );
}

#[cfg(test)]
#[path = "credential_stores_tests.rs"]
mod tests;
