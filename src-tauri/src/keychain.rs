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

#[cfg(target_os = "macos")]
pub struct MacOsKeychainStore;

#[cfg(target_os = "macos")]
impl MacOsKeychainStore {
    pub fn new() -> Self {
        Self
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
        macos_keychain::save(SESSION_TOKEN_ACCOUNT, token.expose_for_request().as_bytes())
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        macos_keychain::load(SESSION_TOKEN_ACCOUNT)?
            .map(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| AuthStoreError::InvalidSecret("session token is not UTF-8"))
                    .and_then(SessionToken::new)
            })
            .transpose()
    }

    fn delete_session_token(&self) -> AuthResult<()> {
        macos_keychain::delete(SESSION_TOKEN_ACCOUNT)
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        macos_keychain::save(WRAPPED_MASTER_KEY_ACCOUNT, wrapped.expose_for_crypto())
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        macos_keychain::load(WRAPPED_MASTER_KEY_ACCOUNT)?
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        macos_keychain::delete(WRAPPED_MASTER_KEY_ACCOUNT)
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        macos_keychain::save(ACCOUNT_EMAIL_ACCOUNT, email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        macos_keychain::load(ACCOUNT_EMAIL_ACCOUNT)?
            .map(|bytes| String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8")))
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        macos_keychain::delete(ACCOUNT_EMAIL_ACCOUNT)
    }
}

#[cfg(not(target_os = "macos"))]
pub struct MacOsKeychainStore;

#[cfg(not(target_os = "macos"))]
impl MacOsKeychainStore {
    pub fn new() -> Self {
        Self
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
pub struct WindowsCredentialStore;

#[cfg(target_os = "windows")]
impl WindowsCredentialStore {
    pub fn new() -> Self {
        Self
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
        windows_credentials::save(SESSION_TOKEN_ACCOUNT, token.expose_for_request().as_bytes())
    }

    fn load_session_token(&self) -> AuthResult<Option<SessionToken>> {
        windows_credentials::load(SESSION_TOKEN_ACCOUNT)?
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
        windows_credentials::delete(SESSION_TOKEN_ACCOUNT)
    }

    fn save_wrapped_master_key(&self, wrapped: SecretBytes) -> AuthResult<()> {
        windows_credentials::save(WRAPPED_MASTER_KEY_ACCOUNT, wrapped.expose_for_crypto())
    }

    fn load_wrapped_master_key(&self) -> AuthResult<Option<SecretBytes>> {
        windows_credentials::load(WRAPPED_MASTER_KEY_ACCOUNT)?
            .map(SecretBytes::new)
            .transpose()
    }

    fn delete_wrapped_master_key(&self) -> AuthResult<()> {
        windows_credentials::delete(WRAPPED_MASTER_KEY_ACCOUNT)
    }

    fn save_account_email(&self, email: &str) -> AuthResult<()> {
        windows_credentials::save(ACCOUNT_EMAIL_ACCOUNT, email.as_bytes())
    }

    fn load_account_email(&self) -> AuthResult<Option<String>> {
        windows_credentials::load(ACCOUNT_EMAIL_ACCOUNT)?
            .map(|bytes| String::from_utf8(bytes).map_err(|_| AuthStoreError::InvalidSecret("account email is not UTF-8")))
            .transpose()
    }

    fn delete_account_email(&self) -> AuthResult<()> {
        windows_credentials::delete(ACCOUNT_EMAIL_ACCOUNT)
    }
}

#[cfg(not(target_os = "windows"))]
pub struct WindowsCredentialStore;

#[cfg(not(target_os = "windows"))]
impl WindowsCredentialStore {
    pub fn new() -> Self {
        Self
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

/// Construct the secret store for the current OS. Used by the Tauri runner so
/// session/vault persistence routes to the platform-native credential vault.
pub fn platform_keychain_store() -> PlatformKeychainStore {
    PlatformKeychainStore::new()
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
