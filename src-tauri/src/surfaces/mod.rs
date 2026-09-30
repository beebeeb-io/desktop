//! Pure surface logic for the macOS menu-bar popover redesign (task 1683,
//! slice 1; spec `docs/specs/2026-09-30-macos-menubar-popover.md`).
//!
//! Nothing in here touches a window, an `AppHandle` or the OS. Every decision
//! the new macOS shell will make is a plain function of plain inputs so it is
//! compiled and TESTED on Linux CI (the macOS job only runs `cargo check`).
//! Slice 6 (the flip) wires these into the window code; until then the only
//! callers in the shipping build are the two platform-neutral policy functions
//! in [`policy`], which reproduce today's behaviour on every platform.
//!
//! Units: all geometry in [`anchor`] is LOGICAL points with a top-left origin
//! and y growing down (tao's convention). Converting AppKit's bottom-left
//! origin, and physical pixels, is the caller's job in slice 6. The one place
//! physical pixels appear is [`window_state`], because that is what
//! `tauri-plugin-window-state` stores.
//!
//! | module | spec |
//! |---|---|
//! | [`anchor`] | section 6 rule 2, section 9 "Mixed-DPI anchoring", open question 6 |
//! | [`window_state`] | section 6 rule 8 (window-state v2) |
//! | [`registry`] | section 2 (window inventory), section 6 rules 3, 6, 7, 10 |
//! | [`blur_guard`] | section 6 rule 4 |
//! | [`failure`] | section 7 (one surface per failure) |
//! | [`phase`] | section 3 "State machine", section 9 "Popover phase" |
//! | [`policy`] | section 11 (Windows and Linux impact) |
//!
//! `combined_tests` (test-only) drives the registry, the failure reducer and the
//! phase reducer together: each is correct alone and they can still disagree.

pub mod anchor;
pub mod blur_guard;
pub mod failure;
pub mod phase;
pub mod policy;
pub mod registry;
pub mod window_state;

#[cfg(test)]
mod combined_tests;
#[cfg(test)]
mod config_tests;
