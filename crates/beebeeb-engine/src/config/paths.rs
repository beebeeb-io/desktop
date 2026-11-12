//! Where `desktop.toml` and the sync root live, and the platform's default
//! sync-root suggestion.
//!
//! Only computes paths and creates the sync-root directory: it must not read or
//! write the config. The two `default_sync_root_suggestion` bodies are `#[cfg]`
//! twins; keep both, one per platform.
//! Tests: `crate::config::tests`.

use super::*;

/// Filename inside the platform config dir.
pub(super) const CONFIG_FILENAME: &str = "desktop.toml";

#[cfg(not(target_os = "macos"))]
pub(super) fn user_visible_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Default local state root.
///
/// On macOS, the File Provider domain is the user-visible Finder surface. The
/// Rust runner still needs durable SQLite/cache/staging space, but that belongs
/// in app-private Application Support rather than a second visible `~/Beebeeb`
/// folder. Other platforms keep the traditional user-chosen folder model until
/// their native virtualization layers are implemented.
#[cfg(target_os = "macos")]
pub fn default_sync_root_suggestion(identity: &'static crate::identity::EngineIdentity) -> PathBuf {
    dirs::data_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(identity.config_dir_name())
        .join("file-provider-state")
}

/// Default sync root if the user accepts the suggestion on platforms that
/// expose an actual local folder as the user-facing sync surface.
#[cfg(not(target_os = "macos"))]
pub fn default_sync_root_suggestion(identity: &'static crate::identity::EngineIdentity) -> PathBuf {
    user_visible_home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(identity.sync_folder_name())
}

/// Ensure `path` exists as a directory, creating it (and any missing
/// parents) if necessary. Returns an error if `path` exists but is
/// a file.
pub fn ensure_directory(path: &Path) -> Result<(), ConfigError> {
    if path.exists() {
        if !path.is_dir() {
            return Err(ConfigError::NotADirectory(path.to_path_buf()));
        }
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|source| ConfigError::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}
