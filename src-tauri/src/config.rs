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

/// User-facing cache limit unit. The UI labels these as GB, so use decimal
/// bytes to keep "100 GB" displaying as exactly that.
pub const LOCAL_CACHE_LIMIT_GB: i64 = 1_000_000_000;
pub const LOCAL_CACHE_LIMIT_100_GB_BYTES: i64 = 100 * LOCAL_CACHE_LIMIT_GB;
pub const DEFAULT_LOCAL_CACHE_LIMIT_BYTES: i64 = 0;

/// Top-level desktop config persisted on disk.
///
/// Field order mirrors the TOML file the user might hand-edit. New
/// fields should be added with `#[serde(default)]` so older files
/// still load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    // ── Task 0800 — multi-account Phase 1: persisted account id ────────────
    //
    // The stable id of the single account this install owns. `None` on a fresh
    // install (and on any pre-Phase-1 config that predates the key) until the
    // first login mints one via `ensure_account_id`. It is the ONLY on-disk
    // record of the account id — there is deliberately NO keychain accounts
    // index at N=1 (decision 0808 Q2); the enumerable index defers to Phase 3.
    //
    // ★ LOAD-BEARING ★ Every keychain secret is segmented under this id
    // (`io.beebeeb.app/<account_id>/{session-token,wrapped-master-key,
    // account-email}`). It MUST be persisted BEFORE any segment is written
    // under it, otherwise those secrets orphan under a dead id on the next
    // relaunch (the Phase-0 ephemeral-id hole — see the loud note in
    // `account::synthesize_single_account`). `ensure_account_id` enforces that
    // ordering by saving before returning.
    //
    // `skip_serializing_if = "Option::is_none"` keeps a fresh-install
    // `desktop.toml` byte-identical to the pre-Phase-1 layout until the first
    // login: the key is simply absent until set. It is deliberately NOT in
    // `DesktopSettings`/`apply_settings` — the id never flows through the
    // settings IPC (same rule as `sync_root`), so a settings save can never
    // clobber it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,

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

    // ── Task 0797 — known-folder backup ("Manage backup", Windows) ────────
    //
    // OneDrive-style one-way mirror of the user's real Windows known folders
    // (Desktop / Documents / Pictures / Music / Videos / Downloads) into a
    // matching subfolder under the sync root (Model 2 — decision 0797). The
    // existing enumeration scan then auto-encrypts + uploads whatever the
    // mirror copies in, so the only new engine code is the source→vault copy.
    //
    // Stored as a `Vec<String>` of stable folder *keys* (e.g. "documents",
    // "pictures") — NOT absolute paths — so the list is portable across
    // machines and small to hand-edit. Empty (the `#[serde(default)]`) means
    // "no folders backed up," which is the v1 default (opt-in; see below).
    //
    // IMPORTANT (v1 default-OFF): this list starts EMPTY on a fresh install.
    // We deliberately do NOT auto-enable Desktop/Documents/Pictures on first
    // launch even though decision 0797 named them as the default-on set —
    // silently mirroring the user's personal folders without an explicit
    // prompt would upload a large amount of private data the user never asked
    // to back up. The "default-on Desktop/Documents/Pictures" belongs in an
    // ONBOARDING PROMPT (like OneDrive's setup step) where the user sees and
    // confirms it.
    // TODO(0797-onboarding): add a first-run "Back up these folders?" step
    //   that pre-checks Desktop/Documents/Pictures and writes them here on
    //   accept. Until then the Manage-backup panel is the only enable path.
    /// Known-folder keys the user has opted into backing up. Empty = none.
    #[serde(default)]
    pub known_folder_backup: Vec<String>,

    // ── Task 0804 — known-folder backup first-run onboarding prompt ────────
    //
    // The one-time "Back up these folders?" prompt (decision 0797's default-ON
    // Desktop/Documents/Pictures) shows ONCE on first run, then never again —
    // this flag records that the user has seen it (whether they accepted or
    // skipped). Setting it is the ONLY thing the "Not now" path does; the
    // accept path also writes `known_folder_backup`. It is deliberately NOT in
    // `DesktopSettings`/`apply_settings`: a settings-page save must never flip
    // it, and the show-once decision belongs only to the onboarding flow. The
    // Manage-backup panel can re-open the prompt manually later, but that does
    // not change this flag — show-once governs only the AUTOMATIC first-run
    // appearance.
    /// `true` once the first-run known-folder backup prompt has been shown.
    /// Defaults `false` so a fresh install (and older configs missing the key)
    /// surfaces the prompt exactly once.
    #[serde(default)]
    pub known_folder_onboarding_seen: bool,

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

    // ── Windows Settings UI toggles ───────────────────────────────────
    //
    // Three on/off switches surfaced on the Windows Settings page. Stored
    // as `Option<bool>` so a missing key round-trips as `None` (toggle
    // never set) rather than forcing a default the frontend didn't choose.
    /// Treat the current connection as metered — when `Some(true)`, the
    /// runner pauses sync on a metered network if metered-state detection
    /// is available. Persisted regardless; honoring is best-effort.
    #[serde(default)]
    pub metered: Option<bool>,
    /// Files On-Demand: when `Some(true)` (or unset), new remote files are
    /// kept as cloud-only placeholders until opened. When `Some(false)`,
    /// the user wants everything kept local (pin/hydrate-all).
    #[serde(default)]
    pub files_on_demand: Option<bool>,
    /// Show sync-status overlay icons in Explorer. UI-only hint today;
    /// persisted so the choice survives a restart.
    #[serde(default)]
    pub sync_overlays: Option<bool>,

    // ── Task 1190 — Advanced settings ─────────────────────────────────
    //
    // `theme` drives the WebView color tokens. Native window chrome is not
    // controlled here yet; if a platform titlebar is introduced, read the same
    // field from the Rust window setup path.
    #[serde(default)]
    pub theme: DesktopTheme,
    /// Total local file-content cap in bytes. Includes pinned + unpinned cached
    /// content; eviction only removes unpinned files. `0` means Unlimited.
    #[serde(default = "default_local_cache_limit_bytes")]
    pub local_cache_limit_bytes: i64,

    // ── Desktop update channel ────────────────────────────────────────
    //
    // Opt-in release channel for the Tauri updater. Stable remains the
    // default and maps to the existing desktop/latest.json manifest so older
    // clients and users who never touch the toggle keep their current update
    // behavior.
    #[serde(default)]
    pub release_channel: ReleaseChannel,

    /// Channel provenance for the build currently installed on this machine.
    ///
    /// This is separate from `release_channel`: that field is the user's
    /// selected channel to CHECK, while this field records the channel manifest
    /// that actually SERVED the installed bits. It is not exposed in
    /// `DesktopSettings`, so a settings save cannot rewrite update provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_release_channel: Option<ReleaseChannel>,
}

/// `#[serde(default = ...)]` needs a function returning the default.
fn default_true() -> bool {
    true
}

fn default_local_cache_limit_bytes() -> i64 {
    DEFAULT_LOCAL_CACHE_LIMIT_BYTES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopTheme {
    Light,
    Dark,
    System,
}

impl Default for DesktopTheme {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    Stable,
    Beta,
    Alpha,
}

impl Default for ReleaseChannel {
    fn default() -> Self {
        Self::Stable
    }
}

impl ReleaseChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        }
    }
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
            account_id: None,
            sync_root: None,
            upload_kbps_limit: 0,
            download_kbps_limit: 0,
            pause_sync: false,
            notify_conflicts: true,
            notify_sync_complete: false,
            notify_quota_warnings: true,
            excluded_folder_ids: None,
            known_folder_backup: Vec::new(),
            known_folder_onboarding_seen: false,
            finder_install_status: None,
            finder_install_last_error: None,
            finder_install_last_attempt_at: None,
            finder_install_reason_category: None,
            sync_mode: None,
            metered: None,
            files_on_demand: None,
            sync_overlays: None,
            theme: DesktopTheme::System,
            local_cache_limit_bytes: DEFAULT_LOCAL_CACHE_LIMIT_BYTES,
            release_channel: ReleaseChannel::Stable,
            installed_release_channel: None,
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
            upload_kbps_limit: c.upload_kbps_limit,
            download_kbps_limit: c.download_kbps_limit,
            pause_sync: c.pause_sync,
            notify_conflicts: c.notify_conflicts,
            notify_sync_complete: c.notify_sync_complete,
            notify_quota_warnings: c.notify_quota_warnings,
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
    pub fn set_known_folder(&mut self, key: &str, enabled: bool) {
        let present = self.known_folder_enabled(key);
        if enabled && !present {
            self.known_folder_backup.push(key.to_string());
        } else if !enabled && present {
            self.known_folder_backup.retain(|k| k != key);
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

    /// Return this install's account id, minting + persisting one on first use.
    ///
    /// If `account_id` is already set, returns it unchanged WITHOUT re-saving
    /// (idempotent: a relaunch must not rewrite the file or change the id). If
    /// it is `None` (fresh install, or a pre-Phase-1 config that predates the
    /// key), mints a fresh UUID v4, sets it, and persists immediately via the
    /// atomic `save()` BEFORE returning.
    ///
    /// ★ INVARIANT ★ The id is on disk before this returns, so the caller can
    /// safely segment keychain secrets under it (see the load-bearing note on
    /// the `account_id` field). Persisting first closes the Phase-0
    /// orphan-on-relaunch hole: a crash after this returns still finds the same
    /// id on the next launch, so the secrets written under it are recoverable.
    pub fn ensure_account_id(&mut self) -> Result<String, String> {
        if let Some(id) = &self.account_id {
            return Ok(id.clone());
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.account_id = Some(id.clone());
        self.save()?;
        Ok(id)
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
    use super::{
        DesktopConfig, DesktopSettings, DesktopTheme, ReleaseChannel, default_sync_root_suggestion,
    };

    // ── Task 0800 — multi-account Phase 1: persisted account id ────────────

    #[test]
    fn account_id_defaults_none_and_is_omitted_when_unset() {
        // A fresh config has no account id, matching a pre-Phase-1 install.
        let cfg = DesktopConfig::default();
        assert!(cfg.account_id.is_none());

        // An older file that predates the key parses to `None` via serde default.
        let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
        assert!(cfg.account_id.is_none());
    }

    #[test]
    fn account_id_none_is_byte_identical_to_pre_phase1_serialisation() {
        // `skip_serializing_if = "Option::is_none"` keeps a fresh-install file
        // byte-identical to the pre-Phase-1 layout: the `account_id` key must NOT
        // appear in the serialised TOML until it is actually set. This is the
        // "byte-identical fresh install" guarantee — onboarding still fires off
        // `sync_root.is_none()`, never file-shape changes.
        let cfg = DesktopConfig::default();
        let toml = toml::to_string_pretty(&cfg).expect("serialize default");
        assert!(
            !toml.contains("account_id"),
            "account_id must be omitted from a fresh-install desktop.toml, got:\n{toml}"
        );

        // Once set, the key DOES appear and round-trips losslessly.
        let mut cfg = DesktopConfig::default();
        cfg.account_id = Some("11111111-2222-3333-4444-555555555555".to_string());
        let toml = toml::to_string_pretty(&cfg).expect("serialize with id");
        assert!(toml.contains("account_id"));
        let back: DesktopConfig = toml::from_str(&toml).expect("parse");
        assert_eq!(back.account_id.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
    }

    #[test]
    fn ensure_account_id_is_idempotent_when_already_set() {
        // Pre-set id → returned unchanged, never re-minted. (No save() runs because
        // the early return fires before the persist; this exercises that branch
        // without touching the filesystem.)
        let mut cfg = DesktopConfig::default();
        cfg.account_id = Some("fixed-id".to_string());
        let returned = cfg.ensure_account_id().expect("idempotent path returns Ok");
        assert_eq!(returned, "fixed-id");
        assert_eq!(cfg.account_id.as_deref(), Some("fixed-id"));

        // Calling again still returns the same id, still unchanged.
        let again = cfg.ensure_account_id().expect("second call returns Ok");
        assert_eq!(again, "fixed-id");
        assert_eq!(cfg.account_id.as_deref(), Some("fixed-id"));
    }

    #[test]
    fn ensure_account_id_mints_persists_and_is_stable_across_relaunch() {
        // This exercises the real mint+persist+readback path against the on-disk
        // `desktop.toml`. To avoid clobbering a developer's real config, snapshot
        // whatever is there, run the test, then restore it (or remove the file if
        // none existed). Serialised behind a mutex so the parallel test harness
        // can't race on the single shared config path.
        use std::sync::Mutex;
        static CONFIG_LOCK: Mutex<()> = Mutex::new(());
        let _guard = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let path = DesktopConfig::path().expect("config path resolves");
        let backup = std::fs::read(&path).ok();
        // Start from a clean slate so the mint branch (account_id == None) fires.
        let _ = std::fs::remove_file(&path);

        let restore = || match &backup {
            Some(bytes) => {
                std::fs::write(&path, bytes).expect("restore original config");
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        };

        // Mint: a fresh config has no id; ensure_account_id mints + persists one.
        let mut cfg = DesktopConfig::load().expect("load (missing → default)");
        assert!(cfg.account_id.is_none(), "fresh config must start without an id");
        let minted = match cfg.ensure_account_id() {
            Ok(id) => id,
            Err(e) => {
                restore();
                panic!("ensure_account_id should mint: {e}");
            }
        };
        assert!(uuid::Uuid::parse_str(&minted).is_ok(), "minted id is a valid uuid");

        // Persist: a fresh load from disk (simulating relaunch) sees the SAME id.
        let reloaded = DesktopConfig::load().expect("reload after persist");
        assert_eq!(
            reloaded.account_id.as_deref(),
            Some(minted.as_str()),
            "persisted id must survive a reload"
        );

        // Idempotent across relaunch: ensure_account_id on the reloaded config
        // returns the same id (no re-mint).
        let mut reloaded = reloaded;
        let again = reloaded.ensure_account_id().expect("idempotent on reload");
        assert_eq!(again, minted, "relaunch must keep the same account id");

        restore();
    }

    #[test]
    fn known_folder_backup_defaults_empty() {
        // v1 default-OFF: a fresh config opts into nothing.
        let cfg = DesktopConfig::default();
        assert!(cfg.known_folder_backup.is_empty());
        assert!(!cfg.known_folder_enabled("documents"));
    }

    #[test]
    fn known_folder_backup_set_is_idempotent_and_dedupes() {
        let mut cfg = DesktopConfig::default();

        // Enable twice → present exactly once.
        cfg.set_known_folder("documents", true);
        cfg.set_known_folder("documents", true);
        assert!(cfg.known_folder_enabled("documents"));
        assert_eq!(cfg.known_folder_backup.iter().filter(|k| *k == "documents").count(), 1);

        // A second key coexists.
        cfg.set_known_folder("pictures", true);
        assert!(cfg.known_folder_enabled("pictures"));
        assert_eq!(cfg.known_folder_backup.len(), 2);

        // Disable removes only that key; disabling an absent key is a no-op.
        cfg.set_known_folder("documents", false);
        cfg.set_known_folder("documents", false);
        assert!(!cfg.known_folder_enabled("documents"));
        assert!(cfg.known_folder_enabled("pictures"));
        assert_eq!(cfg.known_folder_backup.len(), 1);
    }

    #[test]
    fn known_folder_onboarding_seen_defaults_false_and_round_trips() {
        // Fresh config → prompt not yet seen, so first-run shows it.
        let cfg = DesktopConfig::default();
        assert!(!cfg.known_folder_onboarding_seen);

        // Missing key (older file) → false via #[serde(default)].
        let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
        assert!(!cfg.known_folder_onboarding_seen);

        // Once marked, the flag survives a serialise/parse round-trip.
        let mut cfg = DesktopConfig::default();
        cfg.known_folder_onboarding_seen = true;
        let s = toml::to_string_pretty(&cfg).expect("serialize");
        let back: DesktopConfig = toml::from_str(&s).expect("parse");
        assert!(back.known_folder_onboarding_seen);
    }

    #[test]
    fn known_folder_backup_round_trips_through_toml() {
        // Missing key (older file) → empty vec via #[serde(default)].
        let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
        assert!(cfg.known_folder_backup.is_empty());

        // A populated list survives a serialise/parse round-trip.
        let mut cfg = DesktopConfig::default();
        cfg.set_known_folder("desktop", true);
        cfg.set_known_folder("pictures", true);
        let s = toml::to_string_pretty(&cfg).expect("serialize");
        let back: DesktopConfig = toml::from_str(&s).expect("parse");
        assert!(back.known_folder_enabled("desktop"));
        assert!(back.known_folder_enabled("pictures"));
        assert_eq!(back.known_folder_backup.len(), 2);
    }

    #[test]
    fn advanced_settings_defaults_and_round_trips() {
        // Missing keys from older desktop.toml files pick the new Advanced
        // defaults: follow the OS theme and keep local cache Unlimited.
        let cfg: DesktopConfig = toml::from_str("").expect("parse empty toml");
        assert_eq!(cfg.theme, DesktopTheme::System);
        assert_eq!(cfg.local_cache_limit_bytes, 0);
        assert_eq!(cfg.local_cache_limit_for_eviction(), None);
        assert_eq!(DesktopConfig::default().local_cache_limit_bytes, 0);

        let mut settings = DesktopSettings::from(&DesktopConfig::default());
        settings.theme = Some(DesktopTheme::Dark);
        settings.local_cache_limit_bytes = Some(0);

        let mut cfg = DesktopConfig::default();
        cfg.apply_settings(settings);
        assert_eq!(cfg.theme, DesktopTheme::Dark);
        assert_eq!(cfg.local_cache_limit_bytes, 0);
        assert_eq!(cfg.local_cache_limit_for_eviction(), None);

        let toml = toml::to_string_pretty(&cfg).expect("serialize advanced settings");
        let back: DesktopConfig = toml::from_str(&toml).expect("parse advanced settings");
        assert_eq!(back.theme, DesktopTheme::Dark);
        assert_eq!(back.local_cache_limit_bytes, 0);
    }

    #[test]
    fn installed_release_channel_defaults_absent_and_round_trips_separately_from_configured_channel() {
        let cfg: DesktopConfig =
            toml::from_str("release_channel = \"alpha\"").expect("parse legacy channel config");
        assert_eq!(cfg.release_channel, ReleaseChannel::Alpha);
        assert_eq!(cfg.installed_release_channel, None);

        let mut cfg = DesktopConfig {
            release_channel: ReleaseChannel::Alpha,
            installed_release_channel: Some(ReleaseChannel::Beta),
            ..DesktopConfig::default()
        };
        cfg.apply_settings(DesktopSettings {
            release_channel: Some(ReleaseChannel::Stable),
            ..DesktopSettings::from(&cfg)
        });

        assert_eq!(cfg.release_channel, ReleaseChannel::Stable);
        assert_eq!(cfg.installed_release_channel, Some(ReleaseChannel::Beta));

        let toml = toml::to_string_pretty(&cfg).expect("serialize installed release channel");
        let back: DesktopConfig = toml::from_str(&toml).expect("parse installed release channel");
        assert_eq!(back.release_channel, ReleaseChannel::Stable);
        assert_eq!(back.installed_release_channel, Some(ReleaseChannel::Beta));
    }

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
