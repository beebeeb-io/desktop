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
/// change. There is no public `save`: the file is written in one place, `try_update_at_then`, which
/// takes this lock across the whole load, change and write. The ways in are
/// [`DesktopConfig::update_at`] (and [`DesktopConfig::update`], [`HeldConfig::update_held`] and
/// [`DesktopConfig::ensure_account_id`], which are `update_at`), [`DesktopConfig::try_update_at`],
/// [`DesktopConfig::update_at_then`], and [`DesktopConfig::update_at_durable_with`], the one save
/// that flushes its write (task 1887). The lock is not re-entrant: nothing inside an update closure
/// may call another update.
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

    /// `true` once the person has dismissed the Beebeeb window's one-time hint on Home, "Beebeeb lives in the menu bar"
    /// (two-surface spec §7.6, task 1683 slice 6). Defaults `false`: a fresh install shows it once, and so does an install
    /// that predates it (one tip, one tap, and it explains that the window is optional). Like
    /// `known_folder_onboarding_seen` it is not in `DesktopSettings`: a Settings save must never flip it.
    #[serde(default)]
    pub menu_bar_hint_seen: bool,

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
    /// Task 2173: `"api"` forces today's API transport for file downloads; absent or `"auto"`
    /// uses direct storage transfers when the server offers them. A support switch, edited by
    /// hand (a Mac app does not see a shell's `BB_TRANSPORT`, which wins when set). Not in
    /// `DesktopSettings`, so a settings save never touches it; absent from the file while unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,

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

    /// Spec 2026-10-10 W9 (task 1887): the folder a sign-out last copied changes that had not reached the server into.
    /// Settings › Sync shows it until it is dismissed. Saved by the keep step before the purge; never in
    /// `DesktopSettings`, so a settings save cannot clear it; never logged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_unsent_folder: Option<String>,

    /// Spec 2026-10-10 W13 (task 1887): every folder whose sign-out was interrupted after its keep step, oldest first.
    /// Saved with `kept_unsent_folder` in one durable write before the purge's first deletion; a later keep adds to
    /// it; only a purge that cleared the rows clears it. Absent from the file while empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept_unsent_unfinished: Vec<String>,

    /// Task 2190 (D1, review-169 blocker): every copy "Keep the other" kept of a Mac's changes in the
    /// Changes not sent folder, oldest first. Settings › Sync lists each one, with "Show in Finder",
    /// until the person dismisses it. Its own list: never `kept_unsent_folder` (one slot, the
    /// sign-out's) nor `kept_unsent_unfinished`, so a decision never hides a sign-out's folder and a
    /// sign-out never mistakes a decision's copy for an unfinished one. Never logged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept_conflict_copies: Vec<String>,

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
            menu_bar_hint_seen: false,
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
            transport: None,
            release_channel: ReleaseChannel::Stable,
            installed_release_channel: None,
            kept_unsynced_folder: None,
            kept_unsent_folder: None,
            kept_unsent_unfinished: Vec::new(),
            kept_conflict_copies: Vec::new(),
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
/// changed config could not be written. Nothing was written in the first case. A durable save's
/// `Write` can also come after its rename, when the directory flush failed: the change is then in
/// the file, but not flushed ([`DesktopConfig::update_at_durable_with`]).
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

/// A record that its own write failed and that rides the flow's next write instead (fix round 1, M4): the change to
/// apply to the file, and its argument. It returns whether it changed the config.
type QueuedRecord = (fn(&mut DesktopConfig, &str) -> bool, String);

/// A copy of the config that a flow keeps to READ across slow work (task 1889, fix round 1 M2). Add to Finder waits
/// many seconds on the File Provider and the config-write lock must not be held that long, so the Windows and Linux
/// install, Repair and sync-root flows load a copy at the start and read it as they go. The copy can be read (it
/// derefs to [`DesktopConfig`]) and it can only be changed through [`Self::update_held`], which applies the change to the
/// file as it is NOW and then makes the copy equal to what a load of the file returns. There is no `DerefMut`: an edit
/// to the copy outside the closure does not compile, so it cannot be silently dropped by the refresh, and the copy can
/// never be what is written.
#[derive(Debug, Clone)]
pub(crate) struct HeldConfig {
    copy: DesktopConfig,
    queued: Vec<QueuedRecord>,
}

impl std::ops::Deref for HeldConfig {
    type Target = DesktopConfig;

    fn deref(&self) -> &DesktopConfig {
        &self.copy
    }
}

impl From<DesktopConfig> for HeldConfig {
    fn from(copy: DesktopConfig) -> Self {
        Self {
            copy,
            queued: Vec::new(),
        }
    }
}

impl HeldConfig {
    /// [`DesktopConfig::load`], as a copy to hold.
    pub(crate) fn load() -> Result<Self, String> {
        DesktopConfig::load().map(Self::from)
    }

    /// Writes through the file, never through the copy. `change` is applied to the file as it is now
    /// ([`DesktopConfig::update_at`]), so what the copy loaded at the start cannot replace what another writer saved
    /// since (task 1889: the install and Repair commands used to save their copy and could drop the kept-folder record
    /// a sweep saved meanwhile). Afterwards the copy equals what a load of the file returns: the file as it was read
    /// plus `change`'s edits if it said to save, and the file as it was read, unedited, if it said not to (an edit that
    /// is not saved is not kept; M3), so the flow reads its own write and everyone else's.
    ///
    /// Records queued by [`Self::queue_for_next_write`] are applied first, in the same write; they make a write that
    /// says "nothing changed" a write when they changed something.
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
        let queued = self.queued.clone();
        let (fresh, value) = DesktopConfig::update_at(path, |cfg| {
            let mut queued_changed = false;
            for (apply, argument) in &queued {
                queued_changed |= apply(cfg, argument);
            }
            let with_queued = cfg.clone();
            let (save, value) = change(cfg);
            if save {
                (true, (cfg.clone(), value))
            } else {
                // An edit that is not saved is not kept (M3): the file as it was read, plus what was queued.
                *cfg = with_queued.clone();
                (queued_changed, (with_queued, value))
            }
        })?;
        self.copy = fresh;
        self.queued.clear();
        Ok(value)
    }

    /// A record whose own write failed: it is applied to the file by this copy's next [`Self::update_held`], so it lands
    /// with the flow's own result or the flow gets that write's error (M4). Queuing the same record twice is one.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn queue_for_next_write(&mut self, apply: fn(&mut DesktopConfig, &str) -> bool, argument: &str) {
        if !self
            .queued
            .iter()
            .any(|(queued, arg)| std::ptr::fn_addr_eq(*queued, apply) && arg == argument)
        {
            self.queued.push((apply, argument.to_string()));
        }
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
        Self::try_update_at_then(path, change, |_| {}, None)
    }

    /// [`Self::update_at`] with a step that runs while the lock is STILL held, once the file is final: after a successful
    /// write, or at once when `change` said not to save (then `saved` sees the file as it was read, not `change`'s
    /// unsaved edit). For state that mirrors the file and must change in the same order as the file does, such as the
    /// in-memory pause flag (task 1889 fix 1, M6): the mirror is never ahead of a write that failed and never behind
    /// one that completed. `saved` does no I/O and takes no lock.
    pub(crate) fn update_at_then<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> (bool, R),
        saved: impl FnOnce(&Self),
    ) -> Result<R, String> {
        Self::try_update_at_then(path, change, saved, None).map_err(UpdateError::into_message)
    }

    /// Spec 2026-10-10 §7.3 (task 1887): `update_at`, whose write survives a power loss once it returns: the temp file
    /// is flushed before the rename, and the directory after it. Only the keep step's save uses it, with
    /// `RealConfigFs`; every other save stays as it is. The file-system steps are passed in so a test can record their
    /// order and fail one (U43). It is the one write place with a durable write step, so the lock and the read are
    /// still taken in one place. An `Err` can come after the rename, when the directory flush failed: the change is
    /// then in the file, so a caller that undoes its work on an error must take the change back (spec 2026-10-10 W8).
    pub(crate) fn update_at_durable_with<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> (bool, R),
        fs_steps: &dyn ConfigFs,
    ) -> Result<R, String> {
        Self::try_update_at_then(path, change, |_| {}, Some(fs_steps)).map_err(UpdateError::into_message)
    }

    /// The one place the file is written: the lock is taken before the read and held through the write and `saved`.
    /// `durable`: `None` is the plain write every save uses; `Some` is the keep step's durable write (task 1887, spec
    /// 2026-10-10 §7.3), with its file-system steps.
    fn try_update_at_then<R>(
        path: &Path,
        change: impl FnOnce(&mut Self) -> (bool, R),
        saved: impl FnOnce(&Self),
        durable: Option<&dyn ConfigFs>,
    ) -> Result<R, UpdateError> {
        let _writing = config_write_guard();
        let mut cfg = Self::load_from(path).map_err(UpdateError::Read)?;
        let read = cfg.clone();
        let (save, value) = change(&mut cfg);
        if save {
            match durable {
                None => cfg.save_to(path),
                Some(fs_steps) => cfg.save_to_durable(path, fs_steps),
            }
            .map_err(UpdateError::Write)?;
            saved(&cfg);
        } else {
            saved(&read);
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

    /// The durable write. The caller holds the config-write lock.
    fn save_to_durable(&self, path: &Path, fs_steps: &dyn ConfigFs) -> Result<(), String> {
        let toml_str = toml::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        let tmp = path.with_extension("toml.tmp");
        fs_steps
            .write(&tmp, toml_str.as_bytes())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs_steps
            .sync_file(&tmp)
            .map_err(|e| format!("flush {}: {e}", tmp.display()))?;
        fs_steps
            .rename(&tmp, path)
            .map_err(|e| format!("rename {} → {}: {e}", tmp.display(), path.display()))?;
        let dir = path
            .parent()
            .ok_or_else(|| "the config file has no directory".to_string())?;
        fs_steps
            .sync_dir(dir)
            .map_err(|e| format!("flush {}: {e}", dir.display()))
    }
}

/// The file-system steps of a durable save, so a test can record their order and fail one (spec 2026-10-10 U43).
pub(crate) trait ConfigFs {
    /// Write `bytes` to `path`, owner-only.
    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;
    fn sync_file(&self, path: &Path) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()>;
}

// Outside tests, only the macOS sign-out builds one (spec 2026-10-10 W5).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct RealConfigFs;

impl ConfigFs for RealConfigFs {
    fn write(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        fs::write(path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
    fn sync_file(&self, path: &Path) -> std::io::Result<()> {
        fs::File::open(path)?.sync_all()
    }
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        fs::rename(from, to)
    }
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        fs::File::open(dir)?.sync_all()
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
mod tests;
