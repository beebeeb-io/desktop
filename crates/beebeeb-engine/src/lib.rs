//! The Beebeeb desktop engine without a UI toolkit.
//!
//! Everything here is plain Rust over the local state database: no `tauri`, no web view, no window. The Tauri
//! shell (`src-tauri`) re-exports these modules under their old paths (`crate::state_db`, ...), so its code is
//! unchanged; the Rune shell (`native/beebeeb-drive-rune`) calls them directly.
//! Boundary, measurements and the extraction plan: `docs/superpowers/specs/2026-11-11-native-engine-boundary.md`.

// The test helpers behind `test-support` (in-memory databases, injected failures, `*_for_test` writers) never ship:
// a release build with the feature on does not compile. Tests build with debug assertions; a release does not.
// CI also checks the feature graph (`scripts/check-engine-boundary.sh`).
#[cfg(all(feature = "test-support", not(debug_assertions)))]
compile_error!(
    "beebeeb-engine: test-support is for tests only; it is on in a release build (enable it under [dev-dependencies] only)"
);

pub mod account;
pub mod account_binding;
pub mod account_dto;
pub mod api_client;
#[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
pub mod auth_attempts;
pub mod auth_transport;
pub mod backup;
pub mod backup_actions;
pub mod backup_delete;
pub mod backup_protect;
#[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
pub mod callback_gate;
pub mod cf_keepalive;
pub mod cf_range;
pub mod config;
pub mod conflict;
pub mod desktop_capabilities;
pub mod desktop_search;
pub mod diagnostic_redaction;
pub mod direct_download;
pub mod download_pin;
pub mod drive_v2;
pub mod engine_bridge;
pub mod engine_status;
pub mod file_provider_port;
pub mod finder_setup;
#[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
pub mod found_while_paused;
pub mod free_up_space;
pub mod held_queue;
pub mod hexfmt;
pub mod host;
#[cfg(unix)]
pub mod ipc_frame;
#[cfg(unix)]
pub mod ipc_socket;
#[cfg(unix)]
pub mod ipc_write_dedup;
pub mod keychain;
pub mod known_folder;
pub mod link_health;
#[cfg(target_os = "linux")]
pub mod linux_fuse;
#[cfg(target_os = "linux")]
pub mod linux_thumbnail;
pub mod local_state;
pub mod lockfile;
pub mod name_alias;
pub mod no_fetch_scope;
pub mod operation_failure;
pub mod os_state_reconcile;
#[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
pub mod own_data_proof;
pub mod owner_keys;
#[cfg(target_os = "windows")]
pub mod passkey_windows;
pub mod path_safety;
pub mod reauth;
pub mod runner;
pub mod session;
#[cfg(any(target_os = "windows", any(test, feature = "test-support")))]
pub mod session_commands;
pub mod shared_folder_index;
pub mod sign_out;
#[cfg(any(test, feature = "test-support"))]
pub mod source_pin;
pub mod staged_payload;
pub mod state_db;
pub mod state_paths;
pub mod surfaces;
pub mod sync_activity;
#[cfg(any(test, feature = "test-support"))]
pub mod test_api;
#[cfg(any(test, feature = "test-support"))]
pub mod test_sandbox;
pub mod transfer_progress;
pub mod unknown_check;
pub mod upload_finalization_plan;
pub mod vault_tree;
pub mod watcher;
#[cfg(target_os = "windows")]
pub mod windows_cf;
pub mod write_token;

pub use free_up_space::free_up_space_blocking;
#[cfg(target_os = "windows")]
pub use free_up_space::free_up_space_windows;
pub use local_state::{is_disposable_cache_path, purge_macos_hydrate_cache};
pub use path_safety::{is_contained, reject_unsafe_rel_path};
// LD-0105 D2 (W3): the one player allowlist. It lives under `windows_cf/` but is pure and compiled everywhere.
pub mod finder_show;
#[cfg(any(test, feature = "test-support"))]
pub mod root_source;
#[cfg(any(test, feature = "test-support"))]
pub mod source_census;
pub mod status_views;
#[path = "windows_cf/stream_players.rs"]
pub mod stream_players;
