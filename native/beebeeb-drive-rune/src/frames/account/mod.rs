//! The account inside settings (the board's five account groups: `FRAMES.md` here is the
//! checklist): Account, Sign-in and security, Devices and sessions, and what opens from them.
//!
//! These are sections of the settings view, in the main window (DW-A51: "Not a separate
//! window"). The shell routes the section (`Screen::Settings(Account | Security |
//! DevicesAndSessions)`); this lane keeps only what is open INSIDE a section (`AccountUi`: a
//! page under it, a confirm over it), and forgets it whenever the route leaves the section
//! (`sync`, called from `shell::host::dispatch`).
//!
//! The board draws these panes in the shared account kit (`.au-set`), not in Drive's own
//! settings classes: the same sidebar entries, drawn by `ui::account_set`.
//!
//! Navigation out of this lane goes through `frames::settings::go`, so the shell lane has ONE
//! queue of requests to drain (`SettingsUi::take_intents`).

pub mod icons;
mod overview;
mod security;
mod sessions;
pub mod source;
#[cfg(test)]
mod tests;

use crate::State;
use crate::frames::settings;
use crate::shell::app::{Event, Screen, SettingsPane};
use crate::shell::{host, instance, placeholder, registry};
use crate::theme::tokens::{self, Theme};
use crate::ui::{account_set as set, caption};
use rune::{Cx, El, Icon, Key, KeyEvent, Tx, col, row};

use source::AccountSource;

/// A pane of this lane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    /// Account (DW-A51).
    Account,
    /// Sign-in and security (DW-A52).
    Security,
    /// Security › Security checkup (DW-A84, DW-A84a).
    Checkup,
    /// Devices and sessions (DW-A55).
    Sessions,
}

impl Pane {
    /// The sidebar section a pane belongs to.
    pub fn section(self) -> SettingsPane {
        match self {
            Pane::Account => SettingsPane::Account,
            Pane::Security | Pane::Checkup => SettingsPane::Security,
            Pane::Sessions => SettingsPane::DevicesAndSessions,
        }
    }
    /// The pane a section opens on.
    pub fn root(section: SettingsPane) -> Option<Pane> {
        match section {
            SettingsPane::Account => Some(Pane::Account),
            SettingsPane::Security => Some(Pane::Security),
            SettingsPane::DevicesAndSessions => Some(Pane::Sessions),
            _ => None,
        }
    }
    /// The frame Esc goes back to from a page under a section; `None` for a section itself.
    pub fn parent(self) -> Option<&'static str> {
        match self {
            Pane::Checkup => Some("DW-A52"),
            _ => None,
        }
    }
}

/// A confirm over a pane. A destructive choice is confirmed, never one press.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Dialog {
    /// Sign out one device (DW-A56): the server's id of the device.
    SignOutDevice(String),
    /// Sign out every device but this one (DW-A55's button, "after a confirm").
    SignOutElsewhere,
    /// Sign out of this app only (DW-A57).
    SignOutApp,
    /// Sign out of all Beebeeb apps on this PC (DW-A57a).
    SignOutAllApps,
}

/// What is open inside the account sections. (Which section shows is the shell's route.)
pub struct AccountUi {
    pub pane: Pane,
    pub dialog: Option<Dialog>,
    /// `None`: nothing answers yet (a build that ships before the engine lane wires it); the
    /// view then draws the shell's "not built yet" frame, never sample values.
    pub source: Option<Box<dyn AccountSource>>,
}

impl Default for AccountUi {
    fn default() -> Self {
        Self {
            pane: Pane::Account,
            dialog: None,
            source: source::first(instance::DEV_BUILD),
        }
    }
}

impl AccountUi {
    /// Show `pane` with nothing over it.
    pub fn show(&mut self, pane: Pane) {
        self.pane = pane;
        self.dialog = None;
    }
}

/// The section of this lane the shell's route shows, if it shows one.
fn routed(s: &State) -> Option<SettingsPane> {
    match s.shell.app.screen() {
        Screen::Settings(section) if Pane::root(section).is_some() => Some(section),
        _ => None,
    }
}

/// Brings what this lane holds in line with the shell's route (the route moves without a word
/// to this lane: Ctrl+, a link, Esc): what was open inside another section, or before the
/// window left settings, is forgotten.
pub fn sync(s: &mut State) {
    let section = routed(s);
    if section != Some(s.account.pane.section()) {
        s.account.show(section.and_then(Pane::root).unwrap_or(Pane::Account));
    }
}

/// A board frame of this lane's as a state of it: the pane, and the confirm over it.
fn local(frame: &str) -> Option<(Pane, Option<Dialog>)> {
    Some(match frame {
        "DW-A51" => (Pane::Account, None),
        "DW-A52" => (Pane::Security, None),
        "DW-A57" => (Pane::Security, Some(Dialog::SignOutApp)),
        "DW-A57a" => (Pane::Security, Some(Dialog::SignOutAllApps)),
        "DW-A84" | "DW-A84a" => (Pane::Checkup, None),
        "DW-A55" => (Pane::Sessions, None),
        _ => return None,
    })
}

/// Go where a row, a button or a sidebar item points. A frame of this lane's is shown here
/// (the shell's route follows); any other goes to `frames::settings::go`, which shows its own
/// and asks the shell for the rest.
pub fn go(s: &mut State, frame: &'static str) {
    let Some((pane, dialog)) = local(frame) else {
        s.account.dialog = None;
        settings::go(s, frame);
        return;
    };
    let want = pane.section();
    if s.shell.app.screen() != Screen::Settings(want) {
        host::dispatch(s, Event::OpenSettings(want));
    }
    s.account.show(pane);
    s.account.dialog = dialog;
}

/// Ask to sign out the device `id` (DW-A56).
pub fn confirm_device(s: &mut State, id: String) {
    go(s, "DW-A55");
    s.account.dialog = Some(Dialog::SignOutDevice(id));
}

fn open_on(s: &mut State, frame: &'static str) {
    s.account.source = Some(Box::new(source::BoardAccount::new()));
    go(s, frame);
}

/// The frames of this group that are built: the id, and what shows it on a fresh `State`.
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-A51", |s| open_on(s, "DW-A51")),
    ("DW-A52", |s| open_on(s, "DW-A52")),
    ("DW-A84", |s| open_on(s, "DW-A84")),
    ("DW-A84a", |s| {
        open_on(s, "DW-A84a");
        s.account.source = Some(Box::new(source::BoardAccount::new().without_passkeys()));
    }),
    ("DW-A55", |s| open_on(s, "DW-A55")),
    ("DW-A56", |s| {
        open_on(s, "DW-A55");
        confirm_device(s, "dev-iphone".into());
    }),
    ("DW-A57", |s| open_on(s, "DW-A57")),
    ("DW-A57a", |s| open_on(s, "DW-A57a")),
];

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

/// One entry of the settings sidebar: its label, its icon, the frame it shows, and the
/// section it is (for the selection), when it is one of this lane's.
struct Section {
    label: &'static str,
    icon: &'static Icon,
    frame: &'static str,
    section: Option<SettingsPane>,
}

const fn entry(
    label: &'static str,
    icon: &'static Icon,
    frame: &'static str,
    section: Option<SettingsPane>,
) -> Section {
    Section {
        label,
        icon,
        frame,
        section,
    }
}

const SETTINGS_SECTIONS: &[Section] = &[
    entry("General", &icons::GEAR, "DW-0701", None),
    entry("Account", &icons::PEOPLE, "DW-A51", Some(SettingsPane::Account)),
    entry("Security", &icons::SHIELD, "DW-A52", Some(SettingsPane::Security)),
    entry(
        "Devices and sessions",
        &icons::DEVICE_PHONE,
        "DW-A55",
        Some(SettingsPane::DevicesAndSessions),
    ),
    entry("Notifications", &icons::BELL, "DW-0750", None),
];

// The board draws the Drive group's items with a chevron where the other group has a glyph.
const DRIVE_SECTIONS: &[Section] = &[
    entry("Storage", &icons::CHEV_RIGHT, "DW-0712", None),
    entry("Backups", &icons::CHEV_RIGHT, "DW-0737", None),
    entry("Sync and File Explorer", &icons::CHEV_RIGHT, "DW-0746", None),
    entry("Thumbnails and previews", &icons::CHEV_RIGHT, "DW-0744", None),
    entry("Bandwidth", &icons::CHEV_RIGHT, "DW-0785", None),
];

fn sidebar(section: SettingsPane, t: &Theme) -> El {
    let items = |sections: &'static [Section]| -> Vec<El> {
        sections
            .iter()
            .map(|sec| {
                let frame = sec.frame;
                set::nav_item(sec.label, sec.icon, sec.section == Some(section), t)
                    .on_press(move |s: &mut State| go(s, frame))
            })
            .collect()
    };
    let back = set::nav_back("Back to Drive", &icons::BACK, t)
        .shortcut_hint("Esc")
        .on_press(|s: &mut State| go(s, "DW-0401"));
    set::sidebar(
        back,
        vec![("Settings", items(SETTINGS_SECTIONS)), ("Drive", items(DRIVE_SECTIONS))],
        "Settings open inside the app. Esc goes back.",
        t,
    )
}

/// The window while one of the account sections is open.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let ui = &s.account;
    let Some(source) = ui.source.as_deref() else {
        return placeholder::window(s, cx, registry::group_for(&s.shell.app));
    };
    // Keyboard as Windows has it: Tab visits every control, Space and Enter press the focused one.
    cx.web_focus();
    let t = tokens::theme(s.settings.dark(cx.system_dark()));
    // The shell says which section shows. What this lane has open inside a section counts only
    // while it belongs to that section (`sync` forgets the rest on the next input).
    let section = routed(s).unwrap_or(SettingsPane::Account);
    let inside = ui.pane.section() == section;
    let pane = if inside {
        ui.pane
    } else {
        Pane::root(section).unwrap_or(Pane::Account)
    };
    let dialog = ui.dialog.as_ref().filter(|_| inside);
    let (pane_el, overlay) = match pane {
        Pane::Account => (overview::pane(source, &t), None),
        Pane::Security => security::pane(source, dialog, &t),
        Pane::Checkup => (security::checkup(source, &t), None),
        Pane::Sessions => sessions::pane(source, dialog, &t),
    };
    let mut body = row()
        .key("body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(sidebar(section, &t))
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

/// The account sections' keyboard. Esc closes what is open, innermost first: a confirm, a page
/// under a section, then settings. Everything else is Rune's (Tab, Space, Enter on the focused
/// control; in a confirm Enter is its default button, the safe choice).
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    sync(s);
    if ev.key != Key::Escape {
        return false;
    }
    if s.account.dialog.take().is_some() {
        return true;
    }
    go(s, s.account.pane.parent().unwrap_or("DW-0401"));
    true
}
