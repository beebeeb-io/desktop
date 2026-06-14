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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Local folder mirrored against the vault. `None` until the user
    /// completes the first-launch picker.
    #[serde(default)]
    pub sync_root: Option<PathBuf>,

    // ── Task 9 settings (Bandwidth.tsx + Notifications.tsx) ───────
    //
    // All six settings are flat fields rather than a nested table
    // so the TOML stays readable when a user hand-edits the file
    // (`upload_kbps_limit = 200` rather than
    // `[bandwidth] upload_kbps_limit = 200`). Defaults match the
    // TS-side DEFAULT_CONFIG so a missing file produces the same
    // user-visible behaviour as a fresh install.
    /// Upload bandwidth ceiling, KB/s. `0` = unlimited.
    #[serde(default)]
    pub upload_kbps_limit: u64,
    /// Download bandwidth ceiling, KB/s. `0` = unlimited.
    #[serde(default)]
    pub download_kbps_limit: u64,
    /// User-toggled global pause. The engine still tracks remote
    /// state when paused; it just stops actively syncing.
    #[serde(default)]
    pub pause_sync: bool,
    /// Surface a native OS notification when a conflict is created.
    /// Defaults true because conflicts that go unnoticed cause silent
    /// data divergence — the worst kind of failure mode.
    #[serde(default = "default_true")]
    pub notify_conflicts: bool,
    /// Notify the user once a sync settles to "all caught up". Off
    /// by default — most users don't want a chime on every settle,
    /// only on the noteworthy events.
    #[serde(default)]
    pub notify_sync_complete: bool,
    /// Surface quota-warning notifications (80% / 95% / full). On by
    /// default because hitting a hard quota mid-upload silently
    /// leaves a partial sync, which is also a worst-case failure.
    #[serde(default = "default_true")]
    pub notify_quota_warnings: bool,

    // ── Task 0090 — selective sync (SelectiveSync.tsx) ─────────────
    //
    // List of top-level vault folder IDs the user has chosen to keep
    // cloud-only on this device. Stored as `Option` so the absence of
    // the key in `desktop.toml` round-trips cleanly: an empty list is
    // serialised back as a missing key (see `set_selective_sync` IPC)
    // rather than `excluded_folder_ids = []`, which keeps hand-edited
    // config files tidy.
    /// Folder IDs excluded from local sync. `None` (or omitted) means
    /// "sync everything," matching the default behaviour before this
    /// field existed.
    #[serde(default)]
    pub excluded_folder_ids: Option<Vec<String>>,

    /// Last known Finder/File Provider setup status. This is persisted so
    /// a failed deferred setup remains visible after navigation or restart.
    #[serde(default)]
    pub finder_install_status: Option<String>,
    #[serde(default)]
    pub finder_install_last_error: Option<String>,
    #[serde(default)]
    pub finder_install_last_attempt_at: Option<i64>,
    #[serde(default)]
    pub finder_install_reason_category: Option<String>,
    // NOTE: persisted session deferred to a later step. For now the
    // session is in-memory only (set_session IPC). Adding it here
    // requires the OS-keychain wrapping called out in spec 030 §1.

    // ── WS1 — Windows first-run sync mode ─────────────────────────────
    //
    // Set by the `set_sync_mode` IPC after the user picks a mode in
    // WindowsFirstRun.tsx. The `desktop_config` IPC returns `None`
    // when the whole file is absent (first run), which the frontend
    // uses as the "sync-mode not yet picked" signal. Persisting it
    // here means a future engine version can read and act on it.
    #[serde(default)]
    pub sync_mode: Option<String>,
}

/// `#[serde(default = ...)]` needs a function returning the default.
fn default_true() -> bool {
    true
}

impl Default for DesktopConfig {
    /// Mirror the `#[serde(default = ...)]` annotations so a fresh
    /// in-memory config produces the same notification-on defaults
    /// as one parsed from a TOML file with those keys missing.
    /// Important: a missing on-disk config and a missing key inside
    /// the on-disk config must agree, otherwise users see a different
    /// initial state on first launch vs. on a subsequent partial-edit
    /// reload.
    fn default() -> Self {
        Self {
            sync_root: None,
            upload_kbps_limit: 0,
            download_kbps_limit: 0,
            pause_sync: false,
            notify_conflicts: true,
            notify_sync_complete: false,
            notify_quota_warnings: true,
            excluded_folder_ids: None,
            finder_install_status: None,
            finder_install_last_error: None,
            finder_install_last_attempt_at: None,
            finder_install_reason_category: None,
            sync_mode: None,
        }
    }
}

/// Settings-only DTO for the `get_desktop_config` / `set_desktop_config`
/// IPC commands. Does NOT include `sync_root` — the WebView never
/// edits the sync root through these commands (the `pick_sync_root`
/// IPC owns that), and including it here would risk the TS side
/// accidentally clobbering the on-disk path on a settings save when
/// the load failed and DEFAULT_CONFIG was substituted.
///
/// The shape mirrors `DesktopConfig` (the TypeScript `DesktopConfig`
/// interface in Bandwidth.tsx / Notifications.tsx) for everything
/// except `sync_root`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopSettings {
    pub upload_kbps_limit: u64,
    pub download_kbps_limit: u64,
    pub pause_sync: bool,
    pub notify_conflicts: bool,
    pub notify_sync_complete: bool,
    pub notify_quota_warnings: bool,
    /// Sync mode chosen in the Windows first-run wizard. `None` when not
    /// yet set (first run). One of `"everything"`, `"smart"`, `"custom"`,
    /// `"online_only"`.
    #[serde(default)]
    pub sync_mode: Option<String>,
}

impl From<&DesktopConfig> for DesktopSettings {
    fn from(c: &DesktopConfig) -> Self {
        Self {
            upload_kbps_limit: c.upload_kbps_limit,
            download_kbps_limit: c.download_kbps_limit,
            pause_sync: c.pause_sync,
            notify_conflicts: c.notify_conflicts,
            notify_sync_complete: c.notify_sync_complete,
            notify_quota_warnings: c.notify_quota_warnings,
            sync_mode: c.sync_mode.clone(),
        }
    }
}

impl DesktopConfig {
    /// Overlay a settings DTO onto this config without touching
    /// `sync_root`. Used by the `set_desktop_config` IPC to merge a
    /// settings save into whatever's already on disk.
    pub fn apply_settings(&mut self, s: DesktopSettings) {
        self.upload_kbps_limit = s.upload_kbps_limit;
        self.download_kbps_limit = s.download_kbps_limit;
        self.pause_sync = s.pause_sync;
        self.notify_conflicts = s.notify_conflicts;
        self.notify_sync_complete = s.notify_sync_complete;
        self.notify_quota_warnings = s.notify_quota_warnings;
        // Only overwrite sync_mode if the incoming settings carries one;
        // a plain bandwidth/notification save should not clear a persisted mode.
        if s.sync_mode.is_some() {
            self.sync_mode = s.sync_mode;
        }
    }
}

impl DesktopConfig {
    /// Resolve the absolute path to `desktop.toml` for the current user,
    /// creating the parent directory if missing. Errors propagate as
    /// strings so they can flow through Tauri commands.
    pub fn path() -> Result<PathBuf, String> {
        let base = dirs::config_dir().ok_or_else(|| "could not determine user config directory".to_string())?;
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
        let s = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let mut cfg: Self = toml::from_str(&s).map_err(|e| format!("parse {}: {e}", path.display()))?;

        // Reject relative paths defensively. Hand-editing the TOML to a
        // relative path would otherwise let the engine sync against
        // wherever the binary's cwd happens to be.
        if let Some(p) = &cfg.sync_root
            && !p.is_absolute()
        {
            tracing::warn!("ignoring non-absolute sync_root in desktop.toml: {}", p.display());
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
        let toml_str = toml::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;

        // Atomic write: temp + rename. Avoids leaving a half-written
        // file if the process is killed mid-save.
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, toml_str.as_bytes()).map_err(|e| format!("write {}: {e}", tmp.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&tmp, perms).map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
        }

        fs::rename(&tmp, &path).map_err(|e| format!("rename {} → {}: {e}", tmp.display(), path.display()))?;
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
fn user_visible_home_dir() -> Option<PathBuf> {
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
pub fn default_sync_root_suggestion() -> PathBuf {
    dirs::data_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_CONFIG_DIR)
        .join("file-provider-state")
}

/// Default sync root if the user accepts the suggestion on platforms that
/// expose an actual local folder as the user-facing sync surface.
#[cfg(not(target_os = "macos"))]
pub fn default_sync_root_suggestion() -> PathBuf {
    user_visible_home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Beebeeb")
}

/// Ensure `path` exists as a directory, creating it (and any missing
/// parents) if necessary. Returns an error if `path` exists but is
/// a file.
pub fn ensure_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!("{} exists but is not a directory", path.display()));
        }
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|e| format!("create directory {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::default_sync_root_suggestion;

    #[test]
    #[cfg(target_os = "macos")]
    fn default_sync_root_uses_private_file_provider_state_folder() {
        let path = default_sync_root_suggestion();

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("file-provider-state")
        );
        assert!(path.to_string_lossy().contains("beebeeb"));
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn default_sync_root_uses_user_facing_beebeeb_folder() {
        let path = default_sync_root_suggestion();

        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("Beebeeb"));
    }
}
