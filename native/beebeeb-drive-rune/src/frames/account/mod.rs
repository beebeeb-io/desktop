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
mod app_lock;
pub mod approve;
#[cfg(test)]
mod approve_tests;
mod delete;
mod form;
mod forms;
pub mod icons;
mod overview;
mod passkeys;
mod plan;
mod request;
mod security_overview;
mod sessions;
pub mod source;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_security;
mod two_factor;

use crate::State;
use crate::frames::settings;
use crate::shell::app::{Event, Screen, SettingsPane};
use crate::shell::{host, instance, placeholder, registry};
use crate::theme::tokens::{self, Theme};
use crate::ui::{account_set as set, caption};
use rune::{Cx, El, Icon, Key, KeyEvent, Tx, col, el, row};

use source::AccountSource;

/// A pane of this lane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    /// Account (DW-A51).
    Account,
    /// Sign-in and security: ONE page (`security_overview.rs`), in place of DW-A52 and the security checkup
    /// (DW-A84, DW-A84a). Those three frame ids all show it.
    Security,
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

/// A confirm over a pane. A destructive choice is confirmed, never one press. `Debug` is written by hand
/// (below): three of them hold the server's id of a device or a passkey.
#[derive(Clone, PartialEq, Eq)]
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
    /// The plan, read only, over Account (DW-0708).
    Plan,
    /// The menu of App lock's delays (DW-0716): the item under the pointer or the keys.
    LockDelay(Option<usize>),
    /// Approving a sign-in: the review with its code field (DW-A13), Windows Hello asking
    /// (DW-A14), and the three ends (DW-A15, DW-A16, DW-A17; `handled`: "Already handled").
    ApproveReview,
    ApproveHello,
    ApproveApproved,
    ApproveDenied,
    ApproveExpired {
        handled: bool,
    },
    /// No board frame: the server could not be reached, so the end is not known (`denying`:
    /// the denial was not sent; else the approval may or may not have gone through).
    ApproveUnsure {
        denying: bool,
    },
    /// No board frame: Deny came after this PC's own approval had gone through.
    ApproveTooLate,
}

// By hand: which confirm it is, never the id it holds.
impl std::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Dialog::SignOutDevice(_) => f.write_str("SignOutDevice(..)"),
            Dialog::SignOutElsewhere => f.write_str("SignOutElsewhere"),
            Dialog::SignOutApp => f.write_str("SignOutApp"),
            Dialog::SignOutAllApps => f.write_str("SignOutAllApps"),
            Dialog::Code(what) => write!(f, "Code({what:?})"),
            Dialog::PasskeyMenu(_) => f.write_str("PasskeyMenu(..)"),
            Dialog::RemovePasskey(_) => f.write_str("RemovePasskey(..)"),
            Dialog::LeaveCodes => f.write_str("LeaveCodes"),
            Dialog::Plan => f.write_str("Plan"),
            Dialog::LockDelay(hot) => write!(f, "LockDelay({hot:?})"),
            Dialog::ApproveReview => f.write_str("ApproveReview"),
            Dialog::ApproveHello => f.write_str("ApproveHello"),
            Dialog::ApproveApproved => f.write_str("ApproveApproved"),
            Dialog::ApproveDenied => f.write_str("ApproveDenied"),
            Dialog::ApproveExpired { handled } => write!(f, "ApproveExpired {{ handled: {handled} }}"),
            Dialog::ApproveUnsure { denying } => write!(f, "ApproveUnsure {{ denying: {denying} }}"),
            Dialog::ApproveTooLate => f.write_str("ApproveTooLate"),
        }
    }
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
            Dialog::ApproveApproved
            | Dialog::ApproveDenied
            | Dialog::ApproveExpired { .. }
            | Dialog::ApproveUnsure { .. }
            | Dialog::ApproveTooLate => false,
            // Confirmations without key material.
            Dialog::SignOutDevice(_)
            | Dialog::SignOutElsewhere
            | Dialog::SignOutApp
            | Dialog::SignOutAllApps
            | Dialog::PasskeyMenu(_)
            | Dialog::RemovePasskey(_)
            | Dialog::LeaveCodes => false,
            // DW-0708: the plan's size, use, renewal day and billing period. No key material.
            Dialog::Plan => false,
            // DW-0716: four lengths of time. No key material.
            Dialog::LockDelay(_) => false,
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
    /// The pane the source was last told is showing (`None`: no account section shows).
    told_open: Option<Pane>,
    /// A sign-out asked from the Security page's confirm: waiting for its answer, or refused.
    pub signing_out: security_overview::SigningOut,
}

impl Default for AccountUi {
    fn default() -> Self {
        Self {
            pane: Pane::Account,
            dialog: None,
            form: form::Form::default(),
            approve: approve::Approve::default(),
            source: source::first(instance::DEV_BUILD),
            told_open: None,
            signing_out: security_overview::SigningOut::default(),
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
    // A sign-out that waited for its answer: the session it was for is over, its late answer is the source's to drop.
    ui.signing_out = security_overview::SigningOut::default();
    ui.show(Pane::root(ui.pane.section()).unwrap_or(Pane::Account));
}

/// The pane the window shows now, when the route is on one of this lane's sections.
fn showing(s: &State) -> Option<Pane> {
    let section = routed(s)?;
    if s.account.pane.section() == section {
        Some(s.account.pane)
    } else {
        Pane::root(section)
    }
}

/// The window's poll for the account sections: tells the source which pane shows when that changed (it reads
/// then, never from a draw), and takes in what its workers answered. `true`: something a pane draws changed.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let now = showing(s);
    let ui = &mut s.account;
    let Some(source) = ui.source.as_mut() else {
        ui.told_open = None;
        return false;
    };
    if ui.told_open != now {
        ui.told_open = now;
        // A toast belongs to the pane it was said over.
        source.clear_notice();
        match now {
            Some(pane) => source.opened(pane),
            // No account pane shows: the source reads nothing more until one opens.
            None => source.closed(),
        }
    }
    let pumped = source.pump();
    // A sign-out that is out: its answer, when it came. Only then is anything of it drawn.
    let answered = match ui.signing_out.waiting {
        Some(all) => source.sign_out_result().map(|answer| (all, answer)),
        None => None,
    };
    // A name that was sent: its answer, when it came.
    let named = forms::take_in_name(ui);
    if pumped.busy || pumped.watch || ui.signing_out.waiting.is_some() || ui.form.saving_name {
        tx.wake_after(POLL_EVERY);
    }
    // A toast that goes by itself: look again when its time is up.
    if let Some(stay) = pumped.wake_in {
        tx.wake_after(stay);
    }
    if let Some((all, answer)) = answered {
        s.account.signing_out.waiting = None;
        security_overview::settle_sign_out(s, all, answer);
        return true;
    }
    if let Some(frame) = named {
        go(s, frame);
        return true;
    }
    pumped.changed
}

/// The toast over a pane. Something that worked is the settings toast, as before. Something that was not done
/// is the failure toast (`ui::toast`: its own mark, read at once), laid where the settings toast sits, with its
/// button when it can be tried again.
fn notice_toast(source: &dyn AccountSource, t: &Theme) -> Option<El> {
    let words = source.notice()?;
    let action = source.notice_action();
    if !source.notice_failed() && action.is_none() {
        return Some(crate::ui::settings_notice::toast(0, words, t));
    }
    let again = action.map(|action| crate::ui::toast::ToastAction {
        text: action,
        name: action,
        shortcut: None,
        run: |s: &mut State, _: &mut Tx| {
            if let Some(source) = s.account.source.as_mut() {
                source.run_notice_action();
            }
        },
    });
    Some(crate::ui::settings_notice::standing("toast", words, again, 0, t))
}

/// How soon the window looks again while an answer is out (the worker also wakes it).
const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// The heading of the section a pane belongs to.
fn title_of(pane: Pane) -> &'static str {
    match pane.section() {
        SettingsPane::DevicesAndSessions => "Devices and sessions",
        SettingsPane::Security => "Sign-in and security",
        _ => "Account",
    }
}

/// A pane the source does not answer for yet, INSIDE the settings shell: the sidebar stays, so there is always
/// a way back (the shell's full-window frame has none outside Drive). Honest as that frame is: what it is, one
/// sentence, nothing that could be read as the account's data. Keyed as the shell's notice is.
fn unbuilt_pane(pane: Pane, t: &Theme) -> El {
    let title = title_of(pane);
    let notice = col()
        .key("not-built")
        .role("group")
        .label(format!("{title}: not built yet"))
        .gap(4.0)
        .child(crate::ui::kit::label(
            "not-built-label",
            "NOT BUILT YET",
            set::SUB_MONO,
            t.text_tertiary,
        ))
        .child(crate::ui::kit::label(
            "not-built-line",
            "This part of Beebeeb Drive has no screen yet. Nothing here is your data.",
            set::SUB,
            t.text_tertiary,
        ));
    set::pane(vec![set::heading(title, None, None, t), notice], t)
}

/// The pane in a state that is not its content: asked and not answered, nobody signed in, the session ended,
/// or the read failed. Nothing of an account is in it.
fn state_pane(pane: Pane, state: source::PaneState, t: &Theme) -> El {
    use crate::ui::settings_notice as notices;
    let title = title_of(pane);
    let (head, words, again) = match state {
        source::PaneState::Loading => ("Loading", "Asking Beebeeb for your account.", false),
        source::PaneState::SignedOut => (
            "Not signed in",
            "Sign in and unlock Beebeeb to see your account.",
            false,
        ),
        source::PaneState::SessionEnded => (
            "Your session ended",
            "Sign in again to see your account. Nothing was changed.",
            false,
        ),
        _ => (
            "Couldn't load this",
            // Not "nothing was changed": this state also follows a sign-out Beebeeb took, when the list
            // could not be read after it.
            "Beebeeb didn't answer.",
            true,
        ),
    };
    let body = if state == source::PaneState::Loading {
        el().key("account-loading")
            .role("status")
            .label(format!("{head}. {words}"))
            .child(crate::ui::kit::label(
                "account-loading-text",
                words,
                set::SUB,
                t.text_tertiary,
            ))
    } else {
        let action = again.then(|| {
            set::button("btn-0", "Try again", None, set::Look::Plain, set::Size::Small, false, t).on_press(
                move |s: &mut State| {
                    if let Some(source) = s.account.source.as_mut() {
                        source.reload(pane);
                    }
                },
            )
        });
        notices::problem(0, 0, head, words, action, t)
    };
    set::pane(vec![set::heading(title, None, None, t), body], t)
}

/// A board frame of this lane's as a state of it: the pane, and the confirm over it.
fn local(frame: &str) -> Option<(Pane, Option<Dialog>)> {
    Some(match frame {
        "DW-A51" => (Pane::Account, None),
        "DW-A52" => (Pane::Security, None),
        "DW-A57" => (Pane::Security, Some(Dialog::SignOutApp)),
        "DW-A57a" => (Pane::Security, Some(Dialog::SignOutAllApps)),
        // The checkup is on the page itself now: its ids lead there, so a link to one still arrives.
        "DW-A84" | "DW-A84a" => (Pane::Security, None),
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
        "DW-0708" => (Pane::Account, Some(Dialog::Plan)),
        // The menu opens on the current choice (`app_lock::arrive`).
        "DW-0716" => (Pane::Security, Some(Dialog::LockDelay(None))),
        "DW-0716b" => (Pane::Security, None),
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
    // A page under a section opens only when something answers for it. When the section itself is answered
    // for and the page is not, the person stays where they are. (A section nothing answers for says so inside
    // the settings shell, `unbuilt_pane`.)
    if let Some(source) = s.account.source.as_deref()
        && let Some(root) = Pane::root(want)
        && pane != root
        && source.pane_state(pane) == source::PaneState::Unwired
        && source.pane_state(root) != source::PaneState::Unwired
    {
        return;
    }
    if s.shell.app.screen() != Screen::Settings(want) {
        host::dispatch(s, Event::OpenSettings(want));
    }
    s.account.show(pane);
    s.account.dialog = dialog;
    form::arrive(&mut s.account);
    app_lock::arrive(&mut s.account);
    security_overview::arrive(&mut s.account);
    if plan::arrive(&mut s.account) {
        // Nothing can say what the plan is: the website can.
        s.settings.ask(settings::Intent::Web(plan::PLAN_PAGE));
    }
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
    // Sign-in and security, the one page (no board frame: `FRAMES.md`, "Sign-in and security, one page"). Its
    // states have ids of their own, DW-S..; the board ids it replaced (DW-A52, DW-A84, DW-A84a, DW-A57,
    // DW-A57a, DW-0716, DW-0716b) are retired as frames and stay as names `go` understands.
    ("DW-S01", |s| open_on(s, "DW-A52")),
    // The page as a source draws it that knows two-factor and how many are signed in and nothing else (the
    // running app today).
    ("DW-S01u", |s| {
        s.account.source = Some(Box::new(
            source::BoardAccount::new()
                .knowing(|all| source::SecurityFacts {
                    two_factor: all.two_factor,
                    sessions: all.sessions,
                    ..source::SecurityFacts::unknown()
                })
                .sections_only(),
        ));
        go(s, "DW-A52");
    }),
    ("DW-S01a", |s| {
        open_on(s, "DW-A84a");
        s.account.source = Some(Box::new(source::BoardAccount::new().without_passkeys()));
    }),
    ("DW-A55", |s| open_on(s, "DW-A55")),
    ("DW-A56", |s| {
        open_on(s, "DW-A55");
        confirm_device(s, "dev-iphone".into());
    }),
    ("DW-S02", |s| open_on(s, "DW-A57")),
    ("DW-S02a", |s| open_on(s, "DW-A57a")),
    // The confirm while its sign-out is out: it says so and takes no press.
    ("DW-S02w", |s| {
        s.account.source = Some(Box::new(
            source::BoardAccount::new().answering_sign_out(source::SignOutMode::After(u32::MAX)),
        ));
        go(s, "DW-A57");
        if let Some(source) = s.account.source.as_mut() {
            let answer = source.sign_out_this_app();
            security_overview::settle_sign_out(s, false, answer);
        }
    }),
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
    ("DW-0708", |s| open_on(s, "DW-0708")),
    ("DW-S03", |s| open_on(s, "DW-0716")),
    ("DW-S03b", |s| {
        open_on(s, "DW-0716b");
        if let Some(source) = s.account.source.as_mut() {
            source.set_app_lock(false);
        }
    }),
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
    // A source that does not answer for this pane (or not yet): its state, never the pane on empty values.
    let state = source.pane_state(pane);
    if pane == Pane::DeleteUndone && state != source::PaneState::Unwired {
        // The result of undoing a deletion is a screen of its own (the account window's
        // layout, DW-A83), not a pane of settings.
        return delete::undone(s.identity.product_name(), &t, cx);
    }
    let form = &ui.form;
    let (pane_el, overlay) = match pane {
        // Nothing answers for it: said inside the settings shell, where the sidebar leads back.
        _ if state == source::PaneState::Unwired => (unbuilt_pane(pane, &t), None),
        _ if state != source::PaneState::Ready => (state_pane(pane, state, &t), None),
        Pane::Account => (
            overview::pane(source, &t),
            dialog
                .filter(|d| **d == Dialog::Plan)
                .and_then(|_| source.plan_details())
                .map(|plan| plan::overlay(&plan, &t)),
        ),
        Pane::Security => security_overview::pane(source, dialog, &ui.signing_out, &t),
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
        // One scroller per pane (Rune keeps a scroller's offset by its path): a page opened from a page that
        // was scrolled down starts at its own top, never at the other's offset.
        .child(
            row()
                .key(format!("pane-{pane:?}"))
                .grow(1.0)
                .min_w(0.0)
                .min_h(0.0)
                .h_full()
                .child(pane_el),
        );
    if let Some(overlay) = overlay {
        body = body.child(overlay);
    }
    if let Some(toast) = notice_toast(source, &t) {
        body = body.child(toast);
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
    if let Some(Dialog::LockDelay(hot)) = s.account.dialog {
        if let Some(taken) = app_lock::on_key(s, hot, ev) {
            return taken;
        }
    }
    if ev.key != Key::Escape {
        return false;
    }
    if approve::on_escape(s) {
        return true;
    }
    // A sign-out that is out cannot be called back: its confirm stays until the answer.
    if s.account.signing_out.waiting.is_some() {
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
