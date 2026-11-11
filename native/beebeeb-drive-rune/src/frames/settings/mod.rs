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

pub mod about;
pub mod general;
pub mod notifications;
pub mod recent;
pub mod source;
pub mod storage_location;
#[cfg(test)]
mod tests;

use crate::State;
use crate::shell::app::{Event, Screen, SettingsPane};
use crate::shell::host;
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{self, Theme};
use crate::ui::{caption, settings_controls as controls, settings_shell as shell};
use rune::{Cx, El, Key, KeyEvent, Tx, col, row};

use source::{BoardSettings, Channel, SettingsSource, ThemeChoice};

/// A pane of settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    General,
    /// General › About Beebeeb (DW-0751).
    About,
    /// General › Storage location (DW-0788).
    StorageLocation,
    /// Notifications (DW-0750).
    Notifications,
    /// Notifications › Recent notifications (DW-0791).
    Recent,
}

/// Which menu a pop-up button drops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKind {
    /// General › Appearance › Theme (DW-0787).
    Theme,
    /// About › Updates › Release channel (DW-0789).
    Channel,
}

impl MenuKind {
    pub fn labels(self) -> Vec<&'static str> {
        match self {
            Self::Theme => ThemeChoice::ALL.iter().map(|c| c.label()).collect(),
            Self::Channel => Channel::ALL.iter().map(|c| c.label()).collect(),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Channel => "Release channel",
        }
    }
}

/// An open menu; `hot` is the item under the pointer or the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Menu {
    pub kind: MenuKind,
    pub hot: usize,
}

/// A dialog over a pane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialog {
    /// General › Help and report a problem (DW-0782).
    ReportProblem,
    /// About › Release notes (DW-0789b).
    ReleaseNotes,
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
    /// Download the new version's installer and show it in File Explorer (DW-0781, DW-0789b).
    DownloadUpdate,
    /// Save the support bundle and open an email to support in the person's email app (DW-0782).
    SaveBundleAndWriteEmail,
    /// Put this text on the clipboard (About › Copy: the version lines).
    Copy(String),
    /// Ask the updater whether a newer version exists, now (About › Check now).
    CheckForUpdates,
    /// Open Windows Settings › Notifications › Beebeeb (DW-0750b).
    OpenWindowsNotificationSettings,
}

/// The settings state of the window.
pub struct SettingsUi {
    /// `true` while the window shows settings instead of Drive.
    pub open: bool,
    pub pane: Pane,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    /// What a toast over the pane says, until the next thing happens.
    pub toast: Option<&'static str>,
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
            toast: None,
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

    /// Go where a row, a button or a sidebar item points (a board frame id): a state of
    /// this lane's panes, or an intent for the shell.
    pub fn go(&mut self, frame: &'static str) {
        self.menu = None;
        self.dialog = None;
        self.toast = None;
        match frame {
            "DW-0401" => self.leave(),
            "DW-0701" | "DW-0781" => self.pane = Pane::General,
            "DW-0782" => {
                self.pane = Pane::General;
                self.dialog = Some(Dialog::ReportProblem);
            }
            "DW-0787" => {
                self.pane = Pane::General;
                self.toggle_menu(MenuKind::Theme);
            }
            "DW-0751" => self.pane = Pane::About,
            "DW-0750" | "DW-0750b" => self.pane = Pane::Notifications,
            "DW-0791" | "DW-0791b" => self.pane = Pane::Recent,
            "DW-0788" | "DW-0788b" | "DW-0788c" | "DW-0788d" => self.pane = Pane::StorageLocation,
            "DW-0789" => {
                self.pane = Pane::About;
                self.toggle_menu(MenuKind::Channel);
            }
            "DW-0789b" => {
                self.pane = Pane::About;
                self.dialog = Some(Dialog::ReleaseNotes);
            }
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

    /// Store new files in location `index`; says so in a toast when it took.
    pub fn choose_location(&mut self, index: usize) {
        if self.source.choose_location(index) {
            self.toast = Some("Storage location updated");
        }
    }

    /// The index of the current choice in a menu.
    pub fn chosen(&self, kind: MenuKind) -> usize {
        match kind {
            MenuKind::Theme => {
                let now = self.source.general().theme;
                ThemeChoice::ALL.iter().position(|c| *c == now).unwrap_or(0)
            }
            MenuKind::Channel => {
                let now = self.source.about().channel;
                Channel::ALL.iter().position(|c| *c == now).unwrap_or(0)
            }
        }
    }

    /// Open the menu on its current choice, or close it.
    pub fn toggle_menu(&mut self, kind: MenuKind) {
        self.menu = match self.menu {
            Some(open) if open.kind == kind => None,
            _ => Some(Menu {
                kind,
                hot: self.chosen(kind),
            }),
        };
    }

    /// Take item `index` of a menu, and close it.
    pub fn choose(&mut self, kind: MenuKind, index: usize) {
        match kind {
            MenuKind::Theme => self.source.set_theme(ThemeChoice::ALL[index.min(2)]),
            MenuKind::Channel => self.source.set_channel(Channel::ALL[index.min(2)]),
        }
        self.menu = None;
    }
}

/// The menu `kind` at `at` in the window's body, when it is the open one.
pub fn menu_overlay(ui: &SettingsUi, kind: MenuKind, at: (f32, f32), t: &Theme) -> Option<El> {
    let open = ui.menu.filter(|m| m.kind == kind)?;
    let chosen = ui.chosen(kind);
    let items = kind
        .labels()
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            controls::menu_item(0, i, label, i == chosen, i == open.hot, t)
                .on_press(move |s: &mut State| s.settings.choose(kind, i))
        })
        .collect();
    Some(
        controls::menu(0, kind.name(), 130.0, items, t)
            .absolute(at.0, at.1)
            .modal(true, |s: &mut State| s.settings.menu = None),
    )
}

/// The frames of this group that are built: the id, and what shows it on a fresh `State`.
/// (`main.rs` adds this table to its scenes; the shell lane registers it.)
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-0701", |s| open_on(s, "DW-0701")),
    ("DW-0781", |s| {
        s.settings.source = Box::new(BoardSettings::new().with_update_available());
        open_on(s, "DW-0701");
    }),
    ("DW-0782", |s| open_on(s, "DW-0782")),
    ("DW-0787", |s| open_on(s, "DW-0787")),
    ("DW-0751", |s| open_on(s, "DW-0751")),
    ("DW-0789", |s| {
        open_on(s, "DW-0789");
        // The board draws the pointer over Beta.
        s.settings.menu = Some(Menu {
            kind: MenuKind::Channel,
            hot: 1,
        });
    }),
    ("DW-0789b", |s| open_on(s, "DW-0789b")),
    ("DW-0750", |s| open_on(s, "DW-0750")),
    ("DW-0750b", |s| {
        s.settings.source = Box::new(BoardSettings::new().with_notifications_off_in_windows());
        open_on(s, "DW-0750");
    }),
    ("DW-0791", |s| open_on(s, "DW-0791")),
    ("DW-0791b", |s| {
        s.settings.source = Box::new(BoardSettings::new().with_no_recent_alerts());
        open_on(s, "DW-0791");
    }),
    ("DW-0788", |s| open_on(s, "DW-0788")),
    ("DW-0788b", |s| {
        open_on(s, "DW-0788");
        s.settings.choose_location(1);
    }),
    ("DW-0788c", |s| {
        s.settings.source = Box::new(BoardSettings::new().with_location(2, true));
        open_on(s, "DW-0788");
    }),
    ("DW-0788d", |s| {
        s.settings.source = Box::new(BoardSettings::new().with_locations_failed());
        open_on(s, "DW-0788");
    }),
];

fn open_on(s: &mut State, frame: &'static str) {
    s.settings.show(Pane::General);
    go(s, frame);
}

/// The shell's name for a section of this lane's panes.
fn shell_pane(section: Pane) -> SettingsPane {
    match section {
        Pane::Notifications | Pane::Recent => SettingsPane::Notifications,
        _ => SettingsPane::General,
    }
}

/// Go where a row, a button or a sidebar item points, and keep the shell's route in step:
/// the shell owns WHICH section of settings the window shows (and whether it shows settings
/// at all); this lane owns what is open inside a section.
pub fn go(s: &mut State, frame: &'static str) {
    // A sidebar section the shell routes and another lane draws.
    let foreign = match frame {
        "DW-A51" => Some(SettingsPane::Account),
        "DW-A52" => Some(SettingsPane::Security),
        "DW-A55" => Some(SettingsPane::DevicesAndSessions),
        "DW-0712" => Some(SettingsPane::Storage),
        "DW-0737" => Some(SettingsPane::Backups),
        "DW-0746" => Some(SettingsPane::SyncAndFileExplorer),
        "DW-0744" => Some(SettingsPane::ThumbnailsAndPreviews),
        _ => None,
    };
    s.settings.go(frame);
    if let Some(pane) = foreign {
        host::dispatch(s, Event::OpenSettings(pane));
        return;
    }
    let in_settings = matches!(s.shell.app.screen(), Screen::Settings(_));
    if !s.settings.open {
        if in_settings {
            host::dispatch(s, Event::Escape);
        }
        return;
    }
    let want = shell_pane(s.settings.pane.section());
    if s.shell.app.screen() != Screen::Settings(want) {
        host::dispatch(s, Event::OpenSettings(want));
    }
}

/// Show the frame `id` on `s` (a scene). `false`: this group has not built it.
#[cfg(test)]
pub fn scene(id: &str, s: &mut State) -> bool {
    match FRAMES.iter().find(|(frame, _)| *frame == id) {
        Some((_, apply)) => {
            apply(s);
            true
        }
        None => false,
    }
}

impl Pane {
    /// The sidebar section a pane belongs to (a pane opened from General stays under General).
    pub fn section(self) -> Pane {
        match self {
            Pane::General | Pane::About | Pane::StorageLocation => Pane::General,
            Pane::Notifications | Pane::Recent => Pane::Notifications,
        }
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
        pane: Some(Pane::Notifications),
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

fn sidebar(section: Pane, t: &Theme) -> El {
    let items = |sections: &'static [Section]| -> Vec<El> {
        sections
            .iter()
            .map(|sec| {
                let frame = sec.frame;
                shell::nav_item(sec.label, sec.icon, 16.0, sec.pane == Some(section), t)
                    .on_press(move |s: &mut State| go(s, frame))
            })
            .collect()
    };
    let back = shell::nav_item("Back to Drive", &ic::BACK, 15.0, false, t)
        .shortcut_hint("Esc")
        .on_press(|s: &mut State| go(s, "DW-0401"));
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
    // The shell says which section shows. What this lane has open inside a section counts
    // only while it belongs to that section and was opened from here.
    let section = match s.shell.app.screen() {
        Screen::Settings(SettingsPane::Notifications) => Pane::Notifications,
        _ => Pane::General,
    };
    let inside = ui.open && ui.pane.section() == section;
    let pane = if inside { ui.pane } else { section };
    let (pane_el, overlay) = match pane {
        Pane::General => general::pane(ui, &t),
        Pane::About => about::pane(ui, &t),
        Pane::StorageLocation => storage_location::pane(ui, &t),
        Pane::Notifications => notifications::pane(ui, &t),
        Pane::Recent => recent::pane(ui, &t),
    };
    let overlay = overlay.filter(|_| inside);
    let mut body = row()
        .key("body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(sidebar(pane.section(), &t))
        .child(pane_el);
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
    if let Some(Menu { kind, hot }) = ui.menu {
        let last = kind.labels().len() - 1;
        let hot = match ev.key {
            Key::Down => (hot + 1).min(last),
            Key::Up => hot.saturating_sub(1),
            Key::Home => 0,
            Key::End => last,
            Key::Enter => {
                ui.choose(kind, hot);
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
        ui.menu = Some(Menu { kind, hot });
        return true;
    }
    match ev.key {
        Key::Escape if ui.dialog.is_some() => {
            ui.dialog = None;
            true
        }
        Key::Escape => {
            go(s, "DW-0401");
            true
        }
        _ => false,
    }
}
