# Desktop capability contract — 2026-09-29 (task 1611)

The shipping UI is desktop-owned React (`src/main.tsx`, compact `App.tsx`, main
`WindowsApp.tsx`). `?platform=windows` selects a layout even on a Mac; it cannot
authorize host actions. Rust `desktop_capabilities` supplies the actual host.
The active scheduler is desktop `runner.rs` / `engine_bridge.rs` / `state_db.rs`;
core supplies shared crypto/protocol primitives.

| Surface | Compact shell | Main shell | Commands |
| --- | --- | --- | --- |
| Trash | No page; native compact menu falls back to Finder | Trash | desktop_trash_list / desktop_trash_restore / desktop_trash_delete_permanently |
| History | Global dialog, Versions & conflicts | Global dialog, Files action | list_file_versions / restore_file_version / list_version_conflict_center |
| Activity | No timeline; native compact menu falls back to Status | Home summary, Activity; Windows tray recent list | account_activity / account_activity_feed / desktop_file_overview |
| Notifications | Local preferences | Activity inbox, Settings preferences | account_notifications (read-only inbox; no mark-read handler registered) |
| Recipient shares | Mac File Provider namespace; Shared.tsx unmounted | Filesystem read access, no administration page | list_shared_roots; EngineBridge shared metadata/decryption |

Preserve supported routes and File Provider behavior. Unsupported actions are
absent, including deep-link entry points, with a web alternative. Do not expose
the unfinished Shared.tsx. Share creation/invites stay in the web app (1243/1245).

Backend capabilities describe implemented support, not authenticated state or
successful installation. File Provider and CFAPI availability do not mean the
root is registered; existing installation-state commands still own that fact.
Native credentials mean an implemented OS backend, not an unlocked vault.
Linux currently has no native credential backend and runner does not mount the
FUSE prototype: report no on-demand integration, web-only sharing, and unknown
tray visibility. This makes no decision about the future Linux filesystem model.
Unknown package formats remain unknown; only known updater-compatible formats
offer in-app install. Package-managed Linux installs direct users to their package
manager. No credentials or mount probes are performed by this read-only command.

Verification: backend JSON fixtures, frontend route/render checks and browser
screenshots; Linux suites; Desktop CI Rust check (macOS) --all-targets, followed
by operator Mac unlock/Finder enumerate-hydrate-edit/sign-out/re-login before
promotion of shared IPC changes. Native evidence is never inferred from fixtures.

## Windows U1 ruling before implementation — 2026-09-29

Per epic 1638, unsupported controls must be absent. Source inspection at
f3967ca finds `sync_mode` and `sync_overlays` persisted but never consumed by the
runner, and `metered` explicitly deferred in runner. `files_on_demand=false`
only skips cache eviction; it does not implement the promised hydrate-all mode.
The automatic cache cap also updates DB residency without calling CFAPI dehydration
(`EngineBridge::enforce_smart_cache`); hide its Windows selector until native
eviction is implemented (audit C / W6). Manual Free up space remains available.
Windows therefore shows on-demand explanatory text, no four-mode chooser, no
metered/overlay/hydrate-all switches, and a static encryption badge instead of a
clickable no-op encryption switch. Alternatives: open files to download, use
Selective sync to make folders online-only, and pause syncing from the menu
on a metered network. Mac rendering/behavior is retained, including its existing
settings; this Windows contract does not certify those existing Mac controls.

The capability snapshot gates mounting before OS-specific effects run. Failed
resolution offers retry plus web access, never query-parameter authority. Main
window navigation and direct settings mounts use the same support policy;
Windows-shaped onboarding on a Mac mounts the existing Mac onboarding. Unknown
installer format offers the download page instead of in-app install/downgrade.
This is an additive internal IPC contract, with no credentials, crypto, storage
schema, dependency, installer identity, or File Provider changes.

## Capability presentation ruling — 2026-09-29 polish

Guus: “style it with the existing design tokens/components used on the same
page (an info/notice row inside the card system, muted body text size, consistent
spacing), and reuse that component for every alternative-text instance across
shells”. Sync, cache-limit and unsupported-route explanations share a neutral
Card notice with Inter body copy and existing paper/line/ink tokens. No new colours.
Capability resolution failure uses that same card-based error/empty-state pattern
with a heading, explanation and separate Retry / Open web app actions. Desktop
has no BBButton export: use its existing `.button` / `.button.amber` equivalents.
Per 1248/1255, this is a load-blocking inline error, not a transient action toast.

Polish verification: red/green rendering fixtures, bun test, TypeScript and lint;
regenerate the NATIVE.md browser fixtures (including narrow/dark presentation and
retry recovery), or record Chromium launch denial for the lead to rerun.

## Configured sync-root presentation — 2026-09-29 (task 1646)

Files, Settings → Explorer integration, and the Windows tray display
`sync_status.sync_root`. The main window shares its existing status poll with
Files and Settings; the tray uses its existing poll. `default_sync_root` remains
an onboarding suggestion, never the current-root display. One folder card stays
visible while Files loads, is empty, or has an overview error. Missing status is
unavailable; a successful status with no root is not configured. Neither state
invents a path or enables Open folder. Long paths retain their full title.
Open-folder actions let the backend resolve the current configured root at click
time, avoiding a stale path from an earlier poll. macOS File Provider location
resolution stays unchanged: because Mac status deliberately returns a null root,
the Files card names “Beebeeb in Finder” and retains its Open in Finder action
when status is available; Rust resolves the visible File Provider location.
Native verification must compare all three surfaces
and Explorer against a non-default configured root, including after a root change.

## U2 Windows surface checklist

Status is **source reachability**, not a native pass. **Wired** means the action
has a real handler; its runtime correctness, auth/state constraints and errors
remain U2/W-lane gates. **Stub** means persistence/no-op without the promised
behavior. **Dead** means no route/handler from the shipping Windows shell.
**Local** actions (tabs, cancel, close, selection) are wired to React/WebView
state and need no server command. Every row below is **native PENDING**.
Repeat data-dependent rows with empty, populated, locked and failed-load fixtures.
Repeated per-file/per-folder/per-session controls are one action family; exercise
at least two distinct fixture IDs so incorrect closure binding is detectable.

The standalone `windows/` directory has **0 UI files**: its three Rust sources
are the obsolete CLI daemon/config/API, not the shipping shell. Its `--start-minimized`,
`--sync-path`, `--server-url`, `--log-level` flags and log-only remote polling are
**dead to the release**. Do not run that daemon for U2. Active sources are
`src/Windows*.tsx`, `src/windows/`, shared `src/`, and `src-tauri/src/lib.rs`.

| ID | Surface / every action family | Handler / destination | Classification and U2 assertion |
| --- | --- | --- | --- |
| U01 | Sidebar Home, Files, Trash, Account, Insights, Bandwidth, Selective sync, Devices, Security, Activity, Settings | WindowsApp `activeNav`, 11 view cases | Wired/local; each title and populated data must match; signed-out sidebar only Home/Settings |
| U02 | Sign in (Home/auth gate) | `open_onboarding_window` | Wired; raises one onboarding window |
| U03 | Upgrade storage (sidebar) | `plugin:opener|open_url`, web `/billing` | Wired link-out; correct external URL |
| U04 | Search trigger, Ctrl/Cmd+K, query, result click/Enter, Up/Down, close/Escape/backdrop | `desktop_search_files` → `open_in_finder` (Explorer on Windows) | Wired; match fixture name and reveal exact path; query debounce and empty/error states |
| U05 | Version history trigger/shortcut, file query, choose file/version, Back, close/Escape/backdrop | `desktop_search_files`, `list_file_versions`; local selection | Wired; results must belong to selected file |
| U06 | Restore version in history | `restore_file_version` → durable restore queue | Wired; exact restored version/bytes, not just a success toast |
| U07 | Home overview and retry/load states | `sync_status`, `account_profile`, `account_usage`, `account_activity`, `desktop_storage_summary` | Wired reads; no fabricated data when locked/offline |
| U08 | Files Open in Explorer | `default_sync_root`, `open_finder_location` | Wired; existing configured root opens |
| U09 | Files refresh/retry and overview/recent-file rows | `desktop_file_overview` | Wired read; overview rows themselves are display-only, not file-open buttons |
| U10 | Files Manage backup, Set up backup | local event → KnownFolderOnboarding | Wired/local; one modal, current selection |
| U11 | Backup switches for Desktop, Documents, Pictures, Downloads, Music, Videos | `get_known_folder_backup`, `set_known_folder_backup` → runner mirror | Wired; unavailable source rows have no switch; enable/disable each resolvable fixture folder |
| U12 | Backup modal selection, Not now, Back up selected folders | `get_known_folder_onboarding_seen`, `mark_known_folder_onboarding_seen`, `set_known_folder_backup`, `list_vault_folders`, `set_recursive_pin` | Wired; persists dismiss/selection; actual offline pin remains W-lane verification |
| U13 | Trash Refresh/Try again | `desktop_trash_list` | Wired; only trashed fixture items |
| U14 | Trash Restore | `desktop_trash_restore` | Wired; reappears at correct parent with correct bytes |
| U15 | Trash Delete, password field, confirm permanent delete, Cancel | `desktop_confirm_action` → `desktop_trash_delete_permanently` | Wired; cancellation does nothing, wrong password errors, only confirmed fixture item deleted |
| U16 | Account Retry, Manage account, Manage billing | `account_profile`, `account_subscription`, `account_usage`; web account/billing | Wired reads/link-outs |
| U17 | Account Sign out, confirm, Cancel, error Retry | `clear_session` | Wired; U2 must observe signed-out state; lifecycle repair owned by 1639/1538 |
| U18 | Insights Retry, storage/category/largest-file display | `account_storage_breakdown`, `account_usage` | Wired read; rows are not action buttons |
| U19 | Bandwidth Retry, Run speed test/Run again, traffic chart | `account_client_sessions`, `run_speedtest`, `get_bandwidth_history` | Wired; positive measured bytes/samples, error visible; chart is display-only |
| U20 | Selective sync Refresh/Retry, expand/collapse, per-folder checkbox, Apply | `list_vault_folders`, `set_selective_sync` | Wired; exclude reclaims clean unpinned content; include allows future on-demand opens, does NOT guarantee offline hydration |
| U21 | Devices Retry and list | `account_devices` | Wired read; no revoke button on Devices page |
| U22 | Security Retry, Manage/Enable 2FA | `account_security_score`, `account_session_list`; web security | Wired read/link-out; no native 2FA-enrollment promise |
| U23 | Security revoke one session: ask/confirm/Cancel | `account_revoke_session` | Wired; selected fixture session invalidated |
| U24 | Security Sign out all other sessions: ask/confirm/Cancel | `account_revoke_other_sessions` | Wired; preserves current session |
| U25 | Activity Activity/Notifications tabs; each Retry | `account_activity_feed`, `account_notifications` | Wired read; tab navigation local; notifications are read-only |
| U26 | Mark notification read / mark all read | No registered command and no rendered button | Dead/unoffered; no inbox mutation promise |
| U27 | Settings nav Sync/Data residency/Notifications/Launch/Explorer integration/Updates/Advanced | local SettingsNav; capability + auth filter | Wired/local; direct unsupported mounts show alternative |
| U28 | Settings Sync encryption switch | Previously no-op `onChange`; now static “Always on” | Stub removed on Windows; no focusable encryption toggle |
| U29 | Settings Sync metered toggle | `set_desktop_config.metered`, no consumer | Stub removed on Windows; alternative says pause from menu |
| U30 | Settings Sync overlay toggle | `set_desktop_config.sync_overlays`, no consumer | Stub removed on Windows; Explorer owns actual native icons |
| U31 | Settings Sync Files on Demand toggle | `files_on_demand=false` only skips eviction, does not hydrate all | Incomplete promised action removed on Windows; on-demand explanation remains |
| U32 | Settings Sync Free up space | `free_up_space` | Wired; clean unpinned bytes reclaimed, never dirty/pinned; count and native allocation checked |
| U33 | Settings Sync upload/download limit rows | configuration display | Display-only; not editable here; compact Network page owns numeric controls |
| U34 | Data residency region choices, Retry | `account_region`, `account_set_region` | Wired; selected allowed continent persists; capacity-disabled choices explain why |
| U35 | Notification email preference switches (each preference returned by metadata) | `account_notification_preferences`, `account_update_notification_preferences` | Wired; read/write corresponding key, rollback on failure |
| U36 | Local notifications conflicts/sync complete/quota warnings | `set_desktop_config` notify fields → runner/native notifications | Wired; actual OS notification permission/delivery pending |
| U37 | Launch Start at login | `autostart_enabled`, `toggle_autostart` | Wired; verify OS registration and next login, not just toggle state |
| U38 | Explorer integration Enable and state | `windows_shell_integration_state`, `install_windows_shell_integration` | Wired; real root registration/enumeration; Mac uses Finder command pair instead |
| U39 | Updates Check for updates | `check_for_updates_now` | Wired; known package gets matching target; unknown package installation unavailable |
| U40 | Updates release notes modal, close, View on GitHub | returned authored notes + release URL | Wired/local/link-out; exact version |
| U41 | Updates Stable/Beta/Alpha | `set_desktop_config.release_channel` | Wired; subsequent manifest endpoint follows selection |
| U42 | Updates downgrade, confirm/Cancel, error retry | `install_channel_downgrade` | Wired only for detected compatible format; verify with disposable N/N-1 fixture, U5 owns live updater evidence |
| U43 | Update toast Release notes, Restart to update/Try again, dismissal | event `update-available`, `install_update`; local toast state | Wired for compatible packages; unknown/manual/package-managed builds get Downloads/package-manager text, no install action |
| U44 | Advanced Light/Dark/System | `set_desktop_config.theme`, theme CSS | Wired; persists and follows system selection |
| U45 | Advanced cache size choices | `set_desktop_config.local_cache_limit_bytes` → eviction policy | Incomplete native implementation; selector absent on Windows, manual Free up space alternative; repair owned by W-lane |
| U46 | Advanced App activity | `app_activity_snapshot` | Wired read; process measurements, no decorative controls |
| U47 | First run browser sign-in/retry | `start_browser_login`, `browser-login-progress` | Wired; local test API handoff, timeout/error recovery |
| U48 | First run password mode, email/password, submit, Back | `desktop_login`; local mode state | Wired; local fixture account only |
| U49 | First run 2FA code, submit, Back | `desktop_login_2fa`; local mode state | Wired; invalid/expired/valid synthetic code |
| U50 | First run phrase field, Unlock, Continue without unlock | `desktop_unlock_with_recovery_phrase`; local next step | Wired; wrong phrase fails, skip stays honestly locked |
| U51 | First run four sync-mode choices and Continue | Previously `set_sync_mode` persistence only | Stub choices removed; explanatory on-demand page Continue only advances the wizard |
| U52 | First run root chooser, Enable Explorer, Skip | `default_sync_root`, `pick_sync_root`, `install_windows_shell_integration`; local Skip | Wired; cancel picker preserves old root, Skip does not claim integration success |
| U53 | First run Open control center | `show_main_app_window`, current WebView close | Wired; one main window; live engine status |
| U54 | Tray gear/Settings | `show_main_app_settings`, flyout hide | Wired; Settings selected |
| U55 | Tray recent-file row | `desktop_file_overview`, `reveal_and_open_file` | Wired; exact safe path opens in Explorer/default app |
| U56 | Tray deleted-file row, Recycle bin footer | main-window event/storage nav to `trash`, `show_main_app_window` | Wired; Beebeeb trash, not OS recycle bin |
| U57 | Tray View all in Beebeeb | main-window event/storage nav to `activity` | Wired; selected Activity page |
| U58 | Tray Open folder, View online | `open_finder_location`; web app URL | Wired; flyout hides and correct destination opens |
| U59 | Tray context Open Beebeeb, Hide, Start at login, Quit | `show_main_app_window_impl`, window.hide, autostart manager, Tauri quit | Wired; process count/window visibility/autostart state |
| U60 | Auth-expired banner Sign in again | `forceReauth`: `clear_session` then `open_onboarding_window` | Wired; stops if clearing fails; old account never retained |
| U61 | Conflict window Keep mine/Keep theirs/Keep both | `conflict_content_preview`, `resolve_conflict(local/remote/both)` | Wired; exact queued operation and version bytes, W-lane durability |
| U62 | Shared-with-me / share administration | `list_shared_roots` exists but Shared.tsx unmounted | Dead page/unoffered; recipient read/decrypt in engine/filesystem only; creation/invites in web app |
| U63 | Error toasts close; modals Escape/backdrop/Cancel; disabled busy controls | ToastProvider/Modal/local state | Wired/local; focus returns correctly, duplicate submissions prevented |

### Native menu and keyboard checklist

`DESKTOP_MENU_SPECS` and `handle_desktop_menu_action` in `lib.rs` are the active
source. These **20 specification entries** are wired (Preferences and Settings
are aliases). A menu item can still fail at runtime, especially without a root;
its current failure path is tracing, so U2 must inspect both outcome and visible
feedback. Accelerators are platform-native, separate from the search/history
WebView shortcuts.

| Action | Backend / destination | Native check |
| --- | --- | --- |
| Preferences; Settings (Ctrl+,) | main window `settings` | Correct page selected |
| Check for updates (Ctrl+Shift+U) | `menu:check-for-updates`, `consume_menu_update_check`, shared `check_for_updates_now` controller | Same checking/result/error state as Settings; error toast with retry, pending request survives cold window creation |
| Pause/Resume syncing (Ctrl+Shift+P) | `toggle_sync_paused_from_menu` | Persisted pause and stopped/resumed work |
| Sign out (Ctrl+Shift+L) | `clear_session_impl` | Lifecycle gate 1639 |
| Quit (Ctrl+Q) | app.exit | Process exits |
| Open Beebeeb folder (Ctrl+O) | `open_finder_location` | Registered root opens |
| Upload files (Ctrl+U) | `upload_files_to_sync_root_impl` | Picker cancellation; synthetic file copy then upload |
| New folder (Ctrl+Shift+N) | `create_folder_in_sync_root_impl` | Unique folder in configured root then server |
| Open web app (Ctrl+Shift+O) | opener | Correct URL |
| Files (Ctrl+1); Activity (Ctrl+2); Trash (Ctrl+4) | `open_menu_view` | Correct page, including already-open window |
| Zoom in (Ctrl+=); out (Ctrl+-); Actual size (Ctrl+0) | WebView zoom | Visible zoom and reset |
| Documentation; Keyboard shortcuts (Ctrl+/); Service status | external constants in lib.rs | Correct reachable page |
| Report a problem | diagnostics export + support URL | Redacted artifact and external destination |
| Cut/Copy/Paste/Select all | Tauri predefined edit menu | WebView text selection/editing |
| About Beebeeb | Tauri predefined about dialog | Version/product identity |

### Shared compact-only surfaces

These are not mounted by the shipping Windows main/tray windows. Preserve Mac
behavior; do not count their existence as Windows reachability:

- Status: Setup → onboarding; versions/selective-sync/account navigation.
- Finder location: pick root, install, open, reset/cancel, native System Settings
  and continue-without-integration paths. Mac-only File Provider commands must
  not be exposed by a forced compact Finder route on Windows.
- Account: Lock, Unlock, sign out, diagnostics, autostart, web account/billing.
  The Windows Account page has sign-out but **no lock/unlock/diagnostics buttons**;
  diagnostic export is reachable through the native Help menu.
- Network: pause sync, numeric upload/download caps via get/set desktop config.
- Local notifications: three notify toggles via get/set config.
- Versions & conflicts: select issue, open conflict, upgrade, reauthenticate,
  select version and restore. Windows has global history and conflict windows,
  but no mounted VersionCenter issues page; U2 should record that navigation gap.
- Compact selective sync: `list_remote_tree` / `set_recursive_pin`; Windows uses
  `list_vault_folders` / `set_selective_sync`. Do not equate inclusion with pinning.

**Correction of the earlier task note (2026-09-29):** there are no registered
`account_mark_notification_read` or `account_mark_all_notifications_read`
commands. Activity notifications are display-only. The tray reads
`desktop_file_overview`; `tray_recent_activity` remains a registered but unused
frontend command. The old `shared` compact
route redirects to Finder; an unrecognized main-window query goes to Home,
while Rust's unsupported native-menu route fallback is Files. These are not
share-administration routes. The legacy comments claiming the main views are
all placeholders are stale: all 11 nav IDs have concrete switch cases; the
placeholder default is unreachable for a validated NavId.

## Complete registered IPC cross-reference

90 registered application commands at the U1 source revision; task 1665 adds `consume_menu_update_check` (91 total). Every command is indexed below; plugin opener/event/window IPCs are covered by the surface checklist. `desktopApi.ts` entries are wrapper references, not independent views. Native/internal-only and compact-only entries are deliberately retained so their absence from Windows is visible.

| Registered command | Source references (relative to src/) | Windows reachability |
| --- | --- | --- |
| `app_version` | `App.tsx`, `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `check_for_updates_now` | `windows/manualUpdateCheck.ts`, `windows/views/SettingsView.tsx`, `ManualUpdateFeedback.tsx`, `desktopApi.ts` | Shared manual-check state; Settings and native menu |
| `consume_menu_update_check` | `ManualUpdateFeedback.tsx` | Drains the pending native request once, restricted to the app window |
| `install_update` | `UpdateBanner.tsx` | Wired (surface table above) |
| `install_channel_downgrade` | `desktopApi.ts` | Wired (surface table above) |
| `toggle_autostart` | `pages/Account.tsx`, `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `autostart_enabled` | `pages/Account.tsx`, `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `clear_session` | `pages/Account.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `unlock_vault` | `pages/Account.tsx` | Compact-only; no Windows view action |
| `desktop_unlock_with_recovery_phrase` | `WindowsFirstRun.tsx`, `Onboarding.tsx` | Wired (surface table above) |
| `lock_vault` | `pages/Account.tsx` | Compact-only; no Windows view action |
| `sync_status` | `pages/VersionCenter.tsx`, `pages/SyncFolder.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `desktop_storage_summary` | `WindowsApp.tsx`, `pages/Status.tsx`, `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `free_up_space` | `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `export_diagnostics` | `pages/Account.tsx` | Native/internal handler; menu/daemon, no direct Windows TS button |
| `list_version_conflict_center` | `pages/VersionCenter.tsx` | Compact-only; no Windows view action |
| `list_file_versions` | `DesktopVersionHistory.tsx`, `pages/VersionCenter.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `restore_file_version` | `DesktopVersionHistory.tsx`, `pages/VersionCenter.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `get_sync_root` | No TS literal | Compact-only; no Windows view action |
| `pick_sync_root` | `WindowsFirstRun.tsx`, `Onboarding.tsx`, `pages/SyncFolder.tsx` | Wired (surface table above) |
| `default_sync_root` | `WindowsApp.tsx`, `WindowsFirstRun.tsx`, `Onboarding.tsx` | Wired (surface table above) |
| `desktop_platform` | `App.tsx`, `WindowsFirstRun.tsx`, `Onboarding.tsx`, `pages/SyncFolder.tsx`, `platform.ts` | Wired (surface table above) |
| `desktop_capabilities` | `capabilities.tsx` | Wired (surface table above) |
| `finder_location_state` | `Onboarding.tsx`, `pages/SyncFolder.tsx`, `pages/Status.tsx`, `windows/views/SettingsView.tsx` | Mac-only; absent from Windows surface |
| `install_finder_location` | `Onboarding.tsx`, `pages/SyncFolder.tsx`, `windows/views/SettingsView.tsx` | Mac-only; absent from Windows surface |
| `continue_without_finder_location` | `Onboarding.tsx` | Mac-only; absent from Windows surface |
| `finder_domain_user_enabled` | `Onboarding.tsx` | Mac-only; absent from Windows surface |
| `open_login_items_and_extensions_settings` | `Onboarding.tsx` | Mac-only; absent from Windows surface |
| `windows_shell_integration_state` | `WindowsFirstRun.tsx`, `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `install_windows_shell_integration` | `WindowsFirstRun.tsx`, `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `reset_macos_integration` | `pages/SyncFolder.tsx` | Mac-only; absent from Windows surface |
| `open_finder_location` | `WindowsTray.tsx`, `WindowsApp.tsx`, `pages/SyncFolder.tsx` | Wired (surface table above) |
| `open_in_finder` | `DesktopQuickSearch.tsx`, `pages/Shared.tsx` | Wired (surface table above) |
| `get_desktop_config` | `pages/Bandwidth.tsx`, `pages/Notifications.tsx` | Compact-only; no Windows view action |
| `set_desktop_config` | `pages/Bandwidth.tsx`, `pages/Notifications.tsx`, `windows/views/SettingsView.tsx` | Wired (surface table above) |
| `account_email` | `pages/Account.tsx` | Compact-only; no Windows view action |
| `account_profile` | `desktopApi.ts` | Wired (surface table above) |
| `account_subscription` | `desktopApi.ts` | Wired (surface table above) |
| `account_usage` | `desktopApi.ts` | Wired (surface table above) |
| `account_region` | `windows/views/SettingsView.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `account_set_region` | `windows/views/SettingsView.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `account_activity` | `desktopApi.ts` | Wired (surface table above) |
| `account_security_score` | `desktopApi.ts` | Wired (surface table above) |
| `account_session_list` | `desktopApi.ts` | Wired (surface table above) |
| `account_revoke_session` | `desktopApi.ts` | Wired (surface table above) |
| `account_revoke_other_sessions` | `desktopApi.ts` | Wired (surface table above) |
| `account_devices` | `desktopApi.ts` | Wired (surface table above) |
| `account_client_sessions` | `desktopApi.ts` | Wired (surface table above) |
| `account_activity_feed` | `desktopApi.ts` | Wired (surface table above) |
| `account_notifications` | `desktopApi.ts` | Wired (surface table above) |
| `account_notification_preferences` | `desktopApi.ts` | Wired (surface table above) |
| `account_update_notification_preferences` | `desktopApi.ts` | Wired (surface table above) |
| `desktop_confirm_action` | `WindowsApp.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `desktop_trash_list` | `WindowsApp.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `desktop_trash_restore` | `WindowsApp.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `desktop_trash_delete_permanently` | `WindowsApp.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `account_storage_breakdown` | `desktopApi.ts` | Wired (surface table above) |
| `desktop_file_overview` | `desktopApi.ts` | Wired (surface table above) |
| `desktop_search_files` | `DesktopQuickSearch.tsx`, `DesktopVersionHistory.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `app_activity_snapshot` | `desktopApi.ts` | Wired (surface table above) |
| `get_selective_sync` | `desktopApi.ts` | Wired (surface table above) |
| `set_selective_sync` | `desktopApi.ts` | Wired (surface table above) |
| `get_known_folder_backup` | `desktopApi.ts` | Wired (surface table above) |
| `set_known_folder_backup` | `WindowsApp.tsx`, `windows/KnownFolderOnboarding.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `get_known_folder_onboarding_seen` | `desktopApi.ts` | Wired (surface table above) |
| `mark_known_folder_onboarding_seen` | `desktopApi.ts` | Wired (surface table above) |
| `list_shared_roots` | `pages/Shared.tsx` | Dead UI route: Shared.tsx is unmounted; engine reads shares separately |
| `list_vault_folders` | `Onboarding.tsx`, `pages/SelectiveSync.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `list_remote_tree` | `Onboarding.tsx`, `pages/SelectiveSync.tsx` | Compact-only; no Windows view action |
| `set_recursive_pin` | `Onboarding.tsx`, `pages/SelectiveSync.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `open_conflict_window` | `pages/VersionCenter.tsx` | Wired (surface table above) |
| `resolve_conflict` | `ConflictWindow.tsx` | Wired (surface table above) |
| `conflict_content_preview` | `desktopApi.ts` | Wired (surface table above) |
| `notify_conflict` | No TS literal | Native/internal handler; menu/daemon, no direct Windows TS button |
| `desktop_login` | `WindowsFirstRun.tsx`, `onboardingSignIn.ts`, `desktopApi.ts` | Wired (surface table above) |
| `desktop_login_2fa` | `WindowsFirstRun.tsx`, `onboardingSignIn.ts`, `desktopApi.ts` | Wired (surface table above) |
| `start_browser_login` | `WindowsFirstRun.tsx` | Wired (surface table above) |
| `open_onboarding_window` | `WindowsApp.tsx`, `pages/VersionCenter.tsx`, `pages/Status.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `show_main_app_window` | `WindowsTray.tsx`, `WindowsFirstRun.tsx`, `Onboarding.tsx`, `desktopApi.ts` | Wired (surface table above) |
| `show_main_app_settings` | `WindowsTray.tsx` | Wired (surface table above) |
| `desktop_config` | `WindowsFirstRun.tsx`, `windows/views/SettingsView.tsx`, `windows/theme.ts` | Wired (surface table above) |
| `tray_recent_activity` | No TS literal | Backend-only; inspect native/internal caller |
| `tray_pause_sync` | No TS literal | Native/internal handler; menu/daemon, no direct Windows TS button |
| `tray_resume_sync` | No TS literal | Native/internal handler; menu/daemon, no direct Windows TS button |
| `upload_files_to_sync_root` | No TS literal | Native/internal handler; menu/daemon, no direct Windows TS button |
| `create_folder_in_sync_root` | No TS literal | Native/internal handler; menu/daemon, no direct Windows TS button |
| `report_problem` | No TS literal | Native/internal handler; menu/daemon, no direct Windows TS button |
| `set_sync_mode` | `WindowsFirstRun.tsx` | Stub: persisted only; chooser absent |
| `reveal_and_open_file` | `desktopApi.ts` | Wired (surface table above) |
| `run_speedtest` | `desktopApi.ts` | Wired (surface table above) |
| `get_bandwidth_history` | `desktopApi.ts` | Wired (surface table above) |
