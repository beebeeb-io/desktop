//! Revocations the engine still owes the server (LD-0028): a sign-out that could not reach the server leaves a live
//! 30-day token behind, so the token is written here BEFORE the server is asked, and removed once the server answered.
//! What is still here at the next start is paid then (`app::session_revoke::pay_owed`).
//!
//! The tokens live in ONE entry of the OS credential store, as every other credential does: the macOS Keychain, the
//! Windows Credential Manager (DPAPI). Linux has no store (fail closed: nothing is written, so nothing is owed).
//! A test build never touches the machine's own items (macOS: the in-memory `TEST_KEYCHAIN`). Never logs a token.

use super::{AuthResult, AuthStoreError};
use super::EngineIdentity;
use std::sync::Mutex;
use zeroize::Zeroizing;

/// The entry's name: after the account-scoped leaves, never one a person's account can be called.
#[cfg_attr(test, allow(dead_code))] // a test build keeps its list in memory (`Engine::open`)
const ENTRY: &str = "owed-revocations";

/// Where the sealed list lives. One entry, replaced whole.
pub(crate) trait OwedBackend: Send + Sync {
    fn load(&self) -> AuthResult<Option<Vec<u8>>>;
    fn save(&self, bytes: &[u8]) -> AuthResult<()>;
    fn delete(&self) -> AuthResult<()>;
}

/// The tokens owed to the server. Every change takes the lock, so two sign-outs never lose each other's token.
pub(crate) struct OwedRevocations {
    backend: Box<dyn OwedBackend>,
    lock: Mutex<()>,
}

impl OwedRevocations {
    pub(crate) fn with_backend(backend: Box<dyn OwedBackend>) -> Self {
        Self {
            backend,
            lock: Mutex::new(()),
        }
    }

    /// The OS credential store of `identity`.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn platform(identity: &'static EngineIdentity) -> Self {
        Self::with_backend(Box::new(PlatformBackend {
            service: identity.credential_service(),
        }))
    }

    fn read(&self) -> AuthResult<Vec<Zeroizing<String>>> {
        let Some(bytes) = self.backend.load()? else {
            return Ok(Vec::new());
        };
        let bytes = Zeroizing::new(bytes);
        let text = std::str::from_utf8(&bytes).map_err(|_| AuthStoreError::InvalidSecret("owed list is not UTF-8"))?;
        Ok(text
            .split('\n')
            .filter(|t| !t.trim().is_empty())
            .map(|t| Zeroizing::new(t.to_string()))
            .collect())
    }

    fn write(&self, tokens: &[Zeroizing<String>]) -> AuthResult<()> {
        if tokens.is_empty() {
            return self.backend.delete();
        }
        let joined = Zeroizing::new(tokens.iter().map(|t| t.as_str()).collect::<Vec<_>>().join("\n"));
        self.backend.save(joined.as_bytes())
    }

    /// Record `token` as owed (once). An error means it is NOT recorded: the caller then has only its own try.
    pub(crate) fn owe(&self, token: &str) -> AuthResult<()> {
        if token.trim().is_empty() || token.contains('\n') {
            return Err(AuthStoreError::InvalidSecret("session token cannot be recorded"));
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut tokens = self.read()?;
        if tokens.iter().any(|t| t.as_str() == token) {
            return Ok(());
        }
        tokens.push(Zeroizing::new(token.to_string()));
        self.write(&tokens)
    }

    /// The server answered for `token`: it is no longer owed.
    pub(crate) fn settle(&self, token: &str) -> AuthResult<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut tokens = self.read()?;
        let before = tokens.len();
        tokens.retain(|t| t.as_str() != token);
        if tokens.len() == before {
            return Ok(());
        }
        self.write(&tokens)
    }

    /// Every token still owed (wiped as they drop). A store that cannot be read owes nothing visible.
    pub(crate) fn owed(&self) -> Vec<Zeroizing<String>> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.read().unwrap_or_default()
    }
}

/// The OS store: the platform's raw save/load/delete under the identity's service.
#[cfg_attr(test, allow(dead_code))]
struct PlatformBackend {
    #[allow(dead_code)] // Linux has no store
    service: &'static str,
}

#[cfg(all(target_os = "macos", not(any(test, feature = "test-support"))))]
impl OwedBackend for PlatformBackend {
    fn load(&self) -> AuthResult<Option<Vec<u8>>> {
        super::macos_keychain::load(self.service, ENTRY)
    }
    fn save(&self, bytes: &[u8]) -> AuthResult<()> {
        super::macos_keychain::save(self.service, ENTRY, bytes)
    }
    fn delete(&self) -> AuthResult<()> {
        super::macos_keychain::delete(self.service, ENTRY)
    }
}

#[cfg(target_os = "windows")]
impl OwedBackend for PlatformBackend {
    fn load(&self) -> AuthResult<Option<Vec<u8>>> {
        super::windows_credentials::load(self.service, ENTRY)
    }
    fn save(&self, bytes: &[u8]) -> AuthResult<()> {
        super::windows_credentials::save(self.service, ENTRY, bytes)
    }
    fn delete(&self) -> AuthResult<()> {
        super::windows_credentials::delete(self.service, ENTRY)
    }
}

#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
impl OwedBackend for PlatformBackend {
    fn load(&self) -> AuthResult<Option<Vec<u8>>> {
        let key = super::item_name(self.service, ENTRY);
        Ok(super::TEST_KEYCHAIN.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned())
    }
    fn save(&self, bytes: &[u8]) -> AuthResult<()> {
        let key = super::item_name(self.service, ENTRY);
        super::TEST_KEYCHAIN.lock().unwrap_or_else(|e| e.into_inner()).insert(key, bytes.to_vec());
        Ok(())
    }
    fn delete(&self) -> AuthResult<()> {
        let key = super::item_name(self.service, ENTRY);
        super::TEST_KEYCHAIN.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl OwedBackend for PlatformBackend {
    fn load(&self) -> AuthResult<Option<Vec<u8>>> {
        Ok(None)
    }
    fn save(&self, _bytes: &[u8]) -> AuthResult<()> {
        Err(AuthStoreError::Unsupported("this platform keeps no credential store"))
    }
    fn delete(&self) -> AuthResult<()> {
        Ok(())
    }
}

/// An in-memory list for tests, shareable so a test can look at what "the keychain" holds.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct MemoryOwed(pub(crate) std::sync::Arc<Mutex<Option<Vec<u8>>>>);

#[cfg(test)]
impl OwedBackend for MemoryOwed {
    fn load(&self) -> AuthResult<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, bytes: &[u8]) -> AuthResult<()> {
        *self.0.lock().unwrap() = Some(bytes.to_vec());
        Ok(())
    }
    fn delete(&self) -> AuthResult<()> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}
