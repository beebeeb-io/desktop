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

use serde::{Deserialize, Serialize};

/// Filename inside the platform config dir.
const CONFIG_FILENAME: &str = "desktop.toml";

/// Subdirectory inside the user's config dir (e.g.
/// `~/.config/beebeeb/` on Linux, `~/Library/Application Support/beebeeb/`
/// on macOS).
const APP_CONFIG_DIR: &str = "beebeeb";

/// Top-level desktop config persisted on disk.
///
/// Field order mirrors the TOML file the user might hand-edit. New
/// fields should be added with `#[serde(default)]` so older files
/// still load.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Local folder mirrored against the vault. `None` until the user
    /// completes the first-launch picker.
    #[serde(default)]
    pub sync_root: Option<PathBuf>,
    // NOTE: persisted session deferred to a later step. For now the
    // session is in-memory only (set_session IPC). Adding it here
    // requires the OS-keychain wrapping called out in spec 030 §1.
}

impl DesktopConfig {
    /// Resolve the absolute path to `desktop.toml` for the current user,
    /// creating the parent directory if missing. Errors propagate as
    /// strings so they can flow through Tauri commands.
    pub fn path() -> Result<PathBuf, String> {
        let base = dirs::config_dir()
            .ok_or_else(|| "could not determine user config directory".to_string())?;
        let dir = base.join(APP_CONFIG_DIR);
        fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
        Ok(dir.join(CONFIG_FILENAME))
    }

    /// Load the on-disk config. A missing file is normal on first
    /// launch and yields `DesktopConfig::default()` rather than an
    /// error. A corrupt or unparseable file IS an error — surfacing
    /// it loud avoids silently overwriting a user's settings on save.
    pub fn load() -> Result<Self, String> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        // toml 0.8 removed from_slice; read as UTF-8 string and parse.
        let s = fs::read_to_string(&path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut cfg: Self = toml::from_str(&s)
            .map_err(|e| format!("parse {}: {e}", path.display()))?;

        // Reject relative paths defensively. Hand-editing the TOML to a
        // relative path would otherwise let the engine sync against
        // wherever the binary's cwd happens to be.
        if let Some(p) = &cfg.sync_root
            && !p.is_absolute()
        {
            tracing::warn!(
                "ignoring non-absolute sync_root in desktop.toml: {}",
                p.display()
            );
            cfg.sync_root = None;
        }
        Ok(cfg)
    }

    /// Persist `self` to disk atomically (write to a temp file, then
    /// rename). Sets mode 0600 on Unix so future versions storing key
    /// material aren't world-readable. No-op on Windows where ACL
    /// inheritance from the user's profile is the right default.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path()?;
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| format!("serialize: {e}"))?;

        // Atomic write: temp + rename. Avoids leaving a half-written
        // file if the process is killed mid-save.
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, toml_str.as_bytes())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&tmp, perms)
                .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
        }

        fs::rename(&tmp, &path)
            .map_err(|e| format!("rename {} → {}: {e}", tmp.display(), path.display()))?;
        Ok(())
    }
}

/// Default sync root if the user accepts the suggestion: `~/Beebeeb`.
/// Falls back to `./Beebeeb` (cwd) only on systems where `dirs::home_dir()`
/// can't resolve a home — this is rare enough to not warrant a hard
/// error since the user immediately picks a real path through the
/// dialog.
pub fn default_sync_root_suggestion() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Beebeeb")
}

/// Ensure `path` exists as a directory, creating it (and any missing
/// parents) if necessary. Returns an error if `path` exists but is
/// a file.
pub fn ensure_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!(
                "{} exists but is not a directory",
                path.display()
            ));
        }
        return Ok(());
    }
    fs::create_dir_all(path)
        .map_err(|e| format!("create directory {}: {e}", path.display()))?;
    Ok(())
}
