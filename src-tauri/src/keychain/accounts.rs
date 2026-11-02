//! `accounts` items split out of `keychain.rs`.

use super::*;

/// Compose the per-account credential leaf from an optional prefix.
///
/// `Some(id)` → `"<id>/<leaf>"` (segmented, Phase 1); `None` → bare `leaf`
/// (legacy, pre-Phase-1 layout). Shared by every concrete store so the
/// composition rule lives in exactly one place.
pub(super) fn account_path(prefix: Option<&str>, leaf: &str) -> String {
    match prefix {
        Some(id) => format!("{id}/{leaf}"),
        None => leaf.to_string(),
    }
}

/// Construct the secret store for the current OS, segmented under `account_id`
/// (task 0800 — multi-account Phase 1). Used by the Tauri runner so
/// session/vault persistence routes to the platform-native credential vault
/// under `io.beebeeb.app/<account_id>/<leaf>`.
///
/// This is the ONLY constructor production code should use. The bare legacy
/// constructor is deliberately not exposed (see `legacy_platform_keychain_store`)
/// so a reviewer can grep every legacy-keyed access — there is exactly one, in
/// the migration path.
pub fn platform_keychain_store_for(account_id: &str) -> PlatformKeychainStore {
    PlatformKeychainStore::with_account(account_id)
}

/// Construct the legacy (un-segmented) secret store — the pre-Phase-1 flat
/// `io.beebeeb.app/<leaf>` layout.
///
/// Used in EXACTLY two places, both read-or-delete-only:
///   1. the one-time `migrate_legacy_keychain_to_account` (read legacy secrets,
///      then delete them only after verifying the segmented copies); and
///   2. the legacy READ FALLBACK on the auto-unlock path (if the id-keyed
///      lookup returns `None`, fall back to the legacy store before concluding
///      "no session") — the anti-lockout safety net.
///
/// New session WRITES must NEVER go through this store (that would re-create the
/// orphan-able flat entries). Production write paths use
/// `platform_keychain_store_for` exclusively.
pub fn legacy_platform_keychain_store() -> PlatformKeychainStore {
    PlatformKeychainStore::new()
}
