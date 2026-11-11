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

pub mod account_binding;
pub mod diagnostic_redaction;
pub mod drive_v2;
pub mod held_queue;
pub mod hexfmt;
pub mod known_folder;
pub mod operation_failure;
pub mod owner_keys;
pub mod sign_out;
pub mod state_db;
pub mod vault_tree;
pub mod write_token;
