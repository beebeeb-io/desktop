//! The macOS menu bar (`docs/macos/menu-bar-and-windows.md` §1, F1-F15): ONE table of the app's own items, each with
//! its words, its key and the shell action the Windows tray or Settings already has. Rune's `native_menu` draws it
//! (`menu_item`, key equivalents); `App::standard_menus` draws Edit and Window; AppKit's default app menu keeps
//! About, Hide, Hide Others, Show All and Quit (⌘Q, which runs `host::on_quit`). An item whose action is not built
//! on macOS yet is disabled, never hidden.

use rune::{MenuItem, Shortcut, Tx};

use super::app::{Event, Route, Section, SettingsPane};
use crate::State;

/// What an item does. `Native`: AppKit draws and runs it (About, Hide…, Quit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Native,
    Settings,
    CheckForUpdates,
    PauseResume,
    SignOut,
    OpenInFinder,
    UploadFiles,
    NewFolder,
    OpenWebApp,
    Home,
    Back,
    ZoomIn,
    ZoomOut,
    ActualSize,
    Documentation,
    Shortcuts,
    ReportProblem,
    ServiceStatus,
}

impl Act {
    /// Built on macOS: the item is enabled. Anything else is shown disabled.
    pub const fn built(self) -> bool {
        !matches!(
            self,
            Act::UploadFiles
                | Act::NewFolder
                | Act::ZoomIn
                | Act::ZoomOut
                | Act::ActualSize
                | Act::Documentation
                | Act::ServiceStatus
        )
    }
}

/// One row of the table.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub menu: &'static str,
    pub id: &'static str,
    pub label: &'static str,
    /// An accelerator in rune's notation ("cmd+shift+u"); "" for none. Shown by AppKit.
    pub key: &'static str,
    pub act: Act,
}

const fn r(menu: &'static str, id: &'static str, label: &'static str, key: &'static str, act: Act) -> Row {
    Row {
        menu,
        id,
        label,
        key,
        act,
    }
}

/// A line between two groups of one menu.
pub const SEP: Row = r("", "", "", "", Act::Native);

/// The label the pause item has while syncing, and while paused.
pub const PAUSE_LABEL: &str = "Pause Sync";
pub const RESUME_LABEL: &str = "Resume Sync";

/// §1, top to bottom, menu by menu. The app menu's name is the product's (`identity.product_name()`), written
/// "Beebeeb" here for the table and filled in by [`items`].
pub const APP: &str = "Beebeeb";

pub const TABLE: &[Row] = &[
    r(APP, "about", "About Beebeeb", "", Act::Native),
    r(APP, "settings", "Settings…", "cmd+,", Act::Settings),
    r(
        APP,
        "check-for-updates",
        "Check for Updates…",
        "shift+cmd+u",
        Act::CheckForUpdates,
    ),
    r(APP, "pause-resume", PAUSE_LABEL, "shift+cmd+p", Act::PauseResume),
    SEP,
    r(APP, "sign-out", "Sign Out…", "shift+cmd+l", Act::SignOut),
    SEP,
    r(APP, "hide", "Hide Beebeeb", "cmd+h", Act::Native),
    r(APP, "hide-others", "Hide Others", "alt+cmd+h", Act::Native),
    r(APP, "show-all", "Show All", "", Act::Native),
    SEP,
    r(APP, "quit", "Quit Beebeeb", "cmd+q", Act::Native),
    r("File", "open-in-finder", "Open in Finder", "cmd+o", Act::OpenInFinder),
    r("File", "upload-files", "Upload Files…", "cmd+u", Act::UploadFiles),
    r("File", "new-folder", "New Folder…", "shift+cmd+n", Act::NewFolder),
    SEP,
    r("File", "open-web-app", "Open Web App", "shift+cmd+o", Act::OpenWebApp),
    r("View", "home", "Home", "cmd+1", Act::Home),
    SEP,
    r("View", "back", "Back", "cmd+[", Act::Back),
    SEP,
    r("View", "zoom-in", "Zoom in", "cmd+=", Act::ZoomIn),
    r("View", "zoom-out", "Zoom out", "cmd+-", Act::ZoomOut),
    r("View", "actual-size", "Actual size", "cmd+0", Act::ActualSize),
    r("Help", "documentation", "Documentation", "", Act::Documentation),
    r("Help", "shortcuts", "Keyboard shortcuts", "cmd+/", Act::Shortcuts),
    r("Help", "report-a-problem", "Report a problem", "", Act::ReportProblem),
    SEP,
    r("Help", "service-status", "Service status", "", Act::ServiceStatus),
];

/// The pause item's words for the engine's state: what it WILL do (F3).
pub fn pause_label(paused: bool) -> &'static str {
    if paused {
        RESUME_LABEL
    } else {
        PAUSE_LABEL
    }
}

/// The entry of the table the item `id` is (a separator has none).
pub fn row(id: &str) -> Option<&'static Row> {
    TABLE.iter().find(|row| !row.id.is_empty() && row.id == id)
}

/// Runs one action on the main window's turn. The same calls as the tray's menu and Settings (`host::on_menu`).
pub fn run(act: Act, s: &mut State, tx: &mut Tx) {
    use super::app::Intent;
    use super::host::{dispatch, request};
    match act {
        Act::Native => {}
        Act::Settings | Act::CheckForUpdates => {
            // Settings has no About pane of its own yet: both open its first page (the update line is on General).
            request(s, Intent::Settings(SettingsPane::General));
        }
        Act::PauseResume => {
            let result = if s.shell.actions.tray_inputs().paused {
                s.shell.actions.resume_sync()
            } else {
                s.shell.actions.pause_sync()
            };
            if let Err(error) = result {
                super::host::log(format!("the menu bar: pause or resume did not run ({error:?})"));
            }
            s.shell.refresh();
        }
        Act::SignOut => {
            s.shell.open_main();
            // The Security page with its "Sign out of this app" confirmation up (DW-A57).
            crate::frames::account::go(s, "DW-A57");
        }
        Act::OpenInFinder => {
            if let Err(error) = s.shell.actions.open_in_file_explorer() {
                super::host::log(format!("the menu bar: open in Finder did not run ({error:?})"));
            }
        }
        Act::OpenWebApp => super::intents::open_for_signin(WEB_APP),
        Act::Home => {
            let _ = dispatch(s, Event::Navigate(Route::section(Section::Files)));
            s.shell.open_main();
        }
        // Only acts on an open window; it never opens one (rule 2).
        Act::Back => {
            let _ = dispatch(s, Event::Back);
        }
        Act::Shortcuts => {
            let _ = dispatch(s, Event::OpenDialog(super::app::Dialog::Shortcuts));
            s.shell.open_main();
        }
        Act::ReportProblem => super::intents::open_for_signin(&super::web_pages::support_mailto("Beebeeb problem")),
        Act::UploadFiles
        | Act::NewFolder
        | Act::ZoomIn
        | Act::ZoomOut
        | Act::ActualSize
        | Act::Documentation
        | Act::ServiceStatus => {}
    }
    let _ = tx;
}

/// The web app (`repos/web`).
pub const WEB_APP: &str = "https://app.beebeeb.io";

// A menu action is a plain `fn` pointer, so each action has its own function.
macro_rules! actions {
    ($($act:ident),* $(,)?) => {
        fn action_for(act: Act) -> fn(&mut State, &mut Tx) {
            match act {
                $(Act::$act => |s, tx| run(Act::$act, s, tx),)*
                Act::Native => |_, _| {},
            }
        }
    };
}
actions!(
    Settings,
    CheckForUpdates,
    PauseResume,
    SignOut,
    OpenInFinder,
    UploadFiles,
    NewFolder,
    OpenWebApp,
    Home,
    Back,
    ZoomIn,
    ZoomOut,
    ActualSize,
    Documentation,
    Shortcuts,
    ReportProblem,
    ServiceStatus,
);
/// The app's own native items, in §1's order, for the engine's pause state. The AppKit-drawn rows (About, Hide…,
/// Quit) are not here. Items are placed after About in the app menu, in this order; the others in their named menu.
pub fn items(paused: bool) -> Vec<MenuItem<State>> {
    let mut out = Vec::new();
    // A line goes in the menu of the row before it.
    let mut menu = APP;
    for row in TABLE {
        if row.id.is_empty() {
            let line = MenuItem::separator();
            out.push(if menu == APP { line } else { line.menu(menu) });
            continue;
        }
        menu = row.menu;
        if row.act == Act::Native {
            continue;
        }
        let label = if row.act == Act::PauseResume {
            pause_label(paused)
        } else {
            row.label
        };
        let mut item = MenuItem::<State>::new(label, action_for(row.act))
            .id(row.id)
            .enabled(row.act.built());
        if row.menu != APP {
            item = item.menu(row.menu);
        }
        if let Ok(key) = row.key.parse::<Shortcut>() {
            item = item.key_equivalent(key);
        }
        out.push(item);
    }
    out
}

/// Installs the menu on the main app (macOS only): Edit and Window from rune, the rest from [`items`].
#[cfg(target_os = "macos")]
pub fn install(app: rune::App<State>, paused: bool) -> rune::App<State> {
    items(paused)
        .into_iter()
        .fold(app.standard_menus(), |app, item| app.menu_item(item))
}

#[cfg(test)]
#[path = "macos_menu_tests.rs"]
mod tests;
