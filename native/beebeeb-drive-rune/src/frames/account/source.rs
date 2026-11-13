//! What the account panes show and ask for: `AccountSource`, and the fixture that holds exactly
//! the board's sample content (DW-A51 to DW-A84a).
//!
//! The engine lane implements the trait over core; each method names the call it stands for.
//!
//! **The password never leaves the device.** Changing the email, the password and two-factor,
//! and deleting the account, go through core's OPAQUE step-up. The trait's shape is the guard:
//! [`AccountSource::step_up`] takes a [`Password`] and hands back a [`StepUp`]: an opaque
//! confirmation with no text in it. Every guarded action takes that confirmation BY VALUE (used
//! once), never the password, so there is no call a pane can make that puts the current password
//! next to something that is posted. The two other methods that take a `Password` take the NEW
//! one: [`AccountSource::password_strength`] (computed on this device) and
//! [`AccountSource::change_password`] (core's OPAQUE registration and the re-wrap of the vault
//! key, both on this device). `Password` cannot be printed, copied or compared; it gives its
//! bytes up only to `Password::for_opaque`, the one place an implementation reads them (to feed
//! core's OPAQUE client, which sends a blinded message).
//!
//! **Secrets on screen** (the two-factor key, the backup codes) travel as [`Secret`]: no `Debug`,
//! no `Display`, no `Clone`; a pane reads one only to hand it to a secure text node or to the
//! secret clipboard.

use std::fmt;

use super::approve::{Approval, Denial, Lookup, SignInRequest};

/// Who is signed in (DW-A51).
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub initials: String,
    pub email: String,
}

// By hand: the name and the email must not reach a log or a failed assertion's output through `{:?}`.
impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductKind {
    Drive,
    Pass,
    Vpn,
    Chat,
    Mail,
}

impl ProductKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Drive => "Drive",
            Self::Pass => "Pass",
            Self::Vpn => "VPN",
            Self::Chat => "Chat",
            Self::Mail => "Mail",
        }
    }
}

/// A product's state on this PC. "Not signed in" is not an error (the list is the account's
/// products, not subscriptions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductState {
    ThisApp,
    SignedIn,
    NotSignedIn,
}

impl ProductState {
    pub fn label(self) -> &'static str {
        match self {
            Self::ThisApp => "This app",
            Self::SignedIn => "Signed in",
            Self::NotSignedIn => "Not signed in",
        }
    }
}

/// Drive's own part of the Account page (the board's `accountExtra` slot): the plan, read only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// "Drive plan · 1 TB".
    pub title: String,
    /// "240 GB": a machine's value, in mono.
    pub used: String,
    /// "used · renews 14 Mar 2027".
    pub rest: String,
}

/// The plan, read only (DW-0708): what the dialog over Account shows.
#[derive(Clone, PartialEq, Debug)]
pub struct PlanDetails {
    /// "1 TB".
    pub size: String,
    /// "240 GB": a machine's value, in mono.
    pub used: String,
    /// The meter: what is used, as a share of the plan (0 to 1).
    pub share: f32,
    /// What the day is, as the dialog's row names it: "Renews", "Cancels on", "Trial ends". Never "Renews"
    /// for a plan that does not.
    pub day_label: &'static str,
    /// The day itself: "14 Mar 2027".
    pub renews: String,
    /// "Yearly".
    pub billing: String,
}

/// The rows of Sign-in and security (DW-A52).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Security {
    /// App lock (Drive's `securityExtra` slot, DW-0716): on, and how soon it locks again.
    pub app_lock: bool,
    pub lock_after: String,
    /// "3 weeks ago".
    pub password_changed: String,
    pub two_factor: bool,
    pub passkeys: usize,
    pub sign_in_activity: bool,
    pub sessions: usize,
}

/// What the Security page says, each part on its own: `None` is "not known", never "off", never 0. A source
/// that cannot say a thing leaves it `None` and the page says so; it never draws a check or a count from a
/// value nobody read. No `Debug`: the events name devices.
#[derive(Clone, PartialEq, Eq)]
pub struct SecurityFacts {
    /// App lock on this PC: on, and how soon it locks again. `None`: this build cannot set it yet.
    pub app_lock: Option<(bool, String)>,
    /// "3 weeks ago".
    pub password_changed: Option<String>,
    pub two_factor: Option<bool>,
    /// Backup codes left, of how many.
    pub backup_codes: Option<(usize, usize)>,
    pub passkeys: Option<usize>,
    pub sessions: Option<usize>,
    /// Sign-ins and security changes, newest first (the page shows the first `RECENT_EVENTS`). `Some(None)`:
    /// security logging is off.
    pub activity: Option<Option<Vec<Event>>>,
    /// The checkup's factors. `None` or empty: there is no score, and the page says so.
    pub factors: Option<Vec<Factor>>,
    /// This source can sign this app out (`sign_out_this_app`, `sign_out_all_apps`).
    pub can_sign_out: bool,
}

impl SecurityFacts {
    /// Nothing is known and nothing can be done: what a source starts from before it fills in what it read.
    pub fn unknown() -> Self {
        SecurityFacts {
            app_lock: None,
            password_changed: None,
            two_factor: None,
            backup_codes: None,
            passkeys: None,
            sessions: None,
            activity: None,
            factors: None,
            can_sign_out: false,
        }
    }
}

/// How many events the Security page shows before "All sign-in activity".
pub const RECENT_EVENTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactorKind {
    EmailVerified,
    TwoFactor,
    StrongPassword,
    RecoveryPhraseSaved,
    Passkeys,
    PasswordNotInBreach,
    Sessions,
}

impl FactorKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::EmailVerified => "Email verified",
            Self::TwoFactor => "Two-factor authentication",
            Self::StrongPassword => "Strong password",
            Self::RecoveryPhraseSaved => "Recovery phrase saved",
            Self::Passkeys => "Passkeys",
            Self::PasswordNotInBreach => "Password not in a breach",
            Self::Sessions => "Devices and sessions",
        }
    }
    /// The frame that changes it when it is done (DW-A84).
    pub fn frame(self) -> &'static str {
        match self {
            Self::EmailVerified | Self::RecoveryPhraseSaved => "DW-A51",
            Self::TwoFactor => "DW-A71",
            Self::StrongPassword | Self::PasswordNotInBreach => "DW-A67",
            Self::Passkeys => "DW-A73",
            Self::Sessions => "DW-A55",
        }
    }
    /// The frame that fixes it when it is left to do (DW-A84a).
    pub fn fix_frame(self) -> &'static str {
        match self {
            Self::Passkeys => "DW-A74",
            Self::TwoFactor => "DW-A69",
            other => other.frame(),
        }
    }
    /// The button of a to-do row.
    pub fn fix_label(self) -> &'static str {
        match self {
            Self::Passkeys | Self::TwoFactor => "Add",
            Self::EmailVerified => "Verify",
            Self::Sessions => "Review",
            _ => "Change",
        }
    }
}

/// One security factor, as the server computes it (`GET /api/v1/account/security-score`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Factor {
    pub kind: FactorKind,
    pub done: bool,
    /// The state when done ("2 passkeys"), the to-do in the desktop's words when not ("Add a passkey").
    pub says: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Laptop,
    Phone,
    CommandLine,
    Browser,
}

/// One device's sessions, grouped (DW-A55).
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    /// The server's id of the device: what a sign-out names. Never shown.
    pub id: String,
    pub kind: DeviceKind,
    pub name: String,
    /// Which Beebeeb apps are signed in there, as the list says it ("Drive, Pass").
    pub apps: String,
    /// The same apps inside the confirm's sentence ("Drive and Pass").
    pub apps_sentence: String,
    /// "now", "12 min ago".
    pub last_active: String,
    /// Two letters: a country, never a city.
    pub country: String,
    pub this_device: bool,
}

// By hand: a device's name and the server's id of it must not reach a log through `{:?}`.
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("kind", &self.kind)
            .field("this_device", &self.this_device)
            .finish_non_exhaustive()
    }
}

/// A password on its way into a step-up: borrowed from the secure field for one call.
/// It has no `Debug`, `Display`, `Clone`, `PartialEq` or `AsRef<str>`: a pane cannot log it,
/// keep it, or put it in a request.
pub struct Password<'a>(&'a str);

impl<'a> Password<'a> {
    /// From the secure field's text, at the moment of the press.
    pub fn from_field(typed: &'a str) -> Self {
        Password(typed)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// The bytes, for core's OPAQUE client and nothing else (it sends a blinded message; the
    /// password is in no request body). Called by an `AccountSource` implementation only.
    pub fn for_opaque(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// What a step-up is for: the server's confirmation is bound to one purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    ChangeEmail,
    ChangePassword,
    TwoFactor,
    AddPasskey,
    DeleteAccount,
}

/// An opaque confirmation that the person proved the password just now, for one purpose.
/// Nothing in it is text; it cannot be copied, and a guarded action consumes it.
pub struct StepUp {
    purpose: Purpose,
    /// Which confirmation of the source this is (the engine's handle to the server's token).
    handle: u64,
}

impl StepUp {
    /// Made by an `AccountSource` implementation after core's OPAQUE step-up succeeded.
    pub fn granted(purpose: Purpose, handle: u64) -> Self {
        StepUp { purpose, handle }
    }
    pub fn purpose(&self) -> Purpose {
        self.purpose
    }
    pub fn handle(&self) -> u64 {
        self.handle
    }
}

impl fmt::Debug for StepUp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StepUp({:?})", self.purpose)
    }
}

/// Why a step-up or a guarded action did not happen. Nothing changed in any of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The password was wrong (it counts toward the sign-in lockout).
    WrongPassword,
    /// The confirmation was for something else, or was used already.
    NotConfirmed,
    /// The vault is locked: "Unlock first, then change your password."
    Locked,
    /// Beebeeb could not be reached.
    Offline,
}

/// A secret a pane shows once (the two-factor key, a backup code). It cannot be printed, copied
/// or compared; its bytes are overwritten when it is dropped (best effort: the allocator may
/// have moved them before).
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Secret(text.into())
    }
    /// The characters, for a secure text node or the secret clipboard and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
    }
}

/// Two-factor as it stands (DW-A71).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TwoFactor {
    pub on: bool,
    /// "28 Oct".
    pub added: String,
    /// "Today, 09:12".
    pub last_used: String,
    pub codes_left: usize,
    pub codes_total: usize,
}

/// What setting two-factor up shows (DW-A69): the QR code's modules and the same key in words.
pub struct Setup {
    pub modules: Vec<Vec<bool>>,
    /// "JBSW Y3DP EHPK 3PXP", as the server made it.
    pub key: Secret,
}

/// One passkey of the account (DW-A73).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passkey {
    /// The server's id: what a rename or a removal names. Never shown.
    pub id: String,
    /// Where it is saved ("iCloud Keychain"); Beebeeb's own name for it.
    pub name: String,
    /// "28 Oct".
    pub added: String,
    /// Whether it can unlock the data on a new device (PRF), or only sign in.
    pub can_unlock: bool,
}

impl Passkey {
    pub fn does(&self) -> &'static str {
        if self.can_unlock {
            "Can unlock data"
        } else {
            "Sign-in only"
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    SignIn,
    PasskeyAdded,
    DeviceApproved,
    WrongPassword,
    PasswordChanged,
    AccountCreated,
}

/// One sign-in or security change (DW-A77). Never which files or items were opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: EventKind,
    /// "New sign-in on Anna-Laptop".
    pub what: String,
    /// "09:12 · Netherlands · 203.0.113.24": when, the country (never a city), the address as
    /// the server saw it. A machine's line, in mono.
    pub when_where: String,
}

/// How strong a new password is, from core (the leak check runs on Beebeeb's own servers
/// against a hashed prefix; the password is in no request).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    TooShort,
    Weak,
    Fair,
    Good,
    Strong,
}

impl Strength {
    /// How many of the four bars are filled.
    pub fn bars(self) -> usize {
        match self {
            Self::TooShort => 0,
            Self::Weak => 1,
            Self::Fair => 2,
            Self::Good => 3,
            Self::Strong => 4,
        }
    }
    pub fn says(self) -> &'static str {
        match self {
            Self::TooShort => "Too short. At least 12 characters, checked against known leaks.",
            Self::Weak => "Weak. At least 12 characters, checked against known leaks.",
            Self::Fair => "Fair. At least 12 characters, checked against known leaks.",
            Self::Good => "Good. At least 12 characters, checked against known leaks.",
            Self::Strong => "Strong. At least 12 characters, checked against known leaks.",
        }
    }
    /// Whether the server takes it.
    pub fn accepted(self) -> bool {
        matches!(self, Self::Good | Self::Strong)
    }
}

/// Where deleting the account stands (DW-A58 to DW-A83).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deletion {
    /// Nothing asked, or cancelled.
    None,
    /// The confirmation link was sent; nothing is deleted until it is used.
    LinkSent,
    /// Scheduled: "8 November at 19:42", 72 hours after the link was used.
    Scheduled { on: String },
}

/// Why a code was not taken. Nothing changed in any of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeRefused {
    Wrong,
    Expired,
    /// The confirmation that should come with it was missing, used or for something else.
    NotConfirmed,
    Offline,
}

/// A deletion that was not asked for: why, and the step-up's confirmation when it was not used up.
#[derive(Debug)]
pub struct DeletionRefused {
    pub why: CodeRefused,
    pub confirmation: Option<StepUp>,
}

/// Why an email change did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmailRefused {
    /// "This email is already in use."
    InUse,
    NotAnAddress,
    NotConfirmed,
    Offline,
}

/// Where a pane's content stands. A source that answers at once (the board's fixture) is always `Ready`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneState {
    /// What the pane draws is the signed-in account's, read from Beebeeb.
    Ready,
    /// Nothing answers for this pane yet: it says "not built yet" inside the settings shell (the sidebar stays),
    /// and a page under a section that IS answered for does not open at all.
    Unwired,
    /// Asked, not answered yet.
    Loading,
    /// Nobody is signed in, or the app is locked: nothing of an account is drawn.
    SignedOut,
    /// Beebeeb no longer accepts the session: nothing is asked until the person signs in again.
    SessionEnded,
    /// The read failed: nothing of the account is drawn, and the pane offers to try again.
    Failed,
}

/// What a poll of the source found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Something a pane draws changed: the window draws again.
    pub changed: bool,
    /// An answer is still out.
    pub busy: bool,
    /// Nothing is out, but something outside the source can change what a pane says (the engine finds the
    /// session works again): the window looks again soon.
    pub watch: bool,
}

pub trait AccountSource {
    /// Where `pane` stands. Anything but `Ready`: the pane's own accessors are not asked; the view draws the
    /// state instead (`frames::account::state_pane`, or `unbuilt_pane` for `Unwired`).
    fn pane_state(&self, _pane: super::Pane) -> PaneState {
        PaneState::Ready
    }
    /// The window shows `pane` now: a source that reads from Beebeeb asks here, when what it holds is missing
    /// or old. Never called from a draw.
    fn opened(&mut self, _pane: super::Pane) {}
    /// The window shows no account pane any more: nothing is read again until one opens.
    fn closed(&mut self) {}
    /// "Try again" on a pane that failed to load.
    fn reload(&mut self, _pane: super::Pane) {}
    /// The window's poll: take in what arrived, drop what belongs to a session that is over.
    fn pump(&mut self) -> Pumped {
        Pumped::default()
    }
    /// What the toast over the pane says: how the last change ended (taken, refused, not known).
    fn notice(&self) -> Option<&'static str> {
        None
    }
    /// The toast says something was NOT done: it is drawn as a failure, never with the mark of a thing that worked.
    fn notice_failed(&self) -> bool {
        false
    }
    /// The toast's button, when what it says can be tried again ("Try again"): `None` draws the words alone.
    fn notice_action(&self) -> Option<&'static str> {
        None
    }
    /// The toast's button was pressed: do again what the toast says was not done.
    fn run_notice_action(&mut self) {}
    /// The person did something else: the toast goes.
    fn clear_notice(&mut self) {}
    /// Engine: the signed-in account (`session.account`).
    fn identity(&self) -> Identity;
    /// Engine: which Beebeeb products hold a session on this PC (the shared credential store).
    fn products(&self) -> Vec<(ProductKind, ProductState)>;
    /// Engine: the plan summary (`GET /api/v1/billing/plan`); `None` hides the card.
    fn plan(&self) -> Option<Plan>;
    /// Engine: `GET /api/v1/account/security` and the app-lock setting of this PC.
    fn security(&self) -> Security;
    /// Engine: `GET /api/v1/account/security-score`, checked each time the page opens.
    fn factors(&self) -> Vec<Factor>;
    /// What the Security page draws. This body is for a source that knows every part (the board's fixture):
    /// it takes them from the accessors above. A source that knows only some overrides it and leaves the
    /// rest `None`. (Seam: Beebeeb's `GET /api/v1/account/security` and the score's own "can this be said"
    /// mark fill more of it once the engine reads them.)
    fn security_facts(&self) -> SecurityFacts {
        all_known(self)
    }
    /// Engine: `GET /api/v1/account/sessions`, grouped by device.
    fn sessions(&self) -> Vec<Session>;

    /// Engine: the app-lock setting of this PC (Windows Hello), `settings.app_lock`.
    fn set_app_lock(&mut self, on: bool);
    /// Engine: the plan in full (`GET /api/v1/billing/plan`): size, use, renewal day, billing
    /// period. `None`: not known; the plan card then opens the plan page on the website.
    fn plan_details(&self) -> Option<PlanDetails> {
        None
    }
    /// Engine: how soon the app locks again (`settings.app_lock_after`), one of
    /// `app_lock::DELAYS`. Windows Hello is asked first, as for `set_app_lock`.
    fn set_lock_after(&mut self, _delay: &str) {}
    /// Engine: `DELETE /api/v1/account/sessions?device=<id>`: ends all of that device's
    /// Beebeeb sessions. `false`: no such device, or it is this one.
    fn sign_out_device(&mut self, id: &str) -> bool;
    /// Engine: `DELETE /api/v1/account/sessions?except=current`.
    fn sign_out_everywhere_else(&mut self);
    /// Engine: `session.sign_out()`: ends this app's session and clears its keys from memory.
    fn sign_out_this_app(&mut self);
    /// Engine: `session.sign_out_all_on_this_pc()`: ends every Beebeeb session this PC holds
    /// and removes the shared 'Continue as' entry.
    fn sign_out_all_apps(&mut self);

    /// Engine: `core::opaque` step-up (`POST /api/v1/auth/step-up/start` + `/finish`, both
    /// carrying OPAQUE messages only). The one call that takes a password.
    fn step_up(&mut self, password: Password<'_>, purpose: Purpose) -> Result<StepUp, Refused>;

    /// Engine: `PATCH /api/v1/account` `{display_name}`; 1 to 64 characters, trimmed. `false`:
    /// refused (empty, too long, or Beebeeb could not be reached); nothing changed.
    fn set_name(&mut self, name: &str) -> bool;
    /// Engine: `POST /api/v1/account/email-change` with the step-up's token and the new address;
    /// the server sends an 8-digit code there. The old address stays until the code is entered.
    fn start_email_change(&mut self, confirmation: StepUp, new_email: &str) -> Result<(), EmailRefused>;
    /// Engine: `POST /api/v1/account/email-change/confirm` with the 8 digits.
    fn confirm_email_change(&mut self, code: &str) -> Result<(), CodeRefused>;
    /// Engine: seconds until another code or link may be sent (60 after each one); 0: now.
    fn resend_wait(&self) -> u32;
    /// Engine: sends the code (email change) or the link (deletion) again. `false`: too soon.
    fn send_again(&mut self) -> bool;

    /// Engine: `core::password::strength` on this device, and the leak check by hashed prefix.
    fn password_strength(&self, candidate: &Password<'_>) -> Strength;
    /// Engine: core re-wraps the vault key under the new password and registers it with OPAQUE
    /// (`POST /api/v1/auth/password/start` + `/finish`, OPAQUE messages and the wrapped key
    /// only). Devices that never unlocked the data are signed out by the server.
    fn change_password(&mut self, confirmation: StepUp, new: Password<'_>) -> Result<(), Refused>;

    /// Engine: `GET /api/v1/account/two-factor`.
    fn two_factor(&self) -> TwoFactor;
    /// Engine: `POST /api/v1/account/two-factor/setup`: a new secret, not active until a code
    /// from it is entered. `confirmation`: the step-up the server wants when the session is
    /// older than 5 minutes, or a current code's proof when moving to a new authenticator.
    fn begin_two_factor(&mut self, confirmation: Option<StepUp>) -> Result<Setup, Refused>;
    /// Engine: `POST /api/v1/account/two-factor/enable` with the 6 digits: turns it on (or
    /// replaces the secret) and returns the 8 new backup codes, shown once.
    fn turn_on_two_factor(&mut self, code: &str) -> Result<Vec<Secret>, CodeRefused>;
    /// Engine: `POST /api/v1/account/two-factor/backup-codes` with a current code: 8 new codes;
    /// the old ones stop working.
    fn new_backup_codes(&mut self, code: &str) -> Result<Vec<Secret>, CodeRefused>;
    /// Engine: `POST /api/v1/account/two-factor/verify` with a current code, before the set-up
    /// screen is shown again (moving to a new authenticator).
    fn check_code(&mut self, code: &str) -> Result<(), CodeRefused>;
    /// Engine: `DELETE /api/v1/account/two-factor` with a current code.
    fn turn_off_two_factor(&mut self, code: &str) -> Result<(), CodeRefused>;

    /// Engine: `GET /api/v1/account/passkeys`.
    fn passkeys(&self) -> Vec<Passkey>;
    /// Engine: confirms it is the person (Windows Hello), then the system passkey dialog (the
    /// Windows WebAuthn API, no web view) and `POST /api/v1/account/passkeys`.
    fn add_passkey(&mut self) -> Result<(), Refused>;
    /// Engine: `PATCH /api/v1/account/passkeys/<id>` `{name}`: Beebeeb's name for it only.
    fn rename_passkey(&mut self, id: &str, name: &str) -> bool;
    /// Engine: `DELETE /api/v1/account/passkeys/<id>`. The copy in the password manager stays.
    fn remove_passkey(&mut self, id: &str) -> bool;

    /// Engine: `GET /api/v1/account/activity`, by period ("Today", "October", "Earlier");
    /// `None` while security logging is off.
    fn activity(&self) -> Option<Vec<(String, Vec<Event>)>>;
    /// Engine: `PUT /api/v1/account/activity-tracking` on: a new log starts at once.
    fn start_activity(&mut self);
    /// Engine: the same, off, with a current two-factor code (the step-up when two-factor is
    /// off): recording stops and what was recorded is deleted within 30 days.
    fn stop_activity(&mut self, code: &str) -> Result<(), CodeRefused>;

    /// Engine: `GET /api/v1/account/deletion`.
    fn deletion(&self) -> Deletion;
    /// Engine: `POST /api/v1/account/deletion` with the step-up's token and the second factor
    /// (6 digits, or an 8-digit backup code): the server sends the confirmation link. Nothing
    /// is scheduled until that link is used. A wrong code hands the confirmation back, so the
    /// person retypes the code and not the password.
    fn request_deletion(&mut self, confirmation: StepUp, code: &str) -> Result<(), DeletionRefused>;
    /// Engine: `DELETE /api/v1/account/deletion` before the link is used: the link stops working.
    fn cancel_deletion_request(&mut self);
    /// Engine: `GET /api/v1/account/deletion` again ("I've opened the link"). `true`: scheduled.
    fn deletion_link_used(&mut self) -> bool;
    /// Engine: `POST /api/v1/account/deletion/undo` during the 72 hours.
    fn undo_deletion(&mut self) -> bool;

    // ---- Approving a sign-in from another device (DW-A11 to DW-A17). Every method has a body
    // that answers "nothing waits", so a source that does not know them yet keeps compiling.
    /// Engine: the request that waits for this code (cli_auth's lookup). The code is borrowed
    /// for the call; it is never logged and never kept.
    fn find_sign_in_request(&mut self, _code: &str) -> Lookup {
        Lookup::NotFound
    }
    /// Engine: the request `request` that a notification announced (DW-A11), if it still
    /// waits. No code.
    fn waiting_sign_in_request(&self, _request: &str) -> Option<SignInRequest> {
        None
    }
    /// Engine: ask Windows Hello (a confirmation minted for this approval, this code and this
    /// session only), then claim the code and relay the keys, encrypted to the other device.
    /// The answer comes through `approval_answer`.
    fn begin_approval(&mut self, _request: &str, _code: &str) {}
    /// Engine: how an approval that was begun ended, once, with the id of the request it is
    /// for; `None` while Windows Hello is up. The screen applies an answer only to the request
    /// it shows and drops every other.
    fn approval_answer(&mut self) -> Option<(String, Approval)> {
        None
    }
    /// The person left while Windows Hello was up for `request`: dismiss it. Nothing more is
    /// claimed or relayed; a claim already on its way cannot be recalled.
    fn cancel_approval(&mut self, _request: &str) {}
    /// Engine: end the request with nothing shared.
    fn deny_sign_in(&mut self, _request: &str) -> Denial {
        Denial::Expired
    }
    /// Engine: the request under review ran out (`Expired`) or was answered elsewhere.
    fn sign_in_request_gone(&mut self, _request: &str) -> Option<Denial> {
        None
    }
    /// Tests: what a fixture "sent" (`BoardAccount::wire`), read through the trait object.
    #[cfg(test)]
    fn sent(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The Security page's facts of a source that knows every part: taken from its accessors.
pub fn all_known<S: AccountSource + ?Sized>(source: &S) -> SecurityFacts {
    let sec = source.security();
    let two = source.two_factor();
    SecurityFacts {
        backup_codes: sec.two_factor.then_some((two.codes_left, two.codes_total)),
        app_lock: Some((sec.app_lock, sec.lock_after)),
        password_changed: Some(sec.password_changed),
        two_factor: Some(sec.two_factor),
        passkeys: Some(sec.passkeys),
        sessions: Some(sec.sessions),
        activity: Some(
            source
                .activity()
                .map(|periods| periods.into_iter().flat_map(|(_, events)| events).collect()),
        ),
        factors: Some(source.factors()),
        can_sign_out: true,
    }
}

/// Counts what is done: "7 of 7".
pub fn score(factors: &[Factor]) -> (usize, usize) {
    (factors.iter().filter(|f| f.done).count(), factors.len())
}

mod board;
// Every public item of the fixture keeps its path here; `BOARD_BACKUP_CODE` has no reader outside it yet.
#[allow(unused_imports)]
pub use board::{BOARD_BACKUP_CODE, BOARD_CODE, BOARD_EMAIL_CODE, BOARD_PASSWORD, BoardAccount};

/// The source the account panes start on. A development build (and every test and headless
/// render) starts on the board's sample content; a build that ships starts with none, so
/// sample values are never shown as a person's own. The engine lane replaces this.
pub fn first(dev_build: bool) -> Option<Box<dyn AccountSource>> {
    if dev_build {
        Some(Box::new(BoardAccount::new()))
    } else {
        None
    }
}
