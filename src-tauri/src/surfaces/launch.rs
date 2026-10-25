//! When the Beebeeb window opens by itself (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.3 and
//! §12 row 6; spec C C-W1, C-W5): the launch decision as plain functions of plain inputs, and the relaunch flag the
//! updater leaves for the next launch. Nothing here touches a window, an `AppHandle` or the OS beyond one small file in
//! the directory it is handed, so all of it is compiled and tested on Linux CI.
//!
//! | launch | the window |
//! |---|---|
//! | manual (Finder, Dock, Spotlight), first launch after install | opens: sign-in with no account, else Home |
//! | login item | nothing: the b only |
//! | updater relaunch | what the person had open: their window comes back if it was open and they were signed in; sign-in if they were signed out |
//!
//! **What a launch is, today.** The OS gives the app no word on how it was started, and the login item cannot say (task
//! 1845: whether the sandboxed build's LaunchAgent fires at all is open, and its plist passes no argument to tell a login
//! launch from a manual one). So [`login_item_signal`]'s seam answers [`LoginItemSignal::NoSignal`] and every launch that
//! is not an updater relaunch is [`LaunchSource::Manual`]. Task 1845 supplies the signal in that one place; nothing in
//! this file changes when it does, and [`startup_surface`]'s login row is already pinned.
//!
//! Windows and Linux ignore the source: their rows (`policy::startup_surface`) are what they were.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// What the platform told us about how this process was started, beyond "it started".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItemSignal {
    /// This launch came from the login item.
    LoginLaunch,
    /// This launch did not come from the login item (the signal exists and says no).
    NotLoginLaunch,
    /// Nothing tells. The state of the world until task 1845 plugs its signal into [`login_item_signal`].
    NoSignal,
}

/// The one place the login item's signal comes from (the seam task 1845 plugs into). Nothing is detectable today.
pub fn login_item_signal() -> LoginItemSignal {
    LoginItemSignal::NoSignal
}

/// What the running app writes just before the updater restarts it, and what the next launch consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelaunchFlag {
    /// Unix seconds when it was written. A flag older than [`RELAUNCH_FLAG_MAX_AGE_SECS`] is not an update's.
    pub written_at: u64,
    /// The Beebeeb window was open.
    pub window_open: bool,
    /// An account was signed in.
    pub signed_in: bool,
}

/// How a launch happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchSource {
    Manual,
    LoginItem,
    UpdaterRelaunch(RelaunchFlag),
}

/// The restart after an update takes seconds. A flag this old was left by a restart that never came, and a later manual
/// launch must not be taken for it.
pub const RELAUNCH_FLAG_MAX_AGE_SECS: u64 = 120;

/// The file in the state directory.
pub const RELAUNCH_FLAG_FILE: &str = "relaunch.json";

/// How this launch happened, from the login item's signal and the flag the previous run left (already consumed, whether or
/// not it was fresh). A fresh flag wins: only the updater writes it, and only just before it restarts the app.
pub fn launch_source(login: LoginItemSignal, flag: Option<RelaunchFlag>, now: u64) -> LaunchSource {
    if let Some(flag) = flag
        && flag_is_fresh(&flag, now)
    {
        return LaunchSource::UpdaterRelaunch(flag);
    }
    match login {
        LoginItemSignal::LoginLaunch => LaunchSource::LoginItem,
        LoginItemSignal::NotLoginLaunch | LoginItemSignal::NoSignal => LaunchSource::Manual,
    }
}

/// A flag written no earlier than `RELAUNCH_FLAG_MAX_AGE_SECS` ago and not in the future.
pub fn flag_is_fresh(flag: &RelaunchFlag, now: u64) -> bool {
    flag.written_at <= now && now - flag.written_at <= RELAUNCH_FLAG_MAX_AGE_SECS
}

/// Leaves the flag for the next launch.
pub fn write_relaunch_flag(dir: &Path, flag: &RelaunchFlag) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(flag).map_err(std::io::Error::other)?;
    // Written beside it and renamed over it, so a launch never reads half a flag.
    let tmp = dir.join(format!("{RELAUNCH_FLAG_FILE}.tmp"));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, dir.join(RELAUNCH_FLAG_FILE))
}

/// Reads the flag and removes the file, always: a flag is consumed by the launch that finds it. `None` when there is none, it
/// cannot be read, or it is stale.
pub fn take_relaunch_flag(dir: &Path, now: u64) -> Option<RelaunchFlag> {
    let path = dir.join(RELAUNCH_FLAG_FILE);
    let text = std::fs::read(&path).ok();
    // Consumed whatever it held: a flag that cannot be read, or is stale, must not wait for a later launch.
    let _ = std::fs::remove_file(&path);
    let flag: RelaunchFlag = serde_json::from_slice(&text?).ok()?;
    flag_is_fresh(&flag, now).then_some(flag)
}

/// What opening the app again does (Finder, Dock, Spotlight: `RunEvent::Reopen`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenAction {
    /// The window exists: bring it forward as it was.
    BringForward,
    /// There is none: open it on Home (the window shows sign-in itself with no account).
    OpenHome,
}

pub fn reopen_action(window_exists: bool) -> ReopenAction {
    if window_exists {
        ReopenAction::BringForward
    } else {
        ReopenAction::OpenHome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::policy::{startup_surface, Platform, StartupSurface};

    const NOW: u64 = 1_800_000_000;
    fn flag(age: u64, window_open: bool, signed_in: bool) -> RelaunchFlag {
        RelaunchFlag { written_at: NOW - age, window_open, signed_in }
    }
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("beebeeb-launch-{name}-{}-{}", std::process::id(), NOW));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── What a launch is ────────────────────────────────────────────────────────

    #[test]
    fn with_no_signal_and_no_flag_a_launch_is_a_manual_one() {
        assert_eq!(launch_source(LoginItemSignal::NoSignal, None, NOW), LaunchSource::Manual);
        assert_eq!(launch_source(LoginItemSignal::NotLoginLaunch, None, NOW), LaunchSource::Manual);
    }

    #[test]
    fn the_login_items_signal_makes_a_login_launch() {
        assert_eq!(launch_source(LoginItemSignal::LoginLaunch, None, NOW), LaunchSource::LoginItem);
    }

    #[test]
    fn a_fresh_flag_makes_an_updater_relaunch_and_it_wins_over_the_login_signal() {
        let f = flag(5, true, true);
        assert_eq!(launch_source(LoginItemSignal::NoSignal, Some(f), NOW), LaunchSource::UpdaterRelaunch(f));
        // Only the updater writes the flag, just before it restarts the app: that is a stronger word than a signal.
        assert_eq!(launch_source(LoginItemSignal::LoginLaunch, Some(f), NOW), LaunchSource::UpdaterRelaunch(f));
    }

    #[test]
    fn a_stale_flag_is_not_an_update_and_the_launch_is_what_the_rest_says() {
        let old = flag(RELAUNCH_FLAG_MAX_AGE_SECS + 1, true, true);
        assert_eq!(launch_source(LoginItemSignal::NoSignal, Some(old), NOW), LaunchSource::Manual);
        assert_eq!(launch_source(LoginItemSignal::LoginLaunch, Some(old), NOW), LaunchSource::LoginItem);
    }

    #[test]
    fn freshness_is_the_age_limit_to_the_second_and_a_flag_from_the_future_is_not_fresh() {
        assert!(flag_is_fresh(&flag(0, true, true), NOW));
        assert!(flag_is_fresh(&flag(RELAUNCH_FLAG_MAX_AGE_SECS, true, true), NOW));
        assert!(!flag_is_fresh(&flag(RELAUNCH_FLAG_MAX_AGE_SECS + 1, true, true), NOW));
        let future = RelaunchFlag { written_at: NOW + 3600, window_open: true, signed_in: true };
        assert!(!flag_is_fresh(&future, NOW), "a clock that was put back is not an update");
    }

    // ── What each launch opens (a Mac) ──────────────────────────────────────────

    #[test]
    fn a_manual_launch_opens_the_window_and_the_first_launch_opens_sign_in() {
        assert_eq!(startup_surface(Platform::Macos, false, LaunchSource::Manual), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Macos, true, LaunchSource::Manual), StartupSurface::Onboarding);
    }

    #[test]
    fn a_login_launch_shows_nothing_set_up_or_not() {
        assert_eq!(startup_surface(Platform::Macos, false, LaunchSource::LoginItem), StartupSurface::Nothing);
        assert_eq!(startup_surface(Platform::Macos, true, LaunchSource::LoginItem), StartupSurface::Nothing);
    }

    #[test]
    fn an_updater_relaunch_shows_what_the_person_had_open_and_nothing_else() {
        let up = |window_open, signed_in| LaunchSource::UpdaterRelaunch(flag(3, window_open, signed_in));
        // Signed in, window open: it comes back. Signed in, window closed: nothing (the b only).
        assert_eq!(startup_surface(Platform::Macos, false, up(true, true)), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Macos, false, up(false, true)), StartupSurface::Nothing);
        // Signed out: sign-in, whether or not the window was open (spec §7.3).
        assert_eq!(startup_surface(Platform::Macos, false, up(false, false)), StartupSurface::MainWindow);
        assert_eq!(startup_surface(Platform::Macos, false, up(true, false)), StartupSurface::MainWindow);
        // Never set up: the sign-in surface.
        assert_eq!(startup_surface(Platform::Macos, true, up(false, false)), StartupSurface::Onboarding);
    }

    #[test]
    fn windows_and_linux_ignore_how_they_were_launched() {
        let relaunch = LaunchSource::UpdaterRelaunch(flag(3, false, true));
        for platform in [Platform::Windows, Platform::Linux] {
            for source in [LaunchSource::Manual, LaunchSource::LoginItem, relaunch] {
                assert_eq!(startup_surface(platform, false, source), StartupSurface::MainWindow, "{platform:?} {source:?}");
                assert_eq!(startup_surface(platform, true, source), StartupSurface::Onboarding, "{platform:?} {source:?}");
            }
        }
    }

    // ── Opening the app again ───────────────────────────────────────────────────

    #[test]
    fn opening_the_app_again_brings_the_window_forward_or_opens_it() {
        assert_eq!(reopen_action(true), ReopenAction::BringForward);
        assert_eq!(reopen_action(false), ReopenAction::OpenHome);
    }

    // ── The flag on disk ────────────────────────────────────────────────────────

    #[test]
    fn a_flag_written_is_the_flag_read_and_it_is_read_once() {
        let dir = scratch("roundtrip");
        let f = flag(2, true, true);
        write_relaunch_flag(&dir, &f).unwrap();
        assert!(dir.join(RELAUNCH_FLAG_FILE).exists());
        assert_eq!(take_relaunch_flag(&dir, NOW), Some(f));
        assert!(!dir.join(RELAUNCH_FLAG_FILE).exists(), "consumed by the launch that found it");
        assert_eq!(take_relaunch_flag(&dir, NOW), None, "the second launch finds nothing");
    }

    #[test]
    fn a_stale_flag_is_consumed_too_so_it_cannot_surface_later() {
        let dir = scratch("stale");
        write_relaunch_flag(&dir, &flag(RELAUNCH_FLAG_MAX_AGE_SECS + 60, true, true)).unwrap();
        assert_eq!(take_relaunch_flag(&dir, NOW), None);
        assert!(!dir.join(RELAUNCH_FLAG_FILE).exists());
    }

    #[test]
    fn a_missing_directory_or_file_is_no_flag_and_no_panic() {
        assert_eq!(take_relaunch_flag(&scratch("empty"), NOW), None);
        assert_eq!(take_relaunch_flag(&std::env::temp_dir().join("beebeeb-launch-no-such-dir"), NOW), None);
    }

    #[test]
    fn a_file_that_is_not_a_flag_is_no_flag_and_is_removed() {
        let dir = scratch("garbage");
        std::fs::write(dir.join(RELAUNCH_FLAG_FILE), "not json {").unwrap();
        assert_eq!(take_relaunch_flag(&dir, NOW), None);
        assert!(!dir.join(RELAUNCH_FLAG_FILE).exists());
    }

    #[test]
    fn the_flag_holds_three_facts_and_nothing_that_identifies_anyone() {
        let dir = scratch("shape");
        write_relaunch_flag(&dir, &flag(1, true, false)).unwrap();
        let text = std::fs::read_to_string(dir.join(RELAUNCH_FLAG_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["signed_in", "window_open", "written_at"]);
    }

    #[test]
    fn writing_the_flag_again_replaces_it() {
        let dir = scratch("twice");
        write_relaunch_flag(&dir, &flag(9, true, true)).unwrap();
        write_relaunch_flag(&dir, &flag(1, false, true)).unwrap();
        assert_eq!(take_relaunch_flag(&dir, NOW), Some(flag(1, false, true)));
    }

    // ── The seam ────────────────────────────────────────────────────────────────

    #[test]
    fn until_task_1845_the_login_item_says_nothing() {
        assert_eq!(login_item_signal(), LoginItemSignal::NoSignal);
    }
}
