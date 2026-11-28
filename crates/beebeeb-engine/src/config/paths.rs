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
    // Under the App Sandbox `dirs::data_dir()` is `~/Library/Application Support` of the app's container
    // (HOME is the container), so this is container-relative and never `~/Beebeeb`.
    macos_state_root(dirs::data_dir(), dirs::config_dir(), identity.config_dir_name())
}

/// The macOS state root from the OS data dir (or config dir, or nothing): `<base>/<name>/file-provider-state`.
/// Never the visible home-folder `~/<sync folder>`. Pure, so every platform tests the choice.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn macos_state_root(data: Option<PathBuf>, config: Option<PathBuf>, name: &str) -> PathBuf {
    data.or(config)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(name)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_state_root_is_container_relative_never_the_visible_folder() {
        let container = PathBuf::from("/c/Library/Application Support");
        let root = macos_state_root(Some(container.clone()), Some(PathBuf::from("/other")), "io.beebeeb.app");
        assert_eq!(root, container.join("io.beebeeb.app").join("file-provider-state"));
        let fallback = macos_state_root(None, Some(PathBuf::from("/cfg")), "x");
        assert!(fallback.starts_with("/cfg"));
        assert!(!root.ends_with("Beebeeb"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_default_is_not_under_a_visible_home_folder() {
        let id = &crate::identity::REAL;
        let root = default_sync_root_suggestion(id);
        assert!(root.ends_with("file-provider-state"));
        assert_ne!(
            root.parent().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("Beebeeb"))
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn elsewhere_the_default_is_still_the_home_sync_folder() {
        let id = &crate::identity::REAL;
        let root = default_sync_root_suggestion(id);
        assert!(root.ends_with(id.sync_folder_name()));
    }
}
