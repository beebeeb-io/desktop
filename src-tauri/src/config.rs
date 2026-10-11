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

/// Task 1882 r5, task 1889: the one process-wide lock around writing `desktop.toml`.
///
/// Every save of the config goes through the same `desktop.toml.tmp`, so two saves at once could
/// tear each other's temp file, and two load-modify-save passes at once lose the earlier one's
/// change. There is no public `save`: the only way to write the file is [`DesktopConfig::update_at`]
/// (or [`DesktopConfig::update`], [`HeldConfig::update_held`] and
/// [`DesktopConfig::ensure_account_id`], which are `update_at`), and it takes this lock across the
/// whole load, change and write. The lock is not re-entrant: nothing inside an update closure may
/// call another update.
static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Takes [`CONFIG_WRITE_LOCK`]. A holder that panicked leaves the lock usable: the file itself is
/// only ever replaced by a rename, so it is never half-written.
fn config_write_guard() -> std::sync::MutexGuard<'static, ()> {
    CONFIG_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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

    /// The last email that signed in on this install, kept ONLY so the
    /// onboarding sign-in form can prefill it after a sign-out or a
    /// startup-401 auto sign-out (the keychain account-email credential is
    /// deliberately erased by `AuthVault::clear_session` — "logout leaves no
    /// PII behind" for credential material — so without this field the
    /// sign-in form would always start blank).
    ///
    /// Non-secret metadata, same class as `account_id`: it is deliberately
    /// NOT in `DesktopSettings`/`apply_settings` so a settings save can never
    /// read, clobber, or clear it. The file is written mode 0600 (Unix);
    /// hand-editing the TOML removes the prefill. `skip_serializing_if`
    /// keeps configs that never signed in byte-identical to the old layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_signed_in_email: Option<String>,

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
    /// Surface a native OS notification when local cache usage crosses
    /// 90% of `local_cache_limit_bytes` (Settings → Advanced). This is a
    /// desktop-local cache-cap warning, distinct from the account's
    /// cloud-storage-quota notification. On by default because silently
    /// running out of local cache room stops caching new content, which
    /// is also a worst-case failure. Never fires when the cap is
    /// Unlimited (`local_cache_limit_bytes == 0`).
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
    // session is in-memory only (installed via apply_session). Adding it
    // here requires the OS-keychain wrapping called out in spec 030 §1.

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

    /// Task 1882 round 2 (review I2, lead ruling 2026-10-10): the folder where macOS last kept
    /// Finder files that had not reached the server when Beebeeb's Finder location was removed.
    /// Settings › Sync shows it until the person dismisses it. Exactly as the system reported it;
    /// never logged, never sent anywhere. Like `account_id`, it is NOT in
    /// `DesktopSettings`/`apply_settings`, so a settings save can never clear it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_unsynced_folder: Option<String>,

    // ── Spec 2026-10-06 (macOS Finder setup reconciler) §9 ────────────
    //
    // Truth about Finder comes from macOS on every check. These two keys are only memory: the
    // `finder_install_*` keys above stay for Windows/Linux and are no longer written on macOS.
    /// Set when the person signs out (the reconciler then removes Beebeeb from Finder); cleared
    /// at the next sign-in. A startup 401 that discards a revoked session does NOT set it, so
    /// ruling R2 keeps Beebeeb in Finder across that relaunch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finder_signed_out_by_choice: bool,
    /// The last failure, for "Copy details" across a relaunch. Kept last: it serializes as a
    /// TOML table, and a table belongs after the plain values. The `toml` crate we use reorders
    /// on its own (swapping the two fields changes no test result), so this is for the reader and
    /// for any other writer of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finder_last_failure: Option<crate::finder_setup::error::FailureRecord>,
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
#[derive(Default)]
pub enum DesktopTheme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ReleaseChannel {
    #[default]
    Stable,
    Beta,
    Alpha,
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
            last_signed_in_email: None,
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
            kept_unsynced_folder: None,
            finder_signed_out_by_choice: false,
            finder_last_failure: None,
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

// Tests only: the config directory for one future (`CONFIG_DIR_OVERRIDE.scope(dir, future)`), instead of the
// per-process sandbox every test shares. For a test that asserts what a command wrote to `desktop.toml` while other
// tests write the shared one in parallel. Code that runs on another task or thread inside the scope uses the shared
// sandbox, as before.
#[cfg(test)]
tokio::task_local! {
    pub(crate) static CONFIG_DIR_OVERRIDE: PathBuf;
}

/// What stopped a [`DesktopConfig::try_update_at`]: the file could not be read (or parsed), or the
/// changed config could not be written. Nothing was written in the first case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateError {
    Read(String),
    Write(String),
}

impl UpdateError {
    /// The message, as [`DesktopConfig::update_at`] returns it.
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::Read(message) | Self::Write(message) => message,
        }
    }
}

/// A copy of the config that a flow keeps to READ across slow work (task 1889, fix round 1 M2). Add to Finder waits
/// many seconds on the File Provider and the config-write lock must not be held that long, so the Windows and Linux
/// install, Repair and sync-root flows load a copy at the start and read it as they go. The copy can be read (it
/// derefs to [`DesktopConfig`]) and it can only be changed through [`Self::update_held`], which applies the change to the
/// file as it is NOW and then makes the copy equal to what a load of the file returns. There is no `DerefMut`: an edit
/// to the copy outside the closure does not compile, so it cannot be silently dropped by the refresh, and the copy can
/// never be what is written.
#[derive(Debug, Clone)]
pub(crate) struct HeldConfig(DesktopConfig);

impl std::ops::Deref for HeldConfig {
    type Target = DesktopConfig;

    fn deref(&self) -> &DesktopConfig {
        &self.0
    }
}

impl From<DesktopConfig> for HeldConfig {
    fn from(cfg: DesktopConfig) -> Self {
        Self(cfg)
    }
}

impl HeldConfig {
    /// [`DesktopConfig::load`], as a copy to hold.
    pub(crate) fn load() -> Result<Self, String> {
        DesktopConfig::load().map(Self)
    }

    /// Writes through the file, never through the copy. `change` is applied to the file as it is now
    /// ([`DesktopConfig::update_at`]), so what the copy loaded at the start cannot replace what another writer saved
    /// since (task 1889: the install and Repair commands used to save their copy and could drop the kept-folder record
    /// a sweep saved meanwhile). Afterwards the copy equals what a load of the file returns: the file as it was read
    /// plus `change`'s edits if it said to save, and the file as it was read, unedited, if it said not to (an edit that
    /// is not saved is not kept; M3), so the flow reads its own write and everyone else's.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn update_held<R>(&mut self, change: impl FnOnce(&mut DesktopConfig) -> (bool, R)) -> Result<R, String> {
        self.update_held_at(&DesktopConfig::path()?, change)
    }

    /// [`Self::update_held`] on the config file at `path` (a seam for the tests).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn update_held_at<R>(
        &mut self,
        path: &Path,
        change: impl FnOnce(&mut DesktopConfig) -> (bool, R),
    ) -> Result<R, String> {
        let (fresh, value) = DesktopConfig::update_at(path, |cfg| {
            let read = cfg.clone();
            let (save, value) = change(cfg);
            let on_file = if save { cfg.clone() } else { read };
            (save, (on_file, value))
        })?;
        self.0 = fresh;
        Ok(value)
    }
}

impl DesktopConfig {
    /// Resolve the absolute path to `desktop.toml` for the current user,
    /// creating the parent directory if missing. Errors propagate as
    /// strings so they can flow through Tauri commands.
    pub fn path() -> Result<PathBuf, String> {
        let base = Self::config_base_dir()?;
        let dir = base.join(APP_CONFIG_DIR);
        fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
        Ok(dir.join(CONFIG_FILENAME))
    }

    /// The directory that holds the `beebeeb` config folder: the user's config dir.
    #[cfg(not(test))]
    fn config_base_dir() -> Result<PathBuf, String> {
        dirs::config_dir().ok_or_else(|| "could not determine user config directory".to_string())
    }

    /// Unit tests never see the person's real config. In a test build this is a per-process
    /// sandbox next to the test binary (`crate::test_sandbox`), so a test that calls `load()`,
    /// `update()` or `ensure_account_id()` (directly, or through a command it runs) reads and
    /// writes the sandbox, not `~/Library/Application Support/beebeeb/desktop.toml`. The guard
    /// test `unit_tests_resolve_the_config_path_in_a_sandbox_never_the_real_one` pins this.
    #[cfg(test)]
    fn config_base_dir() -> Result<PathBuf, String> {
        if let Ok(dir) = CONFIG_DIR_OVERRIDE.try_with(PathBuf::clone) {
            return Ok(dir);
        }
        crate::test_sandbox::dir("config")
    }

    /// Load the on-disk config. A missing file is normal on first
    /// launch and yields `DesktopConfig::default()` rather than an
    /// error. A corrupt or unparseable file IS an error — surfacing
    /// it loud avoids silently overwriting a user's settings on save.
    pub fn load() -> Result<Self, String> {
        Self::load_from(&Self::path()?)
    }

    /// [`Self::load`] from `path`. A seam so a test can run the real load against its own file.
    pub(crate) fn load_from(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        // toml 0.8 removed from_slice; read as UTF-8 string and parse.
        let s = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
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
        cfg.scrub_legacy_finder_error(cfg!(target_os = "macos"));
        Ok(cfg)
    }

    /// Forget the old `finder_install_last_error` on macOS (lead ruling T4-3). 0.8.11 saved the
    /// OS's own message there, and that prose can name a file or a folder. macOS no longer
    /// writes the `finder_install_*` keys (it reads Finder's state from the OS, spec §9), so
    /// nothing there needs the text and the next update drops it from disk. The other old
    /// keys carry no path: they still load and are kept. Windows and Linux keep reading and
    /// writing the error unchanged. `on_macos` is a parameter so tests exercise both branches
    /// on any host; `load_from` passes `cfg!(target_os = "macos")`.
    fn scrub_legacy_finder_error(&mut self, on_macos: bool) {
        if on_macos {
            self.finder_install_last_error = None;
        }
    }

    /// Return this install's account id, minting + persisting one on first use.
    ///
    /// If `account_id` is already set, returns it unchanged WITHOUT re-saving
    /// (idempotent: a relaunch must not rewrite the file or change the id). If
    /// it is `None` (fresh install, or a pre-Phase-1 config that predates the
    /// key), mints a fresh UUID v4, sets it, and persists immediately via the
    /// atomic save BEFORE returning.
    ///
    /// ★ INVARIANT ★ The id is on disk before this returns, so the caller can
    /// safely segment keychain secrets under it (see the load-bearing note on
    /// the `account_id` field). Persisting first closes the Phase-0
    /// orphan-on-relaunch hole: a crash after this returns still finds the same
    /// id on the next launch, so the secrets written under it are recoverable.
    ///
    /// Task 1889: the read, the mint and the write are one [`Self::update_at`], so two callers at once
    /// cannot mint two ids, and the mint cannot replace a setting another writer saved meanwhile.
    pub fn ensure_account_id() -> Result<String, String> {
        Self::ensure_account_id_at(&Self::path()?)
    }

    /// [`Self::ensure_account_id`] against an explicit file, so a test can run the real read, mint
    /// and write in a temp dir instead of the person's `desktop.toml`.
    pub(crate) fn ensure_account_id_at(path: &Path) -> Result<String, String> {
        Self::update_at(path, |cfg| match &cfg.account_id {
            Some(id) => (false, id.clone()),
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                cfg.account_id = Some(id.clone());
                (true, id)
            }
        })
    }

    /// Load the config, let `change` modify it, and save it if `change` says so, all under the
    /// config-write lock ([`Self::update_at`] on [`Self::path`]). Every writer of `desktop.toml`
    /// outside this file goes through here (task 1889). `change` returns `(save, value)`.
    pub(crate) fn update<R>(change: impl FnOnce(&mut Self) -> (bool, R)) -> Result<R, String> {
        Self::update_at(&Self::path()?, change)
    }

    /// Load the config at `path`, let `change` modify it, and save it if `change` says so, all
    /// under the config-write lock, so no other update lands between the load and the save
    /// (task 1882 r5, task 1889). `change` returns `(save, value)`: whether the config changed, and
    /// what `update_at` hands back. A change that is not saved leaves the file untouched. Callers
    /// pass [`Self::path`]; a test passes its own file, so it runs the real lock, load and save.
    ///
    /// For a short, synchronous change: the lock is held while `change` runs, and it is not
    /// re-entrant, so `change` calls no other update.
    pub(crate) fn update_at<R>(path: &Path, change: impl FnOnce(&mut Self) -> (bool, R)) -> Result<R, String> {
        Self::try_update_at(path, change).map_err(UpdateError::into_message)
    }

    /// [`Self::update_at`] that says whether the read or the write failed, for a caller whose log
    /// line names which. The messages are the same strings `update_at` returns.
    pub(crate) fn try_update_at<R>(path: &Path, change: impl FnOnce(&mut Self) -> (bool, R)) -> Result<R, UpdateError> {
        let _writing = config_write_guard();
        let mut cfg = Self::load_from(path).map_err(UpdateError::Read)?;
        let (save, value) = change(&mut cfg);
        if save {
            cfg.save_to(path).map_err(UpdateError::Write)?;
        }
        Ok(value)
    }

    /// The write itself. The caller holds the config-write lock.
    fn save_to(&self, path: &Path) -> Result<(), String> {
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

        fs::rename(&tmp, path).map_err(|e| format!("rename {} → {}: {e}", tmp.display(), path.display()))?;
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
        DesktopConfig, DesktopSettings, DesktopTheme, HeldConfig, ReleaseChannel, UpdateError,
        default_sync_root_suggestion,
    };
    use std::path::Path;

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
    fn last_signed_in_email_round_trips_and_stays_absent_until_set() {
        // Same byte-identical guarantee as account_id: a config that never
        // signed in must not grow a `last_signed_in_email` key, and older
        // configs missing the key must load as `None` (serde default).
        let cfg = DesktopConfig::default();
        let toml = toml::to_string_pretty(&cfg).expect("serialize default");
        assert!(
            !toml.contains("last_signed_in_email"),
            "last_signed_in_email must be omitted until an email is remembered, got:\n{toml}"
        );

        // A pre-field config (no key) loads as None — backward compatible.
        let legacy: DesktopConfig = toml::from_str("sync_root = \"/tmp/x\"\n").expect("parse legacy");
        assert_eq!(legacy.last_signed_in_email, None);

        // Once set, the key appears and round-trips losslessly.
        let mut cfg = DesktopConfig::default();
        cfg.last_signed_in_email = Some("user@example.com".to_string());
        let toml = toml::to_string_pretty(&cfg).expect("serialize with email");
        assert!(toml.contains("last_signed_in_email"));
        let back: DesktopConfig = toml::from_str(&toml).expect("parse");
        assert_eq!(back.last_signed_in_email.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn ensure_account_id_is_idempotent_when_already_set() {
        // Pre-set id → returned unchanged, never re-minted, and the file is not rewritten
        // (the change says "nothing to save"), on a file in a temp dir.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        DesktopConfig::update_at(&path, |cfg| {
            cfg.account_id = Some("fixed-id".to_string());
            (true, ())
        })
        .expect("seed an id");
        let before = std::fs::read(&path).expect("read seeded config");

        let returned = DesktopConfig::ensure_account_id_at(&path).expect("idempotent path returns Ok");
        assert_eq!(returned, "fixed-id");

        // Calling again still returns the same id, still unchanged on disk.
        let again = DesktopConfig::ensure_account_id_at(&path).expect("second call returns Ok");
        assert_eq!(again, "fixed-id");
        assert_eq!(
            std::fs::read(&path).expect("read again"),
            before,
            "an id already set is not re-saved"
        );
    }

    #[test]
    fn ensure_account_id_mints_persists_and_is_stable_across_relaunch() {
        // The real mint + persist + readback path, against a file in a temp dir. (This used to
        // snapshot, delete and restore the person's real `desktop.toml`; a crash mid-test would
        // have left it deleted. No test may touch the real file, so it runs on a temp path.)
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");

        // Mint: a fresh config has no id; ensure_account_id mints + persists one.
        let cfg = DesktopConfig::load_from(&path).expect("load (missing → default)");
        assert!(cfg.account_id.is_none(), "fresh config must start without an id");
        let minted = DesktopConfig::ensure_account_id_at(&path).expect("ensure_account_id should mint");
        assert!(uuid::Uuid::parse_str(&minted).is_ok(), "minted id is a valid uuid");
        assert!(path.exists(), "the id is on disk before ensure_account_id returns");

        // Persist: a fresh load from disk (simulating relaunch) sees the SAME id.
        let reloaded = DesktopConfig::load_from(&path).expect("reload after persist");
        assert_eq!(
            reloaded.account_id.as_deref(),
            Some(minted.as_str()),
            "persisted id must survive a reload"
        );

        // Idempotent across relaunch: ensure_account_id returns the same id (no re-mint) and does
        // not rewrite the file.
        let before = std::fs::read(&path).expect("read saved config");
        let again = DesktopConfig::ensure_account_id_at(&path).expect("idempotent on reload");
        assert_eq!(again, minted, "relaunch must keep the same account id");
        assert_eq!(
            std::fs::read(&path).expect("read again"),
            before,
            "an id already set is not re-saved"
        );
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
        let cfg: DesktopConfig = toml::from_str("release_channel = \"alpha\"").expect("parse legacy channel config");
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

    // ── Task 1882 r5 — one lock around every load-modify-save ───────────────

    #[test]
    fn test_1882_r5_two_updates_at_once_both_land_and_an_unsaved_change_writes_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let slow_path = path.clone();
        let slow = std::thread::spawn(move || {
            DesktopConfig::update_at(&slow_path, |cfg| {
                cfg.kept_unsynced_folder = Some("/Users/someone/Kept".to_string());
                entered_tx.send(()).expect("the test is waiting");
                std::thread::sleep(std::time::Duration::from_millis(200));
                (true, "slow")
            })
        });
        entered_rx.recv().expect("the slow update started");
        // This one starts while the slow one is between its load and its save.
        let fast = DesktopConfig::update_at(&path, |cfg| {
            cfg.last_signed_in_email = Some("someone@beebeeb.io".to_string());
            (true, "fast")
        });
        assert_eq!(fast, Ok("fast"));
        assert_eq!(slow.join().expect("no panic"), Ok("slow"));
        let on_disk = DesktopConfig::load_from(&path).expect("reads back");
        assert_eq!(on_disk.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept"));
        assert_eq!(on_disk.last_signed_in_email.as_deref(), Some("someone@beebeeb.io"));

        // A change that says "nothing to save" leaves the file byte for byte as it was.
        let written = std::fs::read(&path).expect("exists");
        let untouched = DesktopConfig::update_at(&path, |cfg| {
            cfg.kept_unsynced_folder = None;
            (false, 7)
        });
        assert_eq!(untouched, Ok(7));
        assert_eq!(std::fs::read(&path).expect("exists"), written);
    }

    #[test]
    fn test_1882_r5_an_update_of_a_corrupt_config_fails_and_does_not_overwrite_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        std::fs::write(&path, "this is = not [valid toml").expect("write");
        let result = DesktopConfig::update_at(&path, |_| (true, ()));
        assert!(result.is_err(), "a corrupt config is an error, never overwritten");
        assert_eq!(
            std::fs::read_to_string(&path).expect("exists"),
            "this is = not [valid toml"
        );
    }

    // ── Task 1889 — every load-modify-save runs as one serialized update ─────────────────

    const KEPT: &str = "/Users/someone/Kept";

    /// Runs a slow `update_at` that sets the kept folder and then stays between its load and its save for 200 ms, and,
    /// once it is inside, `fast` on the same file. Returns after both finished, with `fast`'s answer.
    fn overlapped_with_a_slow_update<R>(path: &Path, fast: impl FnOnce(&Path) -> R) -> R {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let slow_path = path.to_path_buf();
        let slow = std::thread::spawn(move || {
            DesktopConfig::update_at(&slow_path, |cfg| {
                cfg.kept_unsynced_folder = Some(KEPT.to_string());
                entered_tx.send(()).expect("the test is waiting");
                std::thread::sleep(std::time::Duration::from_millis(200));
                (true, ())
            })
        });
        entered_rx.recv().expect("the slow update started");
        // `fast` starts while the slow one is between its load and its save.
        let answer = fast(path);
        assert_eq!(slow.join().expect("no panic"), Ok(()));
        answer
    }

    /// The point of the task: whichever helper a writer uses, a change made while another writer is between its load and
    /// its save lands next to that writer's, and neither replaces the other. One field each, so a lost update shows as
    /// the missing field. Without the lock the fast helper's field is gone: the slow update saves the copy it loaded.
    #[test]
    fn test_1889_two_updates_of_different_fields_both_survive_through_every_helper() {
        type Fast = fn(&Path);
        type Landed = fn(&DesktopConfig) -> bool;
        let helpers: [(&str, Fast, Landed); 4] = [
            (
                "update_at",
                |path| {
                    DesktopConfig::update_at(path, |cfg| {
                        cfg.last_signed_in_email = Some("someone@beebeeb.io".to_string());
                        (true, ())
                    })
                    .expect("fast update");
                },
                |cfg| cfg.last_signed_in_email.as_deref() == Some("someone@beebeeb.io"),
            ),
            (
                "update_held_at",
                |path| {
                    // The copy was loaded before the slow update began, as an install flow's is.
                    let mut held = HeldConfig::from(DesktopConfig::default());
                    held.update_held_at(path, |held| {
                        held.pause_sync = true;
                        (true, ())
                    })
                    .expect("fast held update");
                },
                |cfg| cfg.pause_sync,
            ),
            (
                "ensure_account_id_at",
                |path| {
                    DesktopConfig::ensure_account_id_at(path).expect("fast mint");
                },
                |cfg| cfg.account_id.is_some(),
            ),
            (
                "try_update_at",
                |path| {
                    DesktopConfig::try_update_at(path, |cfg| {
                        cfg.sync_mode = Some("smart".to_string());
                        (true, ())
                    })
                    .expect("fast try-update");
                },
                |cfg| cfg.sync_mode.as_deref() == Some("smart"),
            ),
        ];
        let mut lost = Vec::new();
        for (name, fast, landed) in helpers {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("desktop.toml");
            overlapped_with_a_slow_update(&path, fast);
            let on_disk = DesktopConfig::load_from(&path).expect("reads back");
            if on_disk.kept_unsynced_folder.as_deref() != Some(KEPT) {
                lost.push(format!("{name}: the slow update's change is gone"));
            }
            if !landed(&on_disk) {
                lost.push(format!("{name}: the overlapping update's change is gone"));
            }
        }
        assert!(lost.is_empty(), "changes lost to a stale save: {lost:?}");
    }

    /// The exact bug of the ticket (1882 round 5): an install or Repair flow loads its copy at the start, the app-start
    /// sweep saves a kept-folder record meanwhile, and the flow then writes its own result. The flow's write used to put
    /// back the copy it loaded, and the record was gone.
    #[test]
    fn test_1889_a_held_copy_never_puts_back_what_another_writer_saved_since_it_loaded() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let mut held = HeldConfig::from(DesktopConfig::load_from(&path).expect("the flow loads its copy first"));
        assert_eq!(held.kept_unsynced_folder, None);

        // The sweep saves the kept folder while the flow waits on the File Provider.
        DesktopConfig::update_at(&path, |cfg| {
            cfg.kept_unsynced_folder = Some(KEPT.to_string());
            (true, ())
        })
        .expect("the sweep saves");

        // The flow writes its result. Its copy is older than the file.
        let state = held
            .update_held_at(&path, |held| {
                held.finder_install_status = Some("error".to_string());
                held.finder_install_reason_category = Some("timeout".to_string());
                (true, held.finder_install_status.clone())
            })
            .expect("the flow's write");
        assert_eq!(state.as_deref(), Some("error"), "the closure's answer comes back");

        let on_disk = DesktopConfig::load_from(&path).expect("reads back");
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "the sweep's record survived"
        );
        assert_eq!(on_disk.finder_install_status.as_deref(), Some("error"));
        assert_eq!(on_disk.finder_install_reason_category.as_deref(), Some("timeout"));
        assert_eq!(
            held.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "afterwards the copy reads the file, not the load it started with"
        );
        assert_eq!(held.finder_install_status.as_deref(), Some("error"));
    }

    #[test]
    fn test_1889_a_held_update_that_says_nothing_changed_writes_nothing_and_a_corrupt_file_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        DesktopConfig::update_at(&path, |cfg| {
            cfg.pause_sync = true;
            (true, ())
        })
        .expect("seed");
        let written = std::fs::read(&path).expect("exists");
        let mut held = HeldConfig::from(DesktopConfig::default());
        let answer = held
            .update_held_at(&path, |held| {
                held.pause_sync = false; // changed in memory, but the closure says: do not save
                (false, 7)
            })
            .expect("no write, no error");
        assert_eq!(answer, 7);
        assert_eq!(
            std::fs::read(&path).expect("exists"),
            written,
            "the file is byte for byte as it was"
        );
        // Fix round 1 (M3): the copy is what the file holds, not the change that was not saved.
        assert!(
            held.pause_sync,
            "the copy reads the file (pause_sync = true), not the unsaved change"
        );
        assert_eq!(
            toml::to_string_pretty(&*held).expect("serialize the copy"),
            toml::to_string_pretty(&DesktopConfig::load_from(&path).expect("reads back")).expect("serialize the file"),
            "after a no-save update the copy equals what a load of the file returns"
        );

        // A file that cannot be read is an error and is left alone, and the held copy is not touched.
        std::fs::write(&path, "this is = not [valid toml").expect("write");
        let mut held = HeldConfig::from(DesktopConfig {
            last_signed_in_email: Some("kept@beebeeb.io".to_string()),
            ..DesktopConfig::default()
        });
        let result = held.update_held_at(&path, |held| {
            held.pause_sync = true;
            (true, ())
        });
        assert!(result.is_err(), "a corrupt config is an error");
        assert_eq!(
            std::fs::read_to_string(&path).expect("exists"),
            "this is = not [valid toml"
        );
        assert_eq!(held.last_signed_in_email.as_deref(), Some("kept@beebeeb.io"));
        assert!(!held.pause_sync, "a failed update leaves the copy as it was");
    }

    /// The sign-out of an account switch logs a read that failed and a write that failed apart.
    #[test]
    fn test_1889_try_update_at_says_whether_the_read_or_the_write_failed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        std::fs::write(&path, "this is = not [valid toml").expect("write");
        let read = DesktopConfig::try_update_at(&path, |_| (true, ()));
        assert!(matches!(read, Err(UpdateError::Read(_))), "{read:?}");

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        // A save writes `desktop.toml.tmp` first: a directory in its place fails the write.
        std::fs::create_dir(path.with_extension("toml.tmp")).expect("mkdir");
        let write = DesktopConfig::try_update_at(&path, |cfg| {
            cfg.pause_sync = true;
            (true, ())
        });
        assert!(matches!(write, Err(UpdateError::Write(_))), "{write:?}");
        assert!(!path.exists(), "nothing was written");
        // `update_at` says the same thing as a string.
        let as_string = DesktopConfig::update_at(&path, |_| (true, ()));
        assert_eq!(as_string, Err(write.expect_err("failed").into_message()));
    }

    // ── Spec 2026-10-06 (macOS Finder setup reconciler) §9 ─────────────────

    #[test]
    fn a_0_8_11_desktop_toml_with_the_old_finder_keys_still_loads() {
        let cfg: DesktopConfig =
            toml::from_str(include_str!("../tests/fixtures/desktop-0.8.11.toml")).expect("a 0.8.11 config parses");
        // The old keys still load: Windows and Linux keep using them; macOS ignores them.
        assert_eq!(cfg.finder_install_status.as_deref(), Some("error"));
        assert_eq!(cfg.finder_install_reason_category.as_deref(), Some("unknown"));
        assert_eq!(cfg.finder_last_failure, None);
        assert!(!cfg.finder_signed_out_by_choice);
        let back = toml::to_string_pretty(&cfg).expect("serialize");
        assert!(!back.contains("finder_last_failure"), "None is not written:\n{back}");
        assert!(
            !back.contains("finder_signed_out_by_choice"),
            "false is not written:\n{back}"
        );
    }

    #[test]
    fn the_new_finder_keys_round_trip() {
        use crate::finder_setup::error::FailureRecord;
        use crate::surfaces::phase::FinderFailureReason;
        // A struct literal, not field assignments after `default()`: clippy's
        // `field_reassign_with_default` would add a warning over the baseline.
        let cfg = DesktopConfig {
            finder_signed_out_by_choice: true,
            finder_last_failure: Some(FailureRecord {
                reason: FinderFailureReason::FolderTaken,
                domain: "NSCocoaErrorDomain".into(),
                code: 516,
                at: 1_791_291_909,
            }),
            ..DesktopConfig::default()
        };
        let text = toml::to_string_pretty(&cfg).expect("serialize");
        assert!(text.contains("reason = \"folder_taken\""), "{text}");
        let back: DesktopConfig = toml::from_str(&text).expect("parse");
        assert_eq!(back.finder_last_failure, cfg.finder_last_failure);
        assert!(back.finder_signed_out_by_choice);
    }

    /// Forward compatibility (area A M3): a later build can add a failure reason and write it, and a downgrade
    /// (`install_channel_downgrade`) then runs this build on that file. An unknown reason reads as `Unknown`; it never
    /// makes the whole `desktop.toml` unreadable for every caller.
    #[test]
    fn a_failure_reason_from_a_later_build_reads_as_unknown_and_the_rest_still_loads() {
        use crate::surfaces::phase::FinderFailureReason;
        let text = r#"
sync_root = "/tmp/bb-later-build"
finder_signed_out_by_choice = true

[finder_last_failure]
reason = "something_new"
domain = "NSFileProviderErrorDomain"
code = -2099
at = 1791291909
"#;
        let cfg: DesktopConfig = toml::from_str(text).expect("a later build's reason never breaks the file");
        let failure = cfg.finder_last_failure.expect("the record is kept");
        assert_eq!(failure.reason, FinderFailureReason::Unknown);
        assert_eq!(
            (failure.domain.as_str(), failure.code),
            ("NSFileProviderErrorDomain", -2099)
        );
        assert!(cfg.finder_signed_out_by_choice, "and the rest of the file still loads");
    }

    // ── Lead ruling T4-3: the old, unredacted Finder error is scrubbed on macOS ─────────
    //
    // A 0.8.11 `desktop.toml` can hold `finder_install_last_error` = the OS's own message, which
    // can name a file or folder. macOS no longer writes it, so loading drops it and the next save
    // removes it from disk. Windows/Linux still read and write it. The platform is a parameter of
    // `scrub_legacy_finder_error` so both branches run on any host; the two file tests below are
    // `cfg`-split and run the real load/save against a temp dir on whichever host builds them.
    // None of these tests calls `DesktopConfig::load`/`save`/`path`, so none can reach the
    // person's real `desktop.toml`.

    const FIXTURE_ERROR: &str =
        "The file couldn\u{2019}t be saved because a file with the same name already exists. (NSCocoaErrorDomain 516)";
    const PATH_ERROR: &str =
        "The file \u{201c}/Users/sam/Secret Folder/tax.pdf\u{201d} couldn\u{2019}t be saved. (NSCocoaErrorDomain 516)";

    /// The 0.8.11 fixture with a path in the old error key (the fixture itself carries none).
    fn fixture_with_a_path_in_the_old_error() -> String {
        let fixture = include_str!("../tests/fixtures/desktop-0.8.11.toml");
        let with_path = fixture.replace(FIXTURE_ERROR, PATH_ERROR);
        assert_ne!(
            with_path, fixture,
            "the fixture's error line changed: update FIXTURE_ERROR"
        );
        with_path
    }

    #[test]
    fn the_legacy_finder_error_is_scrubbed_on_macos_and_kept_elsewhere() {
        let text = fixture_with_a_path_in_the_old_error();
        let on_macos: DesktopConfig = toml::from_str(&text).expect("parse");
        assert_eq!(
            on_macos.finder_install_last_error.as_deref(),
            Some(PATH_ERROR),
            "precondition"
        );
        let mut elsewhere = on_macos.clone();
        let mut on_macos = on_macos;

        on_macos.scrub_legacy_finder_error(true);
        elsewhere.scrub_legacy_finder_error(false);

        assert_eq!(on_macos.finder_install_last_error, None);
        assert_eq!(elsewhere.finder_install_last_error.as_deref(), Some(PATH_ERROR));
        // Only the one field that can carry a path goes; the other old keys still load.
        assert_eq!(on_macos.finder_install_status.as_deref(), Some("error"));
        assert_eq!(on_macos.finder_install_last_attempt_at, Some(1_791_291_909));
        assert_eq!(on_macos.finder_install_reason_category.as_deref(), Some("unknown"));
        assert_eq!(on_macos.sync_root, elsewhere.sync_root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn on_macos_loading_a_0_8_11_file_drops_the_old_error_and_the_next_save_removes_it_from_disk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        std::fs::write(&path, fixture_with_a_path_in_the_old_error()).expect("write fixture");

        let cfg = DesktopConfig::load_from(&path).expect("a 0.8.11 file loads");
        assert_eq!(cfg.finder_install_last_error, None);
        assert_eq!(
            cfg.finder_install_status.as_deref(),
            Some("error"),
            "the other old keys still load"
        );
        // Loading never rewrites the file (it runs from many threads): the path is on disk until a save.
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("/Users/sam/Secret Folder")
        );

        cfg.save_to(&path).expect("save");
        let saved = std::fs::read_to_string(&path).unwrap();
        for leaked in ["/Users/sam/Secret", "tax.pdf", "finder_install_last_error"] {
            assert!(
                !saved.contains(leaked),
                "the saved file still holds {leaked:?}:\n{saved}"
            );
        }
        assert!(
            saved.contains("finder_install_status"),
            "only the error key is dropped:\n{saved}"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_macos_a_0_8_11_file_with_the_old_error_round_trips_unchanged() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        std::fs::write(&path, fixture_with_a_path_in_the_old_error()).expect("write fixture");

        let cfg = DesktopConfig::load_from(&path).expect("a 0.8.11 file loads");
        assert_eq!(cfg.finder_install_last_error.as_deref(), Some(PATH_ERROR));

        cfg.save_to(&path).expect("save");
        let back = DesktopConfig::load_from(&path).expect("reload");
        assert_eq!(back.finder_install_last_error.as_deref(), Some(PATH_ERROR));
        assert_eq!(back.finder_install_status.as_deref(), Some("error"));
    }

    // ── No unit test may reach the person's real desktop.toml ────────────────────────────

    #[test]
    fn unit_tests_resolve_the_config_path_in_a_sandbox_never_the_real_one() {
        // `DesktopConfig::path()` is what `load()`, `update()` and `ensure_account_id()` resolve.
        // Under `cfg(test)` it must point inside a per-process sandbox next to the test binary,
        // so a test that calls `load()`/`update()` (directly, or through a command it exercises)
        // cannot read, delete or rewrite the real `desktop.toml` in the person's config dir.
        // Only paths are computed here; nothing is read.
        let sandboxed = DesktopConfig::path().expect("config path resolves");
        let real_dir = dirs::config_dir().expect("a user config dir exists on a dev machine or CI runner");
        let real = real_dir.join(super::APP_CONFIG_DIR).join(super::CONFIG_FILENAME);

        assert_ne!(sandboxed, real, "unit tests resolved the REAL desktop.toml");
        // The specific real `beebeeb` config folder, not the whole config dir: a `CARGO_TARGET_DIR`
        // under `~/Library/Application Support` (or `~/.config`) puts the sandbox inside the config
        // dir legitimately.
        let real_app_dir = real_dir.join(super::APP_CONFIG_DIR);
        assert!(
            !sandboxed.starts_with(&real_app_dir),
            "{sandboxed:?} is inside the real config folder {real_app_dir:?}"
        );
        let exe_dir = std::env::current_exe()
            .expect("test binary path")
            .parent()
            .expect("exe dir")
            .to_path_buf();
        assert!(
            sandboxed.starts_with(&exe_dir),
            "{sandboxed:?} is not inside the test binary's directory {exe_dir:?}"
        );
    }
}
