use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use sysinfo::{CpuRefreshKind, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{
    Emitter, Manager, State,
    menu::{AboutMetadata, CheckMenuItemBuilder, Menu, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconEvent},
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tracing_subscriber::EnvFilter;

mod account;
pub(crate) use beebeeb_engine::account_binding;
mod account_dto;
mod api_client;
mod auth_v2; // v2 b10
mod v2_snap; // v2 b03
// LD-0113: what this device refuses to delete, rename or move (the Backups root, its own backups).
mod backup_protect;
mod browser_login;
#[cfg(any(target_os = "windows", test))]
mod callback_gate;
mod config;
mod conflict;
mod desktop_capabilities;
mod desktop_search;
pub(crate) use beebeeb_engine::diagnostic_redaction;
mod direct_download;
mod download_pin;
mod engine_bridge;
mod engine_status;
mod finder_removal;
mod owner_keys;
mod shared_folder_index;
// Task 1885: how the bridge's answer to "open this in Finder" becomes a command's result, and the pins that keep
// the macOS code from spawning `open` again. Only macOS calls it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod finder_open;
// Spec 2026-10-06 (macOS Finder setup reconciler). Only macOS runs the reconciler; on
// Windows/Linux these types exist for `AppState` and the commands.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod finder_setup;
// Spec 2026-10-06 §8, the lifecycle log. Only macOS writes it (the same lifetime as
// `finder_setup`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod lifecycle_log;
#[cfg(test)]
#[path = "../tests/support/bridge.rs"]
mod native_parity_tests;
// Unix-domain-socket IPC (macOS File Provider extension + Linux FUSE). Unix
// sockets don't exist on Windows; the Windows Cloud Files callback runs
// in-process (see `windows_cf`), so this module is `unix`-only.
#[cfg(unix)]
mod ipc_frame;
#[cfg(unix)]
mod ipc_socket;
// Idempotency table for Finder write-queue requests (task 1684).
#[cfg(unix)]
mod ipc_write_dedup;
mod keychain;
// LD-0113: backups from source (`backup`: the host for core's pass, the scheduler, sources, device identity) and
// the known-folder catalog with the old mirror's legacy paths (`known_folder`; the mirror itself is gone).
mod backup;
mod known_folder;
#[cfg(target_os = "linux")]
mod linux_fuse;
#[cfg(target_os = "linux")]
mod linux_thumbnail;
#[cfg(all(test, not(target_os = "linux")))]
mod linux_fuse {
    #[path = "inode_map.rs"]
    pub mod inode_map;
}
mod link_health;
mod lockfile;
#[cfg(target_os = "macos")]
mod macos_file_provider;
// Task 1885: NSWorkspace for the URLs that are not Finder items (System Settings), with no child `open`.
#[cfg(target_os = "macos")]
mod macos_workspace;
// Task 2267: a Windows start refused over unowned local data proves the data is the signed-in account's own before it
// is adopted (Windows never resets; macOS and Linux reset instead, so they never need it).
#[cfg(any(target_os = "windows", test))]
mod own_data_proof;
// Task 2267 (fix round 2): what the person decides about the changes the proof held for review.
#[cfg(any(target_os = "windows", test))]
mod found_while_paused;
// Task 2407 (core 2404): a sync-folder write, rename or delete by name never touches a different entry the name reaches
// (a Windows 8.3 short name), through core's `path_safety::short_name_alias`.
mod name_alias;
// Tasks 1888 + 2226: the reconcile pass over the queue (ghost folder creates folded into their server twins), planned in
// memory. Windows and Linux run it; macOS keeps its own resume.
pub(crate) use beebeeb_engine::held_queue;
// v2: the one lower-case hex encoder and 32-byte decoder (office_v2 pins, share_v2 keys).
pub(crate) use beebeeb_engine::hexfmt;
mod macos_popover;
mod popover_data;
// Ruling R8 (spec 2026-10-06): the pure decision between a first sign-in, the same account signing in again and
// an account switch. Windows keeps its refusal while a session exists and never asks.
#[cfg_attr(target_os = "windows", allow(dead_code))]
mod reauth;
mod runner;
mod sign_out;
mod staged_payload;
pub(crate) use beebeeb_engine::state_db;
mod state_paths;
// 0.8.15: the read Settings › Sync and the Windows tray poll to say what sync is doing (counts only).
mod sync_activity;
mod transfer_progress;
// 0.8.15: WebView2's browser shortcuts (print, reload, find, ...) are off in every window; no-op off Windows.
mod webview_browser_keys;
pub(crate) use beebeeb_engine::write_token;
// Task 1683 slice 1: pure macOS-popover surface logic, compiled and tested on every
// platform. Slices 2-6 wire the rest of it; until then only `policy` has callers,
// so dead-code is allowed for the module (remove the allow when slice 6 lands).
#[allow(dead_code)]
mod surfaces;
// Per-process scratch dirs so unit tests never touch the person's real config or cache.
#[cfg(test)]
mod test_sandbox;
// Layout-independent comparison of source text, for the tests that pin the code by reading it.
#[cfg(test)]
mod source_pin;
// The File Provider extension logs through its one logger and makes only closed values public.
#[cfg(test)]
mod extension_log_pin;
// Sync-root filesystem watcher — the local-create UPLOAD trigger (task 0780).
// Primarily for Windows, where there is no OS extension / IPC socket to fire
// `QueueFinderCreate`; the macOS File Provider extension drives that over the
// Unix socket instead, and the Linux FUSE prototype does not use the socket at
// all (it calls the in-process `EngineBridge` directly). Compiled everywhere (cheap, cross-platform via
// `notify`) but the runner only spawns it on Windows (see `runner::run`).
mod watcher;
// Task 2191: the guard for Windows files of unknown content (synced before task 1640). Its
// decisions are cross-platform (tested everywhere); the runner calls it on Windows only.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod unknown_check;
// Task 2240 slice 2: the placeholder reconcile decided from folder listings. Cross-platform so it is
// tested everywhere; only the Windows tick calls it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod os_state_reconcile;
// Tasks 2270 / 2280: what to do with an upload finalization journal row. Decided on every platform (tested
// everywhere); applied on Windows only (`windows_cf::upload_finalization`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod upload_finalization_plan;
// Task 2284: the no-fetch scope a finalization, stamp or promote holds around its data open.
mod no_fetch_scope;
// LD-0105 S0: when a Cloud Files fetch reports its byte progress to Windows. Decided on every platform
// (tested everywhere); applied on Windows only (`windows_cf::callbacks::keep_fetch_alive`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod cf_keepalive;
// LD-0105 W2: the streaming fetch's rules (demands, the sink, a session's end, the poison key). Pure and tested
// everywhere; the Windows glue is `windows_cf::streaming`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod cf_range;
// LD-0105 D2 (W3): the ONE player allowlist and media types. It lives under `windows_cf/` (the contract's
// path) but is pure and compiled everywhere, so its rules are tested on every platform. Switched on with
// the streaming callback (W2); `windows_cf::streaming` calls it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[path = "windows_cf/stream_players.rs"]
mod stream_players;
// Windows Cloud Files API — Phase 2 Task 6. Gated to Windows only;
// the module's own files start with `#![cfg(target_os = "windows")]`
// so this `mod` declaration plus the conditional are belt-and-braces.
#[cfg(target_os = "windows")]
mod windows_cf;
#[cfg(target_os = "windows")]
static SESSION_TRANSITION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(any(target_os = "windows", test))]
mod session_commands;
#[cfg(target_os = "windows")]
static SESSION_COMMANDS: LazyLock<session_commands::SessionCommands> =
    LazyLock::new(session_commands::SessionCommands::new);

#[cfg(any(target_os = "windows", test))]
mod auth_attempts;
#[cfg(target_os = "windows")]
static AUTH_ATTEMPTS: LazyLock<auth_attempts::AuthAttempts> = LazyLock::new(auth_attempts::AuthAttempts::new);

// Windows only: cancel the entire command, including independently built clients.
// Other platforms poll the original command body directly.
macro_rules! session_command {
    ($work:expr) => {{
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.run($work).await
        }
        #[cfg(not(target_os = "windows"))]
        {
            $work.await
        }
    }};
}

mod session_state;
use session_state::*;
pub use session_state::{AppState, Session};
mod login_commands;
pub(crate) use login_commands::open_onboarding_window_impl;
use login_commands::*;
mod session_apply;
pub(crate) use session_apply::apply_session;
use session_apply::*;
mod sign_out_commands;
use sign_out_commands::*;
mod vault_lock_commands;
use vault_lock_commands::*;
mod local_state_purge;
use local_state_purge::*;
mod vault_unlock;
use vault_unlock::*;
mod account_v2; // v2 b14
mod session_reauth;
use session_reauth::*;
mod bound_engine;
use bound_engine::*;
mod finder_setup_commands;
use finder_setup_commands::*;
mod file_open_commands;
use file_open_commands::*;
pub(crate) use file_open_commands::{FreeUpSpaceResult, free_up_space_blocking, is_contained, reject_unsafe_rel_path};
mod passkey_commands; // v2 b15
#[cfg(target_os = "windows")]
mod passkey_windows; // v2 b15
mod settings_commands;
mod share_v2; // v2 b05
use settings_commands::*;
// Task 2409: the "Not sent" list and its Try again.
mod unsent_changes_commands;
use unsent_changes_commands::*;
mod vault_tree;
use vault_tree::*;
mod account_commands;
// Desktop v2 (B02): the `drive_*` commands behind the window's DriveSource.
mod drive_v2;
use account_commands::*;
mod activity_conflict_commands;
use activity_conflict_commands::*;
pub use activity_conflict_commands::{SpeedtestLatency, SpeedtestResult};
pub(crate) use activity_conflict_commands::{
    notify_conflict_impl, notify_quota_warning_impl, notify_sync_complete_impl, open_conflict_window_impl,
};
mod desktop_main_window;
mod update_window;
use desktop_main_window::*;
use update_window::*;
mod app_run;
pub use app_run::run;
mod native_menu;
use native_menu::*;
mod tray_window;
use tray_window::*;

use config::{DesktopConfig, HeldConfig, ReleaseChannel};
// `platform_keychain_store_for(id)` resolves to the macOS Keychain store on
// macOS and the Windows Credential Manager store on Windows (Linux keeps the
// fail-closed stub), segmenting every secret under the per-account id
// (`io.beebeeb.app/<id>/<leaf>`). `legacy_platform_keychain_store()` is the
// un-segmented pre-Phase-1 layout, used read-only by the migration + the
// anti-lockout legacy fallback. All session/vault persistence below routes
// through these so secrets land in the OS-native credential vault.
use account::{AccountId, AccountRuntime, synthesize_single_account};
use keychain::{AuthVault, SecretBytes, SessionToken, platform_keychain_store_for};
use runner::EngineRunner;
// Set on a thread that holds `EPOCH_GATE` exclusively, so its own `lock_vault` bump does not wait for itself.
#[cfg(test)]
thread_local! {
    static HOLDS_EPOCH_GATE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
tokio::task_local! {
    /// Set by [`with_sign_out_api_for_test`]: the API this task's sign-out revokes its session at.
    static SIGN_OUT_API_FOR_TEST: String;
}
#[cfg(all(test, not(target_os = "windows")))]
tokio::task_local! {
    /// Set inside [`with_the_shared_state_dir`]: this task already holds the gate, so its own sign-out must not wait for it.
    static PURGE_GATE_HELD: ();
}
#[cfg(test)]
tokio::task_local! {
    static SIGN_OUT_SEAM: SignOutSeam;
}
#[cfg(test)]
tokio::task_local! {
    /// Set by [`with_upload_staging_for_test`]: the upload-staging folder this task's sign-out and Lock empty.
    static UPLOAD_STAGING_FOR_TEST: std::path::PathBuf;
}
// A test shortens the waits for the one test that needs a wait to run out, without a global that
// other tests would see: it is scoped to the future that runs the command.
#[cfg(all(test, target_os = "macos"))]
tokio::task_local! {
    static FINDER_WAIT_OVERRIDE: std::time::Duration;
}
// A test learns the moment `lock_vault` (or a sign-out) holds the engine slot from these signals, never by
// polling the slot (task 1891): anything else that briefly holds it looks the same to a poll, such as the
// stand-in reconciler's own look at the slot when it handles the Lock or the removal, and a start that was
// released on that poll gets the slot BEFORE the lock does. Each is scoped to the future that runs the command,
// like the waits above, only signals (it never waits, and it never holds anything), and is compiled out of
// every build that is not a macOS test build.
#[cfg(all(test, target_os = "macos"))]
tokio::task_local! {
    static LOCK_HOLDS_THE_ENGINE_SLOT: std::sync::Arc<tokio::sync::Notify>;
    // A channel, because the sign-out test runs the command on its own thread and waits for this on another.
    static SIGN_OUT_HOLDS_THE_ENGINE_SLOT: std::sync::mpsc::Sender<()>;
}
#[cfg(all(test, not(target_os = "macos")))]
mod open_folder_tests;
// ── Unit tests for reveal_and_open_file security helpers ─────────────────────
#[cfg(test)]
mod diagnostics_export_tests;
/// Task 1683 slice 1 (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md`): the Beebeeb window's request
/// slot, what the menu may offer around it, and the macOS menu bar (§5.F rows F1-F15, §9.2). Pure, so they run on
/// every platform; the window code that uses them is macOS only.
#[cfg(test)]
mod main_window_tests;
#[cfg(test)]
mod menu_update_check_tests;
#[cfg(test)]
mod reveal_and_open_file_tests;
#[cfg(test)]
mod selective_sync_wave2_tests;
#[cfg(test)]
mod state_db_purge_tests;
#[cfg(test)]
mod state_db_shell_tests;
#[cfg(test)]
mod tests;

/// Flow 7 P0: a recovery-phrase unlock must prove the phrase belongs to the
/// signed-in account (server-side `recovery_check` compare) BEFORE the derived
/// key is persisted. `recover_from_phrase` accepts ANY checksum-valid BIP39
/// phrase, so without this gate a wrong phrase (or a typo that still passes the
/// checksum, ~1 in 16) keys the desktop engine with a foreign key and every
/// upload from this device becomes undecryptable everywhere else.
#[cfg(test)]
mod recovery_phrase_unlock_tests;

/// Runs the REAL `popover_snapshot` command (task 1683 slice 2) on a mock Tauri runtime: real
/// `AppState`, real `PopoverRuntime`, the real session gate, the real storage fetch over HTTP
/// against a loopback API, the real cache. No webview and no window exist, and no state DB is
/// read (a test process has no state directory; that leg is `popover_data::gather_db_view`'s
/// own test against a real DB).
/// Test support: a loopback API for tests that drive real HTTP (the popover's storage fetch, the sign-out's revoke).
#[cfg(all(test, not(target_os = "windows")))]
mod loopback_api_for_tests;

#[cfg(all(test, not(target_os = "windows")))]
mod popover_snapshot_command_tests;

/// Registration wiring for the popover (task 1683 slice 2), asserted on the source because `run()`
/// needs a real window system. Runs on every platform, unlike the mock-runtime tests above.
#[cfg(test)]
mod popover_wiring_tests;

/// The macOS Finder reconciler's commands and wiring (spec 2026-10-06, plan Task 8). The
/// reconciler needs the OS, so the wiring is asserted on the source (like the popover's) and the
/// commands on a handle that has no reconciler behind it.
#[cfg(test)]
mod finder_setup_command_tests;

/// A check's engine stop (spec 2026-10-06 §5.5; Task 8 fix round 1), on a real `AccountRuntime` and a
/// real `EngineRunner` wrapped around a test task. macOS only: `stop_check_engine` is.
#[cfg(all(test, target_os = "macos"))]
mod finder_engine_tests;

/// Sign-out teardown regression tests (engine-stop gate, already-signed-out
/// no-op guard). These drive the REAL `clear_session_impl` against a
/// synthesized in-memory account; the platform credential store is the
/// non-macOS/Linux stub (every method → `Unsupported`), the Tauri app-local
/// state dir is never initialized in tests (the purge logs and continues),
/// and the Windows Cloud Files block is cfg'd out — so these tests exercise
/// exactly the cross-platform control flow, not OS integration.
#[cfg(test)]
mod signout_teardown_tests;

/// Task 1886 (P0): a sign-out on macOS and Linux ends its sessions on the server too (`POST /api/v1/auth/logout` with
/// each token), bounded and best effort as on Windows. These drive the REAL `clear_session_impl`, signed in, against a
/// loopback API the test started (`with_sign_out_api_for_test`). On Linux the test build's credential store is the
/// fail-closed stub, so a signed-in sign-out there ends in the Keychain clear's error; the stub holds nothing, so the
/// tokens the sign-out took are gone from the computer all the same and are ended. Each sign-out is compared with the
/// one whose revoke succeeded: a revoke never changes what the sign-out returns.
#[cfg(all(test, not(target_os = "windows")))]
mod sign_out_revoke_tests;

/// Startup session-probe regression tests (definitive-401 auto sign-out).
/// The probe is pointed at a raw-TCP loopback mock (same technique as the
/// `recovery_phrase_unlock_tests` server; desktop has no axum dev-dep), so
/// the classification is exercised against real HTTP status lines.
#[cfg(test)]
mod startup_session_tests;

/// Lead ruling 4 (Task 12): the owner record of the local data, completed with the account's user id and the server's
/// spelling of its email while the session still works. Only an owner that is this account is completed.
#[cfg(test)]
mod owner_backfill_tests;

/// Lead ruling T1-4: the OS's message never reaches a Tauri command's return value, a saved config
/// or a toast. `FpError::redacted()` is tested in `finder_setup::error`; these pin the two
/// `lib.rs` shims that hand a bridge error to those places.
#[cfg(all(test, target_os = "macos"))]
mod finder_error_redaction_tests;

/// Task 9 fix round 1 (item D): no unit test of the lib reaches the developer's real Keychain. The
/// sign-out tests used to: a sign-out ends in `clear_keychain_session`, which also clears the legacy
/// flat Keychain items. In a macOS test build `keychain::PlatformKeychainStore` is an in-memory store
/// (`keychain::TestKeychainStore`), and no source file but `keychain.rs` can name the real one.
#[cfg(all(test, target_os = "macos"))]
mod keychain_isolation_tests;

/// A stand-in for the Finder reconciler that confirms every Lock and every removal and ignores the
/// rest. On a Mac, Lock and Sign-out wait for the reconciler and report one they cannot confirm
/// (Task 9), so a test that runs the real command and expects success installs this.
#[cfg(test)]
async fn confirm_every_finder_event(mut rx: tokio::sync::mpsc::UnboundedReceiver<finder_setup::driver::Event>) {
    while let Some(event) = rx.recv().await {
        match event {
            finder_setup::driver::Event::Remove { ack, .. } => {
                let _ = ack.send((Ok(()), finder_removal::KeptFolder::default()));
            }
            finder_setup::driver::Event::Lock { ack } => {
                let _ = ack.send(());
            }
            _ => {}
        }
    }
}

/// Task 9 of plan 1834 (spec 2026-10-06 §5.3 and §8) and the lead's rulings on it: every trigger
/// reaches the reconciler, and the order of Lock and Sign-out against the reconciler, the engine and
/// the keys is pinned. The wiring is asserted on the source (like the other wiring tests: `run()`
/// needs a real window system); the behaviour is in `finder_lock_and_sign_out_tests`.
#[cfg(test)]
mod finder_setup_wiring_tests;

/// Lock and Sign-out against a stand-in for the Finder reconciler (Task 9; lead rulings 1, 2, 3, 5
/// and 7). macOS only: nothing else consults the reconciler. They drive the REAL `lock_vault` and
/// `clear_session_impl` on a mock app; a stand-in answers (or does not answer) the events, and
/// records what the engine slot held when it handled each one, which is how the order shows.
///
/// Sign-out is driven only as far as it can go without touching the platform credential store: the
/// already-signed-out path, and the refusals that return before the Keychain clear (a full sign-out
/// ends in `clear_keychain_session`, which also clears the legacy flat Keychain items of the
/// developer running the tests).
#[cfg(all(test, target_os = "macos"))]
mod finder_lock_and_sign_out_tests;

/// R10 (spec 2026-10-06 §5.6): local data is bound to the account that created it.
#[cfg(test)]
mod account_binding_tests;

/// Ruling R8 (spec 2026-10-06 §3): signing in again as the account that is already on this Mac
/// replaces only the session token; another account is an account switch, and nothing local changes until the person
/// confirms it; Windows is unchanged. Behaviour is tested against the in-memory test Keychain and throwaway state
/// directories (macOS), the wiring as source pins (`run()` needs a real window system).
#[cfg(test)]
mod reauth_tests;

/// Task 1882 (spec `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md` §5): every
/// removal path in this file hands the folder macOS kept to the person. The pure pieces are
/// tested in `finder_removal`; these pin the wiring between them, which no unit can see.
#[cfg(test)]
mod finder_removal_wiring_tests;

/// 1882 r4: the pool helper really leaves the runtime thread free. A single-threaded runtime
/// drives both the job and the test: if the job ran inline it would hold the only thread while it
/// waits for a release that only this test can send, and it would give up after the timeout.
#[cfg(test)]
mod blocking_pool_tests;

/// Re-review P1 (round 3): Repair removes the Finder location, then saves the config. A save that
/// fails after the removal must still show the kept folder, as a failed sign-out does (review I1):
/// the removal cannot be undone, and a retry finds no domain, so the folder would never be named.
#[cfg(test)]
mod repair_kept_folder_tests;

/// Task 1889: every `desktop.toml` load-modify-save runs as one serialized update (`DesktopConfig::update_at`, or a
/// helper that is one: `update`, `try_update_at`, `update_held`, `ensure_account_id`). There is no `save()` to call, and
/// these pins read the source so that Windows and Linux code, which a Mac does not compile, is held to it as well. They
/// were each seen to fail on a deliberate mutation (the task's notes).
#[cfg(test)]
mod config_write_pin_tests;

/// Spec 2026-10-10 (task 1887): the sign-out's keep step and its purge, on a Mac.
#[cfg(all(test, target_os = "macos"))]
mod sign_out_keep_tests;

/// U32, the other half (spec 2026-10-10 §10): Linux and Windows keep today's sign-out. Runs in the public CI only.
#[cfg(all(test, not(target_os = "macos")))]
mod sign_out_unchanged_elsewhere_tests;

// ── Task 2176: a Mac downloads its update, it never installs one in place ─────────────────────────────────────────
//
// The App Sandbox denies replacing the running bundle (and the administrator prompt the updater falls back to), so on a
// Mac `install_update` can only fail, after downloading 12 MB, with an error nobody can diagnose. A Mac is offered a
// download instead (two-surface spec §0 ruling 5). These pins hold the Rust half: the commands refuse with a typed error
// before they touch the updater, and nothing else in the app installs an update.
#[cfg(test)]
mod mac_update_tests;

// ── Speed test: a run without figures is a failure, and one run at a time (PR #140 review, P2-7 / P3) ───────────────
#[cfg(test)]
mod speedtest_tests;

// ── The Finder install's "not ready in time" sentence keeps its category (PR #140 review, P2-8) ─────────────────────
//
// The sentence was reworded for people (no "daemon", no "Timed out waiting for"), and the Linux and Windows classifier
// keyed on those words: the saved category would have become "unknown". The sentence is one constant, the classifier
// recognises it, and these pins run on every OS (the classifier itself runs on Windows and Linux only).
#[cfg(test)]
mod finder_ipc_timeout_tests;

// ── Task 1683 slice 6: launch and lifecycle (two-surface spec §7.3, §12 row 6) ────────────────────────────────────
//
// The decisions are plain functions in `surfaces::launch` and `surfaces::policy::startup_surface`, tested there. These pins
// hold the wiring: the startup reads how the app was launched, nothing else decides it, the updater leaves its flag before
// every restart, and the login item's mechanism is what it was (task 1845 owns changing it).
#[cfg(test)]
mod launch_wiring_tests;

/// Task 2243: what the window and tray polls cost the disk. Each command opens a fresh connection, so SQLite's
/// page cache is empty every time, and each count below includes `StateDb::open` itself: every poll pays it.
#[cfg(test)]
mod task_2243_poll_cost_tests;

#[cfg(all(test, target_os = "macos"))]
mod task_2190_kept_copies_tests;

#[cfg(test)]
mod root_source;
