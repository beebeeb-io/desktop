//! `accounts` items split out of `keychain.rs`.

use super::*;

/// One account's id (the runner's `account.id`), used only as a path segment of
/// the credential item names. It carries no secret: the leaves do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl From<&str> for AccountId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for AccountId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::ops::Deref for AccountId {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

/// The secret leaves a store holds per account. Each has ONE wire name;
/// the platform stores only ever see `leaf()`, so a typo cannot mint an
/// extra item.
/// macOS and Windows stores (and the in-memory test store) are the only users:
/// the Linux build has no credential vault to name items in.
#[cfg(any(any(test, feature = "test-support"), target_os = "macos", target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeychainAccount {
    SessionToken,
    WrappedMasterKey,
    /// Account email (PII) persisted alongside the session so the Account page
    /// can show it after an auto-unlock on relaunch — the credential store is
    /// the only place it survives, since neither the token nor the wrapped key
    /// carries it. Stored in the SAME protected vault as the secrets (NOT a
    /// plaintext config file): account metadata, not key material, so it is a
    /// plain UTF-8 string rather than `SecretBytes`.
    AccountEmail,
    /// Server user id of the account whose vault key is stored in
    /// `WrappedMasterKey` (task 2447). A sibling credential entry, not
    /// derived crypto. An entry with no `VaultKeyOwner` is "unknown owner"
    /// (legacy, pre-tag). Written whenever the vault key is stored after a
    /// VERIFIED unlock (recovery phrase, device approval, browser handoff).
    VaultKeyOwner,
    /// Server user id of the account the stored `SessionToken` belongs to (task 2447): written only from the
    /// server's own answer for that token, and removed before any other token is stored. Not a secret.
    SessionOwner,
}

#[cfg(any(any(test, feature = "test-support"), target_os = "macos", target_os = "windows"))]
impl KeychainAccount {
    #[cfg(any(test, feature = "test-support"))]
    pub const ALL: [Self; 5] = [
        Self::SessionToken,
        Self::WrappedMasterKey,
        Self::AccountEmail,
        Self::VaultKeyOwner,
        Self::SessionOwner,
    ];

    pub const fn leaf(self) -> &'static str {
        match self {
            Self::SessionToken => "session-token",
            Self::WrappedMasterKey => "wrapped-master-key",
            Self::AccountEmail => "account-email",
            Self::VaultKeyOwner => "vault-key-owner",
            Self::SessionOwner => "session-owner",
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_leaf(leaf: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|account| account.leaf() == leaf)
    }
}

/// Compose the per-account credential leaf.
///
/// `Some(id)` → `"<id>/<leaf>"` (segmented, Phase 1); `None` → bare `leaf`
/// (legacy, pre-Phase-1 layout). Shared by every concrete store so the
/// composition rule lives in exactly one place.
/// The per-account credential leaf name, used by the platform stores and the
/// tests that model them; the Linux build has no vault to name items in.
#[cfg(any(any(test, feature = "test-support"), target_os = "macos", target_os = "windows"))]
pub(super) fn account_path(prefix: Option<&AccountId>, account: KeychainAccount) -> String {
    match prefix {
        Some(id) => format!("{id}/{}", account.leaf()),
        None => account.leaf().to_string(),
    }
}

/// The whole name of one stored item: `<the identity's credential service>/<account path>`. On Windows it is
/// the credential's target name; on macOS the Keychain item's service and account. The one place the two halves
/// are joined, so a store cannot name an item outside its identity's service.
#[cfg(any(any(test, feature = "test-support"), target_os = "macos", target_os = "windows"))]
pub(super) fn item_name(service: &str, path: &str) -> String {
    format!("{service}/{path}")
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
pub fn platform_keychain_store_for(identity: &'static EngineIdentity, account_id: &str) -> PlatformKeychainStore {
    PlatformKeychainStore::with_account(identity, account_id)
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
pub fn legacy_platform_keychain_store(identity: &'static EngineIdentity) -> PlatformKeychainStore {
    PlatformKeychainStore::new(identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three secret leaves are the wire names the platform stores use; a
    /// fourth name cannot appear and a leaf round-trips to exactly one account.
    #[test]
    fn keychain_account_names_every_service_leaf_once() {
        let mut leaves = Vec::new();
        for account in KeychainAccount::ALL {
            // Exhaustive on purpose: a new variant must decide its wire name here.
            let leaf = match account {
                KeychainAccount::SessionToken => "session-token",
                KeychainAccount::WrappedMasterKey => "wrapped-master-key",
                KeychainAccount::AccountEmail => "account-email",
                KeychainAccount::VaultKeyOwner => "vault-key-owner",
                KeychainAccount::SessionOwner => "session-owner",
            };
            assert_eq!(account.leaf(), leaf);
            assert_eq!(KeychainAccount::from_leaf(leaf), Some(account));
            leaves.push(leaf);
        }
        assert_eq!(
            leaves,
            [
                "session-token",
                "wrapped-master-key",
                "account-email",
                "vault-key-owner",
                "session-owner"
            ]
        );
        assert_eq!(KeychainAccount::from_leaf("master-key"), None);
    }

    /// The account id segments the leaf (`<id>/<leaf>`); without one the leaf
    /// keeps the legacy flat name, so an upgrade reads the entries it wrote.
    #[test]
    fn account_id_segments_every_leaf_and_legacy_stays_flat() {
        let id = AccountId::new("acct-uuid-0001");
        assert_eq!(
            account_path(Some(&id), KeychainAccount::SessionToken),
            "acct-uuid-0001/session-token"
        );
        assert_eq!(
            account_path(Some(&id), KeychainAccount::AccountEmail),
            "acct-uuid-0001/account-email"
        );
        for account in KeychainAccount::ALL {
            assert_eq!(account_path(None, account), account.leaf());
        }
    }
}
