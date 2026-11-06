//! The persisted config data model: `DesktopConfig` (the on-disk TOML shape),
//! its serde field defaults, and the enums and cache-limit units the UI and
//! eviction read.
//!
//! The defaults (`default_true`, `default_local_cache_limit_bytes`) live here
//! because the derived deserializers call them by unqualified name.
//! Must not touch files: `paths` owns locations, `store` owns IO.
//! Tests: `crate::config::tests`.

use super::*;

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

    /// LD-0113 Amendment 9 (Guus, decision 2338, "Ask once"): for a known folder OneDrive's Known Folder Move has
    /// moved, which folder the person chose to back up (`onedrive`: where the files are; `default`: the default
    /// location). A moved folder with no answer here is not backed up until the person answers.
    #[serde(default)]
    pub known_folder_source: std::collections::BTreeMap<String, KnownFolderSource>,

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
pub(super) fn default_true() -> bool {
    true
}

pub(super) fn default_local_cache_limit_bytes() -> i64 {
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

/// Sync mode picked in the Windows first-run wizard. `desktop.toml` stores its
/// own string, so an unknown value there is an error, never a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigMode {
    Everything,
    Smart,
    Custom,
    OnlineOnly,
}

impl ConfigMode {
    /// The only way in from the stored string: the four values
    /// `WindowsFirstRun.tsx` sends, and nothing else.
    pub fn parse(raw: &str) -> Result<Self, ConfigError> {
        match raw {
            "everything" => Ok(Self::Everything),
            "smart" => Ok(Self::Smart),
            "custom" => Ok(Self::Custom),
            "online_only" => Ok(Self::OnlineOnly),
            other => Err(ConfigError::UnknownSyncMode(other.to_string())),
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
            known_folder_source: std::collections::BTreeMap::new(),
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

/// LD-0113 Amendment 9: which folder a moved known folder is backed up from (`known_folder_source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownFolderSource {
    /// OneDrive's folder, where Known Folder Move put the files. Only the files on this device are read.
    #[serde(rename = "onedrive")]
    OneDrive,
    /// The folder's default location (`KF_FLAG_DEFAULT_PATH`), as before the move.
    Default,
}
