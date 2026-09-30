//! Platform policy that must NOT change for Windows and Linux (spec section 11).
//!
//! Linux shares the `settings` label, `show_compact_app_window_with_nav`, the
//! startup `show_main_app_window_impl` call and the runner's conflict auto-open
//! with macOS. The redesign changes those for macOS only (slice 6), so each
//! decision is a function of [`Platform`] and the non-macOS rows are pinned by
//! tests. The two call sites in `lib.rs` (startup) and `runner.rs`
//! (`handle_new_conflict`) go through these functions TODAY, with identical
//! behaviour on every platform; slice 6 changes only the macOS rows.
//!
//! What is NOT rewired in slice 1: the window-building code still spells its
//! labels as literals. [`main_window_label`] is the contract those literals must
//! keep, not a value the window code reads.

use super::registry::{ClosePolicy, SurfaceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

impl Platform {
    pub const fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupSurface {
    /// First launch (no sync root configured yet): the onboarding window.
    Onboarding,
    /// Already configured: the platform's main window, shown after startup.
    MainWindow,
}

/// What the app opens ~300 ms after launch. Today: onboarding on first run, else
/// the main window, on every platform. Slice 6 changes macOS to "nothing at
/// login" (ruling 1); Windows and Linux keep this.
pub fn startup_surface(_platform: Platform, no_sync_root: bool) -> StartupSurface {
    if no_sync_root {
        StartupSurface::Onboarding
    } else {
        StartupSurface::MainWindow
    }
}

/// The label of the window `show_main_app_window_impl` opens. Windows has a
/// dedicated `main-app` shell; macOS and Linux reuse the compact `settings`
/// window.
pub fn main_window_label(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "main-app",
        Platform::Macos | Platform::Linux => "settings",
    }
}

/// Whether `handle_new_conflict` opens a conflict window by itself. Today: yes
/// everywhere. Slice 6 makes macOS "never" (conflicts become a popover row, a
/// badge and a notification; the user opens `review`); Windows and Linux keep
/// opening the window.
pub fn conflict_auto_opens_window(_platform: Platform) -> bool {
    true
}

/// What the global close handler does to a window (spec rule 10). Windows and
/// Linux keep today's handler: `prevent_close` then `hide` for EVERY label.
/// macOS is label-aware: onboarding and review are destroyed, everything else
/// hidden (a legacy label that still exists before slice 6 removes it is hidden,
/// which is what it does today).
pub fn close_policy(platform: Platform, label: &str) -> ClosePolicy {
    match platform {
        Platform::Windows | Platform::Linux => ClosePolicy::Hide,
        Platform::Macos => SurfaceKind::from_label(label)
            .map(SurfaceKind::close_policy)
            .unwrap_or(ClosePolicy::Hide),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY_LABELS: [&str; 7] = [
        "settings",
        "onboarding",
        "tray",
        "windows-onboarding",
        "main-app",
        "conflict-7f3a",
        "popover",
    ];

    #[test]
    fn linux_still_opens_settings_after_startup() {
        assert_eq!(startup_surface(Platform::Linux, false), StartupSurface::MainWindow);
        assert_eq!(main_window_label(Platform::Linux), "settings");
    }

    #[test]
    fn linux_first_run_still_opens_onboarding() {
        assert_eq!(startup_surface(Platform::Linux, true), StartupSurface::Onboarding);
    }

    #[test]
    fn windows_startup_is_unchanged() {
        assert_eq!(startup_surface(Platform::Windows, false), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Windows, true), StartupSurface::Onboarding);
        assert_eq!(main_window_label(Platform::Windows), "main-app");
    }

    #[test]
    fn macos_startup_is_todays_behaviour_until_slice_6() {
        // TODAY's rows. Slice 6 (ruling 1: nothing at login) replaces the
        // MainWindow row for macOS and updates this test in the same commit.
        assert_eq!(startup_surface(Platform::Macos, false), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Macos, true), StartupSurface::Onboarding);
        assert_eq!(main_window_label(Platform::Macos), "settings");
    }

    #[test]
    fn linux_conflict_still_opens_a_window() {
        assert!(conflict_auto_opens_window(Platform::Linux));
    }

    #[test]
    fn windows_conflict_still_opens_a_window() {
        assert!(conflict_auto_opens_window(Platform::Windows));
    }

    #[test]
    fn macos_conflict_is_todays_behaviour_until_slice_6() {
        // TODAY's row; slice 6 flips it to false (conflicts never open a window
        // by themselves) and updates this test in the same commit.
        assert!(conflict_auto_opens_window(Platform::Macos));
    }

    #[test]
    fn windows_and_linux_hide_every_window_on_close() {
        for platform in [Platform::Windows, Platform::Linux] {
            for label in LEGACY_LABELS {
                assert_eq!(close_policy(platform, label), ClosePolicy::Hide, "{platform:?} {label}");
            }
        }
    }

    #[test]
    fn macos_destroys_only_onboarding_and_review() {
        assert_eq!(close_policy(Platform::Macos, "onboarding"), ClosePolicy::Destroy);
        assert_eq!(close_policy(Platform::Macos, "review"), ClosePolicy::Destroy);
        for label in [
            "popover",
            "settings",
            "tray",
            "windows-onboarding",
            "main-app",
            "conflict-7f3a",
        ] {
            assert_eq!(close_policy(Platform::Macos, label), ClosePolicy::Hide, "{label}");
        }
    }

    #[test]
    fn current_platform_matches_the_build_target() {
        let expected = if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        };
        assert_eq!(Platform::current(), expected);
    }
}
