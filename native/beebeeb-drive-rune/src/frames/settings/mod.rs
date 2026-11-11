//! Settings, inside the main window (the board's three settings groups: `FRAMES.md` here is
//! the checklist).
//!
//! Settings is a state of the main window, never a window of its own (DW-0701): the caption
//! stays, the settings sidebar and one pane take the place of the Drive sidebar and the list.
//! `State::settings` holds that state; `view` draws it; `on_key` is its keyboard.
//!
//! For the shell lane: `FRAMES` is the table of what is built (frame id, and the scene that
//! shows it on a fresh `State`); `SettingsUi::intents` is what settings asks the shell to do
//! (leave settings, show a frame another lane owns, open a web page).

pub mod general;
pub mod source;
#[cfg(test)]
mod tests;

use crate::State;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{self, Theme};
use crate::ui::{caption, settings_shell as shell};
use rune::{Cx, El, Key, KeyEvent, Tx, col, row};

use source::{BoardSettings, SettingsSource, ThemeChoice};

/// A pane of settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    General,
}

/// A menu dropped from a pop-up button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    /// General › Appearance › Theme (DW-0787); `hot` is the item under the keyboard.
    Theme { hot: usize },
}

/// A dialog over a pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialog {
    /// General › Help and report a problem (DW-0782).
    ReportProblem,
}

/// What settings asks of the app around it. The shell lane drains `SettingsUi::intents`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Intent {
    /// Show a board frame that another lane owns (its id, e.g. `DW-A51`).
    Show(&'static str),
    /// Something the board draws a row for and no frame: named for the shell to route.
    Open(&'static str),
    /// Open a web page in the browser: the page's NAME (the shell holds the addresses).
    Web(&'static str),
    /// Download the new version's installer and show it in File Explorer (DW-0781).
    DownloadUpdate,
    /// Save the support bundle and open an email to support in the person's email app (DW-0782).
    SaveBundleAndWriteEmail,
}

/// The settings state of the window.
pub struct SettingsUi {
    /// `true` while the window shows settings instead of Drive.
    pub open: bool,
    pub pane: Pane,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    pub source: Box<dyn SettingsSource>,
    pub intents: Vec<Intent>,
}

impl Default for SettingsUi {
    fn default() -> Self {
        Self {
            open: false,
            pane: Pane::General,
            menu: None,
            dialog: None,
            source: Box::new(BoardSettings::new()),
            intents: Vec::new(),
        }
    }
}

impl SettingsUi {
    /// Ctrl+, and the gear: settings opens on General.
    pub fn show(&mut self, pane: Pane) {
        self.open = true;
        self.pane = pane;
        self.menu = None;
        self.dialog = None;
    }

    /// Back to Drive, and Esc: the window as it was.
    pub fn leave(&mut self) {
        self.open = false;
        self.menu = None;
        self.dialog = None;
    }

    /// Go where a row or a sidebar item points: a pane of this lane, or an intent for the shell.
    pub fn go(&mut self, frame: &'static str) {
        self.menu = None;
        self.dialog = None;
        match frame {
            "DW-0701" => self.pane = Pane::General,
            "DW-0782" => {
                self.pane = Pane::General;
                self.dialog = Some(Dialog::ReportProblem);
            }
            "DW-0401" => self.leave(),
            other => self.intents.push(Intent::Show(other)),
        }
    }

    pub fn ask(&mut self, intent: Intent) {
        self.menu = None;
        self.intents.push(intent);
    }

    /// The theme the window draws in: the person's choice, or the system's.
    pub fn dark(&self, system_dark: bool) -> bool {
        match self.source.general().theme {
            ThemeChoice::System => system_dark,
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        }
    }

    pub fn toggle_theme_menu(&mut self) {
        self.menu = match self.menu {
            Some(Menu::Theme { .. }) => None,
            None => {
                let now = self.source.general().theme;
                Some(Menu::Theme {
                    hot: ThemeChoice::ALL.iter().position(|c| *c == now).unwrap_or(0),
                })
            }
        };
    }

    pub fn choose_theme(&mut self, theme: ThemeChoice) {
        self.source.set_theme(theme);
        self.menu = None;
    }
}

/// The frames of this group that are built: the id, and what shows it on a fresh `State`.
/// (`main.rs` lists the same ids as scenes; the shell lane registers this table.)
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-0701", |s| s.settings.show(Pane::General)),
    ("DW-0781", |s| {
        s.settings.source = Box::new(BoardSettings::new().with_update_available());
        s.settings.show(Pane::General);
    }),
    ("DW-0782", |s| {
        s.settings.show(Pane::General);
        s.settings.go("DW-0782");
    }),
    ("DW-0787", |s| {
        s.settings.show(Pane::General);
        s.settings.toggle_theme_menu();
    }),
];

/// Show the frame `id` on `s` (a scene). `false`: this group has not built it.
pub fn scene(id: &str, s: &mut State) -> bool {
    match FRAMES.iter().find(|(frame, _)| *frame == id) {
        Some((_, apply)) => {
            apply(s);
            true
        }
        None => false,
    }
}

/// One entry of the settings sidebar: its label, its icon, and the frame it shows.
struct Section {
    label: &'static str,
    icon: &'static rune::Icon,
    frame: &'static str,
    pane: Option<Pane>,
}

const SETTINGS_SECTIONS: &[Section] = &[
    Section {
        label: "General",
        icon: &ic::GEAR,
        frame: "DW-0701",
        pane: Some(Pane::General),
    },
    Section {
        label: "Account",
        icon: &ic::PEOPLE,
        frame: "DW-A51",
        pane: None,
    },
    Section {
        label: "Security",
        icon: &ic::SHIELD,
        frame: "DW-A52",
        pane: None,
    },
    Section {
        label: "Devices and sessions",
        icon: &ic::LAPTOP,
        frame: "DW-A55",
        pane: None,
    },
    Section {
        label: "Notifications",
        icon: &ic::BELL,
        frame: "DW-0750",
        pane: None,
    },
];

// The board draws the Drive group's items with a chevron where the other group has a glyph.
const DRIVE_SECTIONS: &[Section] = &[
    Section {
        label: "Storage",
        icon: &ic::CHEV_RIGHT,
        frame: "DW-0712",
        pane: None,
    },
    Section {
        label: "Backups",
        icon: &ic::CHEV_RIGHT,
        frame: "DW-0737",
        pane: None,
    },
    Section {
        label: "Sync and File Explorer",
        icon: &ic::CHEV_RIGHT,
        frame: "DW-0746",
        pane: None,
    },
    Section {
        label: "Thumbnails and previews",
        icon: &ic::CHEV_RIGHT,
        frame: "DW-0744",
        pane: None,
    },
    Section {
        label: "Bandwidth",
        icon: &ic::CHEV_RIGHT,
        frame: "DW-0785",
        pane: None,
    },
];

fn sidebar(ui: &SettingsUi, t: &Theme) -> El {
    let items = |sections: &'static [Section]| -> Vec<El> {
        sections
            .iter()
            .map(|sec| {
                let frame = sec.frame;
                shell::nav_item(sec.label, sec.icon, 16.0, sec.pane == Some(ui.pane), t)
                    .on_press(move |s: &mut State| s.settings.go(frame))
            })
            .collect()
    };
    let back = shell::nav_item("Back to Drive", &ic::BACK, 15.0, false, t)
        .shortcut_hint("Esc")
        .on_press(|s: &mut State| s.settings.leave());
    shell::sidebar(
        back,
        vec![("Settings", items(SETTINGS_SECTIONS)), ("Drive", items(DRIVE_SECTIONS))],
        "Settings open inside the app. Esc goes back.",
        t,
    )
}

/// The window while settings is open.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let ui = &s.settings;
    let t = tokens::theme(ui.dark(cx.system_dark()));
    let (pane, overlay) = match ui.pane {
        Pane::General => general::pane(ui, &t),
    };
    let mut body = row()
        .key("body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(sidebar(ui, &t))
        .child(pane);
    if let Some(overlay) = overlay {
        body = body.child(overlay);
    }
    col()
        .key("window")
        .role("window")
        .label("Beebeeb Drive, Settings")
        .on_key(on_key)
        .w_full()
        .h_full()
        .fill(t.surface)
        .child(caption::caption_bar(&t, cx))
        .child(body)
}

/// Settings' keyboard. Esc closes what is open, innermost first: a menu, then settings.
/// A menu takes the arrows, Home, End and Enter. Everything else is Rune's (Tab, Space,
/// Enter on the focused control).
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let ui = &mut s.settings;
    if !ui.open {
        if ev.is_command(',') {
            ui.show(Pane::General);
            return true;
        }
        return false;
    }
    if let Some(Menu::Theme { hot }) = ui.menu {
        let last = ThemeChoice::ALL.len() - 1;
        let hot = match ev.key {
            Key::Down => (hot + 1).min(last),
            Key::Up => hot.saturating_sub(1),
            Key::Home => 0,
            Key::End => last,
            Key::Enter => {
                ui.choose_theme(ThemeChoice::ALL[hot]);
                return true;
            }
            Key::Escape => {
                ui.menu = None;
                return true;
            }
            Key::Tab => {
                ui.menu = None;
                return false;
            }
            _ => return false,
        };
        ui.menu = Some(Menu::Theme { hot });
        return true;
    }
    match ev.key {
        Key::Escape if ui.dialog.is_some() => {
            ui.dialog = None;
            true
        }
        Key::Escape => {
            ui.leave();
            true
        }
        _ => false,
    }
}
