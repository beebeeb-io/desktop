//! Desktop v2 (batch B02): the `drive_*` commands behind the window's `DriveSource`
//! (`src/v2/source/desktop.ts`; spec `docs/superpowers/specs/2026-11-08-desktop-v2-design.md` 3.1).
//!
//! The boundary (spec 2.1): the session token, the master key and every file key stay in Rust.
//! A command returns view data only (decrypted names, sizes, times, states; thumbnails and file
//! content as binary IPC) and never a key, a wrapped key, a token, a URL, a nonce or a ciphertext.
//! Reads come from the local state DB; changes are made on the sync root so the engine uploads them
//! under core's rules; trash and restore go through the API as today's Trash page does.
//!
//! Errors are `"<code>: <detail>"` with a `DriveSourceError` code; the detail is a fixed word,
//! never a path or a name.

#[cfg(test)]
mod absent_row; // slice 58
mod commands;
mod dto;
mod events;
mod fsops;
pub(crate) mod model;
mod photo_meta; // v2 b02b
mod status; // v2 b02b
mod status_cmds; // v2 b02b
#[cfg(test)]
mod status_tests; // v2 b02b
#[cfg(test)]
mod tests;

pub(crate) use commands::*;
pub(crate) use status_cmds::*; // v2 b02b

use std::fmt;

/// A failed drive command, as the `DriveSourceError` code the UI maps to the board's copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DriveError {
    Offline,
    Unsupported,
    NotFound,
    NameTaken,
    InvalidName,
    Quota,
    ReadOnly,
    Conflict,
    TooLarge,
    Locked,
    SignedOut,
    Failed(&'static str),
}

impl DriveError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            DriveError::Offline => "offline",
            DriveError::Unsupported | DriveError::TooLarge => "unsupported",
            DriveError::NotFound => "not-found",
            DriveError::NameTaken => "name-taken",
            DriveError::Quota => "quota",
            DriveError::ReadOnly => "read-only",
            DriveError::Conflict => "conflict",
            DriveError::Locked => "locked",
            DriveError::SignedOut => "signed-out",
            DriveError::InvalidName | DriveError::Failed(_) => "failed",
        }
    }

    fn detail(&self) -> &'static str {
        match self {
            DriveError::InvalidName => "name",
            DriveError::TooLarge => "too large",
            DriveError::Failed(what) => what,
            _ => "drive",
        }
    }

    pub(crate) fn db<E>(_: E) -> Self {
        DriveError::Failed("state db")
    }

    pub(crate) fn io(error: std::io::Error) -> Self {
        use std::io::ErrorKind;
        match error.kind() {
            ErrorKind::NotFound => DriveError::NotFound,
            ErrorKind::AlreadyExists => DriveError::NameTaken,
            ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => DriveError::ReadOnly,
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded => DriveError::Quota,
            _ => DriveError::Failed("disk"),
        }
    }

    /// An API or session error's text as a code. The text itself is never passed on.
    pub(crate) fn from_text(text: &str) -> Self {
        let t = text.to_ascii_lowercase();
        if t.contains("vault is locked") {
            DriveError::Locked
        } else if t.contains("not signed in")
            || t.contains("401")
            || t.contains("unauthorized")
            || t.contains("no active account")
        {
            DriveError::SignedOut
        } else if t.contains("404") || t.contains("not found") {
            DriveError::NotFound
        } else if t.contains("409") || t.contains("conflict") {
            DriveError::Conflict
        } else if t.contains("402") || t.contains("413") || t.contains("507") || t.contains("quota") {
            DriveError::Quota
        } else if t.contains("403") || t.contains("forbidden") {
            DriveError::ReadOnly
        } else if t.contains("network")
            || t.contains("connect")
            || t.contains("timed out")
            || t.contains("dns")
            || t.contains("sending request")
        {
            DriveError::Offline
        } else {
            DriveError::Failed("request")
        }
    }
}

impl fmt::Display for DriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.detail())
    }
}

impl From<DriveError> for String {
    fn from(error: DriveError) -> String {
        error.to_string()
    }
}
