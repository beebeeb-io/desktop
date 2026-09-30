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
//! Every function here is CALLED by the shipping code, and `wiring_tests` proves
//! it structurally (reads `lib.rs` and fails if a call site goes back to a
//! literal): startup (`startup_surface`), the main-window labels
//! (`main_window_label`, both the compact `settings` window and the Windows
//! `main-app` shell), the global close handler (`close_policy`) and
//! `runner::handle_new_conflict` (`conflict_auto_opens_window`). Each returns
//! today's answer on every platform until slice 6 changes the macOS rows.

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

/// Slice 6 flips this to `true` (in the same commit that removes the legacy
/// macOS windows) and updates `macos_close_is_todays_behaviour_until_slice_6`.
/// Until then the macOS row is today's behaviour, like the two rows above, so
/// that routing the real close handler through [`close_policy`] changes nothing.
pub const MACOS_LABEL_AWARE_CLOSE: bool = false;

/// What the global close handler does to a window (spec rule 10). Windows and
/// Linux keep today's handler: `prevent_close` then `hide` for EVERY label.
/// macOS, once [`MACOS_LABEL_AWARE_CLOSE`] is on, is label-aware: onboarding and
/// review are destroyed, everything else hidden (a legacy label that still
/// exists before slice 6 removes it is hidden, which is what it does today).
/// The real handler in `lib.rs` calls this and only this.
pub fn close_policy(platform: Platform, label: &str) -> ClosePolicy {
    close_policy_with(platform, label, MACOS_LABEL_AWARE_CLOSE)
}

fn close_policy_with(platform: Platform, label: &str, macos_label_aware: bool) -> ClosePolicy {
    match platform {
        Platform::Windows | Platform::Linux => ClosePolicy::Hide,
        Platform::Macos if !macos_label_aware => ClosePolicy::Hide,
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
    fn macos_close_is_todays_behaviour_until_slice_6() {
        // TODAY's row: the real close handler is wired to `close_policy`, so a
        // label-aware macOS row would destroy onboarding NOW, a behaviour change
        // in a slice that promises none. Slice 6 flips the switch and this test.
        assert!(!MACOS_LABEL_AWARE_CLOSE);
        for label in ["popover", "settings", "onboarding", "review", "tray", "conflict-7f3a"] {
            assert_eq!(close_policy(Platform::Macos, label), ClosePolicy::Hide, "{label}");
        }
    }

    #[test]
    fn windows_and_linux_hide_every_label_even_when_the_macos_switch_is_on() {
        for platform in [Platform::Windows, Platform::Linux] {
            for label in LEGACY_LABELS.iter().chain(["review"].iter()) {
                assert_eq!(
                    close_policy_with(platform, label, true),
                    ClosePolicy::Hide,
                    "{platform:?} {label}"
                );
            }
        }
    }

    #[test]
    fn macos_destroys_only_onboarding_and_review() {
        // The label-aware table slice 6 switches on.
        let close_policy = |platform, label| close_policy_with(platform, label, true);
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

/// Structural proof that the shipping code CALLS the functions above. A unit
/// test on `close_policy` alone stays green if `lib.rs` is edited back to a
/// hard-coded `prevent_close` + `hide`, which is the drift slice 1's review found.
#[cfg(test)]
mod wiring_tests {
    /// The source with line endings normalised: on a Windows checkout the file
    /// has CRLF, and a multi-line match that works on Linux would miss there.
    fn source(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    fn lib_rs() -> String {
        source(include_str!("../lib.rs"))
    }

    fn runner_rs() -> String {
        source(include_str!("../runner.rs"))
    }

    /// The text from `signature` to the next top-level item.
    fn item_from<'a>(src: &'a str, signature: &str) -> &'a str {
        let start = src
            .find(signature)
            .unwrap_or_else(|| panic!("{signature:?} not found: was the function renamed?"));
        let rest = &src[start..];
        let end = rest[signature.len()..]
            .find("\n}\n")
            .map_or(rest.len(), |i| i + signature.len() + 3);
        &rest[..end]
    }

    #[test]
    fn the_global_close_handler_asks_close_policy_and_no_longer_hides_unconditionally() {
        let src = lib_rs();
        let start = src.find(".on_window_event(").expect("global on_window_event handler");
        let handler = &src[start..start + src[start..].find(".run(").expect("handler is followed by run")];
        assert!(
            handler.contains("surfaces::policy::close_policy("),
            "the close handler must route through surfaces::policy::close_policy:\n{handler}"
        );
        assert!(
            handler.contains("ClosePolicy::Hide"),
            "the hide branch must be keyed on the policy:\n{handler}"
        );
        // `prevent_close` and `hide` only inside the policy check.
        let check = handler.find("close_policy(").unwrap();
        for call in ["api.prevent_close()", "window.hide()"] {
            assert_eq!(handler.matches(call).count(), 1, "{call} in:\n{handler}");
            assert!(
                handler.find(call).unwrap() > check,
                "{call} runs before the policy is asked"
            );
        }
    }

    #[test]
    fn the_compact_window_takes_its_label_from_main_window_label() {
        let src = lib_rs();
        let body = item_from(&src, "fn show_compact_app_window_with_nav(");
        assert!(
            body.contains("surfaces::policy::main_window_label("),
            "show_compact_app_window_with_nav must read its label from policy::main_window_label"
        );
        assert!(
            !body.contains("\"settings\","),
            "the compact window's label is a literal again"
        );
    }

    #[test]
    fn the_windows_main_app_shell_takes_its_label_from_main_window_label() {
        let src = lib_rs();
        let body = item_from(&src, "fn show_main_app_window_with_nav(");
        assert!(
            body.contains("surfaces::policy::main_window_label("),
            "the Windows main-app branch must read its label from policy::main_window_label"
        );
        assert!(
            !body.contains("let label = \"main-app\""),
            "the main-app label is a literal again"
        );
    }

    #[test]
    fn startup_and_the_conflict_handler_still_go_through_policy() {
        assert!(lib_rs().contains("surfaces::policy::startup_surface("));
        assert!(runner_rs().contains("surfaces::policy::conflict_auto_opens_window("));
    }

    #[test]
    fn the_helper_finds_what_it_claims_to() {
        // A wiring test that matched nothing would pass for the wrong reason.
        let src = lib_rs();
        let body = item_from(&src, "fn show_compact_app_window_with_nav(");
        assert!(body.len() > 200, "extracted {} bytes", body.len());
        assert!(body.contains("get_webview_window"));
    }
}
