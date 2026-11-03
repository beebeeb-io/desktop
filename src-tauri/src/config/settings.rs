//! The settings view of the config: `DesktopSettings` as the frontend reads and
//! writes it, its conversion from `DesktopConfig`, and the per-key setters on
//! `DesktopConfig` that merge one setting at a time.
//!
//! Must not persist by itself: every writer goes through `store` so one lock
//! serialises all config writes.
//! Tests: `crate::config::tests`.

use super::*;

/// One known folder's state in the backup set. The command's boolean means
/// these two states; `From<bool>` keeps the frozen `lib.rs` call site working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownFolderBackup {
    Enabled,
    Disabled,
}

impl From<bool> for KnownFolderBackup {
    fn from(enabled: bool) -> Self {
        if enabled { Self::Enabled } else { Self::Disabled }
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
    // Task 1889, fix round 1 (M5): these six are merge-only like the fields below. A write carries the fields that
    // changed; one it leaves out (`None`) keeps the value on the file. A Settings window that sent back the whole object
    // it read on open used to undo a pause made from the tray meanwhile. A read (`From<&DesktopConfig>`) always fills them.
    #[serde(default)]
    pub upload_kbps_limit: Option<u64>,
    #[serde(default)]
    pub download_kbps_limit: Option<u64>,
    #[serde(default)]
    pub pause_sync: Option<bool>,
    #[serde(default)]
    pub notify_conflicts: Option<bool>,
    #[serde(default)]
    pub notify_sync_complete: Option<bool>,
    #[serde(default)]
    pub notify_quota_warnings: Option<bool>,
    /// Sync mode chosen in the Windows first-run wizard. `None` when not
    /// yet set (first run). One of `"everything"`, `"smart"`, `"custom"`,
    /// `"online_only"`.
    #[serde(default)]
    pub sync_mode: Option<String>,
    /// Windows Settings UI toggles. `None` when never set by the user.
    #[serde(default)]
    pub metered: Option<bool>,
    #[serde(default)]
    pub files_on_demand: Option<bool>,
    #[serde(default)]
    pub sync_overlays: Option<bool>,
    /// Advanced appearance preference. `None` means an older caller did not
    /// include the field, so `apply_settings` must preserve the saved value.
    #[serde(default)]
    pub theme: Option<DesktopTheme>,
    /// Advanced local cache cap. `Some(0)` means Unlimited; `None` means an
    /// older caller omitted the field and the saved value should be preserved.
    #[serde(default)]
    pub local_cache_limit_bytes: Option<i64>,
    /// Opt-in desktop update channel. `None` means the caller did not touch
    /// this setting; `DesktopConfig` itself defaults missing on-disk values to
    /// Stable.
    #[serde(default)]
    pub release_channel: Option<ReleaseChannel>,
}

impl From<&DesktopConfig> for DesktopSettings {
    fn from(c: &DesktopConfig) -> Self {
        Self {
            upload_kbps_limit: Some(c.upload_kbps_limit),
            download_kbps_limit: Some(c.download_kbps_limit),
            pause_sync: Some(c.pause_sync),
            notify_conflicts: Some(c.notify_conflicts),
            notify_sync_complete: Some(c.notify_sync_complete),
            notify_quota_warnings: Some(c.notify_quota_warnings),
            sync_mode: c.sync_mode.clone(),
            metered: c.metered,
            files_on_demand: c.files_on_demand,
            sync_overlays: c.sync_overlays,
            theme: Some(c.theme),
            local_cache_limit_bytes: Some(c.local_cache_limit_bytes),
            release_channel: Some(c.release_channel),
        }
    }
}

impl DesktopConfig {
    /// Overlay a settings DTO onto this config without touching
    /// `sync_root`. Used by the `set_desktop_config` IPC to merge a
    /// settings save into whatever's already on disk. Only the fields the
    /// DTO carries (`Some`) are written; every other field keeps its value.
    pub fn apply_settings(&mut self, s: DesktopSettings) {
        if let Some(limit) = s.upload_kbps_limit {
            self.upload_kbps_limit = limit;
        }
        if let Some(limit) = s.download_kbps_limit {
            self.download_kbps_limit = limit;
        }
        if let Some(paused) = s.pause_sync {
            self.pause_sync = paused;
        }
        if let Some(notify) = s.notify_conflicts {
            self.notify_conflicts = notify;
        }
        if let Some(notify) = s.notify_sync_complete {
            self.notify_sync_complete = notify;
        }
        if let Some(notify) = s.notify_quota_warnings {
            self.notify_quota_warnings = notify;
        }
        // Only overwrite sync_mode if the incoming settings carries one;
        // a plain bandwidth/notification save should not clear a persisted mode.
        if s.sync_mode.is_some() {
            self.sync_mode = s.sync_mode;
        }
        // Same merge rule for the Windows toggles: a save from a page that
        // doesn't touch them (None) leaves the persisted value intact;
        // only an explicit Some(true/false) updates it.
        if s.metered.is_some() {
            self.metered = s.metered;
        }
        if s.files_on_demand.is_some() {
            self.files_on_demand = s.files_on_demand;
        }
        if s.sync_overlays.is_some() {
            self.sync_overlays = s.sync_overlays;
        }
        if let Some(theme) = s.theme {
            self.theme = theme;
        }
        if let Some(limit) = s.local_cache_limit_bytes {
            self.local_cache_limit_bytes = limit.max(0);
        }
        if let Some(release_channel) = s.release_channel {
            self.release_channel = release_channel;
        }
    }

    /// Cache budget used by eviction. `None` means Unlimited.
    pub fn local_cache_limit_for_eviction(&self) -> Option<i64> {
        if self.local_cache_limit_bytes <= 0 {
            None
        } else {
            Some(self.local_cache_limit_bytes)
        }
    }

    // ── Known-folder backup helpers (task 0797) ───────────────────────────

    /// `true` if `key` is currently opted into known-folder backup.
    pub fn known_folder_enabled(&self, key: &str) -> bool {
        self.known_folder_backup.iter().any(|k| k == key)
    }

    /// Add or remove a known-folder key from the backup set. Idempotent:
    /// enabling an already-enabled key (or disabling an absent one) is a
    /// no-op. Keeps the list de-duplicated. Does NOT persist — the caller
    /// (the `set_known_folder_backup` IPC) saves.
    pub fn set_known_folder(&mut self, key: &str, backup: impl Into<KnownFolderBackup>) {
        match backup.into() {
            KnownFolderBackup::Enabled if !self.known_folder_enabled(key) => {
                self.known_folder_backup.push(key.to_string());
            }
            KnownFolderBackup::Disabled => self.known_folder_backup.retain(|k| k != key),
            KnownFolderBackup::Enabled => {}
        }
    }
}
