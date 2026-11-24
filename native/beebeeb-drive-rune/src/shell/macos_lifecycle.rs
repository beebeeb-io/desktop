//! How the windows behave on a Mac (docs/macos/menu-bar-and-windows.md §3 rules 2-6, §7). Pure: every decision is a
//! function of its inputs, compiled on every OS so its tests run everywhere; `host.rs` and `mod.rs` call it and do
//! what it says.

/// How the app was started (§7): the launch table's rows that matter to the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    /// Finder, Dock, Spotlight, or the first launch after install.
    Manual,
    /// The login item.
    LoginItem,
    /// The updater's restart, with what the person had open.
    UpdaterRelaunch { signed_in: bool, window_was_open: bool },
}

/// What the window does at the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartWindow {
    /// No window: the menu-bar icon only (the app starts as an Accessory).
    None,
    /// The window opens (sign-in with no account, else Home).
    Open,
}

pub fn start_window(launch: Launch) -> StartWindow {
    match launch {
        Launch::Manual => StartWindow::Open,
        Launch::LoginItem => StartWindow::None,
        Launch::UpdaterRelaunch {
            signed_in,
            window_was_open,
        } => {
            // Signed out: sign-in comes back; signed in: only if the window was open.
            if !signed_in || window_was_open {
                StartWindow::Open
            } else {
                StartWindow::None
            }
        }
    }
}

/// A start with no window switches to Accessory itself (no Dock icon, no app menu).
pub fn starts_accessory(start: StartWindow) -> bool {
    start == StartWindow::None
}

/// An exit request ends the app only when asked with a code (Quit Beebeeb, ⌘Q, the b's menu); the last window closing
/// asks with none. True: stop the exit.
pub fn keeps_running(mac: bool, exit_code: Option<i32>) -> bool {
    mac && exit_code.is_none()
}

/// The Dock icon follows the Beebeeb window: on (Regular) while it exists, off (Accessory) otherwise.
pub fn dock_icon_wanted(window_open: bool) -> bool {
    window_open
}

/// The change to make, if any: applied only when it changes. `current` is what was last applied (None: never).
pub fn dock_change(current: Option<bool>, window_open: bool) -> Option<bool> {
    let wanted = dock_icon_wanted(window_open);
    (current != Some(wanted)).then_some(wanted)
}

/// One step of opening the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A window shown in a hidden app stays hidden: unhide first.
    UnhideApp,
    /// The Dock icon, turned on before the window shows.
    DockIconOn,
    ShowWindow,
    /// The app comes forward, after the window shows.
    ActivateApp,
}

/// Every opener, in order. Off a Mac only the show.
pub fn open_steps(mac: bool, dock_icon_on: bool) -> Vec<Step> {
    if !mac {
        return vec![Step::ShowWindow];
    }
    let mut steps = vec![Step::UnhideApp];
    if !dock_icon_on {
        steps.push(Step::DockIconOn);
    }
    steps.push(Step::ShowWindow);
    steps.push(Step::ActivateApp);
    steps
}

/// A reopen (Dock, Finder, Spotlight) shows the Beebeeb window: forward if it is open, else opened on Home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reopen {
    BringForward,
    OpenOnHome,
}

pub fn reopen(window_open: bool) -> Reopen {
    if window_open {
        Reopen::BringForward
    } else {
        Reopen::OpenOnHome
    }
}

/// The b's Hide: hides the app on a Mac (⌘H too), the window alone elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideKind {
    App,
    Window,
}

pub fn hide_kind(mac: bool) -> HideKind {
    if mac {
        HideKind::App
    } else {
        HideKind::Window
    }
}

#[cfg(test)]
#[path = "macos_lifecycle_tests.rs"]
mod tests;
