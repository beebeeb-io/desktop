//! `session` items split out of `keychain.rs`.

use super::*;

#[derive(Clone, PartialEq, Eq, Debug)]
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
