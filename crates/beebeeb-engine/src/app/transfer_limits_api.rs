//! This PC's transfer limits, for a host without a UI toolkit (the native Drive settings): how fast files may go up
//! and come down, and how much room downloaded copies may take. They are this PC's own settings (`desktop.toml`),
//! not an account's, and the engine honours them where it sends, fetches and frees space: the upload and download
//! paths read the two speeds for every transfer, the cache limit is the one eviction works to.
//!
//! WRITES the settings file: [`Engine::set_transfer_limits_for`], for the session of the press and no other: a
//! session that moved on since (a lock, a sign-out, another sign-in) changes nothing. The session is asked before
//! the file is written, with no session change half done; the file itself is written under the config-write lock,
//! merged into what is there (no other setting is touched). Reads nothing of an account. Must not: log a value.

use std::path::Path;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::config::DesktopConfig;

/// The limits as the settings file holds them. `0`: no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferLimits {
    /// Upload speed, in KiB per second (the unit the upload path works in).
    pub upload_kib_per_s: u64,
    /// Download speed, in KiB per second.
    pub download_kib_per_s: u64,
    /// Room for downloaded copies that are not kept, in bytes.
    pub downloaded_copies_bytes: i64,
}

/// What to change; a limit left `None` keeps its value. `Some(0)`: no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferLimitsChange {
    pub upload_kib_per_s: Option<u64>,
    pub download_kib_per_s: Option<u64>,
    pub downloaded_copies_bytes: Option<i64>,
}

/// Why the limits were not read or not changed. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferLimitsError {
    /// The session is not the one the change was asked under (or none is in memory): nothing was written.
    SessionChanged,
    /// The settings file could not be read or written.
    SettingsFile,
}

impl TransferLimitsError {
    pub fn words(self) -> &'static str {
        match self {
            Self::SessionChanged => "the session changed",
            Self::SettingsFile => "the settings file could not be read or written",
        }
    }
}

impl std::fmt::Display for TransferLimitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.words())
    }
}

impl std::error::Error for TransferLimitsError {}

fn limits_of(config: &DesktopConfig) -> TransferLimits {
    TransferLimits {
        upload_kib_per_s: config.upload_kbps_limit,
        download_kib_per_s: config.download_kbps_limit,
        downloaded_copies_bytes: config.local_cache_limit_bytes.max(0),
    }
}

/// The limits in the settings file at `path` (a file that is not there yet: the defaults).
pub fn transfer_limits_at(path: &Path) -> Result<TransferLimits, TransferLimitsError> {
    DesktopConfig::load_from(path)
        .map(|config| limits_of(&config))
        .map_err(|_| TransferLimitsError::SettingsFile)
}

/// WRITES: `change` merged into the settings file at `path`, under the config-write lock. The limits the file holds
/// after it. A change that changes nothing writes nothing.
pub fn set_transfer_limits_at(
    path: &Path,
    change: TransferLimitsChange,
) -> Result<TransferLimits, TransferLimitsError> {
    DesktopConfig::update_at(path, |config| {
        let before = limits_of(config);
        if let Some(limit) = change.upload_kib_per_s {
            config.upload_kbps_limit = limit;
        }
        if let Some(limit) = change.download_kib_per_s {
            config.download_kbps_limit = limit;
        }
        if let Some(limit) = change.downloaded_copies_bytes {
            config.local_cache_limit_bytes = limit.max(0);
        }
        let after = limits_of(config);
        (after != before, after)
    })
    .map_err(|_| TransferLimitsError::SettingsFile)
}

impl<C: CredentialStores> Engine<C> {
    /// This PC's transfer limits, from the settings file. Reads a file: a worker's.
    pub fn transfer_limits(&self) -> Result<TransferLimits, TransferLimitsError> {
        let path = DesktopConfig::path_for(self.identity).map_err(|_| TransferLimitsError::SettingsFile)?;
        transfer_limits_at(&path)
    }

    /// WRITES: change this PC's transfer limits, for the session a person pressed under and no other. `session` is
    /// [`Engine::session_turn`] as the host read it AT THE PRESS. A session that moved on since, or a session change
    /// under way: [`TransferLimitsError::SessionChanged`], and the file is not touched. The answer is what the file
    /// holds after the change. Writes a file: a worker's.
    pub fn set_transfer_limits_for(
        &self,
        session: SessionGeneration,
        change: TransferLimitsChange,
    ) -> Result<TransferLimits, TransferLimitsError> {
        if self.acct.session.is_poisoned() {
            return Err(TransferLimitsError::SessionChanged);
        }
        self.acct
            .while_session_is(&self.writes, session, |_| ())
            .ok_or(TransferLimitsError::SessionChanged)?;
        let path = DesktopConfig::path_for(self.identity).map_err(|_| TransferLimitsError::SettingsFile)?;
        set_transfer_limits_at(&path, change)
    }
}

#[cfg(test)]
#[path = "transfer_limits_api_tests.rs"]
mod tests;
