//! Settings, inside the main window (the board's three settings groups: `FRAMES.md` here is
//! the checklist).
//!
//! Settings is a state of the main window, never a window of its own (DW-0701): the caption
//! stays, the settings sidebar and one pane take the place of the Drive sidebar and the list.
//! `State::settings` holds that state; `view` draws it; `on_key` is its keyboard.
//!
//! **Whether settings shows, and which section, is the shell's route** (`Screen::Settings`):
//! this lane keeps no flag of its own for it. `SettingsUi` holds only what is open INSIDE a
//! section (a sub-pane, a menu, a dialog, a toast), and it is forgotten whenever the window
//! leaves settings, whoever asked (`SettingsUi::reset`).
//!
//! For the shell lane: `FRAMES` is the table of what is built (frame id, and the scene that
//! shows it on a fresh `State`); `SettingsUi::take_intents` is what settings asks the shell to
//! do (show a frame another lane owns, open a web page, start a download).

pub mod about;
pub mod bandwidth;
pub mod bandwidth_data;
pub mod drive;
pub mod general;
pub mod insights;
pub mod notifications;
pub mod recent;
pub mod source;
pub mod storage;
pub mod storage_location;
pub mod sync;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_bandwidth;
#[cfg(test)]
mod tests_drive;

use crate::State;
use crate::shell::app::{Event, Screen, SettingsPane};
use crate::shell::{host, instance, placeholder, registry};
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{self, Theme};
use crate::ui::{caption, settings_controls as controls, settings_shell as shell};
use rune::{Align, Cx, El, Key, KeyEvent, Side, Tx, col, row};

use drive::{DriveSettings, Kept};
use source::{AlertTarget, BoardSettings, Channel, SettingsSource, ThemeChoice};

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
    /// Storage (DW-0712).
    Storage,
    /// Storage › Storage insights (DW-0783).
    Insights,
    /// Sync and File Explorer (DW-0746).
    Sync,
    /// Bandwidth (DW-0785), with the speed test (DW-0786).
    Bandwidth,
}

/// Which menu a pop-up button drops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKind {
    /// General › Appearance › Theme (DW-0787).
    Theme,
    /// About › Updates › Release channel (DW-0789).
    Channel,
    /// Bandwidth › Limits › Upload, Download (DW-0785d).
    SpeedLimit(bandwidth_data::Direction),
}

impl MenuKind {
    pub fn labels(self) -> Vec<&'static str> {
        match self {
            Self::Theme => ThemeChoice::ALL.iter().map(|c| c.label()).collect(),
            Self::Channel => Channel::ALL.iter().map(|c| c.label()).collect(),
            Self::SpeedLimit(_) => bandwidth_data::PRESET_LABELS.to_vec(),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Channel => "Release channel",
            Self::SpeedLimit(bandwidth_data::Direction::Upload) => "Upload limit",
            Self::SpeedLimit(bandwidth_data::Direction::Download) => "Download limit",
        }
    }
    /// The key of the point in the menu's own row where its top-left corner goes
    /// (`settings_list::Trailing::Pop::menu_at`): the menu follows its row when the pane scrolls
    /// or the window is another size.
    pub fn anchor(self) -> &'static str {
        match self {
            Self::Theme => "item-0-0-menu-at",
            Self::Channel => "item-1-1-menu-at",
            Self::SpeedLimit(bandwidth_data::Direction::Upload) => bandwidth::UPLOAD_MENU_AT,
            Self::SpeedLimit(bandwidth_data::Direction::Download) => bandwidth::DOWNLOAD_MENU_AT,
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
    /// Storage › Change limit (DW-0713, DW-0714).
    Limit(Kept),
    /// Clear… downloaded files (DW-0780).
    FreeUp,
    /// Turn off Beebeeb in File Explorer… (DW-0746b).
    TurnOffExplorer,
    /// Repair… (DW-0746c).
    Repair,
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
    /// Show this file in Files, selected: its whole path (DW-0783, a largest file).
    ShowInFiles(String),
}

/// The most requests kept for the shell while nothing takes them: the oldest fall off.
pub const INTENT_LIMIT: usize = 32;

/// What is open inside settings. (Whether the window shows settings is the shell's route.)
pub struct SettingsUi {
    pub pane: Pane,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    /// What a toast over the pane says, until the next thing happens.
    pub toast: Option<&'static str>,
    pub source: Box<dyn SettingsSource>,
    pub intents: Vec<Intent>,
    /// What Drive's own panes show (Storage, Sync and File Explorer, Bandwidth, ...).
    pub drive: Box<dyn DriveSettings>,
    /// The limit a limit dialog's slider stands on, until Save (DW-0713, DW-0714).
    pub draft_gb: u32,
    /// What the Bandwidth pane holds between draws: the chart's hovered bucket (DW-0785).
    pub bandwidth: bandwidth::Held,
}

impl Default for SettingsUi {
    fn default() -> Self {
        Self {
            pane: Pane::General,
            menu: None,
            dialog: None,
            toast: None,
            // Sample content only where tooling runs; a build that ships starts unwired.
            source: source::first(instance::DEV_BUILD),
            intents: Vec::new(),
            drive: drive::first(instance::DEV_BUILD),
            draft_gb: 0,
            bandwidth: bandwidth::Held::default(),
        }
    }
}

impl SettingsUi {
    /// The window left settings (Back to Drive, Esc, another section's lane, a link): nothing
    /// stays open for the next visit, which starts on the section's own pane.
    pub fn reset(&mut self) {
        self.show(Pane::General);
    }

    /// Show `pane` with nothing over it. Leaving Recent notifications marks its alerts seen
    /// (DW-0791: "New ones are marked until this page has been opened").
    pub fn show(&mut self, pane: Pane) {
        if self.pane == Pane::Recent && pane != Pane::Recent {
            self.source.mark_recent_seen();
        }
        self.pane = pane;
        self.bandwidth.tip = None;
        self.menu = None;
        self.dialog = None;
        self.toast = None;
    }

    /// Open a limit dialog on the stored limit (DW-0713, DW-0714).
    pub fn open_limit(&mut self, which: Kept) {
        let st = self.drive.storage();
        self.draft_gb = match which {
            Kept::Downloaded => st.downloaded.limit.gb,
            Kept::Previews => st.previews.limit.gb,
        };
        self.dialog = Some(Dialog::Limit(which));
    }

    /// What settings asked of the shell since the last call, oldest first.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    pub fn ask(&mut self, intent: Intent) {
        self.menu = None;
        // Nothing takes these yet (the shell lane's); never more than the newest few are kept.
        if self.intents.len() >= INTENT_LIMIT {
            self.intents.remove(0);
        }
        self.intents.push(intent);
    }

    /// About › Check now: the row says Checking… until the updater answers.
    pub fn check_for_updates(&mut self) {
        self.source.check_for_updates();
        self.ask(Intent::CheckForUpdates);
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
            // A saved limit that is no preset is the menu's ninth item: Enter on it changes nothing.
            MenuKind::SpeedLimit(direction) => {
                bandwidth::chosen(self, direction).unwrap_or(bandwidth_data::PRESETS.len())
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

    /// Take item `index` of a menu, and close it. An index the menu does not have chooses nothing.
    pub fn choose(&mut self, kind: MenuKind, index: usize) {
        match kind {
            MenuKind::Theme => {
                if let Some(theme) = ThemeChoice::ALL.get(index) {
                    self.source.set_theme(*theme);
                }
            }
            MenuKind::Channel => {
                if let Some(channel) = Channel::ALL.get(index) {
                    self.source.set_channel(*channel);
                }
            }
            MenuKind::SpeedLimit(direction) => {
                if let Some(limit) = bandwidth_data::PRESETS.get(index) {
                    self.drive.set_speed_limit(direction, *limit);
                }
            }
        }
        self.menu = None;
    }
}

/// The menu `kind`, when it is the open one: its top-left corner on the point its row marks
/// (`MenuKind::anchor`), so it stays with its pop-up button wherever the row is drawn.
pub fn menu_overlay(ui: &SettingsUi, kind: MenuKind, t: &Theme) -> Option<El> {
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
            .anchored(kind.anchor(), Side::Bottom, Align::Start, 0.0)
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
    ("DW-0746", |s| open_on(s, "DW-0746")),
    ("DW-0746b", |s| open_on(s, "DW-0746b")),
    ("DW-0746c", |s| open_on(s, "DW-0746c")),
    ("DW-0746d", |s| {
        s.settings.drive = Box::new(drive::BoardDrive::new().with_repair_failed());
        open_on(s, "DW-0746");
    }),
    ("DW-0746e", |s| {
        s.settings.drive = Box::new(drive::BoardDrive::new().with_metered_wait());
        open_on(s, "DW-0746");
    }),
    ("DW-0780", |s| open_on(s, "DW-0780")),
    ("DW-0712", |s| open_on(s, "DW-0712")),
    ("DW-0713", |s| open_on(s, "DW-0713")),
    ("DW-0714", |s| open_on(s, "DW-0714")),
    ("DW-0783", |s| open_on(s, "DW-0783")),
    ("DW-0783b", |s| {
        s.settings.drive = Box::new(drive::BoardDrive::new().with_insights(drive::Insights::Loading));
        open_on(s, "DW-0783");
    }),
    ("DW-0783c", |s| {
        s.settings.drive = Box::new(
            drive::BoardDrive::new().with_insights(drive::Insights::Failed("Can't reach Beebeeb right now.".into())),
        );
        open_on(s, "DW-0783");
    }),
    ("DW-0783d", |s| {
        s.settings.drive = Box::new(drive::BoardDrive::new().with_insights(drive::Insights::Empty));
        open_on(s, "DW-0783");
    }),
    ("DW-0785", |s| bandwidth::scene(s, "DW-0785")),
    ("DW-0785b", |s| bandwidth::scene(s, "DW-0785b")),
    ("DW-0785c", |s| bandwidth::scene(s, "DW-0785c")),
    ("DW-0785d", |s| bandwidth::scene(s, "DW-0785d")),
    ("DW-0785e", |s| bandwidth::scene(s, "DW-0785e")),
    ("DW-0785f", |s| bandwidth::scene(s, "DW-0785f")),
    ("DW-0786", |s| bandwidth::scene(s, "DW-0786")),
    ("DW-0786b", |s| bandwidth::scene(s, "DW-0786b")),
    ("DW-0786c", |s| bandwidth::scene(s, "DW-0786c")),
    ("DW-0786d", |s| bandwidth::scene(s, "DW-0786d")),
];

pub(super) fn open_on(s: &mut State, frame: &'static str) {
    s.settings.reset();
    go(s, frame);
}

/// The section of this lane the shell's route shows, if it shows one.
fn routed(s: &State) -> Option<Pane> {
    match s.shell.app.screen() {
        Screen::Settings(SettingsPane::General) => Some(Pane::General),
        Screen::Settings(SettingsPane::Notifications) => Some(Pane::Notifications),
        Screen::Settings(SettingsPane::Storage) => Some(Pane::Storage),
        Screen::Settings(SettingsPane::SyncAndFileExplorer) => Some(Pane::Sync),
        Screen::Settings(SettingsPane::Bandwidth) => Some(Pane::Bandwidth),
        _ => None,
    }
}

/// A board frame of this lane's as a state of settings: the pane, and what is open over it.
fn local(frame: &str) -> Option<(Pane, Option<Dialog>, Option<MenuKind>)> {
    Some(match frame {
        "DW-0701" | "DW-0781" => (Pane::General, None, None),
        "DW-0782" => (Pane::General, Some(Dialog::ReportProblem), None),
        "DW-0787" => (Pane::General, None, Some(MenuKind::Theme)),
        "DW-0751" => (Pane::About, None, None),
        "DW-0789" => (Pane::About, None, Some(MenuKind::Channel)),
        "DW-0789b" => (Pane::About, Some(Dialog::ReleaseNotes), None),
        "DW-0750" | "DW-0750b" => (Pane::Notifications, None, None),
        "DW-0791" | "DW-0791b" => (Pane::Recent, None, None),
        "DW-0788" | "DW-0788b" | "DW-0788c" | "DW-0788d" => (Pane::StorageLocation, None, None),
        "DW-0746" | "DW-0746d" | "DW-0746e" => (Pane::Sync, None, None),
        "DW-0746b" => (Pane::Sync, Some(Dialog::TurnOffExplorer), None),
        "DW-0746c" => (Pane::Sync, Some(Dialog::Repair), None),
        "DW-0780" => (Pane::Sync, Some(Dialog::FreeUp), None),
        "DW-0712" => (Pane::Storage, None, None),
        "DW-0713" => (Pane::Storage, Some(Dialog::Limit(Kept::Previews)), None),
        "DW-0714" => (Pane::Storage, Some(Dialog::Limit(Kept::Downloaded)), None),
        "DW-0783" | "DW-0783b" | "DW-0783c" | "DW-0783d" => (Pane::Insights, None, None),
        "DW-0785" | "DW-0785b" | "DW-0785c" | "DW-0785e" | "DW-0785f" => (Pane::Bandwidth, None, None),
        "DW-0785d" => (
            Pane::Bandwidth,
            None,
            Some(MenuKind::SpeedLimit(bandwidth_data::Direction::Upload)),
        ),
        "DW-0786" | "DW-0786b" | "DW-0786c" | "DW-0786d" => (Pane::Bandwidth, None, None),
        _ => return None,
    })
}

/// Brings what this lane holds in line with the shell's route. The shell calls it after every
/// change of route (`shell::host::dispatch`), because the route moves without a word to this
/// lane (Ctrl+, a link, the b's menu, Esc): what was open inside ANOTHER section, or before
/// the window left settings, is forgotten, so nothing is still open on the next visit.
pub fn sync(s: &mut State) {
    match routed(s) {
        Some(section) if s.settings.pane.section() == section => {}
        Some(section) => s.settings.show(section),
        None => s.settings.reset(),
    }
}

/// The frame a recent alert leads to.
pub fn frame_of(target: AlertTarget) -> &'static str {
    match target {
        AlertTarget::DevicesAndSessions => "DW-A55",
        AlertTarget::Shared => "DW-0501",
        AlertTarget::SharedRequests => "DW-0503",
        AlertTarget::SecurityLog => "DW-A77",
    }
}

/// The shell's name for a section of this lane's panes.
fn shell_pane(section: Pane) -> SettingsPane {
    match section {
        Pane::Notifications | Pane::Recent => SettingsPane::Notifications,
        Pane::Storage | Pane::Insights => SettingsPane::Storage,
        Pane::Sync => SettingsPane::SyncAndFileExplorer,
        Pane::Bandwidth => SettingsPane::Bandwidth,
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
        "DW-0737" => Some(SettingsPane::Backups),
        "DW-0744" => Some(SettingsPane::ThumbnailsAndPreviews),
        _ => None,
    };
    if let Some(pane) = foreign {
        s.settings.reset();
        host::dispatch(s, Event::OpenSettings(pane));
        return;
    }
    if frame == "DW-0401" {
        s.settings.reset();
        // Only settings is this lane's to close: in Drive, Esc belongs to what is open there.
        if matches!(s.shell.app.screen(), Screen::Settings(_)) {
            host::dispatch(s, Event::Escape);
        }
        return;
    }
    let Some((pane, dialog, menu)) = local(frame) else {
        // Not a frame of this lane's: asked of the shell, and the window stays where it is.
        let ui = &mut s.settings;
        (ui.dialog, ui.toast) = (None, None);
        ui.ask(Intent::Show(frame));
        return;
    };
    // The shell's route first (it follows the pane's section), then what is open inside it.
    let want = shell_pane(pane.section());
    if s.shell.app.screen() != Screen::Settings(want) {
        host::dispatch(s, Event::OpenSettings(want));
    }
    let ui = &mut s.settings;
    ui.show(pane);
    match dialog {
        Some(Dialog::Limit(which)) => ui.open_limit(which),
        other => ui.dialog = other,
    }
    if let Some(kind) = menu {
        ui.toggle_menu(kind);
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
            Pane::Storage | Pane::Insights => Pane::Storage,
            Pane::Sync => Pane::Sync,
            Pane::Bandwidth => Pane::Bandwidth,
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
        pane: Some(Pane::Storage),
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
        // The board draws no sidebar item selected on its Sync and File Explorer frames
        // (DW-0746 and the nine over it), unlike every other section: followed, and flagged.
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
        pane: Some(Pane::Bandwidth),
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
    // Nothing answers for settings yet (a build that ships before the engine lane wires it):
    // the shell's "not built yet" frame, never a pane of sample values.
    if !ui.source.wired() {
        return placeholder::window(s, cx, registry::group_for(&s.shell.app));
    }
    // Keyboard as Windows has it: Tab visits every control, Space and Enter press the focused
    // one (Rune's default leaves Enter on a button to the default button, as macOS does).
    cx.web_focus();
    let t = tokens::theme(ui.dark(cx.system_dark()));
    // The shell says which section shows. What this lane has open inside a section counts
    // only while it belongs to that section (`sync` forgets the rest on the next input).
    let section = routed(s).unwrap_or(Pane::General);
    let inside = ui.pane.section() == section;
    let pane = if inside { ui.pane } else { section };
    let (pane_el, overlay) = match pane {
        Pane::General => general::pane(ui, &t),
        Pane::About => about::pane(ui, &t),
        Pane::StorageLocation => storage_location::pane(ui, &t),
        Pane::Notifications => notifications::pane(ui, &t),
        Pane::Recent => recent::pane(ui, &t),
        // Drive's own panes: never sample values in a build nothing answers for yet.
        _ if !ui.drive.wired() => return placeholder::window(s, cx, registry::group_for(&s.shell.app)),
        Pane::Storage => storage::pane(ui, &t),
        Pane::Insights => insights::pane(ui, &t),
        Pane::Sync => sync::pane(ui, &t),
        Pane::Bandwidth => bandwidth::pane(ui, &t),
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
        .label(format!("{}, Settings", s.identity.main_window_title()))
        .on_key(on_key)
        .w_full()
        .h_full()
        .fill(t.surface)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(body)
}

/// Settings' keyboard. Esc closes what is open, innermost first: a menu, then settings.
/// A menu takes the arrows, Home, End and Enter. Everything else is Rune's (Tab, Space,
/// Enter on the focused control).
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    sync(s);
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
