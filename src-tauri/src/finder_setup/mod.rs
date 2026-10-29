//! macOS Finder setup reconciler (spec `docs/specs/2026-10-06-macos-finder-setup-reconciler.md`,
//! plan `docs/superpowers/plans/2026-10-06-macos-finder-setup-reconciler.md`).
//!
//! Pure units (`error`, `policy`, `launch_location`, `core`) compile and are tested on every
//! platform. `driver` is the one async task; `macos_ports` (macOS only) is the only part that
//! touches FileProvider.framework, the engine and the app.

pub mod core;
pub mod driver;
pub mod error;
pub mod launch_location;
#[cfg(target_os = "macos")]
pub mod macos_ports;
pub mod policy;
