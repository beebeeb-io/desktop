//! Pure surface logic for the macOS app (task 1683): the 30 Sep popover spec
//! (`docs/specs/2026-09-30-macos-menubar-popover.md`) and, since slice 1 of the
//! two-surface design (`docs/specs/2026-10-24-macos-two-surface-app.md`), the
//! popover plus the one Beebeeb window `main`.
//!
//! Nothing in here touches a window, an `AppHandle` or the OS. Every decision
//! the macOS shell makes is a plain function of plain inputs so it is compiled
//! and TESTED on Linux CI (the macOS job only runs `cargo check`). `lib.rs` wires
//! `policy` (labels, close, the Dock icon, the exit request, the tray's Hide) and
//! `window_state` (the Beebeeb window's placement); the rest is groundwork for the
//! popover (slice 3).
//!
//! Units: [`anchor`] is in LOGICAL points with a top-left origin and y growing
//! down (tao's convention). [`window_state`] is in logical points too: on a Mac
//! tao and tray-icon report positions as points × the scale of the window or
//! display they belong to, so it divides each display's physical numbers by that
//! display's own scale, and the Beebeeb window keeps its frame in points (a Mac
//! does not run the window-state plugin, which stores physical numbers without
//! their scale).
//!
//! | module | spec |
//! |---|---|
//! | [`anchor`] | popover spec section 6 rule 2, section 9 "Mixed-DPI anchoring", open question 6 |
//! | [`window_state`] | two-surface spec §7.2 (where the Beebeeb window opens) |
//! | [`registry`] | two-surface spec §4 (two surfaces, the visibility rule) |
//! | [`blur_guard`] | popover spec section 6 rule 4 |
//! | [`failure`] | popover spec section 7 (one surface per failure) |
//! | [`phase`] | popover spec section 3 "State machine", section 9 "Popover phase" |
//! | [`policy`] | popover spec section 11 (Windows and Linux impact); two-surface §4, §7.3, §9.1 |
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
