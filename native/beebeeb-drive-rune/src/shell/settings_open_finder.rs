//! Settings' Open targets of Beebeeb in Finder (macOS, dm-18): `finder-show-app`, `finder-open`, `kept-folder-show`,
//! `kept-folder-dismiss` and `login-items-and-extensions` (System Settings › General › Login Items & Extensions).
//!
//! Must not: decide what the page shows (the reconciler does). Each target asks the OS for one thing; what the OS
//! is asked is an [`Action`], so a test records it and no test opens a window, Finder or System Settings.

use rune::Tx;

use super::host::log;
use crate::State;

/// What a target asks of the OS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Show the running app's bundle in Finder (the page says to move it to Applications).
    ShowApp,
    /// Open the Beebeeb location in Finder.
    OpenLocation,
    /// Show the folder macOS kept, in Finder.
    ShowKept(String),
    /// System Settings › Login Items & Extensions.
    LoginItems,
}

/// Opens `target`; `false`: not one of this page's.
pub fn open(s: &mut State, tx: &mut Tx, target: &str) -> bool {
    open_with(s, target, &mut |s, action| run(s, tx, action))
}

/// [`open`] with the OS's part given.
pub fn open_with(s: &mut State, target: &str, run: &mut dyn FnMut(&mut State, Action)) -> bool {
    match target {
        "finder-show-app" => run(s, Action::ShowApp),
        "finder-open" => run(s, Action::OpenLocation),
        "kept-folder-show" => match s.settings.finder.kept().map(str::to_owned) {
            Some(folder) => run(s, Action::ShowKept(folder)),
            None => log("settings: no kept folder to show"),
        },
        // The row is already gone (`finder::press`); nothing is held that needs letting go.
        "kept-folder-dismiss" => {}
        "login-items-and-extensions" => run(s, Action::LoginItems),
        _ => return false,
    }
    true
}

#[cfg(target_os = "macos")]
fn run(s: &mut State, tx: &mut Tx, action: Action) {
    use crate::shell::platform::macos::macos_fp::open as fp;
    match action {
        Action::OpenLocation => crate::ui::toolbar::open_beebeeb_location(s, tx),
        Action::LoginItems => {
            if fp::open_system_url(fp::LOGIN_ITEMS_URL).is_err() {
                log("settings: System Settings did not open");
                s.settings.toast = Some("Could not open System Settings");
            }
        }
        Action::ShowApp => reveal(
            std::env::current_exe().ok().as_deref().and_then(bundle_of).as_deref(),
            s,
        ),
        Action::ShowKept(folder) => reveal(Some(std::path::Path::new(&folder)), s),
    }
}

#[cfg(not(target_os = "macos"))]
fn run(_: &mut State, _: &mut Tx, _: Action) {
    log("settings: a Finder target on another system");
}

/// The `.app` a binary inside it belongs to.
#[cfg(any(target_os = "macos", test))]
fn bundle_of(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(std::path::Path::to_path_buf)
}

/// Shows an ordinary folder or bundle (not a File Provider item) in Finder: `open -R`, the path one argument.
#[cfg(target_os = "macos")]
fn reveal(path: Option<&std::path::Path>, s: &mut State) {
    let started = path.is_some_and(|p| {
        std::process::Command::new("/usr/bin/open")
            .arg("-R")
            .arg(p)
            .spawn()
            .is_ok()
    });
    if !started {
        log("settings: Finder did not open");
        s.settings.toast = Some("Could not open Finder");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::state;

    fn run_target(target: &str, kept: Option<&str>) -> (bool, Vec<Action>) {
        let mut s = state::init();
        s.settings.finder.set_kept_for_test(kept);
        let mut seen = Vec::new();
        let taken = open_with(&mut s, target, &mut |_, a| seen.push(a));
        (taken, seen)
    }

    #[test]
    fn each_target_asks_the_os_for_its_one_thing() {
        assert_eq!(run_target("finder-show-app", None), (true, vec![Action::ShowApp]));
        assert_eq!(run_target("finder-open", None), (true, vec![Action::OpenLocation]));
        assert_eq!(
            run_target("login-items-and-extensions", None),
            (true, vec![Action::LoginItems])
        );
        assert_eq!(
            run_target("kept-folder-show", Some("/Users/x/Kept")),
            (true, vec![Action::ShowKept("/Users/x/Kept".into())])
        );
        assert_eq!(run_target("kept-folder-show", None), (true, vec![]));
        assert_eq!(run_target("kept-folder-dismiss", Some("/k")), (true, vec![]));
        assert_eq!(run_target("nothing", None), (false, vec![]));
    }

    #[test]
    fn the_app_to_show_is_the_bundle_around_the_binary() {
        let exe = std::path::Path::new("/Applications/Beebeeb.app/Contents/MacOS/beebeeb-drive");
        assert_eq!(bundle_of(exe), Some("/Applications/Beebeeb.app".into()));
        assert_eq!(bundle_of(std::path::Path::new("/usr/bin/beebeeb")), None);
    }
}
