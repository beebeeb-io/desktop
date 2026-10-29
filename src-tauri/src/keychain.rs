//! Desktop auth, unlock, and secret-storage primitives.
//!
//! This module backs the Tauri runner's login, lock, unlock, and logout flow.
//! It establishes the fail-closed state model and Keychain-backed storage
//! surface used before the sync engine receives any file-content key material.

use std::fmt;

const KEYCHAIN_SERVICE: &str = "io.beebeeb.app";
const SESSION_TOKEN_ACCOUNT: &str = "session-token";
const WRAPPED_MASTER_KEY_ACCOUNT: &str = "wrapped-master-key";
// Account email (PII) persisted alongside the session so the Account page can
// show it after an auto-unlock on relaunch — the credential store is the only
// place it survives, since neither the token nor the wrapped key carries it.
// Stored in the SAME protected credential vault as the secrets above (NOT a
// plaintext config file). It is account metadata, not key material, so it is
// handled as a plain UTF-8 string rather than `SecretBytes`.
const ACCOUNT_EMAIL_ACCOUNT: &str = "account-email";
const MASTER_KEY_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStoreError {
    Unsupported(&'static str),
    NotFound,
    InvalidSecret(&'static str),
    Backend(String),
}

impl fmt::Display for AuthStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(msg) => write!(f, "{msg}"),
            Self::NotFound => write!(f, "secret not found"),
            Self::InvalidSecret(msg) => write!(f, "{msg}"),
            Self::Backend(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AuthStoreError {}

pub type AuthResult<T> = Result<T, AuthStoreError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultLockState {
    Locked,
    Unlocked,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new(token: impl Into<String>) -> AuthResult<Self> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(AuthStoreError::InvalidSecret("session token is empty"));
        }
        Ok(Self(token))
    }

    pub fn expose_for_request(&self) -> &str {
        &self.0
    }
}

impl zeroize::Zeroize for SessionToken {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for SessionToken {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(<redacted>)")
    }
}

#[derive(PartialEq, Eq)]
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
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

pub trait AuthSecretStore: Send + Sync {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()>;
    fn load_session_token(&self) -> AuthResult<Option<SessionToken>>;
    fn delete_session_token(&self) -> AuthResult<()>;
    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()>;
    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>>;
    fn delete_wrapped_master_key(&self) -> AuthResult<()>;
    /// Persist the signed-in account email (PII metadata) in the credential
    /// vault. Stored as a UTF-8 blob under a dedicated account so it can be
    /// recovered on the auto-unlock path. Default impls below provide
    /// backward-compatible behaviour for any store that predates this entry.
    fn save_account_email(&self, _email: &str) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "account email storage not supported by this store",
        ))
    }
    /// Read the persisted account email. Returns `Ok(None)` when none was ever
    /// stored (an existing session created before this entry existed), so the
    /// auto-unlock path falls back to `email = None` without erroring.
    fn load_account_email(&self) -> AuthResult<Option<String>> {
        Ok(None)
    }
    /// Remove the persisted account email. A no-op success when absent, matching
    /// the token/key delete semantics so logout stays idempotent.
    fn delete_account_email(&self) -> AuthResult<()> {
        Ok(())
    }
    /// Is a session token stored? A store that can answer without reading the secret does (the real macOS Keychain
    /// asks the item's attributes); the default reads it and drops it at once (`SessionToken` wipes itself).
    fn holds_session_token(&self) -> AuthResult<bool> {
        self.load_session_token().map(|token| token.is_some())
    }
    /// Is a vault key stored? Same contract as [`Self::holds_session_token`] (`SecretBytes` wipes itself).
    fn holds_wrapped_master_key(&self) -> AuthResult<bool> {
        self.load_wrapped_master_key().map(|key| key.is_some())
    }
    /// Is an account email stored? Same contract as [`Self::holds_session_token`].
    fn holds_account_email(&self) -> AuthResult<bool> {
        self.load_account_email().map(|email| email.is_some())
    }
}

/// One rule for every "is anything of an account left?" probe: present is present; a store that cannot hold secrets
/// (`Unsupported`) or has none (`NotFound`) holds none; any other answer counts as present, so a caller fails closed.
fn presence(answer: AuthResult<bool>) -> bool {
    match answer {
        Ok(present) => present,
        Err(AuthStoreError::Unsupported(_) | AuthStoreError::NotFound) => false,
        Err(_) => true,
    }
}

/// Does this store still hold a session token? Never reads the token where the store can avoid it; fails closed.
pub fn holds_session_token<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_session_token())
}

/// Does this store still hold an account email? Fails closed like [`holds_session_token`].
pub fn holds_account_email<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_account_email())
}

/// Does this store still hold a vault key? It never unlocks and never reads the key where the store can avoid it. A
/// store that cannot hold secrets (`Unsupported`) or has none (`NotFound`) holds none. Any other answer counts as
/// present, so a caller that asks whether anything of an account is left fails closed.
pub fn holds_vault_key<S: AuthSecretStore>(store: &S) -> bool {
    presence(store.holds_wrapped_master_key())
}

pub struct AuthVault<S: AuthSecretStore> {
    store: S,
    state: VaultLockState,
    master_key: Option<SecretBytes>,
}

impl<S: AuthSecretStore> AuthVault<S> {
    pub fn new(store: S) -> Self {
        Self {
            store,
            state: VaultLockState::Locked,
            master_key: None,
        }
    }

    pub fn lock_state(&self) -> VaultLockState {
        self.state
    }

    pub fn install_session(&self, token: SessionToken) -> AuthResult<()> {
        self.store.save_session_token(&token)
    }

    pub fn session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.store.load_session_token()
    }

    pub fn store_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.store.save_wrapped_master_key(wrapped)
    }

    /// Persist the account email alongside the session. Empty/whitespace-only
    /// input is treated as "no email" and skips the write so we never store a
    /// blank credential blob (the backends reject empty blobs anyway).
    pub fn store_account_email(&self, email: &str) -> AuthResult<()> {
        if email.trim().is_empty() {
            return Ok(());
        }
        self.store.save_account_email(email)
    }

    /// Read the persisted account email, or `None` if none was stored (e.g. a
    /// session created before email persistence existed). On a store that does
    /// not support email persistence (the Unsupported error), treat it as
    /// "no email" rather than failing the whole restore.
    pub fn account_email(&self) -> AuthResult<Option<String>> {
        match self.store.load_account_email() {
            Ok(email) => Ok(email),
            Err(AuthStoreError::Unsupported(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn unlock(&mut self) -> AuthResult<()> {
        let Some(master_key) = self.store.load_wrapped_master_key()? else {
            self.lock();
            return Err(AuthStoreError::NotFound);
        };
        if master_key.expose_for_crypto().len() != MASTER_KEY_BYTES {
            self.lock();
            return Err(AuthStoreError::InvalidSecret("master key must be 32 bytes"));
        }
        self.master_key = Some(master_key);
        self.state = VaultLockState::Unlocked;
        Ok(())
    }

    pub fn lock(&mut self) {
        self.master_key.take();
        self.state = VaultLockState::Locked;
    }

    /// Remove only the session token: R8, a token the server rejected, found at startup. The wrapped vault key and the
    /// account email stay; [`Self::clear_session`] remains the full sign-out.
    pub fn clear_session_token(&mut self) -> AuthResult<()> {
        self.store.delete_session_token()
    }

    pub fn clear_session(&mut self) -> AuthResult<()> {
        self.lock();
        self.store.delete_session_token()?;
        self.store.delete_wrapped_master_key()?;
        // Also drop the persisted account email so logout leaves no PII behind.
        // `delete_account_email` is a no-op success when absent, so this stays
        // idempotent and never fails just because email was never stored.
        self.store.delete_account_email()?;
        Ok(())
    }

    pub fn master_key(&self) -> AuthResult<&[u8]> {
        if self.state != VaultLockState::Unlocked {
            return Err(AuthStoreError::InvalidSecret("vault is locked"));
        }
        self.master_key
            .as_ref()
            .map(SecretBytes::expose_for_crypto)
            .ok_or(AuthStoreError::InvalidSecret("vault is locked"))
    }

    pub fn can_hydrate_or_upload(&self) -> bool {
        self.master_key().is_ok()
    }
}

impl<S: AuthSecretStore> fmt::Debug for AuthVault<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthVault")
            .field("state", &self.state)
            .field("master_key", &self.master_key)
            .finish_non_exhaustive()
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

/// Compose the per-account credential leaf from an optional prefix.
///
/// `Some(id)` → `"<id>/<leaf>"` (segmented, Phase 1); `None` → bare `leaf`
/// (legacy, pre-Phase-1 layout). Shared by every concrete store so the
/// composition rule lives in exactly one place.
fn account_path(prefix: Option<&str>, leaf: &str) -> String {
    match prefix {
        Some(id) => format!("{id}/{leaf}"),
        None => leaf.to_string(),
    }
}

// The real Keychain store does not EXIST in a macOS test build (Task 9 fix round 2): a test that names it
// fails to compile, instead of reaching the developer's own Keychain items. `TestKeychainStore` takes its
// place (see `PlatformKeychainStore`).
#[cfg(all(target_os = "macos", not(test)))]
pub struct MacOsKeychainStore {
    account_prefix: Option<String>,
}

#[cfg(all(target_os = "macos", not(test)))]
impl MacOsKeychainStore {
    /// Legacy flat store (no account segmentation). Prefer `with_account`.
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    /// Store segmented under `account_id` (Phase 1).
    pub fn with_account(account_id: impl Into<String>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }

    fn account_path(&self, leaf: &str) -> String {
        account_path(self.account_prefix.as_deref(), leaf)
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
            &self.account_path(SESSION_TOKEN_ACCOUNT),
            token.expose_for_request().as_bytes(),
        )
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        macos_keychain::load(&self.account_path(SESSION_TOKEN_ACCOUNT))?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| AuthStoreError::InvalidSecret("session token is not UTF-8"))
                    .and_then(SessionToken::new)
            })
            .transpose()
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(SESSION_TOKEN_ACCOUNT))
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        macos_keychain::save(
            &self.account_path(WRAPPED_MASTER_KEY_ACCOUNT),
            wrapped.expose_for_crypto(),
        )
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        macos_keychain::load(&self.account_path(WRAPPED_MASTER_KEY_ACCOUNT))?
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(WRAPPED_MASTER_KEY_ACCOUNT))
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        macos_keychain::save(&self.account_path(ACCOUNT_EMAIL_ACCOUNT), email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        macos_keychain::load(&self.account_path(ACCOUNT_EMAIL_ACCOUNT))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8"))
            })
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(ACCOUNT_EMAIL_ACCOUNT))
    }

    fn holds_session_token(&self) -> AuthResult<bool> {
        macos_keychain::exists(&self.account_path(SESSION_TOKEN_ACCOUNT))
    }

    fn holds_wrapped_master_key(&self) -> AuthResult<bool> {
        macos_keychain::exists(&self.account_path(WRAPPED_MASTER_KEY_ACCOUNT))
    }

    fn holds_account_email(&self) -> AuthResult<bool> {
        macos_keychain::exists(&self.account_path(ACCOUNT_EMAIL_ACCOUNT))
    }
}

#[cfg(not(target_os = "macos"))]
pub struct MacOsKeychainStore {
    // Carried for type parity with the macOS store so the same
    // `with_account`/`platform_keychain_store_for` plumbing compiles on every
    // target. Unused on the stub path (every method returns Unsupported).
    #[allow(dead_code)]
    account_prefix: Option<String>,
}

#[cfg(not(target_os = "macos"))]
impl MacOsKeychainStore {
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    pub fn with_account(account_id: impl Into<String>) -> Self {
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
    account_prefix: Option<String>,
}

#[cfg(target_os = "windows")]
impl WindowsCredentialStore {
    /// Legacy flat store (no account segmentation). Prefer `with_account`.
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    /// Store segmented under `account_id` (Phase 1).
    pub fn with_account(account_id: impl Into<String>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }

    fn account_path(&self, leaf: &str) -> String {
        account_path(self.account_prefix.as_deref(), leaf)
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
            &self.account_path(SESSION_TOKEN_ACCOUNT),
            token.expose_for_request().as_bytes(),
        )
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        windows_credentials::load(&self.account_path(SESSION_TOKEN_ACCOUNT))?
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
        windows_credentials::delete(&self.account_path(SESSION_TOKEN_ACCOUNT))
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        windows_credentials::save(
            &self.account_path(WRAPPED_MASTER_KEY_ACCOUNT),
            wrapped.expose_for_crypto(),
        )
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        windows_credentials::load(&self.account_path(WRAPPED_MASTER_KEY_ACCOUNT))?
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        windows_credentials::delete(&self.account_path(WRAPPED_MASTER_KEY_ACCOUNT))
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        windows_credentials::save(&self.account_path(ACCOUNT_EMAIL_ACCOUNT), email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        windows_credentials::load(&self.account_path(ACCOUNT_EMAIL_ACCOUNT))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8"))
            })
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        windows_credentials::delete(&self.account_path(ACCOUNT_EMAIL_ACCOUNT))
    }
}

#[cfg(not(target_os = "windows"))]
pub struct WindowsCredentialStore {
    // Type-parity with the Windows store (see MacOsKeychainStore stub note).
    #[allow(dead_code)]
    account_prefix: Option<String>,
}

#[cfg(not(target_os = "windows"))]
impl WindowsCredentialStore {
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    pub fn with_account(account_id: impl Into<String>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }
}

#[cfg(not(target_os = "windows"))]
impl Default for WindowsCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_os = "windows"))]
impl AuthSecretStore for WindowsCredentialStore {
    fn save_session_token(&self, _token: &SessionToken) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "Windows Credential Manager is unavailable on this platform",
        ))
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        Err(AuthStoreError::Unsupported(
            "Windows Credential Manager is unavailable on this platform",
        ))
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "Windows Credential Manager is unavailable on this platform",
        ))
    }

    fn save_wrapped_master_key(&self, _wrapped: SecretBytes) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "Windows Credential Manager is unavailable on this platform",
        ))
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        Err(AuthStoreError::Unsupported(
            "Windows Credential Manager is unavailable on this platform",
        ))
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported(
            "Windows Credential Manager is unavailable on this platform",
        ))
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
    account_prefix: Option<String>,
}

#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
impl TestKeychainStore {
    pub fn new() -> Self {
        Self { account_prefix: None }
    }

    pub fn with_account(account_id: impl Into<String>) -> Self {
        Self {
            account_prefix: Some(account_id.into()),
        }
    }

    fn key(&self, leaf: &str) -> String {
        TEST_KEYCHAIN_TOUCHED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.account_prefix.clone());
        format!(
            "{KEYCHAIN_SERVICE}/{}",
            account_path(self.account_prefix.as_deref(), leaf)
        )
    }

    fn save(&self, leaf: &str, bytes: &[u8]) -> AuthResult<()> {
        TEST_KEYCHAIN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.key(leaf), bytes.to_vec());
        Ok(())
    }

    fn load(&self, leaf: &str) -> Option<Vec<u8>> {
        TEST_KEYCHAIN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.key(leaf))
            .cloned()
    }

    fn delete(&self, leaf: &str) -> AuthResult<()> {
        TEST_KEYCHAIN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.key(leaf));
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
impl AuthSecretStore for TestKeychainStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        self.save(SESSION_TOKEN_ACCOUNT, token.expose_for_request().as_bytes())
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        self.load(SESSION_TOKEN_ACCOUNT)
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| AuthStoreError::InvalidSecret("session token is not UTF-8"))
                    .and_then(SessionToken::new)
            })
            .transpose()
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        self.delete(SESSION_TOKEN_ACCOUNT)
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        self.save(WRAPPED_MASTER_KEY_ACCOUNT, wrapped.expose_for_crypto())
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        self.load(WRAPPED_MASTER_KEY_ACCOUNT).map(SecretBytes::new).transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        self.delete(WRAPPED_MASTER_KEY_ACCOUNT)
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        if TEST_KEYCHAIN_EMAIL_WRITES_FAIL
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&self.account_prefix)
        {
            return Err(AuthStoreError::Backend(
                "the test Keychain refuses this write".to_string(),
            ));
        }
        self.save(ACCOUNT_EMAIL_ACCOUNT, email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        self.load(ACCOUNT_EMAIL_ACCOUNT)
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8"))
            })
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        self.delete(ACCOUNT_EMAIL_ACCOUNT)
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

// ── Lazy migration: legacy flat trio → id-segmented trio (task 0800) ─────────
//
// VAULT-LOCKOUT-class change. The contract that protects against lockout:
//   • write-new → VERIFY readback → ONLY THEN delete legacy (§3.4-3.6); and
//   • a legacy READ FALLBACK on the unlock path (lib.rs) — two independent nets.
// The function is IDEMPOTENT and INTERRUPT-CONVERGENT: interrupted after a crash
// at any step, a re-run ends fully-migrated-with-legacy-gone.

/// Migrate the legacy flat keychain trio into the id-segmented layout.
///
/// `migrate_legacy_keychain_to_account(id)` builds the segmented store for `id`
/// and the legacy flat store, then runs [`migrate_legacy_keychain_between`].
/// LOG+SWALLOW errors at the call site — a migration failure must never block
/// startup, because the legacy READ FALLBACK keeps the vault usable.
pub fn migrate_legacy_keychain_to_account(account_id: &str) -> AuthResult<()> {
    let new = platform_keychain_store_for(account_id);
    let legacy = legacy_platform_keychain_store();
    migrate_legacy_keychain_between(&new, &legacy)
}

/// Core migration algorithm, generic over the two stores so it is unit-testable
/// with in-memory stores (the platform credential vaults can't run in CI).
///
/// `new` is the id-segmented destination, `legacy` the flat source. Steps mirror
/// blueprint §3:
///
/// 1/2. SHORT-CIRCUIT (interrupt-safe). If the new token is present AND
///      (the new key is present OR legacy has no key) → migration is already
///      complete; opportunistically sweep any leftover legacy entries and
///      return. The `OR legacy has no key` guards the partial-write hole: if the
///      new token landed but the new key did NOT and legacy still HAS a key, we
///      must fall THROUGH and re-run 4-6 (don't declare victory with a missing
///      key while a recoverable legacy key still exists).
/// 3.   Read the legacy token. `None` → nothing to migrate (fresh/clean) → Ok.
///      Read legacy key + email (each `Option`).
/// 4.   WRITE new: first remove any email the new store holds (it is not the
///      legacy session's), then token always; key/email only if legacy had them.
///      `SecretBytes` is `!Clone`, so the loaded key is MOVED straight into the
///      save (never bound + reused). `SessionToken` IS `Clone`.
/// 5.   VERIFY readback BEFORE any delete: new token present; if legacy had a
///      key → new key present; if legacy had email → new email present. ANY
///      failure → return `Err` WITHOUT deleting (vault still usable via the
///      legacy fallback).
/// 6.   ONLY now delete the legacy trio (each delete idempotent; NotFound → Ok).
pub fn migrate_legacy_keychain_between<N: AuthSecretStore, L: AuthSecretStore>(new: &N, legacy: &L) -> AuthResult<()> {
    // ── Steps 1-2: short-circuit if already (effectively) migrated ──────────
    let new_token_present = new.load_session_token()?.is_some();
    if new_token_present {
        let new_key_present = new.load_wrapped_master_key()?.is_some();
        let legacy_has_key = legacy.load_wrapped_master_key()?.is_some();
        if new_key_present || !legacy_has_key {
            // Migration complete. Opportunistically sweep any leftover legacy
            // entries (e.g. a crash between writing the new trio and deleting
            // legacy). Each delete is idempotent (NotFound → Ok).
            sweep_legacy(legacy)?;
            return Ok(());
        }
        // else: new token present but new key MISSING while legacy STILL HAS a
        // key → partial write. Fall through to re-run the write/verify/delete.
    }

    // ── Step 3: read legacy; nothing to migrate if there is no legacy token ──
    let Some(legacy_token) = legacy.load_session_token()? else {
        // No legacy session at all (fresh install, or already swept) → nothing
        // to do. If a new token exists it's handled by the short-circuit above.
        return Ok(());
    };
    let legacy_key = legacy.load_wrapped_master_key()?; // Option<SecretBytes>
    let legacy_email = legacy.load_account_email()?; // Option<String>
    let key_existed = legacy_key.is_some();
    let email_existed = legacy_email.is_some();

    // ── Step 4: write the new (segmented) trio ──────────────────────────────
    // An email the segmented store already holds belongs to whatever it held before, not to the legacy session:
    // it is removed BEFORE the legacy token is written, so the token is never kept next to another account's email
    // (the restore knows a stored session by its email). The legacy email, if any, is written below.
    new.delete_account_email()?;
    // A vault key the segmented store kept (a startup 401 keeps the key and drops the token) belongs to the session
    // stored there before, not to the legacy one: with no legacy key it is removed BEFORE the legacy token is written,
    // so the two are never paired (spec §5.6). A legacy key of its own replaces it below.
    if !key_existed {
        new.delete_wrapped_master_key()?;
    }
    // Token: SessionToken is Clone, but we own `legacy_token` here so pass by ref.
    new.save_session_token(&legacy_token)?;
    // Key: SecretBytes is !Clone → MOVE the loaded value straight into save.
    if let Some(key) = legacy_key {
        new.save_wrapped_master_key(key)?;
    }
    // Email: plain string metadata; store best-effort. An Unsupported store
    // (Linux stub) would error here, but on Linux there is no legacy token to
    // migrate in the first place, so this branch is unreachable there.
    if let Some(email) = &legacy_email {
        new.save_account_email(email)?;
    }

    // ── Step 5: VERIFY readback BEFORE deleting anything ────────────────────
    if new.load_session_token()?.is_none() {
        return Err(AuthStoreError::Backend(
            "migration verify failed: new session token not readable after write".to_string(),
        ));
    }
    if key_existed && new.load_wrapped_master_key()?.is_none() {
        return Err(AuthStoreError::Backend(
            "migration verify failed: new wrapped master key not readable after write".to_string(),
        ));
    }
    if email_existed && new.load_account_email()?.is_none() {
        return Err(AuthStoreError::Backend(
            "migration verify failed: new account email not readable after write".to_string(),
        ));
    }

    // ── Step 6: only now delete the legacy trio (idempotent) ────────────────
    sweep_legacy(legacy)
}

/// Delete the legacy flat trio. Each delete is idempotent (NotFound → Ok), so
/// this is safe to call when some/all entries are already gone — used both as
/// the final migration step and the opportunistic short-circuit sweep.
fn sweep_legacy<L: AuthSecretStore>(legacy: &L) -> AuthResult<()> {
    legacy.delete_session_token()?;
    legacy.delete_wrapped_master_key()?;
    legacy.delete_account_email()?;
    Ok(())
}

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

#[cfg(test)]
mod tests {
    //! Decision-boundary tests for the startup auto-unlock.
    //!
    //! On launch, `lib::restore_session_on_startup` resumes a fully UNLOCKED
    //! session iff the credential store holds both the session token and a
    //! valid 32-byte master key; otherwise it leaves onboarding to prompt for
    //! the recovery phrase. The exact gate is `AuthVault::unlock()`, exercised
    //! here against an in-memory store so it runs on any platform (the real
    //! OS credential vaults are platform-gated and can't run in CI).

    use super::*;
    use std::sync::Mutex;

    /// In-memory `AuthSecretStore` standing in for the OS credential vault.
    #[derive(Default)]
    struct MemoryStore {
        token: Mutex<Option<Vec<u8>>>,
        key: Mutex<Option<Vec<u8>>>,
        email: Mutex<Option<String>>,
    }

    impl AuthSecretStore for MemoryStore {
        fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
            *self.token.lock().unwrap() = Some(token.expose_for_request().as_bytes().to_vec());
            Ok(())
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            self.token
                .lock()
                .unwrap()
                .clone()
                .map(|bytes| SessionToken::new(String::from_utf8(bytes).unwrap()))
                .transpose()
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            *self.token.lock().unwrap() = None;
            Ok(())
        }
        fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
            *self.key.lock().unwrap() = Some(wrapped.expose_for_crypto().to_vec());
            Ok(())
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            self.key.lock().unwrap().clone().map(SecretBytes::new).transpose()
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            *self.key.lock().unwrap() = None;
            Ok(())
        }
        fn save_account_email(&self, email: &str) -> AuthResult<()> {
            *self.email.lock().unwrap() = Some(email.to_string());
            Ok(())
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            Ok(self.email.lock().unwrap().clone())
        }
        fn delete_account_email(&self) -> AuthResult<()> {
            *self.email.lock().unwrap() = None;
            Ok(())
        }
    }

    /// Token + 32-byte key present → unlock succeeds and the key is readable.
    /// This is the startup auto-unlock path: relaunch resumes signed-in +
    /// unlocked with no recovery-phrase prompt.
    #[test]
    fn unlock_succeeds_when_master_key_present() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]))
            .unwrap();

        assert_eq!(vault.lock_state(), VaultLockState::Locked);
        vault.unlock().expect("present 32-byte key must unlock");
        assert_eq!(vault.lock_state(), VaultLockState::Unlocked);
        assert_eq!(vault.master_key().unwrap(), &[7u8; MASTER_KEY_BYTES]);
    }

    /// Token present but master key genuinely ABSENT → unlock returns NotFound
    /// and the vault stays locked. This is the only state where onboarding must
    /// still show "this PC doesn't have your keys" / prompt for the recovery
    /// phrase. The startup restore treats this as a no-op.
    #[test]
    fn unlock_reports_not_found_when_master_key_absent() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        // No master key stored.

        assert_eq!(vault.unlock(), Err(AuthStoreError::NotFound));
        assert_eq!(vault.lock_state(), VaultLockState::Locked);
        assert!(vault.master_key().is_err());
    }

    /// A stored blob of the wrong length is rejected rather than unlocking with
    /// a malformed key — defensive, in case the credential blob is corrupt.
    #[test]
    fn unlock_rejects_wrong_length_key() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new(vec![1u8; 16]).unwrap())
            .unwrap();

        assert!(matches!(vault.unlock(), Err(AuthStoreError::InvalidSecret(_))));
        assert_eq!(vault.lock_state(), VaultLockState::Locked);
    }

    /// The account email round-trips through the store so the auto-unlock path
    /// can recover it: persist on login, read back on relaunch.
    #[test]
    fn account_email_round_trips() {
        let vault = AuthVault::new(MemoryStore::default());
        vault.store_account_email("user@example.com").unwrap();
        assert_eq!(vault.account_email().unwrap().as_deref(), Some("user@example.com"));
    }

    /// Backward-compat: a session stored BEFORE email persistence existed has no
    /// email entry, so `account_email()` returns `None` cleanly — never an error
    /// or panic. This is the existing-stored-session upgrade path.
    #[test]
    fn account_email_absent_returns_none() {
        let vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]))
            .unwrap();
        // No email was ever stored.
        assert_eq!(vault.account_email().unwrap(), None);
    }

    /// An empty/whitespace email is treated as "no email" and never written, so
    /// we don't persist a blank credential blob.
    #[test]
    fn empty_account_email_is_not_stored() {
        let vault = AuthVault::new(MemoryStore::default());
        vault.store_account_email("   ").unwrap();
        assert_eq!(vault.account_email().unwrap(), None);
    }

    /// Logout (`clear_session`) wipes the persisted email along with the token
    /// and key — no PII left behind.
    #[test]
    fn clear_session_removes_account_email() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("token").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]))
            .unwrap();
        vault.store_account_email("user@example.com").unwrap();
        assert_eq!(vault.account_email().unwrap().as_deref(), Some("user@example.com"));

        vault.clear_session().unwrap();

        assert_eq!(vault.account_email().unwrap(), None);
        assert_eq!(vault.session_token().unwrap(), None);
    }

    // ── Lazy migration tests (task 0800) ────────────────────────────────────
    //
    // A single shared backing map keyed by the FULL account string models one OS
    // keychain namespace: the legacy store reads/writes bare leaves
    // (`session-token`), the segmented store reads/writes `<id>/session-token`.
    // Two `KeyedMemoryStore` handles over the same `Arc<Mutex<HashMap>>` give us
    // exactly the legacy-vs-segmented separation the real credential vault has.

    use std::collections::HashMap;
    use std::sync::Arc;

    type SharedVault = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    /// In-memory store backed by a shared map, mirroring the per-OS store's
    /// `account_path` prefixing so legacy and segmented entries land at distinct
    /// keys in the same namespace.
    struct KeyedMemoryStore {
        vault: SharedVault,
        prefix: Option<String>,
    }

    impl KeyedMemoryStore {
        fn legacy(vault: SharedVault) -> Self {
            Self { vault, prefix: None }
        }
        fn with_account(vault: SharedVault, id: &str) -> Self {
            Self {
                vault,
                prefix: Some(id.to_string()),
            }
        }
        fn key(&self, leaf: &str) -> String {
            account_path(self.prefix.as_deref(), leaf)
        }
    }

    impl AuthSecretStore for KeyedMemoryStore {
        fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
            self.vault.lock().unwrap().insert(
                self.key(SESSION_TOKEN_ACCOUNT),
                token.expose_for_request().as_bytes().to_vec(),
            );
            Ok(())
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            self.vault
                .lock()
                .unwrap()
                .get(&self.key(SESSION_TOKEN_ACCOUNT))
                .cloned()
                .map(|bytes| SessionToken::new(String::from_utf8(bytes).unwrap()))
                .transpose()
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            self.vault.lock().unwrap().remove(&self.key(SESSION_TOKEN_ACCOUNT));
            Ok(())
        }
        fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
            self.vault.lock().unwrap().insert(
                self.key(WRAPPED_MASTER_KEY_ACCOUNT),
                wrapped.expose_for_crypto().to_vec(),
            );
            Ok(())
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            self.vault
                .lock()
                .unwrap()
                .get(&self.key(WRAPPED_MASTER_KEY_ACCOUNT))
                .cloned()
                .map(SecretBytes::new)
                .transpose()
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            self.vault.lock().unwrap().remove(&self.key(WRAPPED_MASTER_KEY_ACCOUNT));
            Ok(())
        }
        fn save_account_email(&self, email: &str) -> AuthResult<()> {
            self.vault
                .lock()
                .unwrap()
                .insert(self.key(ACCOUNT_EMAIL_ACCOUNT), email.as_bytes().to_vec());
            Ok(())
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            Ok(self
                .vault
                .lock()
                .unwrap()
                .get(&self.key(ACCOUNT_EMAIL_ACCOUNT))
                .map(|bytes| String::from_utf8(bytes.clone()).unwrap()))
        }
        fn delete_account_email(&self) -> AuthResult<()> {
            self.vault.lock().unwrap().remove(&self.key(ACCOUNT_EMAIL_ACCOUNT));
            Ok(())
        }
    }

    /// A store that, on demand, drops EVERY write to the wrapped master key —
    /// used to simulate "verify readback fails" without touching a real vault.
    /// Token + email writes still succeed (so verify fails on the KEY specifically).
    struct KeyDropStore {
        inner: KeyedMemoryStore,
    }
    impl AuthSecretStore for KeyDropStore {
        fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
            self.inner.save_session_token(token)
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            self.inner.load_session_token()
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            self.inner.delete_session_token()
        }
        fn save_wrapped_master_key(&self, _wrapped: SecretBytes) -> AuthResult<()> {
            // Silently drop — write "succeeds" but the value never lands, so the
            // verify-readback in step 5 sees a missing key and must NOT delete legacy.
            Ok(())
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            self.inner.load_wrapped_master_key()
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            self.inner.delete_wrapped_master_key()
        }
        fn save_account_email(&self, email: &str) -> AuthResult<()> {
            self.inner.save_account_email(email)
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            self.inner.load_account_email()
        }
        fn delete_account_email(&self) -> AuthResult<()> {
            self.inner.delete_account_email()
        }
    }

    const TEST_ID: &str = "acct-uuid-0001";

    fn seed_legacy(vault: &SharedVault, token: &str, key: Option<[u8; 32]>, email: Option<&str>) {
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        legacy.save_session_token(&SessionToken::new(token).unwrap()).unwrap();
        if let Some(k) = key {
            legacy.save_wrapped_master_key(SecretBytes::new_master_key(&k)).unwrap();
        }
        if let Some(e) = email {
            legacy.save_account_email(e).unwrap();
        }
    }

    fn legacy_present(vault: &SharedVault) -> bool {
        let m = vault.lock().unwrap();
        m.contains_key(SESSION_TOKEN_ACCOUNT)
            || m.contains_key(WRAPPED_MASTER_KEY_ACCOUNT)
            || m.contains_key(ACCOUNT_EMAIL_ACCOUNT)
    }

    /// Happy path: a full legacy trio migrates into the segmented layout, the
    /// segmented entries are present + correct, and the legacy trio is GONE.
    #[test]
    fn migration_happy_path_moves_trio_and_deletes_legacy() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("happy migration");

        // Segmented copies present + correct.
        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32]
        );
        assert_eq!(new.load_account_email().unwrap().as_deref(), Some("u@example.com"));
        // Legacy trio gone.
        assert!(
            !legacy_present(&vault),
            "legacy entries must be deleted after migration"
        );
    }

    /// Idempotent: re-running after a complete migration is a no-op that leaves
    /// the segmented trio intact and legacy still gone.
    #[test]
    fn migration_is_idempotent() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());

        migrate_legacy_keychain_between(&new, &legacy).expect("first run");
        // Second run: short-circuits (new token + new key present) → no-op.
        migrate_legacy_keychain_between(&new, &legacy).expect("idempotent re-run");

        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32]
        );
        assert!(!legacy_present(&vault));
    }

    /// Interrupted at step 4 (after the new token+key+email were written, before
    /// the legacy delete): the short-circuit sees new token + new key present and
    /// sweeps the leftover legacy entries. Converges to fully-migrated-legacy-gone.
    #[test]
    fn migration_interrupted_before_delete_converges() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));
        // Simulate a crash right after step 4: new trio written, legacy NOT yet
        // deleted. Pre-write the segmented copies manually to model that state.
        {
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            new.save_session_token(&SessionToken::new("tok").unwrap()).unwrap();
            new.save_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
                .unwrap();
            new.save_account_email("u@example.com").unwrap();
        }
        assert!(legacy_present(&vault), "precondition: legacy still present pre-rerun");

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("re-run converges");

        assert!(!legacy_present(&vault), "leftover legacy swept on re-run");
        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
    }

    /// Interrupted at step 6 (partway through deleting the legacy trio): some
    /// legacy entries remain alongside the full new trio. The short-circuit sweep
    /// finishes the deletion — convergent, idempotent.
    #[test]
    fn migration_interrupted_during_legacy_delete_converges() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        // New trio complete; legacy token already deleted but legacy key+email
        // still linger (crash mid-sweep).
        {
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            new.save_session_token(&SessionToken::new("tok").unwrap()).unwrap();
            new.save_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
                .unwrap();
            new.save_account_email("u@example.com").unwrap();
        }
        {
            let legacy = KeyedMemoryStore::legacy(vault.clone());
            // Only key + email remain (token was deleted before the crash).
            legacy
                .save_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
                .unwrap();
            legacy.save_account_email("u@example.com").unwrap();
        }

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("re-run converges");

        assert!(!legacy_present(&vault), "leftover legacy key/email swept");
    }

    /// PARTIAL-WRITE HOLE (the §3 step-2 fix): new token present but new key
    /// MISSING while legacy STILL HAS a key. The short-circuit must NOT declare
    /// victory — it must fall through and re-run 4-6 so the key is recovered.
    #[test]
    fn migration_partial_write_new_token_no_key_falls_through() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));
        // Model the partial write: new TOKEN landed, new KEY did not.
        {
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            new.save_session_token(&SessionToken::new("tok").unwrap()).unwrap();
            // deliberately NO key write
        }

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("falls through and completes");

        // The fall-through must have recovered the key from legacy.
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32],
            "partial-write hole: key must be recovered, not stranded"
        );
        assert!(!legacy_present(&vault), "legacy swept after completing migration");
    }

    /// Verify-fails: the new-key write silently drops, so step 5's readback finds
    /// the key missing. The migration MUST return Err WITHOUT deleting legacy, so
    /// the vault stays usable through the legacy fallback.
    #[test]
    fn migration_verify_fails_keeps_legacy() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok", Some([9u8; 32]), Some("u@example.com"));

        let new = KeyDropStore {
            inner: KeyedMemoryStore::with_account(vault.clone(), TEST_ID),
        };
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        let result = migrate_legacy_keychain_between(&new, &legacy);

        assert!(result.is_err(), "verify-readback failure must surface as Err");
        // CRITICAL: legacy NOT deleted — the vault is still recoverable.
        assert!(
            legacy_present(&vault),
            "legacy MUST survive a failed verify (anti-lockout invariant)"
        );
    }

    /// Fresh/clean install: no legacy token at all → migration is a no-op Ok and
    /// writes nothing. (Models the legacy-fallback's "nothing to migrate" case.)
    #[test]
    fn migration_no_legacy_is_noop() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());

        migrate_legacy_keychain_between(&new, &legacy).expect("no-op on clean install");

        assert!(new.load_session_token().unwrap().is_none());
        assert!(!legacy_present(&vault));
        assert!(vault.lock().unwrap().is_empty(), "nothing written on a clean install");
    }

    /// LEGACY FALLBACK semantics: after migration the segmented store serves the
    /// session; but a store that only ever had legacy entries (migration skipped)
    /// still reads them through the legacy handle — the read path the anti-lockout
    /// fallback relies on. Token-only legacy (no key) also migrates cleanly.
    #[test]
    fn migration_token_only_legacy_migrates_and_legacy_fallback_reads() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        // Token-only legacy (an install that signed in but never provisioned a
        // local key — onboarding would prompt for the recovery phrase).
        seed_legacy(&vault, "tok", None, None);

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("token-only migration");

        assert_eq!(new.load_session_token().unwrap().unwrap().expose_for_request(), "tok");
        assert!(new.load_wrapped_master_key().unwrap().is_none());
        assert!(!legacy_present(&vault), "legacy token swept after token-only migration");

        // Legacy-fallback read sanity: a legacy handle over a vault that still has
        // a flat token returns it (the fallback the unlock path performs).
        let vault2: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault2, "legacy-tok", Some([3u8; 32]), Some("e@example.com"));
        let new2 = KeyedMemoryStore::with_account(vault2.clone(), TEST_ID);
        let legacy2 = KeyedMemoryStore::legacy(vault2.clone());
        // Segmented lookup is empty (migration hasn't run) → fall back to legacy.
        assert!(new2.load_session_token().unwrap().is_none());
        assert_eq!(
            legacy2.load_session_token().unwrap().unwrap().expose_for_request(),
            "legacy-tok"
        );
    }

    /// The segmented store keeps an email only next to the token it names. A legacy session migrates with its own
    /// email, or with none when it has none: an email the segmented store already held (next to a token of its own,
    /// or alone) is removed, never kept next to the legacy token.
    #[test]
    fn migration_never_keeps_another_email_next_to_the_legacy_token() {
        for (name, new_token) in [
            ("a segmented token without a key", Some("tok-new")),
            ("an email alone", None),
        ] {
            let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
            seed_legacy(&vault, "tok-legacy", Some([9u8; 32]), None);
            {
                let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
                if let Some(token) = new_token {
                    new.save_session_token(&SessionToken::new(token).unwrap()).unwrap();
                }
                new.save_account_email("someone-else@beebeeb.io").unwrap();
            }
            let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
            let legacy = KeyedMemoryStore::legacy(vault.clone());
            migrate_legacy_keychain_between(&new, &legacy).expect("migrates");
            assert_eq!(
                new.load_session_token().unwrap().unwrap().expose_for_request(),
                "tok-legacy",
                "{name}"
            );
            assert_eq!(
                new.load_account_email().unwrap(),
                None,
                "{name}: no email is kept next to the legacy token"
            );
            assert!(!legacy_present(&vault), "{name}");
        }
        // With an email of its own, the legacy session takes it along (the email that was there is replaced).
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok-legacy", Some([9u8; 32]), Some("u@beebeeb.io"));
        KeyedMemoryStore::with_account(vault.clone(), TEST_ID)
            .save_account_email("someone-else@beebeeb.io")
            .unwrap();
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        migrate_legacy_keychain_between(&new, &KeyedMemoryStore::legacy(vault.clone())).expect("migrates");
        assert_eq!(new.load_account_email().unwrap().as_deref(), Some("u@beebeeb.io"));
    }

    /// A keyless legacy session never lands next to a vault key the segmented store kept (a startup 401 keeps the key
    /// and drops only the token, R9): that key belonged to the segmented session, so it is removed before the legacy
    /// token is written, and the restore can never pair the two (spec §5.6: a key is never left next to another
    /// session). A legacy session with its own key replaces the kept one as before.
    #[test]
    fn a_keyless_legacy_token_never_lands_next_to_a_kept_vault_key() {
        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok-legacy", None, None);
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        new.save_wrapped_master_key(SecretBytes::new_master_key(&[4u8; 32]))
            .unwrap();
        migrate_legacy_keychain_between(&new, &KeyedMemoryStore::legacy(vault.clone())).expect("migrates");
        assert_eq!(
            new.load_session_token().unwrap().unwrap().expose_for_request(),
            "tok-legacy"
        );
        assert!(
            new.load_wrapped_master_key().unwrap().is_none(),
            "the kept key is gone: it is never paired with the legacy token"
        );
        assert!(!legacy_present(&vault));

        let vault: SharedVault = Arc::new(Mutex::new(HashMap::new()));
        seed_legacy(&vault, "tok-legacy", Some([9u8; 32]), None);
        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        new.save_wrapped_master_key(SecretBytes::new_master_key(&[4u8; 32]))
            .unwrap();
        migrate_legacy_keychain_between(&new, &KeyedMemoryStore::legacy(vault.clone())).expect("migrates");
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32],
            "a legacy key of its own replaces it"
        );
    }

    /// R8: a token the server rejected at startup is removed alone; the vault key and the email stay.
    #[test]
    fn clearing_the_session_token_keeps_the_key_and_the_email() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("tok").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[9u8; 32]))
            .unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        vault.clear_session_token().unwrap();
        assert!(vault.session_token().unwrap().is_none(), "the revoked token is gone");
        assert_eq!(vault.account_email().unwrap().as_deref(), Some("sam@beebeeb.io"));
        assert!(
            vault.store.load_wrapped_master_key().unwrap().is_some(),
            "R8: the vault key stays"
        );
    }

    /// After the token-only clear of a startup 401, the full clear of "Sign out and switch" leaves no vault key and no
    /// email for the next account.
    #[test]
    fn a_full_clear_after_a_token_only_clear_leaves_no_vault_key() {
        let mut vault = AuthVault::new(MemoryStore::default());
        vault.install_session(SessionToken::new("tok-a").unwrap()).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
            .unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        vault.clear_session_token().unwrap();
        assert!(
            holds_vault_key(&vault.store),
            "R9: the revoked token goes, the key stays"
        );
        vault.clear_session().unwrap();
        assert!(!holds_vault_key(&vault.store), "after the switch the old key is gone");
        assert_eq!(vault.account_email().unwrap(), None);
    }

    /// Lead ruling 13 (Task 12): the vault key is BORROWED by the constructor that copies it into the wiped buffer, so
    /// no by-value copy of the key is left behind in a frame that nobody wipes.
    #[test]
    fn the_master_key_constructor_borrows_the_key() {
        let source = include_str!("keychain.rs").replace("\r\n", "\n");
        let at = source
            .find("pub fn new_master_key(bytes: &[u8; MASTER_KEY_BYTES]) -> Self {")
            .expect("the constructor borrows");
        let body = &source[at..];
        let body = &body[body.find('{').unwrap() + 1..body.find("\n    }").unwrap()];
        assert_eq!(
            body.trim(),
            "Self(bytes.to_vec())",
            "the borrowed key goes straight into the wiped buffer, through no local copy"
        );
        // Read without whitespace: a by-value signature with more parameters is wrapped by rustfmt, which puts
        // `bytes:` on a line of its own. (This file is also compiled into `tests/keychain.rs`, where the crate's
        // `source_pin` helper does not exist.)
        let squeezed: String = source.split_whitespace().collect();
        assert!(
            !squeezed.contains(concat!("new_master_key(bytes:[u8", ";")),
            "no by-value constructor"
        );
    }

    /// A retained vault key is seen without unlocking; a store that cannot hold secrets holds none; any other
    /// read error counts as present, so a caller that asks "is anything of an account left?" fails closed.
    #[test]
    fn holds_vault_key_sees_a_retained_key_and_fails_closed() {
        let vault = AuthVault::new(MemoryStore::default());
        assert!(!holds_vault_key(&vault.store));
        vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&[7u8; 32]))
            .unwrap();
        assert!(holds_vault_key(&vault.store));

        struct Answers(fn() -> AuthStoreError);
        impl AuthSecretStore for Answers {
            fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
                Err((self.0)())
            }
            fn delete_session_token(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
                Err((self.0)())
            }
            fn delete_wrapped_master_key(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_account_email(&self, _: &str) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_account_email(&self) -> AuthResult<Option<String>> {
                Err((self.0)())
            }
            fn delete_account_email(&self) -> AuthResult<()> {
                Err((self.0)())
            }
        }
        assert!(!holds_vault_key(&Answers(|| AuthStoreError::Unsupported(
            "no keychain here"
        ))));
        assert!(!holds_vault_key(&Answers(|| AuthStoreError::NotFound)));
        assert!(
            holds_vault_key(&Answers(|| AuthStoreError::Backend("keychain locked".into()))),
            "fail closed"
        );
    }

    /// A store that answers the three presence questions and PANICS on any read of the secret itself: a presence check
    /// that reads key or token bytes would fail the test that uses it.
    struct PresenceOnly {
        token: fn() -> AuthResult<bool>,
        key: fn() -> AuthResult<bool>,
        email: fn() -> AuthResult<bool>,
    }

    impl PresenceOnly {
        const ABSENT: fn() -> AuthResult<bool> = || Ok(false);
    }

    impl AuthSecretStore for PresenceOnly {
        fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
            panic!("a presence check must not read the token")
        }
        fn delete_session_token(&self) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
            panic!("a presence check must not read the key")
        }
        fn delete_wrapped_master_key(&self) -> AuthResult<()> {
            unreachable!("no write")
        }
        fn load_account_email(&self) -> AuthResult<Option<String>> {
            panic!("a presence check must not read the email")
        }
        fn holds_session_token(&self) -> AuthResult<bool> {
            (self.token)()
        }
        fn holds_wrapped_master_key(&self) -> AuthResult<bool> {
            (self.key)()
        }
        fn holds_account_email(&self) -> AuthResult<bool> {
            (self.email)()
        }
    }

    /// The three presence probes behind a sign-in's "is anything of an account left?" share one rule: present is
    /// present, a store that cannot hold secrets or has none holds none, and ANY other answer counts as present
    /// (fail closed). Each probe is checked on its own, so a probe that fails open is seen.
    #[test]
    fn every_presence_probe_sees_a_retained_item_and_fails_closed() {
        type Probe = fn(&PresenceOnly) -> bool;
        type Answer = fn() -> AuthResult<bool>;
        type Build = fn(Answer) -> PresenceOnly;
        let probes: [(&str, Probe, Build); 3] = [
            (
                "the session token",
                |s| holds_session_token(s),
                |answer| PresenceOnly {
                    token: answer,
                    key: PresenceOnly::ABSENT,
                    email: PresenceOnly::ABSENT,
                },
            ),
            (
                "the vault key",
                |s| holds_vault_key(s),
                |answer| PresenceOnly {
                    token: PresenceOnly::ABSENT,
                    key: answer,
                    email: PresenceOnly::ABSENT,
                },
            ),
            (
                "the account email",
                |s| holds_account_email(s),
                |answer| PresenceOnly {
                    token: PresenceOnly::ABSENT,
                    key: PresenceOnly::ABSENT,
                    email: answer,
                },
            ),
        ];
        for (name, probe, store_with) in probes {
            assert!(probe(&store_with(|| Ok(true))), "{name}: present is present");
            assert!(!probe(&store_with(|| Ok(false))), "{name}: absent is absent");
            assert!(
                !probe(&store_with(|| Err(AuthStoreError::Unsupported("no keychain here")))),
                "{name}: a store that cannot hold it holds none"
            );
            assert!(
                !probe(&store_with(|| Err(AuthStoreError::NotFound))),
                "{name}: not found is absent"
            );
            assert!(
                probe(&store_with(|| Err(AuthStoreError::Backend("keychain locked".into())))),
                "{name}: a locked store fails closed"
            );
            assert!(
                probe(&store_with(|| Err(AuthStoreError::InvalidSecret("not UTF-8")))),
                "{name}: an unreadable item fails closed"
            );
        }
    }

    /// A store that implements only the reads (every older store) gets presence from the default, which reads the
    /// item and drops it at once: its answers follow the read's, errors included.
    #[test]
    fn the_default_presence_answers_follow_the_reads_and_fail_closed() {
        let vault = AuthVault::new(MemoryStore::default());
        assert!(
            !holds_session_token(&vault.store) && !holds_vault_key(&vault.store) && !holds_account_email(&vault.store)
        );
        vault.install_session(SessionToken::new("tok").unwrap()).unwrap();
        vault.store_account_email("sam@beebeeb.io").unwrap();
        assert!(
            holds_session_token(&vault.store) && holds_account_email(&vault.store) && !holds_vault_key(&vault.store)
        );

        struct Reads(fn() -> AuthStoreError);
        impl AuthSecretStore for Reads {
            fn save_session_token(&self, _: &SessionToken) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
                Err((self.0)())
            }
            fn delete_session_token(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_wrapped_master_key(&self, _: SecretBytes) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
                Err((self.0)())
            }
            fn delete_wrapped_master_key(&self) -> AuthResult<()> {
                Err((self.0)())
            }
            fn save_account_email(&self, _: &str) -> AuthResult<()> {
                Err((self.0)())
            }
            fn load_account_email(&self) -> AuthResult<Option<String>> {
                Err((self.0)())
            }
            fn delete_account_email(&self) -> AuthResult<()> {
                Err((self.0)())
            }
        }
        let locked = Reads(|| AuthStoreError::Backend("keychain locked".into()));
        assert!(
            holds_session_token(&locked) && holds_vault_key(&locked) && holds_account_email(&locked),
            "a read error counts as present"
        );
        let nothing = Reads(|| AuthStoreError::NotFound);
        assert!(!holds_session_token(&nothing) && !holds_vault_key(&nothing) && !holds_account_email(&nothing));
    }

    /// The session token wipes itself: `zeroize` empties its text, and its `Drop` calls it.
    #[test]
    fn a_session_token_is_wiped_when_it_goes() {
        use zeroize::Zeroize as _;
        let mut token = SessionToken::new("a-secret-session-token").unwrap();
        token.zeroize();
        assert_eq!(token.expose_for_request(), "", "the text is gone");
        let source = include_str!("keychain.rs").replace("\r\n", "\n");
        let drop_impl = &source[source
            .find("impl Drop for SessionToken {")
            .expect("the token wipes on drop")..];
        let drop_impl = &drop_impl[..drop_impl.find("\n}\n").unwrap()];
        assert!(drop_impl.contains("zeroize::Zeroize::zeroize(self)"), "{drop_impl}");
    }

    /// A secret is wiped with the `zeroize` crate (volatile writes the optimizer may not drop), never with a plain fill
    /// before the free, which a compiler may remove as a dead store: `SecretBytes` wipes itself on drop, the macOS read
    /// that fails to free, and the Windows read that fails to decode. No plain fill is left in this file.
    #[test]
    fn secrets_are_wiped_with_zeroize_never_a_plain_fill() {
        use zeroize::Zeroize as _;
        let mut key = SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]);
        key.zeroize();
        assert!(key.expose_for_crypto().is_empty(), "the bytes are gone");
        let source = include_str!("keychain.rs").replace("\r\n", "\n");
        let plain_fill = concat!(".fill", "(0)");
        assert_eq!(
            source.matches(plain_fill).count(),
            0,
            "a plain fill is still used to wipe a secret"
        );
        let secret_bytes_drop = &source[source
            .find("impl Drop for SecretBytes {")
            .expect("SecretBytes wipes on drop")..];
        let secret_bytes_drop = &secret_bytes_drop[..secret_bytes_drop.find("\n}\n").unwrap()];
        assert!(
            secret_bytes_drop.contains("zeroize::Zeroize::zeroize(self)"),
            "{secret_bytes_drop}"
        );
        let windows = &source[source
            .find("impl AuthSecretStore for WindowsCredentialStore {")
            .expect("the Windows store")..];
        let windows = &windows[..windows.find("\n}\n").unwrap()];
        assert!(
            windows.contains("zeroize::Zeroize::zeroize(&mut raw)"),
            "the Windows read that fails to decode wipes with zeroize:\n{windows}"
        );
    }

    /// The real macOS Keychain answers "is it there?" from the item's attributes (`find_item`), never by reading the
    /// secret, and a free that fails after a read wipes what it copied before it reports the error.
    #[test]
    fn the_real_macos_store_answers_presence_without_reading_the_secret() {
        let source = include_str!("keychain.rs").replace("\r\n", "\n");
        let store = &source[source
            .find("impl AuthSecretStore for MacOsKeychainStore {")
            .expect("the real store")..];
        let store = &store[..store.find("\n}\n").unwrap()];
        for presence in [
            "fn holds_session_token(",
            "fn holds_wrapped_master_key(",
            "fn holds_account_email(",
        ] {
            let body = &store[store
                .find(presence)
                .unwrap_or_else(|| panic!("the real store answers {presence}"))..];
            let body = &body[..body.find("\n    }").unwrap_or(body.len())];
            assert!(
                body.contains("macos_keychain::exists("),
                "{presence} asks the item's attributes:\n{body}"
            );
            assert!(
                !body.contains("::load("),
                "{presence} must not read the secret:\n{body}"
            );
        }
        // The FFI module is the last thing in the file, after this test, so the LAST match is the real one.
        let macos = &source[source.rfind("mod macos_keychain {").expect("the macOS FFI module")..];
        let ffi = &macos[macos
            .find("pub fn exists(account: &str)")
            .expect("the attribute-only query")..];
        let exists = &ffi[..ffi.find("\n    }\n").unwrap()];
        assert!(
            exists.contains("find_item(") && !exists.contains("password_data"),
            "{exists}"
        );
        let load = &macos[macos.find("pub fn load(account: &str)").unwrap()..];
        let load = &load[..load.find("\n    }\n").unwrap()];
        assert!(
            load.contains("zeroize::Zeroize::zeroize(&mut bytes)"),
            "a failed free wipes the copy before the error is returned:\n{load}"
        );
    }
}

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
