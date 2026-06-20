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

    pub fn new_master_key(bytes: [u8; MASTER_KEY_BYTES]) -> Self {
        Self(bytes.to_vec())
    }

    pub fn expose_for_crypto(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.0.fill(0);
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
        Err(AuthStoreError::Unsupported("account email storage not supported by this store"))
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

#[cfg(target_os = "macos")]
pub struct MacOsKeychainStore {
    account_prefix: Option<String>,
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
impl Default for MacOsKeychainStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
impl AuthSecretStore for MacOsKeychainStore {
    fn save_session_token(&self, token: &SessionToken) -> AuthResult<()> {
        macos_keychain::save(&self.account_path(SESSION_TOKEN_ACCOUNT), token.expose_for_request().as_bytes())
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
        macos_keychain::save(&self.account_path(WRAPPED_MASTER_KEY_ACCOUNT), wrapped.expose_for_crypto())
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
            .map(|bytes| String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8")))
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        macos_keychain::delete(&self.account_path(ACCOUNT_EMAIL_ACCOUNT))
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
        windows_credentials::save(&self.account_path(SESSION_TOKEN_ACCOUNT), token.expose_for_request().as_bytes())
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        windows_credentials::load(&self.account_path(SESSION_TOKEN_ACCOUNT))?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|e| {
                        // Bearer-token hygiene: the raw blob is the (malformed)
                        // session token. Recover the bytes from the error and
                        // zero them before discarding so the secret doesn't
                        // linger in a freed allocation.
                        e.into_bytes().fill(0);
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
        windows_credentials::save(&self.account_path(WRAPPED_MASTER_KEY_ACCOUNT), wrapped.expose_for_crypto())
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
            .map(|bytes| String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8")))
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

#[cfg(target_os = "macos")]
pub type PlatformKeychainStore = MacOsKeychainStore;

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
/// 4.   WRITE new: token always; key/email only if legacy had them. `SecretBytes`
///      is `!Clone`, so the loaded key is MOVED straight into the save (never
///      bound + reused). `SessionToken` IS `Clone`.
/// 5.   VERIFY readback BEFORE any delete: new token present; if legacy had a
///      key → new key present; if legacy had email → new email present. ANY
///      failure → return `Err` WITHOUT deleting (vault still usable via the
///      legacy fallback).
/// 6.   ONLY now delete the legacy trio (each delete idempotent; NotFound → Ok).
pub fn migrate_legacy_keychain_between<N: AuthSecretStore, L: AuthSecretStore>(
    new: &N,
    legacy: &L,
) -> AuthResult<()> {
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
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_ENTERPRISE,
        CRED_TYPE_GENERIC,
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
        let result = unsafe {
            CredReadW(
                PCWSTR(target.as_ptr()),
                CRED_TYPE_GENERIC,
                0,
                &mut credential,
            )
        };

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
            .store_wrapped_master_key(SecretBytes::new_master_key([7u8; MASTER_KEY_BYTES]))
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
            .store_wrapped_master_key(SecretBytes::new_master_key([7u8; MASTER_KEY_BYTES]))
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
            .store_wrapped_master_key(SecretBytes::new_master_key([7u8; MASTER_KEY_BYTES]))
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
            self.vault
                .lock()
                .unwrap()
                .insert(self.key(SESSION_TOKEN_ACCOUNT), token.expose_for_request().as_bytes().to_vec());
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
            self.vault
                .lock()
                .unwrap()
                .insert(self.key(WRAPPED_MASTER_KEY_ACCOUNT), wrapped.expose_for_crypto().to_vec());
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
            legacy.save_wrapped_master_key(SecretBytes::new_master_key(k)).unwrap();
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
        assert_eq!(
            new.load_session_token().unwrap().unwrap().expose_for_request(),
            "tok"
        );
        assert_eq!(
            new.load_wrapped_master_key().unwrap().unwrap().expose_for_crypto(),
            &[9u8; 32]
        );
        assert_eq!(new.load_account_email().unwrap().as_deref(), Some("u@example.com"));
        // Legacy trio gone.
        assert!(!legacy_present(&vault), "legacy entries must be deleted after migration");
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

        assert_eq!(
            new.load_session_token().unwrap().unwrap().expose_for_request(),
            "tok"
        );
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
            new.save_wrapped_master_key(SecretBytes::new_master_key([9u8; 32])).unwrap();
            new.save_account_email("u@example.com").unwrap();
        }
        assert!(legacy_present(&vault), "precondition: legacy still present pre-rerun");

        let new = KeyedMemoryStore::with_account(vault.clone(), TEST_ID);
        let legacy = KeyedMemoryStore::legacy(vault.clone());
        migrate_legacy_keychain_between(&new, &legacy).expect("re-run converges");

        assert!(!legacy_present(&vault), "leftover legacy swept on re-run");
        assert_eq!(
            new.load_session_token().unwrap().unwrap().expose_for_request(),
            "tok"
        );
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
            new.save_wrapped_master_key(SecretBytes::new_master_key([9u8; 32])).unwrap();
            new.save_account_email("u@example.com").unwrap();
        }
        {
            let legacy = KeyedMemoryStore::legacy(vault.clone());
            // Only key + email remain (token was deleted before the crash).
            legacy.save_wrapped_master_key(SecretBytes::new_master_key([9u8; 32])).unwrap();
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

        assert_eq!(
            new.load_session_token().unwrap().unwrap().expose_for_request(),
            "tok"
        );
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
}

#[cfg(target_os = "macos")]
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
        let bytes = unsafe { std::slice::from_raw_parts(password_data.cast::<u8>(), password_len as usize).to_vec() };
        let free_status = unsafe { SecKeychainItemFreeContent(ptr::null_mut(), password_data) };
        status_ok("free keychain content", free_status)?;
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
