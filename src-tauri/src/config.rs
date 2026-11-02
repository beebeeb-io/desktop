//! Persistent desktop config — `~/.config/beebeeb/desktop.toml`.
//!
//! Stores user-pickable settings that need to survive a restart:
//! the sync root directory chosen on first launch and (eventually,
//! once auth persistence lands) the cached session token + master key.
//!
//! Lives in a TOML file so a user can hand-edit it for recovery if
//! the WebView is broken. Mode `0600` because future versions will
//! contain the master key.
//!
//! All paths are absolute — relative paths in the TOML are rejected
//! at load time so a corrupted file can't make the engine sync from
//! whatever happens to be the cwd.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// Tests only: the config directory for one future (`CONFIG_DIR_OVERRIDE.scope(dir, future)`), instead of the
// per-process sandbox every test shares. For a test that asserts what a command wrote to `desktop.toml` while other
// tests write the shared one in parallel. Code that runs on another task or thread inside the scope uses the shared
// sandbox, as before.
#[cfg(test)]
tokio::task_local! {
    pub(crate) static CONFIG_DIR_OVERRIDE: PathBuf;
}

mod error;
mod lock;
mod paths;
mod settings;
mod store;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use error::ConfigError;
pub(crate) use lock::{HeldConfig, UpdateError};
pub use paths::{default_sync_root_suggestion, ensure_directory};
pub use settings::DesktopSettings;
#[cfg(test)]
pub(crate) use settings::KnownFolderBackup;
pub(crate) use store::ConfigFs;
/// Tests build hosts; outside tests only the macOS sign-out and keep paths build the real file-system steps.
#[cfg(test)]
pub(crate) use store::ConfigHost;
#[cfg(any(test, target_os = "macos"))]
pub(crate) use store::RealConfigFs;
pub(crate) use types::ConfigMode;
pub use types::{DesktopConfig, DesktopTheme, LOCAL_CACHE_LIMIT_100_GB_BYTES, ReleaseChannel};
