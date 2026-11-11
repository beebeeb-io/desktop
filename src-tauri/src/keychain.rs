//! Desktop auth, unlock, and secret-storage primitives.
//!
//! This module backs the Tauri runner's login, lock, unlock, and logout flow.
//! It establishes the fail-closed state model and Keychain-backed storage
//! surface used before the sync engine receives any file-content key material.

use std::fmt;

/// The service every platform store (and the test store) names its items with; the Linux stub has no item to name.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const KEYCHAIN_SERVICE: &str = crate::app_identity::AppIdentity::CURRENT.credential_service();
const MASTER_KEY_BYTES: usize = 32;

#[derive(PartialEq, Eq)]
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    /// The load path of the macOS and Windows stores, and of tests: stored bytes become a secret only once proven
    /// non-empty. A vault key is built with `new_master_key` instead, never from raw bytes.
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn new(bytes: impl Into<Vec<u8>>) -> AuthResult<Self> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(AuthStoreError::InvalidSecret("secret bytes are empty"));
        }
        Ok(Self(bytes))
    }

    /// The vault key, copied once into the wiped buffer (`Drop` zeroizes it). Borrowed, so the caller's array is the
    /// only other copy and stays the caller's to wipe: no by-value copy is left on this function's stack.
    pub fn new_master_key(bytes: &[u8; MASTER_KEY_BYTES]) -> Self {
        Self(bytes.to_vec())
    }

    pub fn expose_for_crypto(&self) -> &[u8] {
        &self.0
    }
}

impl zeroize::Zeroize for SecretBytes {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        // Volatile writes (`zeroize`): a plain fill before the free may be removed as a dead store.
        zeroize::Zeroize::zeroize(self);
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBytes(<redacted>, len={})", self.0.len())
    }
}

// ── Keychain account segmentation (task 0800 — multi-account Phase 1) ─────────
//
// Every store carries an optional `account_prefix`. When set, all six secret
// leaves are stored under `<account_id>/<leaf>` (e.g.
// `io.beebeeb.app/<uuid>/session-token`) instead of the bare legacy leaf
// (`io.beebeeb.app/session-token`). `account_path` composes that. The platform
// free functions (`macos_keychain::save/load/delete`, `windows_credentials::…`)
// are UNCHANGED — they already take an `account: &str`, so the segmentation is
// purely a matter of what string we pass them. `AuthVault` is likewise
// unchanged: the id lives entirely in the store.
//
// `None` prefix = the legacy flat layout, used ONLY by `legacy_platform_keychain_store`
// for the one-time migration read/sweep. New code must construct stores via
// `platform_keychain_store_for(id)` so a reviewer can grep every legacy access.

// The real Keychain store does not EXIST in a macOS test build (Task 9 fix round 2): a test that names it
// fails to compile, instead of reaching the developer's own Keychain items. `TestKeychainStore` takes its
// place (see `PlatformKeychainStore`).
#[cfg(all(target_os = "macos", not(test)))]
pub struct MacOsKeychainStore {
    account_prefix: Option<AccountId>,
}

#[cfg(all(target_os = "macos", not(test)))]
impl MacOsKeychainStore {
    /// Legacy flat store (no account segmentation). Prefer `with_account`.
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    /// Store segmented under `account_id` (Phase 1).
    pub fn with_account(account_id: impl Into<AccountId>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }

    fn account_path(&self, account: KeychainAccount) -> String {
        account_path(self.account_prefix.as_ref(), account)
    }
}

#[cfg(all(target_os = "macos", not(test)))]
impl Default for MacOsKeychainStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(target_os = "macos", not(test)))]
impl AuthSecretStore for MacOsKeychainStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        macos_keychain::save(
            &self.account_path(KeychainAccount::SessionToken),
            token.expose_for_request().as_bytes(),
        )
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        macos_keychain::load(&self.account_path(KeychainAccount::SessionToken))?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| AuthStoreError::InvalidSecret("session token is not UTF-8"))
                    .and_then(SessionToken::new)
            })
            .transpose()
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(KeychainAccount::SessionToken))
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        macos_keychain::save(
            &self.account_path(KeychainAccount::WrappedMasterKey),
            wrapped.expose_for_crypto(),
        )
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        macos_keychain::load(&self.account_path(KeychainAccount::WrappedMasterKey))?
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(KeychainAccount::WrappedMasterKey))
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        macos_keychain::save(&self.account_path(KeychainAccount::AccountEmail), email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        macos_keychain::load(&self.account_path(KeychainAccount::AccountEmail))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8"))
            })
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(KeychainAccount::AccountEmail))
    }

    fn holds_session_token(&self) -> AuthResult<bool> {
        macos_keychain::exists(&self.account_path(KeychainAccount::SessionToken))
    }

    fn holds_wrapped_master_key(&self) -> AuthResult<bool> {
        macos_keychain::exists(&self.account_path(KeychainAccount::WrappedMasterKey))
    }

    fn holds_account_email(&self) -> AuthResult<bool> {
        macos_keychain::exists(&self.account_path(KeychainAccount::AccountEmail))
    }

    fn save_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        macos_keychain::save(&self.account_path(KeychainAccount::VaultKeyOwner), user_id.as_bytes())
    }

    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        macos_keychain::load(&self.account_path(KeychainAccount::VaultKeyOwner))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("vault key owner is not UTF-8"))
            })
            .transpose()
    }

    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(KeychainAccount::VaultKeyOwner))
    }

    fn save_session_owner(&self, user_id: &str) -> AuthResult<()> {
        macos_keychain::save(&self.account_path(KeychainAccount::SessionOwner), user_id.as_bytes())
    }

    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        macos_keychain::load(&self.account_path(KeychainAccount::SessionOwner))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("session owner is not UTF-8"))
            })
            .transpose()
    }

    fn delete_session_owner(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(KeychainAccount::SessionOwner))
    }
}

#[cfg(not(target_os = "macos"))]
pub struct MacOsKeychainStore {
    // Carried for type parity with the macOS store so the same
    // `with_account`/`platform_keychain_store_for` plumbing compiles on every
    // target. Unused on the stub path (every method returns Unsupported).
    #[allow(dead_code)]
    account_prefix: Option<AccountId>,
}

#[cfg(not(target_os = "macos"))]
impl MacOsKeychainStore {
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    pub fn with_account(account_id: impl Into<AccountId>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl Default for MacOsKeychainStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_os = "macos"))]
impl AuthSecretStore for MacOsKeychainStore {
    fn save_session_token(&self, _token: &SessionToken) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "macOS Keychain is unavailable on this platform",
        ))
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        Err(AuthStoreError::Unsupported(
            "macOS Keychain is unavailable on this platform",
        ))
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "macOS Keychain is unavailable on this platform",
        ))
    }

    fn save_wrapped_master_key(&self, _wrapped: SecretBytes) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "macOS Keychain is unavailable on this platform",
        ))
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        Err(AuthStoreError::Unsupported(
            "macOS Keychain is unavailable on this platform",
        ))
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "macOS Keychain is unavailable on this platform",
        ))
    }
}

// ── Windows Credential Manager store ──────────────────────────────────────────
//
// Win32-Credential-Manager-backed parallel to `MacOsKeychainStore`. Mirrors the
// same two-item model the macOS store uses:
//   - "io.beebeeb.app/session-token"      → UTF-8 session token blob
//   - "io.beebeeb.app/wrapped-master-key" → raw wrapped master-key bytes
//
// Target names embed the service prefix so credentials are namespaced under the
// app identifier, the same way the macOS store scopes items by service
// `io.beebeeb.app` + account. Generic credentials (`CRED_TYPE_GENERIC`) persisted
// per-user (`CRED_PERSIST_ENTERPRISE`) so the secrets follow the signed-in user
// rather than the machine.

#[cfg(target_os = "windows")]
pub struct WindowsCredentialStore {
    account_prefix: Option<AccountId>,
}

#[cfg(target_os = "windows")]
impl WindowsCredentialStore {
    /// Legacy flat store (no account segmentation). Prefer `with_account`.
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    /// Store segmented under `account_id` (Phase 1).
    pub fn with_account(account_id: impl Into<AccountId>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }

    fn account_path(&self, account: KeychainAccount) -> String {
        account_path(self.account_prefix.as_ref(), account)
    }
}

#[cfg(target_os = "windows")]
impl Default for WindowsCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl AuthSecretStore for WindowsCredentialStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        windows_credentials::save(
            &self.account_path(KeychainAccount::SessionToken),
            token.expose_for_request().as_bytes(),
        )
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        windows_credentials::load(&self.account_path(KeychainAccount::SessionToken))?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|e| {
                        // Bearer-token hygiene: the raw blob is the (malformed)
                        // session token. Recover the bytes from the error and
                        // zero them (with `zeroize`, not a plain fill the
                        // compiler may drop) before discarding so the secret
                        // doesn't linger in a freed allocation.
                        let mut raw = e.into_bytes();
                        zeroize::Zeroize::zeroize(&mut raw);
                        AuthStoreError::InvalidSecret("session token is not UTF-8")
                    })
                    .and_then(SessionToken::new)
            })
            .transpose()
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        windows_credentials::delete(&self.account_path(KeychainAccount::SessionToken))
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        windows_credentials::save(
            &self.account_path(KeychainAccount::WrappedMasterKey),
            wrapped.expose_for_crypto(),
        )
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        windows_credentials::load(&self.account_path(KeychainAccount::WrappedMasterKey))?
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        windows_credentials::delete(&self.account_path(KeychainAccount::WrappedMasterKey))
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        windows_credentials::save(&self.account_path(KeychainAccount::AccountEmail), email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        windows_credentials::load(&self.account_path(KeychainAccount::AccountEmail))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8"))
            })
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        windows_credentials::delete(&self.account_path(KeychainAccount::AccountEmail))
    }

    fn save_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        windows_credentials::save(&self.account_path(KeychainAccount::VaultKeyOwner), user_id.as_bytes())
    }

    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        windows_credentials::load(&self.account_path(KeychainAccount::VaultKeyOwner))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("vault key owner is not UTF-8"))
            })
            .transpose()
    }

    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        windows_credentials::delete(&self.account_path(KeychainAccount::VaultKeyOwner))
    }

    fn save_session_owner(&self, user_id: &str) -> AuthResult<()> {
        windows_credentials::save(&self.account_path(KeychainAccount::SessionOwner), user_id.as_bytes())
    }

    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        windows_credentials::load(&self.account_path(KeychainAccount::SessionOwner))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("session owner is not UTF-8"))
            })
            .transpose()
    }

    fn delete_session_owner(&self) -> AuthResult<()> {
        windows_credentials::delete(&self.account_path(KeychainAccount::SessionOwner))
    }
}

// ── In-memory store for unit tests (macOS test builds) ───────────────────────

/// Every item a test-build store holds, keyed `<service>/<account path>`, shared by the whole test
/// process like the real Keychain is shared by the whole machine (but never touching it).
#[cfg(all(test, target_os = "macos"))]
static TEST_KEYCHAIN: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>> =
    std::sync::LazyLock::new(Default::default);

/// Which account ids a test-build store was asked about (`None` prefix: the legacy flat layout).
/// Lets a test prove that the code it ran went through THIS store: `test_store_touched`.
#[cfg(all(test, target_os = "macos"))]
static TEST_KEYCHAIN_TOUCHED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<Option<String>>>> =
    std::sync::LazyLock::new(Default::default);

/// Account prefixes whose email writes fail, as a locked Keychain's would (Task 12 fix round 2, item 3). Tests only.
#[cfg(all(test, target_os = "macos"))]
static TEST_KEYCHAIN_EMAIL_WRITES_FAIL: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<Option<String>>>,
> = std::sync::LazyLock::new(Default::default);

/// Make the test-build store's email writes for this account prefix fail (`true`) or work again (`false`).
#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)] // used by the lib's tests; `tests/keychain.rs` includes this file and does not
pub fn test_fail_account_email_writes(account_prefix: Option<&str>, fail: bool) {
    let mut failing = TEST_KEYCHAIN_EMAIL_WRITES_FAIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if fail {
        failing.insert(account_prefix.map(str::to_string));
    } else {
        failing.remove(&account_prefix.map(str::to_string));
    }
}

/// Account prefixes whose deletes fail, as a Keychain that refuses a removal would (task 1886). Tests only.
#[cfg(all(test, target_os = "macos"))]
static TEST_KEYCHAIN_DELETES_FAIL: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<Option<String>>>> =
    std::sync::LazyLock::new(Default::default);

/// Make the test-build store's deletes for this account prefix fail (`true`) or work again (`false`).
#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)] // used by the lib's tests; `tests/keychain.rs` includes this file and does not
pub fn test_fail_deletes(account_prefix: Option<&str>, fail: bool) {
    let mut failing = TEST_KEYCHAIN_DELETES_FAIL.lock().unwrap_or_else(|e| e.into_inner());
    if fail {
        failing.insert(account_prefix.map(str::to_string));
    } else {
        failing.remove(&account_prefix.map(str::to_string));
    }
}

/// Whether any test-build store with this account prefix has been read, written or cleared.
#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)] // used by the lib's tests; `tests/keychain.rs` includes this file and does not
pub fn test_store_touched(account_prefix: Option<&str>) -> bool {
    TEST_KEYCHAIN_TOUCHED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&account_prefix.map(str::to_string))
}

/// The in-memory stand-in for `MacOsKeychainStore` (which does not exist in a test build), with the same semantics (absent → `None`, a
/// delete of an absent item succeeds) and the same item layout.
#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)] // `tests/keychain.rs` includes this file and never uses it
pub struct TestKeychainStore {
    account_prefix: Option<AccountId>,
}

#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
impl TestKeychainStore {
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    pub fn with_account(account_id: impl Into<AccountId>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }

    fn key(&self, account: KeychainAccount) -> String {
        TEST_KEYCHAIN_TOUCHED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.account_prefix.as_deref().map(str::to_string));
        format!(
            "{KEYCHAIN_SERVICE}/{}",
            account_path(self.account_prefix.as_ref(), account)
        )
    }

    fn save(&self, account: KeychainAccount, bytes: &[u8]) -> AuthResult<()> {
        TEST_KEYCHAIN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.key(account), bytes.to_vec());
        Ok(())
    }

    fn load(&self, account: KeychainAccount) -> Option<Vec<u8>> {
        TEST_KEYCHAIN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.key(account))
            .cloned()
    }

    fn delete(&self, account: KeychainAccount) -> AuthResult<()> {
        if TEST_KEYCHAIN_DELETES_FAIL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&self.account_prefix.as_deref().map(str::to_string))
        {
            return Err(AuthStoreError::Backend(
                "the test Keychain refuses this removal".to_string(),
            ));
        }
        TEST_KEYCHAIN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.key(account));
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
impl AuthSecretStore for TestKeychainStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        self.save(KeychainAccount::SessionToken, token.expose_for_request().as_bytes())
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.load(KeychainAccount::SessionToken)
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| AuthStoreError::InvalidSecret("session token is not UTF-8"))
                    .and_then(SessionToken::new)
            })
            .transpose()
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        self.delete(KeychainAccount::SessionToken)
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.save(KeychainAccount::WrappedMasterKey, wrapped.expose_for_crypto())
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        self.load(KeychainAccount::WrappedMasterKey)
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        self.delete(KeychainAccount::WrappedMasterKey)
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        if TEST_KEYCHAIN_EMAIL_WRITES_FAIL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&self.account_prefix.as_deref().map(str::to_string))
        {
            return Err(AuthStoreError::Backend(
                "the test Keychain refuses this write".to_string(),
            ));
        }
        self.save(KeychainAccount::AccountEmail, email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        self.load(KeychainAccount::AccountEmail)
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8"))
            })
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        self.delete(KeychainAccount::AccountEmail)
    }

    fn save_vault_key_owner(&self, user_id: &str) -> AuthResult<()> {
        self.save(KeychainAccount::VaultKeyOwner, user_id.as_bytes())
    }

    fn load_vault_key_owner(&self) -> AuthResult<Option<String>> {
        self.load(KeychainAccount::VaultKeyOwner)
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("vault key owner is not UTF-8"))
            })
            .transpose()
    }

    fn delete_vault_key_owner(&self) -> AuthResult<()> {
        self.delete(KeychainAccount::VaultKeyOwner)
    }

    fn save_session_owner(&self, user_id: &str) -> AuthResult<()> {
        self.save(KeychainAccount::SessionOwner, user_id.as_bytes())
    }

    fn load_session_owner(&self) -> AuthResult<Option<String>> {
        self.load(KeychainAccount::SessionOwner)
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("session owner is not UTF-8"))
            })
            .transpose()
    }

    fn delete_session_owner(&self) -> AuthResult<()> {
        self.delete(KeychainAccount::SessionOwner)
    }
}

// ── Per-OS store selection ────────────────────────────────────────────────────
//
// `PlatformKeychainStore` is the concrete store the Tauri runner constructs.
// It mirrors macOS wiring: on macOS the secrets live in the Keychain, on Windows
// in Credential Manager.
//
// On Linux (and any other non-macOS, non-Windows target) the alias resolves to
// `MacOsKeychainStore`, which on those targets is compiled from its
// `#[cfg(not(target_os = "macos"))]` stub — every `AuthSecretStore` method
// returns `AuthStoreError::Unsupported`. So Linux has no native secret backend
// yet and stays fail-closed (no persistence, no silent plaintext fallback);
// its behaviour is unchanged by the Windows work.

#[cfg(all(target_os = "macos", not(test)))]
pub type PlatformKeychainStore = MacOsKeychainStore;

/// A unit-test build of the lib never reaches the real Keychain: `PlatformKeychainStore` is an
/// in-memory store, so no test can read, write or clear the developer's own items (the installed app
/// uses the same service name; a full sign-out also clears the legacy flat items). Task 9 fix round 1:
/// the sign-out tests wrote to the real Keychain through `clear_keychain_session`. Pinned by
/// `keychain_isolation_tests` in `lib.rs`. Windows and Linux are unchanged.
#[cfg(all(target_os = "macos", test))]
#[allow(dead_code)] // `tests/keychain.rs` includes this file and never uses the alias
pub type PlatformKeychainStore = TestKeychainStore;

#[cfg(target_os = "windows")]
pub type PlatformKeychainStore = WindowsCredentialStore;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub type PlatformKeychainStore = MacOsKeychainStore;

// ── Lazy migration: legacy flat trio → id-segmented trio (task 0800) ─────────
//
// VAULT-LOCKOUT-class change. The contract that protects against lockout:
//   • write-new → VERIFY readback → ONLY THEN delete legacy (§3.4-3.6); and
//   • a legacy READ FALLBACK on the unlock path (lib.rs) — two independent nets.
// The function is IDEMPOTENT and INTERRUPT-CONVERGENT: interrupted after a crash
// at any step, a re-run ends fully-migrated-with-legacy-gone.

#[cfg(target_os = "windows")]
mod windows_credentials {
    //! Win32 Credential Manager backing for `WindowsCredentialStore`.
    //!
    //! Uses generic credentials (`CRED_TYPE_GENERIC`). Each call builds a
    //! wide (UTF-16) target name `io.beebeeb.app/<account>` and round-trips
    //! the secret as an opaque blob via `CredWriteW` / `CredReadW` /
    //! `CredDeleteW`. `CredReadW` allocates a `CREDENTIALW`; we copy the blob
    //! out and free it with `CredFree` before returning.

    use super::{AuthResult, AuthStoreError, KEYCHAIN_SERVICE};
    use windows::Win32::Security::Credentials::{
        CRED_PERSIST_ENTERPRISE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
    };
    use windows::core::{HRESULT, PCWSTR, PWSTR};

    /// `HRESULT_FROM_WIN32(ERROR_NOT_FOUND)` — `ERROR_NOT_FOUND` (1168) wrapped as
    /// an `HRESULT` (`0x80070490`). The windows 0.58 wrappers for `CredReadW` /
    /// `CredDeleteW` already capture the Win32 error and surface it through
    /// `Error::code()`, so we compare against this instead of a second, racy
    /// `GetLastError()`. The `u32 as i32` cast reproduces the correct negative
    /// `HRESULT` bit pattern (the literal exceeds `i32::MAX`).
    const HRESULT_ERROR_NOT_FOUND: HRESULT = HRESULT(0x80070490u32 as i32);

    /// Build the UTF-16, NUL-terminated target name `io.beebeeb.app/<account>`.
    /// Returned `Vec<u16>` must outlive every pointer derived from it.
    fn target_name(account: &str) -> Vec<u16> {
        let combined = format!("{KEYCHAIN_SERVICE}/{account}");
        combined.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn save(account: &str, secret: &[u8]) -> AuthResult<()> {
        // `target` and `secret` must stay alive for the whole `CredWriteW` call
        // because the struct holds raw pointers into them.
        let mut target = target_name(account);
        if secret.len() > u32::MAX as usize {
            return Err(AuthStoreError::InvalidSecret("secret too large for credential blob"));
        }

        // SAFETY: zeroed CREDENTIALW is a valid "empty" credential; we fill in
        // every field the API reads (Type, TargetName, CredentialBlob*, Persist).
        let mut cred: CREDENTIALW = unsafe { std::mem::zeroed() };
        cred.Type = CRED_TYPE_GENERIC;
        // `cred.TargetName` is a raw PWSTR borrowing `target`'s buffer; the
        // borrow is untracked by the type system, so `target` (and `secret`,
        // borrowed by `CredentialBlob` below) MUST outlive `cred` and the
        // `CredWriteW` call. Both are owned locals dropped after the call, so
        // the pointers stay valid for the entire `unsafe` block.
        cred.TargetName = PWSTR(target.as_mut_ptr());
        cred.CredentialBlobSize = secret.len() as u32;
        // Cast away const — the API treats the blob as read-only for a write.
        cred.CredentialBlob = secret.as_ptr() as *mut u8;
        cred.Persist = CRED_PERSIST_ENTERPRISE;

        // SAFETY: `cred` is fully initialised above and all its pointers
        // (`target`, `secret`) outlive this call. Flags = 0 per the API.
        let result = unsafe { CredWriteW(&cred, 0) };
        result.map_err(|e| status_error("CredWriteW", e.code().0))
    }

    pub fn load(account: &str) -> AuthResult<Option<Vec<u8>>> {
        let target = target_name(account);
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();

        // SAFETY: `target` is a valid NUL-terminated UTF-16 string that outlives
        // the call; `credential` receives an owned pointer we must `CredFree`.
        let result = unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0, &mut credential) };

        if let Err(e) = result {
            // The windows 0.58 `CredReadW` wrapper already captured the failing
            // Win32 code into `e`; use it directly rather than a second,
            // stale-race `GetLastError()`. On failure the out-pointer is
            // normally left null, but guard CredFree anyway in case the API
            // populated it before erroring (latent-leak hardening).
            if !credential.is_null() {
                // SAFETY: `credential` was allocated by `CredReadW`; free once.
                unsafe { CredFree(credential as *const _ as *const core::ffi::c_void) };
            }
            // Map "no such credential" to `Ok(None)`, mirroring the macOS
            // store's ERR_SEC_ITEM_NOT_FOUND → Ok(None) handling.
            if e.code() == HRESULT_ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(status_error("CredReadW", e.code().0));
        }

        if credential.is_null() {
            return Ok(None);
        }

        // SAFETY: `credential` is non-null and owned by us until `CredFree`.
        // Copy the blob out before freeing. A zero-length blob yields an empty
        // Vec, which the caller's `SecretBytes::new` rejects as InvalidSecret.
        let bytes = unsafe {
            let cred_ref = &*credential;
            let len = cred_ref.CredentialBlobSize as usize;
            if len == 0 || cred_ref.CredentialBlob.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(cred_ref.CredentialBlob, len).to_vec()
            }
        };

        // SAFETY: `credential` was allocated by `CredReadW`; free exactly once.
        unsafe { CredFree(credential as *const _ as *const core::ffi::c_void) };

        if bytes.is_empty() {
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    pub fn delete(account: &str) -> AuthResult<()> {
        let target = target_name(account);

        // SAFETY: `target` is a valid NUL-terminated UTF-16 string for the call.
        let result = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0) };

        if let Err(e) = result {
            // Deleting a missing credential is a no-op success, matching the
            // macOS store's `delete` (find returns None → Ok(())). The 0.58
            // `CredDeleteW` wrapper already captured the Win32 code in `e`, so
            // compare against it rather than a second, racy `GetLastError()`.
            if e.code() == HRESULT_ERROR_NOT_FOUND {
                return Ok(());
            }
            return Err(status_error("CredDeleteW", e.code().0));
        }
        Ok(())
    }

    fn status_error(action: &'static str, code: i32) -> AuthStoreError {
        // Never include secret bytes — only the API name and the HRESULT.
        // Format as hex (`0x{:08X}`, via `u32`) so the high bit reads as the
        // canonical HRESULT (e.g. 0x80070490) rather than a signed decimal.
        AuthStoreError::Backend(format!(
            "Credential Manager {action} failed with code 0x{:08X}",
            code as u32
        ))
    }
}

#[path = "../src/keychain/tests.rs"]
mod tests;

#[cfg(all(target_os = "macos", not(test)))]
mod macos_keychain {
    use super::{AuthResult, AuthStoreError, KEYCHAIN_SERVICE};
    use std::ffi::c_void;
    use std::ptr;

    type OSStatus = i32;
    type UInt32 = u32;
    type SecKeychainRef = *const c_void;
    type SecKeychainItemRef = *mut c_void;

    const ERR_SEC_SUCCESS: OSStatus = 0;
    const ERR_SEC_DUPLICATE_ITEM: OSStatus = -25299;
    const ERR_SEC_ITEM_NOT_FOUND: OSStatus = -25300;

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecKeychainAddGenericPassword(
            keychain: SecKeychainRef,
            service_name_length: UInt32,
            service_name: *const i8,
            account_name_length: UInt32,
            account_name: *const i8,
            password_length: UInt32,
            password_data: *const c_void,
            item_ref: *mut SecKeychainItemRef,
        ) -> OSStatus;
        fn SecKeychainFindGenericPassword(
            keychain: SecKeychainRef,
            service_name_length: UInt32,
            service_name: *const i8,
            account_name_length: UInt32,
            account_name: *const i8,
            password_length: *mut UInt32,
            password_data: *mut *mut c_void,
            item_ref: *mut SecKeychainItemRef,
        ) -> OSStatus;
        fn SecKeychainItemModifyAttributesAndData(
            item_ref: SecKeychainItemRef,
            attr_list: *const c_void,
            length: UInt32,
            data: *const c_void,
        ) -> OSStatus;
        fn SecKeychainItemDelete(item_ref: SecKeychainItemRef) -> OSStatus;
        fn SecKeychainItemFreeContent(attr_list: *mut c_void, data: *mut c_void) -> OSStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    pub fn save(account: &str, secret: &[u8]) -> AuthResult<()> {
        let service = KEYCHAIN_SERVICE.as_bytes();
        let account = account.as_bytes();
        let status = unsafe {
            SecKeychainAddGenericPassword(
                ptr::null(),
                service.len() as UInt32,
                service.as_ptr().cast(),
                account.len() as UInt32,
                account.as_ptr().cast(),
                secret.len() as UInt32,
                secret.as_ptr().cast(),
                ptr::null_mut(),
            )
        };
        if status == ERR_SEC_SUCCESS {
            return Ok(());
        }
        if status != ERR_SEC_DUPLICATE_ITEM {
            return Err(status_error("add generic password", status));
        }

        let Some(item) = find_item(account)? else {
            return Err(AuthStoreError::NotFound);
        };
        let update_status = unsafe {
            SecKeychainItemModifyAttributesAndData(item, ptr::null(), secret.len() as UInt32, secret.as_ptr().cast())
        };
        unsafe { CFRelease(item.cast()) };
        status_ok("update generic password", update_status)
    }

    /// Is there an item for `account`? Asks the item's attributes only: no password out-parameter, so no secret is
    /// read (the same lookup `delete` and `save` use to find the item).
    pub fn exists(account: &str) -> AuthResult<bool> {
        match find_item(account.as_bytes())? {
            Some(item) => {
                unsafe { CFRelease(item.cast()) };
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn load(account: &str) -> AuthResult<Option<Vec<u8>>> {
        let service = KEYCHAIN_SERVICE.as_bytes();
        let account = account.as_bytes();
        let mut password_len: UInt32 = 0;
        let mut password_data: *mut c_void = ptr::null_mut();
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null(),
                service.len() as UInt32,
                service.as_ptr().cast(),
                account.len() as UInt32,
                account.as_ptr().cast(),
                &mut password_len,
                &mut password_data,
                ptr::null_mut(),
            )
        };
        if status == ERR_SEC_ITEM_NOT_FOUND {
            return Ok(None);
        }
        status_ok("find generic password", status)?;
        let mut bytes =
            unsafe { std::slice::from_raw_parts(password_data.cast::<u8>(), password_len as usize).to_vec() };
        let free_status = unsafe { SecKeychainItemFreeContent(ptr::null_mut(), password_data) };
        if let Err(error) = status_ok("free keychain content", free_status) {
            // The copy is a secret: it is wiped (with `zeroize`, not a plain fill the compiler may drop before the
            // free) before the error leaves, not dropped as it is.
            zeroize::Zeroize::zeroize(&mut bytes);
            return Err(error);
        }
        Ok(Some(bytes))
    }

    pub fn delete(account: &str) -> AuthResult<()> {
        let account = account.as_bytes();
        let Some(item) = find_item(account)? else {
            return Ok(());
        };
        let status = unsafe { SecKeychainItemDelete(item) };
        unsafe { CFRelease(item.cast()) };
        status_ok("delete generic password", status)
    }

    fn find_item(account: &[u8]) -> AuthResult<Option<SecKeychainItemRef>> {
        let service = KEYCHAIN_SERVICE.as_bytes();
        let mut item: SecKeychainItemRef = ptr::null_mut();
        let status = unsafe {
            SecKeychainFindGenericPassword(
                ptr::null(),
                service.len() as UInt32,
                service.as_ptr().cast(),
                account.len() as UInt32,
                account.as_ptr().cast(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut item,
            )
        };
        if status == ERR_SEC_ITEM_NOT_FOUND {
            return Ok(None);
        }
        status_ok("find generic password item", status)?;
        Ok(Some(item))
    }

    fn status_ok(action: &'static str, status: OSStatus) -> AuthResult<()> {
        if status == ERR_SEC_SUCCESS {
            Ok(())
        } else {
            Err(status_error(action, status))
        }
    }

    fn status_error(action: &'static str, status: OSStatus) -> AuthStoreError {
        AuthStoreError::Backend(format!("Keychain {action} failed with OSStatus {status}"))
    }
}

#[path = "../src/keychain/errors.rs"]
mod errors;
pub use errors::{AuthResult, AuthStoreError, VaultLockState};

#[path = "../src/keychain/session.rs"]
mod session;
pub use session::SessionToken;

#[path = "../src/keychain/vault.rs"]
mod vault;
pub use vault::{AuthSecretStore, AuthVault};
/// The non-Windows trace and "what is left of this account" paths ask these, and so do the keychain tests.
#[cfg(any(test, not(target_os = "windows")))]
pub use vault::{holds_account_email, holds_session_token, holds_vault_key};

#[path = "../src/keychain/accounts.rs"]
mod accounts;
use accounts::*;
pub use accounts::{legacy_platform_keychain_store, platform_keychain_store_for};

#[path = "../src/keychain/legacy.rs"]
mod legacy;
/// Tests only: the migration's own tests drive the two-store variant directly; production runs the account variant.
#[cfg(test)]
pub use legacy::migrate_legacy_keychain_between;
pub use legacy::migrate_legacy_keychain_to_account;
