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

mod activity;
pub mod approve;
#[cfg(test)]
mod approve_tests;
mod delete;
mod form;
mod forms;
pub mod icons;
mod overview;
mod passkeys;
mod request;
mod security;
mod sessions;
pub mod source;
#[cfg(test)]
mod tests;
mod two_factor;

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
    /// Account › Name (DW-A64).
    Name,
    /// Account › Change email (DW-A65), and its code (DW-A66).
    Email,
    EmailCode,
    /// Security › Change password (DW-A67, DW-A68).
    Password,
    /// Security › Two-factor: its status (DW-A71), setting it up (DW-A69), the backup codes (DW-A70).
    TwoFactor,
    TwoFactorSetup,
    BackupCodes,
    /// Security › Passkeys (DW-A73, DW-A74).
    Passkeys,
    /// Security › Sign-in activity (DW-A77, DW-A79).
    Activity,
    /// Security › Delete the account, its four steps (DW-A58, DW-A80, DW-A81, DW-A82) and the
    /// result of undoing it (DW-A83).
    Delete,
    DeleteFactor,
    DeleteLink,
    DeleteScheduled,
    DeleteUndone,
    /// Security › Approve a sign-in: the code (DW-A12).
    ApproveCode,
    /// The request under review and how it ended (DW-A13 to DW-A17), over Sign-in and security.
    ApproveReview,
}

impl Pane {
    /// The sidebar section a pane belongs to.
    pub fn section(self) -> SettingsPane {
        match self {
            Pane::Account | Pane::Name | Pane::Email | Pane::EmailCode => SettingsPane::Account,
            Pane::Sessions => SettingsPane::DevicesAndSessions,
            _ => SettingsPane::Security,
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
    /// Whether this pane shows key material: a password field, a 2FA secret/QR, backup codes,
    /// or a recovery phrase. Used for screen-capture exclusion (task 2460). A new variant is a
    /// compile error (exhaustive match).
    pub fn shows_secret(self) -> bool {
        match self {
            // DW-A65: change email asks for the password.
            Pane::Email => true,
            // DW-A67, DW-A68: change password (current + new).
            Pane::Password => true,
            // DW-A69: 2FA setup shows the secret key and QR code.
            Pane::TwoFactorSetup => true,
            // DW-A70: the 8 single-use backup codes, shown once.
            Pane::BackupCodes => true,
            // DW-A58: delete step 1 asks for the password.
            Pane::Delete => true,
            // DW-A80: delete step 2 asks for the 2FA code.
            Pane::DeleteFactor => true,
            // DW-A12: the code that lets another device receive the account's keys.
            Pane::ApproveCode => true,
            // What is over it decides (`Dialog::shows_secret`: DW-A13 and DW-A14 hold the code).
            Pane::ApproveReview => false,
            // The rest: overviews, lists, confirmations without key material.
            Pane::Account
            | Pane::Security
            | Pane::Checkup
            | Pane::Sessions
            | Pane::Name
            | Pane::EmailCode
            | Pane::TwoFactor
            | Pane::Passkeys
            | Pane::Activity
            | Pane::DeleteLink
            | Pane::DeleteScheduled
            | Pane::DeleteUndone => false,
        }
    }

    /// The frame Esc goes back to from a page under a section; `None` for a section itself.
    pub fn parent(self) -> Option<&'static str> {
        match self {
            Pane::Account | Pane::Security | Pane::Sessions => None,
            Pane::Name | Pane::Email => Some("DW-A51"),
            Pane::EmailCode => Some("DW-A65"),
            Pane::BackupCodes => Some("DW-A71"),
            Pane::DeleteFactor => Some("DW-A58"),
            Pane::DeleteUndone => Some("DW-0401"),
            _ => Some("DW-A52"),
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
    /// A change that a current two-factor code confirms (DW-A72, DW-A78).
    Code(CodeFor),
    /// The menu on one passkey (DW-A75): the server's id of the passkey.
    PasskeyMenu(String),
    /// Remove a passkey (DW-A76).
    RemovePasskey(String),
    /// Leaving the backup codes without "I've saved them" (DW-A70's rule).
    LeaveCodes,
    /// Approving a sign-in: the review with its code field (DW-A13), Windows Hello asking
    /// (DW-A14), and the three ends (DW-A15, DW-A16, DW-A17; `handled`: "Already handled").
    ApproveReview,
    ApproveHello,
    ApproveApproved,
    ApproveDenied,
    ApproveExpired {
        handled: bool,
    },
}

impl Dialog {
    /// Whether this dialog shows or takes key material (a 2FA code). Used for screen-capture
    /// exclusion (task 2460): a secret dialog over a non-secret pane sets the exclusion.
    pub fn shows_secret(&self) -> bool {
        match self {
            // DW-A72, DW-A78: takes a current 2FA code.
            Dialog::Code(_) => true,
            // DW-A13, DW-A14: the approval code is typed and shown.
            Dialog::ApproveReview | Dialog::ApproveHello => true,
            // DW-A15 to DW-A17: results; the code is wiped before they show.
            Dialog::ApproveApproved | Dialog::ApproveDenied | Dialog::ApproveExpired { .. } => false,
            // Confirmations without key material.
            Dialog::SignOutDevice(_)
            | Dialog::SignOutElsewhere
            | Dialog::SignOutApp
            | Dialog::SignOutAllApps
            | Dialog::PasskeyMenu(_)
            | Dialog::RemovePasskey(_)
            | Dialog::LeaveCodes => false,
        }
    }
}

/// What a code in a confirm is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodeFor {
    /// Turn two-factor off (DW-A72).
    TurnOffTwoFactor,
    /// Make new backup codes (DW-A71: "Asks for a code, then shows 8 new codes").
    NewBackupCodes,
    /// Move to a new authenticator (DW-A71: "Asks for the current code, then the set-up screen").
    MoveAuthenticator,
    /// Stop security logging (DW-A78).
    StopActivity,
}

/// What is open inside the account sections. (Which section shows is the shell's route.)
pub struct AccountUi {
    pub pane: Pane,
    pub dialog: Option<Dialog>,
    /// What is typed and shown on the pages with a field, a code or a secret.
    pub form: form::Form,
    /// Approving a sign-in (DW-A12 to DW-A17): the code, the request, the wait on Windows Hello.
    pub approve: approve::Approve,
    /// `None`: nothing answers yet (a build that ships before the engine lane wires it); the
    /// view then draws the shell's "not built yet" frame, never sample values.
    pub source: Option<Box<dyn AccountSource>>,
}

impl Default for AccountUi {
    fn default() -> Self {
        Self {
            pane: Pane::Account,
            dialog: None,
            form: form::Form::default(),
            approve: approve::Approve::default(),
            source: source::first(instance::DEV_BUILD),
        }
    }
}

impl AccountUi {
    /// Show `pane` with nothing over it.
    /// Whether the current pane or dialog shows key material (task 2460).
    pub fn shows_secret(&self) -> bool {
        self.pane.shows_secret() || self.dialog.as_ref().is_some_and(|d| d.shows_secret())
    }

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
        // What was typed, and every secret a page held, goes with the page.
        s.account.form = form::Form::default();
        approve::leave(&mut s.account);
    }
}

/// The session ended under what was open (the app locked, the account signed out, another
/// account signed in): every flow of this lane ends. What was typed is wiped, a Windows Hello
/// that is up is cancelled (its late answer finds no wait), the request under review is
/// forgotten, and the section is back on its list, so the person starts again from there.
pub fn end_flows(s: &mut State) {
    let ui = &mut s.account;
    approve::leave(ui);
    ui.form = form::Form::default();
    ui.show(Pane::root(ui.pane.section()).unwrap_or(Pane::Account));
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
        "DW-A64" => (Pane::Name, None),
        "DW-A65" => (Pane::Email, None),
        "DW-A66" => (Pane::EmailCode, None),
        "DW-A67" | "DW-A68" => (Pane::Password, None),
        "DW-A69" => (Pane::TwoFactorSetup, None),
        "DW-A70" => (Pane::BackupCodes, None),
        "DW-A71" => (Pane::TwoFactor, None),
        "DW-A72" => (Pane::TwoFactor, Some(Dialog::Code(CodeFor::TurnOffTwoFactor))),
        "DW-A73" | "DW-A74" => (Pane::Passkeys, None),
        "DW-A77" | "DW-A79" => (Pane::Activity, None),
        "DW-A78" => (Pane::Activity, Some(Dialog::Code(CodeFor::StopActivity))),
        "DW-A58" => (Pane::Delete, None),
        "DW-A80" => (Pane::DeleteFactor, None),
        "DW-A81" => (Pane::DeleteLink, None),
        "DW-A82" => (Pane::DeleteScheduled, None),
        "DW-A83" => (Pane::DeleteUndone, None),
        "DW-A12" => (Pane::ApproveCode, None),
        "DW-A13" => (Pane::ApproveReview, Some(Dialog::ApproveReview)),
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
    form::arrive(&mut s.account);
    approve::arrive(&mut s.account);
}

/// A handler of a page: it works on what this lane holds, and names the frame to show next
/// (`None`: stay).
pub(super) fn act(f: fn(&mut AccountUi) -> Option<&'static str>) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(frame) = f(&mut s.account) {
            go(s, frame);
        }
    }
}

/// Open the confirm `dialog` over the page that shows now.
pub(super) fn ask(s: &mut State, dialog: Dialog) {
    s.account.dialog = Some(dialog);
    form::arrive(&mut s.account);
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
/// A page whose board frame shows typed text gets that text; the board draws most of them at
/// a later moment than their opening (fields filled, no field in focus).
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
    ("DW-A64", |s| open_on(s, "DW-A64")),
    ("DW-A65", |s| {
        open_on(s, "DW-A65");
        let f = &mut s.account.form;
        form::type_in(&mut f.email, "anna.visser@northwind.studio");
        form::type_in(&mut f.current, "............");
    }),
    ("DW-A66", |s| {
        s.account.source = Some(Box::new(
            source::BoardAccount::new().changing_email_to("anna.visser@northwind.studio"),
        ));
        go(s, "DW-A65");
        form::type_in(&mut s.account.form.email, "anna.visser@northwind.studio");
        go(s, "DW-A66");
        form::type_in(&mut s.account.form.code, "4827");
    }),
    ("DW-A67", |s| {
        open_on(s, "DW-A67");
        form::fill_passwords(&mut s.account.form);
    }),
    ("DW-A68", |s| {
        open_on(s, "DW-A68");
        form::fill_passwords(&mut s.account.form);
        s.account.form.problem = Some(form::Problem::WrongPassword);
    }),
    ("DW-A69", |s| {
        open_on(s, "DW-A69");
        form::type_in(&mut s.account.form.code, "51");
    }),
    ("DW-A70", |s| {
        open_on(s, "DW-A69");
        form::type_in(&mut s.account.form.code, source::BOARD_CODE);
        if let Some(frame) = two_factor::turn_on(&mut s.account) {
            go(s, frame);
        }
    }),
    ("DW-A71", |s| open_on(s, "DW-A71")),
    ("DW-A72", |s| {
        open_on(s, "DW-A72");
        form::type_in(&mut s.account.form.code, "51");
    }),
    ("DW-A73", |s| open_on(s, "DW-A73")),
    ("DW-A74", |s| {
        open_on(s, "DW-A74");
        s.account.source = Some(Box::new(source::BoardAccount::new().without_passkeys()));
    }),
    ("DW-A75", |s| {
        open_on(s, "DW-A73");
        ask(s, Dialog::PasskeyMenu("pk-icloud".into()));
    }),
    ("DW-A76", |s| {
        open_on(s, "DW-A73");
        ask(s, Dialog::RemovePasskey("pk-icloud".into()));
    }),
    ("DW-A77", |s| open_on(s, "DW-A77")),
    ("DW-A78", |s| {
        open_on(s, "DW-A78");
        form::type_in(&mut s.account.form.code, "51");
    }),
    ("DW-A79", |s| {
        open_on(s, "DW-A79");
        s.account.source = Some(Box::new(source::BoardAccount::new().without_activity()));
    }),
    ("DW-A58", |s| {
        open_on(s, "DW-A58");
        form::type_in(&mut s.account.form.current, "............");
        s.account.form.current.focused = false;
    }),
    ("DW-A80", |s| {
        open_on(s, "DW-A58");
        form::type_in(&mut s.account.form.current, source::BOARD_PASSWORD);
        if let Some(frame) = delete::check_password(&mut s.account) {
            go(s, frame);
        }
        form::type_in(&mut s.account.form.code, "51");
    }),
    ("DW-A81", |s| {
        s.account.source = Some(Box::new(
            source::BoardAccount::new().with_deletion(source::Deletion::LinkSent),
        ));
        go(s, "DW-A81");
    }),
    ("DW-A82", |s| {
        s.account.source = Some(Box::new(source::BoardAccount::new().with_deletion(
            source::Deletion::Scheduled {
                on: "8 November at 19:42".into(),
            },
        )));
        go(s, "DW-A82");
    }),
    ("DW-A83", |s| open_on(s, "DW-A83")),
    ("DW-A12", |s| approve::scene(s, "DW-A12")),
    ("DW-A13", |s| approve::scene(s, "DW-A13")),
    ("DW-A14", |s| approve::scene(s, "DW-A14")),
    ("DW-A15", |s| approve::scene(s, "DW-A15")),
    ("DW-A16", |s| approve::scene(s, "DW-A16")),
    ("DW-A17", |s| approve::scene(s, "DW-A17")),
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
    if pane == Pane::DeleteUndone {
        // The result of undoing a deletion is a screen of its own (the account window's
        // layout, DW-A83), not a pane of settings.
        return delete::undone(s.identity.product_name(), &t, cx);
    }
    let form = &ui.form;
    let (pane_el, overlay) = match pane {
        Pane::Account => (overview::pane(source, &t), None),
        Pane::Security => security::pane(source, dialog, &t),
        Pane::Checkup => (security::checkup(source, &t), None),
        Pane::Sessions => sessions::pane(source, dialog, &t),
        Pane::Name => (forms::name(form, &t), None),
        Pane::Email => (forms::email(form, &t), None),
        Pane::EmailCode => (forms::email_code(source, form, &t), None),
        Pane::Password => (forms::password(source, form, &t), None),
        Pane::TwoFactor | Pane::TwoFactorSetup | Pane::BackupCodes => two_factor::pane(pane, source, form, dialog, &t),
        Pane::Passkeys => passkeys::pane(source, form, dialog, &t),
        Pane::Activity => activity::pane(source, form, dialog, &t),
        Pane::Delete | Pane::DeleteFactor | Pane::DeleteLink | Pane::DeleteScheduled | Pane::DeleteUndone => {
            (delete::pane(pane, source, form, &t), None)
        }
        Pane::ApproveCode => (approve::code_pane(&ui.approve, &t), None),
        Pane::ApproveReview => approve::review_pane(source, &ui.approve, dialog, &t),
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
        .label(format!("{}, Settings", s.identity.main_window_title()))
        .on_key(on_key)
        .w_full()
        .h_full()
        .fill(t.surface)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
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
    if approve::on_escape(s) {
        return true;
    }
    if s.account.dialog.take().is_some() {
        form::arrive(&mut s.account);
        return true;
    }
    if s.account.form.renaming.take().is_some() {
        return true;
    }
    if s.account.pane == Pane::BackupCodes {
        // "Did you save your backup codes? They won't be shown again."
        ask(s, Dialog::LeaveCodes);
        return true;
    }
    go(s, s.account.pane.parent().unwrap_or("DW-0401"));
    true
}
