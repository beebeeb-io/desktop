//! Desktop auth, unlock, and secret-storage primitives.
//!
//! This module backs the Tauri runner's login, lock, unlock, and logout flow.
//! It establishes the fail-closed state model and Keychain-backed storage
//! surface used before the sync engine receives any file-content key material.

use std::fmt;

const KEYCHAIN_SERVICE: &str = "io.beebeeb.app";
const SESSION_TOKEN_ACCOUNT: &str = "session-token";
const WRAPPED_MASTER_KEY_ACCOUNT: &str = "wrapped-master-key";
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
