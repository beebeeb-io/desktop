//! `errors` items split out of `keychain.rs`.

use super::*;

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
