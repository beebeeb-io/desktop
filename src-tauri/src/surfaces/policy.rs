//! Platform policy that must NOT change for Windows and Linux (spec section 11).
//!
//! Linux shares the `settings` label, `show_compact_app_window_with_nav`, the
//! startup `show_main_app_window_impl` call and the runner's conflict auto-open
//! with macOS. The redesign changes those for macOS only, so each decision is a
//! function of [`Platform`] and the non-macOS rows are pinned by tests.
//! Two-surface slice 1 (task 1683, spec `docs/specs/2026-10-24-macos-two-surface-app.md`)
//! changed the macOS rows of `main_window_label` (`main`) and `close_policy`
//! (`main` is destroyed) and added `compact_window_label` and `dock_icon_wanted`; slice 2 added `onboarding_surface`
//! (a Mac signs in in the Beebeeb window) and took the `onboarding` window out of `dock_icon_wanted`;
//! the conflict auto-open keeps today's macOS row until slice 4; slice 6 (task 1683) made a Mac's startup a function of
//! how it was launched (`surfaces::launch`).
//!
//! Every function here is CALLED by the shipping code, and `wiring_tests` proves
//! it structurally (reads `lib.rs` and fails if a call site goes back to a
//! literal): startup (`startup_surface`), the main-window labels
//! (`main_window_label`, both the compact `settings` window and the Windows
//! `main-app` shell), the global close handler (`close_policy`), the sign-in openers (`onboarding_surface`) and
//! `runner::handle_new_conflict` (`conflict_auto_opens_window`). Each returns
//! today's answer on Windows and Linux.

use super::launch::LaunchSource;
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
    /// Nothing opens by itself: a Mac's login launch, and an update's relaunch when the person had nothing open. The b
    /// in the menu bar is all there is (two-surface spec §7.3). Only a Mac ever answers this.
    Nothing,
}

/// What the app opens ~300 ms after launch. Windows and Linux: onboarding on first run, else the main window, however
/// the app was started. A Mac (two-surface spec §7.3, task 1683 slice 6) follows how it was started: a manual launch
/// opens the window (sign-in on first run), a login launch opens nothing, and an update's relaunch opens what the person
/// had open and nothing else (sign-in if they were signed out).
pub fn startup_surface(platform: Platform, no_sync_root: bool, source: LaunchSource) -> StartupSurface {
    let configured_or_first_run = if no_sync_root { StartupSurface::Onboarding } else { StartupSurface::MainWindow };
    if platform != Platform::Macos {
        return configured_or_first_run;
    }
    match source {
        LaunchSource::Manual => configured_or_first_run,
        LaunchSource::LoginItem => StartupSurface::Nothing,
        // A set-up Mac that was signed in gets back what it had open and nothing else. One that was signed out, or was
        // never set up, gets sign-in: the window (or, on first run, the onboarding surface) shows it.
        LaunchSource::UpdaterRelaunch(flag) => {
            if no_sync_root || !flag.signed_in {
                configured_or_first_run
            } else if flag.window_open {
                StartupSurface::MainWindow
            } else {
                StartupSurface::Nothing
            }
        }
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

/// Where "Sign in" opens (task 1683 slice 2, two-surface spec §7.4 and §12 row 2). A Mac signs in in the Beebeeb window:
/// the signed-out window shows sign-in itself, so the opener only shows `main`, and the `onboarding` window is never
/// built there. Windows and Linux keep a window of their own, under the labels they always had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingSurface {
    /// The Beebeeb window (`main`), on its signed-out screens.
    BeebeebWindow,
    /// A separate first-run window with this label.
    Window(&'static str),
}

pub fn onboarding_surface(platform: Platform) -> OnboardingSurface {
    match platform {
        Platform::Macos => OnboardingSurface::BeebeebWindow,
        Platform::Windows => OnboardingSurface::Window("windows-onboarding"),
        Platform::Linux => OnboardingSurface::Window("onboarding"),
    }
}

/// The label of the compact window (`App.tsx`): Linux's main window, and on a Mac the tray flyout until slice 3
/// builds the popover. Never `main`: the two must not share a label.
pub fn compact_window_label(_platform: Platform) -> &'static str {
    "settings"
}

/// Two-surface spec §9.1: whether a Mac should show a Dock icon (activation policy Regular) for these windows, each
/// given as its label and whether it is shown (visible or minimized). The Beebeeb window counts whenever it exists:
/// it is destroyed on close, and a minimized one keeps its Dock tile only while the icon stays. Conflict windows are
/// still separate until slice 4, hide on close, and had a Dock icon before slice 1, so they count while shown. The
/// compact flyout and the popover never add one. Slice 2: sign-in is a screen of the Beebeeb window, so a Mac builds
/// no `onboarding` window and none counts here.
pub fn dock_icon_wanted<'a>(windows: impl IntoIterator<Item = (&'a str, bool)>) -> bool {
    windows
        .into_iter()
        .any(|(label, shown)| label == "main" || (shown && label.starts_with("conflict-")))
}

/// Whether an `ExitRequested` is turned down (review P1 on PR #134). Tauri sends it with `code: None` when the last
/// window is destroyed, and with `Some(code)` for `app.exit(code)` (the menu's Quit) and a restart; AppKit's own
/// terminate (the tray menu's predefined Quit, logout) never sends it. On a Mac the Beebeeb window is destroyed on
/// close, and the app lives on in the menu bar without it, so only the last window's close is turned down. Windows and
/// Linux hide every window instead and keep today's exit.
pub fn keep_running_on_exit_request(platform: Platform, code: Option<i32>) -> bool {
    platform == Platform::Macos && code.is_none()
}

/// What the tray menu's "Hide" hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayHide {
    /// Hide the app (⌘H): every Beebeeb window, unhidden by the Dock or a reopen.
    App,
    /// Hide one window.
    Window(&'static str),
}

/// The tray menu's "Hide". A Mac hides the app, as ⌘H does (spec A10), which takes the Beebeeb window and the
/// compact flyout with it; Windows and Linux hide the window "Open Beebeeb" shows there, as before.
pub fn tray_hide(platform: Platform) -> TrayHide {
    match platform {
        Platform::Macos => TrayHide::App,
        Platform::Windows => TrayHide::Window(main_window_label(platform)),
        Platform::Linux => TrayHide::Window(compact_window_label(platform)),
    }
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
        assert_eq!(startup_surface(Platform::Linux, false, LaunchSource::Manual), StartupSurface::MainWindow);
        assert_eq!(main_window_label(Platform::Linux), "settings");
    }

    #[test]
    fn linux_first_run_still_opens_onboarding() {
        assert_eq!(startup_surface(Platform::Linux, true, LaunchSource::Manual), StartupSurface::Onboarding);
    }

    #[test]
    fn windows_startup_is_unchanged() {
        assert_eq!(startup_surface(Platform::Windows, false, LaunchSource::Manual), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Windows, true, LaunchSource::Manual), StartupSurface::Onboarding);
        assert_eq!(main_window_label(Platform::Windows), "main-app");
    }

    #[test]
    fn macos_manual_launch_keeps_todays_rows() {
        // Slice 6 (task 1683) gave a Mac a row per way of launching (`surfaces::launch`'s tests pin them all); a manual
        // launch is the row it always had.
        assert_eq!(startup_surface(Platform::Macos, false, LaunchSource::Manual), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Macos, true, LaunchSource::Manual), StartupSurface::Onboarding);
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
        // must not vanish with the icon). Conflict windows still exist until slice 4 and are hidden on close, so they
        // count while shown. The compact flyout and the popover never add one. Slice 2: sign-in is a screen of the
        // Beebeeb window, so a Mac never builds an `onboarding` window and it counts for nothing (it used to count
        // while shown, as every window did before slice 1).
        assert!(!dock_icon_wanted([]));
        assert!(dock_icon_wanted([("main", false)]));
        assert!(dock_icon_wanted([("main", true)]));
        assert!(dock_icon_wanted([("settings", true), ("main", true)]));
        assert!(dock_icon_wanted([("conflict-7f3a", true)]));
        assert!(!dock_icon_wanted([("onboarding", true), ("onboarding", false), ("conflict-7f3a", false)]));
        assert!(!dock_icon_wanted([("settings", true), ("popover", true), ("tray", true), ("onboarding", true)]));
    }

    #[test]
    fn a_mac_signs_in_in_the_beebeeb_window_and_windows_and_linux_keep_their_own() {
        // Two-surface §12 row 2: `open_onboarding_window` and `open_reauth_window` route to `main` on macOS, and the
        // `onboarding` window is never built there. The other rows are today's labels, pinned.
        assert_eq!(onboarding_surface(Platform::Macos), OnboardingSurface::BeebeebWindow);
        assert_eq!(onboarding_surface(Platform::Windows), OnboardingSurface::Window("windows-onboarding"));
        assert_eq!(onboarding_surface(Platform::Linux), OnboardingSurface::Window("onboarding"));
    }

    #[test]
    fn a_mac_keeps_running_when_its_last_window_closes_and_quits_when_asked() {
        // Review P1 on PR #134: closing the Beebeeb window (red button, ⌘W) when it is the only window made Tauri
        // send `ExitRequested { code: None }` and the app quit: the status item gone, sync stopped. A Mac keeps
        // running; an explicit exit (`app.exit(n)`, the menu's Quit; a restart) carries a code and still exits.
        assert!(keep_running_on_exit_request(Platform::Macos, None));
        for code in [Some(0), Some(1), Some(tauri::RESTART_EXIT_CODE)] {
            assert!(!keep_running_on_exit_request(Platform::Macos, code), "{code:?}");
        }
        // Windows and Linux are unchanged: they hide every window on close, so this never fires there, and if it
        // did they exit as before.
        for platform in [Platform::Windows, Platform::Linux] {
            assert!(!keep_running_on_exit_request(platform, None), "{platform:?}");
            assert!(!keep_running_on_exit_request(platform, Some(0)), "{platform:?}");
        }
    }

    #[test]
    fn the_tray_hide_hides_the_app_on_a_mac_and_the_old_window_elsewhere() {
        // Review item 2: "Open Beebeeb" opens `main`, so "Hide" must not hide only the compact flyout. On a Mac it is
        // ⌘H (spec A10: "Hide → click away / ⌘H"): the Beebeeb window and the flyout go, and the Dock or a reopen
        // brings them back. Windows and Linux are unchanged.
        assert_eq!(tray_hide(Platform::Macos), TrayHide::App);
        assert_eq!(tray_hide(Platform::Windows), TrayHide::Window("main-app"));
        assert_eq!(tray_hide(Platform::Linux), TrayHide::Window("settings"));
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
    fn the_run_loop_asks_keep_running_on_exit_request_and_prevents_only_then() {
        // Review P1: the app is built, then run with a callback; `.run(context)` alone would let the last window's
        // close quit the app.
        let src = lib_rs();
        assert!(
            !src.contains(".run(tauri::generate_context!())"),
            "the app runs without an exit-request callback again"
        );
        let start = src.find(".build(tauri::generate_context!())").expect("the app is built first");
        let run = &src[start..start + src[start..].find("\n}\n").expect("the end of run()")];
        let asked = run
            .find("surfaces::policy::keep_running_on_exit_request(surfaces::policy::Platform::current(), *code)")
            .unwrap_or_else(|| panic!("the callback asks the policy:\n{run}"));
        let prevented = run.find("api.prevent_exit();").expect("and prevents the exit");
        assert!(asked < prevented, "{run}");
        assert!(run.contains("tauri::RunEvent::ExitRequested { code, api, .. }"), "{run}");
        assert_eq!(run.matches("api.prevent_exit();").count(), 1, "{run}");
    }

    #[test]
    fn a_reopen_shows_the_beebeeb_window() {
        // Review round 2, P2-1: with the window closed the app lives on in the menu bar, so double-clicking
        // Beebeeb.app, Spotlight or the Dock must bring the window back (spec §7.3, `RunEvent::Reopen`).
        let src = lib_rs();
        let start = src.find(".build(tauri::generate_context!())").expect("the app is built first");
        let run = &src[start..start + src[start..].find("\n}\n").expect("the end of run()")];
        assert!(
            run.contains("if let tauri::RunEvent::Reopen { .. } = event {\n                reopen_main_window(app);"),
            "{run}"
        );
        let body = item_from(&src, "fn reopen_main_window(");
        assert!(body.contains("show_main_window(app, MainNav::Home)"), "a closed window opens on Home:\n{body}");
        assert!(body.contains("app.get_webview_window("), "an open one is shown as it is:\n{body}");
    }

    /// Every `win.show()` in `body` has an unhide of the app (`unhide_app(app)`) since the previous one: a window shown
    /// in a hidden app stays hidden, and tao's `set_focus` does nothing there.
    fn every_show_unhides_first(name: &str, body: &str) -> usize {
        let mut from = 0;
        let mut shows = 0;
        while let Some(at) = body[from..].find("win.show()") {
            let before = &body[from..from + at];
            assert!(before.contains("unhide_app(app);"), "{name}: a window is shown without unhiding the app first:\n{body}");
            from += at + "win.show()".len();
            shows += 1;
        }
        shows
    }

    #[test]
    fn opening_a_window_on_a_mac_unhides_the_app_first() {
        // Review round 2, P2-2: the tray's Hide hides the app; every way back must unhide it.
        let src = lib_rs();
        assert_eq!(every_show_unhides_first("show_main_window", item_from(&src, "fn show_main_window(")), 2);
        assert_eq!(every_show_unhides_first("reopen_main_window", item_from(&src, "fn reopen_main_window(")), 1);
        let compact = item_from(&src, "fn show_compact_app_window_with_nav(");
        assert_eq!(every_show_unhides_first("show_compact_app_window_with_nav", compact), 2);
        assert!(
            compact.matches("#[cfg(target_os = \"macos\")]\n        unhide_app(app);").count()
                + compact.matches("#[cfg(target_os = \"macos\")]\n            unhide_app(app);").count()
                == 2,
            "the compact window's unhide is macOS only (Linux keeps today's open):\n{compact}"
        );
        assert!(item_from(&src, "fn unhide_app(").contains("let _ = app.show();"));
    }

    #[test]
    fn the_tray_hide_item_asks_tray_hide() {
        let src = lib_rs();
        let start = src.find("\"tray_hide\" =>").expect("the tray's Hide arm");
        let arm = &src[start..start + src[start..].find("\"tray_autostart\" =>").expect("the next arm")];
        assert!(arm.contains("surfaces::policy::tray_hide(surfaces::policy::Platform::current())"), "{arm}");
        assert!(arm.contains("TrayHide::App => {"), "{arm}");
        assert!(!arm.contains("get_webview_window(\"settings\")"), "a literal label again:\n{arm}");
    }

    #[test]
    fn opening_the_window_turns_the_dock_icon_on_before_it_shows_and_activates_after() {
        // Review item 9: Accessory → Regular, then show, then `set_focus`, which in tao is `makeKeyAndOrderFront`
        // followed by `activateIgnoringOtherApps:` (tao util/async.rs `set_focus`): the app is activated after the
        // switch, which is the AppKit mitigation. Both branches (an existing window, a new one) keep that order.
        let src = lib_rs();
        let body = item_from(&src, "fn show_main_window(");
        let dock = body.find("set_macos_dock_icon(app, true);").expect("the Dock icon first");
        let mut checked = 0;
        for branch in [&body[..body.find("let Some(config)").unwrap()], &body[body.find("let Some(config)").unwrap()..]] {
            let show = branch.find("win.show();").unwrap_or_else(|| panic!("a show in:\n{branch}"));
            let focus = branch.find("win.set_focus();").unwrap_or_else(|| panic!("a focus in:\n{branch}"));
            assert!(show < focus, "{branch}");
            checked += 1;
        }
        assert!(dock < body.find("win.show();").unwrap());
        assert_eq!(checked, 2);
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
    fn the_sign_in_openers_route_through_onboarding_surface_and_a_mac_never_builds_the_onboarding_window() {
        // Two-surface §12 row 2 ("cargo (open-sign-in routes to `main`; `onboarding` never built on macOS)").
        let src = lib_rs();
        let open = item_from(&src, "pub(crate) fn open_onboarding_window_impl(");
        assert!(
            open.contains("surfaces::policy::onboarding_surface(surfaces::policy::Platform::current())"),
            "the opener asks the policy:\n{open}"
        );
        assert!(
            open.contains("OnboardingSurface::BeebeebWindow => show_sign_in_in_beebeeb_window(app),"),
            "a Mac signs in in the Beebeeb window:\n{open}"
        );
        assert!(!open.contains("WebviewWindowBuilder"), "the opener itself builds no window:\n{open}");

        // The macOS hand-over opens `main` on Home: the window shows sign-in by itself when there is no session.
        let mac = item_from(&src, "fn show_sign_in_in_beebeeb_window(app: &tauri::AppHandle) -> Result<(), String> {\n    show_main_window(");
        assert!(mac.contains("show_main_window(app, MainNav::Home);"), "{mac}");
        assert!(!mac.contains("WebviewWindowBuilder"), "{mac}");

        // The only code that builds an `onboarding` window is compiled off macOS.
        let builder_at = src
            .find("fn open_onboarding_window_labelled(")
            .expect("the labelled opener exists");
        // The two lines above the signature are its attributes (no byte arithmetic: the source holds non-ASCII text).
        let head = src[..builder_at].lines().rev().take(2).collect::<Vec<_>>().join("\n");
        assert!(
            head.contains("#[cfg(not(target_os = \"macos\"))]"),
            "the window builder for sign-in is not compiled on a Mac:\n{head}"
        );
        let builder = item_from(&src, "fn open_onboarding_window_labelled(");
        assert!(builder.contains("WebviewWindowBuilder::new(") && builder.contains("\"Welcome to Beebeeb\""), "{builder}");

        // Sign in again (R8): macOS opens the same window and asks it for the card (`MainNav::Reauth`, review P3-1, whatever
        // the session says); Windows keeps its refusal; Linux keeps its reload-in-mode.
        let reauth = item_from(&src, "async fn open_reauth_window(");
        let mac_arm = reauth
            .find("#[cfg(target_os = \"macos\")]\n    {")
            .map(|at| &reauth[at..])
            .unwrap_or_else(|| panic!("a macOS arm:\n{reauth}"));
        assert!(mac_arm.contains("show_reauth_in_beebeeb_window(&app)"), "{reauth}");
        let others = "#[cfg(not(any(target_os = \"windows\", target_os = \"macos\")))]";
        assert!(reauth.contains(others), "Linux keeps its own arm:\n{reauth}");
        assert!(!mac_arm[..mac_arm.find(others).unwrap_or(mac_arm.len())].contains("WebviewWindowBuilder"), "{reauth}");
    }

    #[test]
    fn quit_beebeeb_is_the_menus_exit() {
        // The sign-in card's “Quit Beebeeb” (spec C §7.1): the same `app.exit(0)` as ⌘Q, which carries a code and so
        // is not turned down by `keep_running_on_exit_request`.
        let src = lib_rs();
        let body = item_from(&src, "fn quit_beebeeb(");
        assert!(body.contains("app.exit(0)"), "{body}");
        let from = src.find("tauri::generate_handler![").expect("the command list");
        let list = &src[from..from + src[from..].find("])").expect("the end of the command list")];
        assert!(list.contains("quit_beebeeb,"), "registered:\n{list}");
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
