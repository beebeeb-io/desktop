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
#[cfg(any(test, feature = "test-support"))]
tokio::task_local! {
    pub static CONFIG_DIR_OVERRIDE: PathBuf;
}

mod error;
mod lock;
mod paths;
mod settings;
mod store;
mod types;

#[cfg(test)]
mod tests;

pub use error::ConfigError;
pub use lock::{HeldConfig, UpdateError};
pub use paths::{default_sync_root_suggestion, ensure_directory};
pub use settings::DesktopSettings;
#[cfg(any(test, feature = "test-support"))]
pub use settings::KnownFolderBackup;
pub use store::ConfigFs;
/// Tests build hosts; outside tests only the macOS sign-out and keep paths build the real file-system steps.
#[cfg(any(test, feature = "test-support"))]
pub use store::ConfigHost;
#[cfg(any(any(test, feature = "test-support"), target_os = "macos"))]
pub use store::RealConfigFs;
pub use types::ConfigMode;
pub use types::{DesktopConfig, DesktopTheme, KnownFolderSource, LOCAL_CACHE_LIMIT_100_GB_BYTES, ReleaseChannel};
