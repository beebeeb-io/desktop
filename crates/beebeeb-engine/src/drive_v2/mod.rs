//! The drive window's data, read from the local state database: the DTOs a shell shows (`dto`) and the view
//! models that build them (`model`). No Tauri type appears here; the 0.8.x Tauri shell wrapped these in its `drive_*`
//! commands (`src-tauri/src/drive_v2/commands.rs`); the Rune shell calls them directly.
//!
//! Errors are `"<code>: <detail>"` with a `DriveSourceError` code; the detail is a fixed word,
//! never a path or a name.

pub mod case_place;
pub mod dto;
pub mod model;
pub mod model_shell;
pub mod photo_meta;
pub mod status;

#[cfg(test)]
mod case_place_tests;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod photo_meta_tests;
#[cfg(test)]
mod status_tests;

use std::fmt;

/// A failed drive command, as the `DriveSourceError` code the UI maps to the board's copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveError {
    Offline,
    Unsupported,
    NotFound,
    NameTaken,
    InvalidName,
    Quota,
    ReadOnly,
    Conflict,
    /// The entry at the item's place on disk is another item's (`case_place`: two server items with one name, or one
    /// name but for case on a volume that compares names without case). Refused; the other item's file is untouched.
    SharedPlace(case_place::Clash),
    TooLarge,
    Locked,
    SignedOut,
    Failed(&'static str),
}

impl DriveError {
    pub fn code(&self) -> &'static str {
        match self {
            DriveError::Offline => "offline",
            DriveError::Unsupported | DriveError::TooLarge => "unsupported",
            DriveError::NotFound => "not-found",
            DriveError::NameTaken => "name-taken",
            DriveError::Quota => "quota",
            DriveError::ReadOnly => "read-only",
            DriveError::Conflict | DriveError::SharedPlace(_) => "conflict",
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

    pub fn db<E>(_: E) -> Self {
        DriveError::Failed("state db")
    }

    pub fn io(error: std::io::Error) -> Self {
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
    pub fn from_text(text: &str) -> Self {
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
