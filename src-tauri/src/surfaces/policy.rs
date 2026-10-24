//! Platform policy that must NOT change for Windows and Linux (spec section 11).
//!
//! Linux shares the `settings` label, `show_compact_app_window_with_nav`, the
//! startup `show_main_app_window_impl` call and the runner's conflict auto-open
//! with macOS. The redesign changes those for macOS only, so each decision is a
//! function of [`Platform`] and the non-macOS rows are pinned by tests.
//! Two-surface slice 1 (task 1683, spec `docs/specs/2026-10-24-macos-two-surface-app.md`)
//! changed the macOS rows of `main_window_label` (`main`) and `close_policy`
//! (`main` is destroyed) and added `compact_window_label` and `dock_icon_wanted`;
//! startup and the conflict auto-open keep today's macOS rows until slices 6 and 4.
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

/// The label of the window `show_main_app_window_impl` opens. Windows has a dedicated `main-app` shell; macOS has
/// the Beebeeb window `main` (two-surface spec §4, task 1683 slice 1); Linux reuses the compact `settings` window.
pub fn main_window_label(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "main-app",
        Platform::Macos => "main",
        Platform::Linux => "settings",
    }
}

/// The label of the compact window (`App.tsx`): Linux's main window, and on a Mac the tray flyout until slice 3
/// builds the popover. Never `main`: the two must not share a label.
pub fn compact_window_label(_platform: Platform) -> &'static str {
    "settings"
}

/// Two-surface spec §9.1: whether a Mac should show a Dock icon (activation policy Regular) for these windows, each
/// given as its label and whether it is shown (visible or minimized). The Beebeeb window counts whenever it exists:
/// it is destroyed on close, and a minimized one keeps its Dock tile only while the icon stays. Onboarding and
/// conflict windows are still separate until slices 2 and 4, hide on close, and had a Dock icon before slice 1, so
/// they count while shown. The compact flyout and the popover never add one.
pub fn dock_icon_wanted<'a>(windows: impl IntoIterator<Item = (&'a str, bool)>) -> bool {
    windows
        .into_iter()
        .any(|(label, shown)| label == "main" || (shown && (label == "onboarding" || label.starts_with("conflict-"))))
}

/// Whether `handle_new_conflict` opens a conflict window by itself. Today: yes
/// everywhere. Slice 6 makes macOS "never" (conflicts become a popover row, a
/// badge and a notification; the user opens `review`); Windows and Linux keep
/// opening the window.
pub fn conflict_auto_opens_window(_platform: Platform) -> bool {
    true
}

/// Two-surface slice 1: on a Mac the close handler is label-aware, so the Beebeeb window is destroyed on close
/// (spec §4) while every other label keeps today's hide. `close_policy_with(.., false)` is the old table, kept for
/// its test.
pub const MACOS_LABEL_AWARE_CLOSE: bool = true;

/// What the global close handler does to a window (spec rule 10). Windows and Linux keep today's handler:
/// `prevent_close` then `hide` for EVERY label. macOS is label-aware: `main` is destroyed, everything else hidden (a
/// legacy label that still exists before its slice removes it is hidden, which is what it does today).
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

    const LEGACY_LABELS: [&str; 8] = [
        "main",
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
        // TODAY's startup rows. Two-surface slice 6 (nothing at login) replaces the MainWindow row for macOS and
        // updates this test in the same commit.
        assert_eq!(startup_surface(Platform::Macos, false), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Macos, true), StartupSurface::Onboarding);
    }

    #[test]
    fn macos_main_window_is_the_beebeeb_window() {
        // Two-surface spec §4, slice 1: the window every opener shows on a Mac is `main`.
        assert_eq!(main_window_label(Platform::Macos), "main");
    }

    #[test]
    fn the_compact_window_keeps_its_label_on_macos_and_linux() {
        // Until slice 3 the tray click still shows the compact flyout on a Mac; Linux keeps it as its main window.
        assert_eq!(compact_window_label(Platform::Macos), "settings");
        assert_eq!(compact_window_label(Platform::Linux), "settings");
        assert_eq!(compact_window_label(Platform::Linux), main_window_label(Platform::Linux));
        assert_ne!(compact_window_label(Platform::Macos), main_window_label(Platform::Macos));
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
    fn macos_destroys_the_beebeeb_window_on_close_and_hides_everything_else() {
        // Two-surface spec §4: the Beebeeb window closes by destroy (one idle webview). Every other label keeps
        // today's hide: the compact flyout (until slice 3), onboarding (until slice 2), conflict windows (until slice
        // 4), and a popover once slice 3 builds it.
        assert!(MACOS_LABEL_AWARE_CLOSE);
        assert_eq!(close_policy(Platform::Macos, "main"), ClosePolicy::Destroy);
        for label in ["popover", "settings", "onboarding", "tray", "conflict-7f3a", "macos-settings"] {
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
    fn with_the_switch_off_macos_hides_every_label_as_before() {
        let close_policy = |label| close_policy_with(Platform::Macos, label, false);
        for label in ["main", "popover", "settings", "onboarding", "conflict-7f3a"] {
            assert_eq!(close_policy(label), ClosePolicy::Hide, "{label}");
        }
    }

    #[test]
    fn the_dock_icon_follows_the_beebeeb_window_and_the_legacy_decorated_windows() {
        // Two-surface spec §9.1: a Dock icon while the Beebeeb window is open, none otherwise. `main` is destroyed on
        // close, so it counts whenever it exists, minimized or hidden by ⌘H included (a minimized window's Dock tile
        // must not vanish with the icon). Onboarding and conflict windows still exist until slices 2 and 4 and are
        // hidden on close, so they count while shown; before slice 1 every window had a Dock icon. The compact flyout
        // and the popover never add one.
        assert!(!dock_icon_wanted([]));
        assert!(dock_icon_wanted([("main", false)]));
        assert!(dock_icon_wanted([("main", true)]));
        assert!(dock_icon_wanted([("settings", true), ("onboarding", true)]));
        assert!(dock_icon_wanted([("conflict-7f3a", true)]));
        assert!(!dock_icon_wanted([("onboarding", false), ("conflict-7f3a", false)]));
        assert!(!dock_icon_wanted([("settings", true), ("popover", true), ("tray", true)]));
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
    fn the_compact_window_takes_its_label_from_compact_window_label() {
        // Two-surface slice 1: on a Mac `main_window_label` is the Beebeeb window, so the compact window must not
        // take it (two windows would share a label). Its own function answers `settings` everywhere.
        let src = lib_rs();
        let body = item_from(&src, "fn show_compact_app_window_with_nav(");
        assert!(
            body.contains("surfaces::policy::compact_window_label("),
            "show_compact_app_window_with_nav must read its label from policy::compact_window_label"
        );
        assert!(
            !body.contains("surfaces::policy::main_window_label("),
            "the compact window took the Beebeeb window's label"
        );
        assert!(
            !body.contains("\"settings\","),
            "the compact window's label is a literal again"
        );
    }

    #[test]
    fn the_beebeeb_window_takes_its_label_from_main_window_label() {
        let src = lib_rs();
        for signature in ["fn show_main_window(", "fn back_in_main_window(", "fn take_main_nav("] {
            let body = item_from(&src, signature);
            assert!(
                body.contains("surfaces::policy::main_window_label("),
                "{signature} must read the label from policy::main_window_label"
            );
            assert!(!body.contains("\"main\""), "{signature}: the label is a literal");
        }
    }

    #[test]
    fn every_macos_opener_of_the_compact_app_now_opens_the_beebeeb_window() {
        // Two-surface §12 row 1: "Launch and every opener that showed the compact window now show `main`"; only the
        // tray click keeps the compact flyout until slice 3. `show_main_app_window_with_nav` is what launch, the tray
        // menu's "Open Beebeeb", `show_main_app_window` and the zoom fallback call.
        let src = lib_rs();
        let body = item_from(&src, "fn show_main_app_window_with_nav(");
        let mac = body
            .find("#[cfg(target_os = \"macos\")]\n    show_main_window(app, main_nav_for_route(nav));")
            .expect("the macOS arm opens the Beebeeb window");
        let linux = body
            .find("#[cfg(not(any(target_os = \"windows\", target_os = \"macos\")))]")
            .expect("Linux keeps its own arm");
        assert!(mac < linux);
        assert!(
            body[linux..].contains("show_compact_app_window_with_nav(app, compact_nav);"),
            "Linux still opens the compact window"
        );
    }

    #[test]
    fn the_global_window_handler_runs_the_macos_lifecycle_for_every_event() {
        let src = lib_rs();
        let start = src.find(".on_window_event(").expect("global on_window_event handler");
        let handler = &src[start..start + src[start..].find(".run(").expect("handler is followed by run")];
        assert!(
            handler.contains("#[cfg(target_os = \"macos\")]\n            macos_window_lifecycle(window, event);"),
            "{handler}"
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

    #[test]
    fn the_settings_menu_action_opens_the_beebeeb_window_on_settings_on_macos() {
        // F1 (task 1683 slice 1): ⌘, opens the Beebeeb window on Settings, the last section used in it. Lives here
        // (not next to the function) so the signature search cannot self-match this test file.
        let src = lib_rs();
        let body = item_from(&src, "fn handle_desktop_menu_action(");
        assert!(
            body.contains("show_main_window(app, MainNav::settings(None));"),
            "the OpenSettings arm must open the Beebeeb window on Settings:\n{body}"
        );
        assert!(!body.contains("show_macos_settings_window"), "the retired Settings window is back");
        // F11 and F15: View › Home opens the window on Home; View › Back never opens one.
        assert!(body.contains("MainMenuNav::Home => show_main_window(app, MainNav::Home),"));
        assert!(body.contains("MainMenuNav::Back => back_in_main_window(app),"));
    }

    #[test]
    fn the_menu_update_check_opens_the_beebeeb_window_on_about_on_macos() {
        // F2: a menu-triggered update check opens the Beebeeb window on Settings › About, so its update row drains
        // the pending check and answers inline.
        let src = lib_rs();
        let body = item_from(&src, "fn request_menu_update_check(");
        assert!(
            body.contains("show_main_window(app, MainNav::settings(Some(\"about\")));"),
            "a menu-triggered update check must open the Beebeeb window on About:\n{body}"
        );
        assert!(!src.contains("fn show_macos_settings_window("), "the retired Settings window's opener is back");
    }
}
