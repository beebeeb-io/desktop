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

/// Who is signed in (DW-A51).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub initials: String,
    pub email: String,
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
    /// "14 Mar 2027".
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

pub trait AccountSource {
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
}

/// Counts what is done: "7 of 7".
pub fn score(factors: &[Factor]) -> (usize, usize) {
    (factors.iter().filter(|f| f.done).count(), factors.len())
}

/// The board's sample account: Anna Visser, anna@northwind.studio.
pub struct BoardAccount {
    /// `true`: the plan's details are not known (a test's state).
    plan_unknown: bool,
    security: Security,
    sessions: Vec<Session>,
    products: Vec<(ProductKind, ProductState)>,
    /// What the fixture "sent": one line per call that would reach the server, as the request
    /// would carry it. Tests read it to prove what is, and is not, in a request.
    pub wire: Vec<String>,
    /// The password the fixture's account has (the fixture stands in for the server's OPAQUE
    /// record; a real source never holds it).
    accepts: &'static str,
    granted: Vec<(u64, Purpose)>,
    next_handle: u64,
    pub signed_out: Option<&'static str>,
    name: String,
    email: String,
    /// The address an email change waits on.
    new_email: Option<String>,
    two_factor: TwoFactor,
    /// A set-up that waits for its first code.
    setting_up: bool,
    passkeys: Vec<Passkey>,
    activity: bool,
    deletion: Deletion,
    wait: u32,
}

/// The password the fixture's account has.
pub const BOARD_PASSWORD: &str = "correct horse battery staple";
/// The codes the fixture's server takes: the authenticator's, the email's, one backup code.
pub const BOARD_CODE: &str = "514208";
pub const BOARD_EMAIL_CODE: &str = "48271590";
pub const BOARD_BACKUP_CODE: &str = "30617724";
/// The board's eight backup codes (DW-A70) and its set-up key (DW-A69): sample values.
const BOARD_CODES: [&str; 8] = [
    "48271590", "30617724", "99150382", "62405518", "18739046", "55023197", "73682251", "04198873",
];
const BOARD_KEY: &str = "JBSW Y3DP EHPK 3PXP";

fn board_codes() -> Vec<Secret> {
    BOARD_CODES.iter().map(|c| Secret::new(*c)).collect()
}

impl BoardAccount {
    pub fn new() -> Self {
        let session = |id: &str, kind, name: &str, apps: &str, sentence: &str, last: &str, this| Session {
            id: id.into(),
            kind,
            name: name.into(),
            apps: apps.into(),
            apps_sentence: sentence.into(),
            last_active: last.into(),
            country: "DE".into(),
            this_device: this,
        };
        Self {
            plan_unknown: false,
            security: Security {
                app_lock: true,
                lock_after: "5 min".into(),
                password_changed: "3 weeks ago".into(),
                two_factor: true,
                passkeys: 2,
                sign_in_activity: true,
                sessions: 4,
            },
            // The board's own words, row by row (DW-A55, DW-A56). The board writes this app's name
            // into both from one template (`${c.PR.name}, Drive · now`, `${c.PR.name} and Drive on
            // that device are signed out…`), so on the Drive board the laptop reads "Drive, Drive"
            // and the confirm for the iPhone ("Drive, Pass" in its row) reads "Drive and Drive".
            // These are the board's sample words, kept as drawn (the gate reads them; review of
            // #298, P2 1 and 2; the Decision line is in the PR). A real source gives each device
            // its own apps in both fields.
            sessions: vec![
                session(
                    "dev-laptop",
                    DeviceKind::Laptop,
                    "Anna-Laptop",
                    "Drive, Drive",
                    "Drive and Drive",
                    "now",
                    true,
                ),
                session(
                    "dev-iphone",
                    DeviceKind::Phone,
                    "Anna's iPhone",
                    "Drive, Pass",
                    "Drive and Drive",
                    "12 min ago",
                    false,
                ),
                session(
                    "dev-cli",
                    DeviceKind::CommandLine,
                    "Studio-PC (bb)",
                    "Command line",
                    "The command line",
                    "3 days ago",
                    false,
                ),
                session(
                    "dev-web",
                    DeviceKind::Browser,
                    "Chrome on Windows",
                    "Drive web",
                    "Drive web",
                    "9 days ago",
                    false,
                ),
            ],
            products: vec![
                (ProductKind::Drive, ProductState::ThisApp),
                (ProductKind::Pass, ProductState::NotSignedIn),
                (ProductKind::Vpn, ProductState::SignedIn),
                (ProductKind::Chat, ProductState::NotSignedIn),
                (ProductKind::Mail, ProductState::NotSignedIn),
            ],
            wire: Vec::new(),
            accepts: BOARD_PASSWORD,
            granted: Vec::new(),
            next_handle: 1,
            signed_out: None,
            name: "Anna Visser".into(),
            email: "anna@northwind.studio".into(),
            new_email: None,
            two_factor: TwoFactor {
                on: true,
                added: "28 Oct".into(),
                last_used: "Today, 09:12".into(),
                codes_left: 6,
                codes_total: 8,
            },
            setting_up: false,
            passkeys: vec![
                Passkey {
                    id: "pk-icloud".into(),
                    name: "iCloud Keychain".into(),
                    added: "28 Oct".into(),
                    can_unlock: true,
                },
                Passkey {
                    id: "pk-1password".into(),
                    name: "1Password".into(),
                    added: "2 Nov".into(),
                    can_unlock: false,
                },
            ],
            activity: true,
            deletion: Deletion::None,
            wait: 42,
        }
    }

    /// DW-A79: security logging off.
    pub fn without_activity(mut self) -> Self {
        self.activity = false;
        self.security.sign_in_activity = false;
        self
    }
    /// DW-A66: an email change that waits for its code.
    pub fn changing_email_to(mut self, address: &str) -> Self {
        self.new_email = Some(address.into());
        self
    }
    /// DW-A81, DW-A82: a deletion that waits for its link, or is scheduled.
    pub fn with_deletion(mut self, deletion: Deletion) -> Self {
        self.deletion = deletion;
        self
    }
    /// A two-factor code as the fixture's server judges it.
    fn code_ok(code: &str) -> Result<(), CodeRefused> {
        if code == BOARD_CODE {
            Ok(())
        } else {
            Err(CodeRefused::Wrong)
        }
    }

    /// A source that cannot say what the plan is (DW-0708 then opens the website instead).
    #[cfg(test)]
    pub fn without_plan_details(mut self) -> Self {
        self.plan_unknown = true;
        self
    }

    /// DW-A84a, DW-A74: no passkey yet.
    pub fn without_passkeys(mut self) -> Self {
        self.passkeys.clear();
        self
    }

    /// Uses up a confirmation: `true` once, for the purpose it was granted for.
    pub fn redeem(&mut self, confirmation: StepUp, purpose: Purpose) -> bool {
        let wanted = (confirmation.handle(), purpose);
        match self.granted.iter().position(|g| *g == wanted) {
            Some(at) if confirmation.purpose() == purpose => {
                self.granted.remove(at);
                true
            }
            _ => false,
        }
    }
}

impl AccountSource for BoardAccount {
    fn identity(&self) -> Identity {
        Identity {
            name: self.name.clone(),
            initials: self
                .name
                .split_whitespace()
                .filter_map(|w| w.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase(),
            email: self.email.clone(),
        }
    }
    fn products(&self) -> Vec<(ProductKind, ProductState)> {
        self.products.clone()
    }
    fn plan(&self) -> Option<Plan> {
        Some(Plan {
            title: "Drive plan · 1 TB".into(),
            used: "240 GB".into(),
            rest: "used · renews 14 Mar 2027".into(),
        })
    }
    fn security(&self) -> Security {
        Security {
            sessions: self.sessions.len(),
            two_factor: self.two_factor.on,
            passkeys: self.passkeys.len(),
            sign_in_activity: self.activity,
            ..self.security.clone()
        }
    }
    fn factors(&self) -> Vec<Factor> {
        let sec = self.security();
        let factor = |kind, done, says: String| Factor { kind, done, says };
        vec![
            factor(FactorKind::EmailVerified, true, self.identity().email),
            factor(
                FactorKind::TwoFactor,
                sec.two_factor,
                if sec.two_factor {
                    "Authenticator app".into()
                } else {
                    "Add a second factor".into()
                },
            ),
            factor(FactorKind::StrongPassword, true, "Strong when it was set".into()),
            factor(FactorKind::RecoveryPhraseSaved, true, "Confirmed at sign-up".into()),
            factor(
                FactorKind::Passkeys,
                sec.passkeys > 0,
                match sec.passkeys {
                    0 => "Add a passkey".into(),
                    1 => "1 passkey".into(),
                    n => format!("{n} passkeys"),
                },
            ),
            factor(FactorKind::PasswordNotInBreach, true, "Checked when it was set".into()),
            factor(FactorKind::Sessions, true, format!("{} signed in", sec.sessions)),
        ]
    }
    fn sessions(&self) -> Vec<Session> {
        self.sessions.clone()
    }
    fn set_app_lock(&mut self, on: bool) {
        self.security.app_lock = on;
    }
    fn plan_details(&self) -> Option<PlanDetails> {
        (!self.plan_unknown).then(|| PlanDetails {
            size: "1 TB".into(),
            used: "240 GB".into(),
            share: 0.24,
            renews: "14 Mar 2027".into(),
            billing: "Yearly".into(),
        })
    }
    fn set_lock_after(&mut self, delay: &str) {
        self.security.lock_after = delay.into();
    }
    fn sign_out_device(&mut self, id: &str) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|s| s.this_device || s.id != id);
        let gone = self.sessions.len() < before;
        if gone {
            self.wire.push(format!("DELETE sessions device={id}"));
        }
        gone
    }
    fn sign_out_everywhere_else(&mut self) {
        self.sessions.retain(|s| s.this_device);
        self.wire.push("DELETE sessions except=current".into());
    }
    fn sign_out_this_app(&mut self) {
        self.signed_out = Some("this app");
    }
    fn sign_out_all_apps(&mut self) {
        self.signed_out = Some("all apps on this PC");
        for product in &mut self.products {
            product.1 = ProductState::NotSignedIn;
        }
    }
    fn step_up(&mut self, password: Password<'_>, purpose: Purpose) -> Result<StepUp, Refused> {
        // The request carries an OPAQUE message: a blinded element, never the password. The
        // fixture writes the message's length where a real client writes the element.
        self.wire.push(format!(
            "POST auth/step-up purpose={purpose:?} opaque-message-bytes={}",
            32
        ));
        if password.for_opaque() != self.accepts.as_bytes() {
            return Err(Refused::WrongPassword);
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        self.granted.push((handle, purpose));
        Ok(StepUp::granted(purpose, handle))
    }

    fn set_name(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return false;
        }
        self.name = name.to_string();
        self.wire.push("PATCH account display_name".into());
        true
    }
    fn start_email_change(&mut self, confirmation: StepUp, new_email: &str) -> Result<(), EmailRefused> {
        if !self.redeem(confirmation, Purpose::ChangeEmail) {
            return Err(EmailRefused::NotConfirmed);
        }
        let address = new_email.trim();
        if !address.contains('@') || address.starts_with('@') || address.ends_with('@') {
            return Err(EmailRefused::NotAnAddress);
        }
        if address.eq_ignore_ascii_case("taken@northwind.studio") {
            return Err(EmailRefused::InUse);
        }
        self.wire
            .push("POST account/email-change step-up-token new-address".into());
        self.new_email = Some(address.to_string());
        self.wait = 60;
        Ok(())
    }
    fn confirm_email_change(&mut self, code: &str) -> Result<(), CodeRefused> {
        if code != BOARD_EMAIL_CODE {
            return Err(CodeRefused::Wrong);
        }
        match self.new_email.take() {
            Some(address) => {
                self.email = address;
                self.wire.push("POST account/email-change/confirm code".into());
                Ok(())
            }
            None => Err(CodeRefused::Expired),
        }
    }
    fn resend_wait(&self) -> u32 {
        self.wait
    }
    fn send_again(&mut self) -> bool {
        if self.wait > 0 {
            return false;
        }
        self.wait = 60;
        self.wire.push("POST send-again".into());
        true
    }
    fn password_strength(&self, candidate: &Password<'_>) -> Strength {
        // Stands in for core's estimate: length only.
        match candidate.for_opaque().len() {
            0..=11 => Strength::TooShort,
            12 => Strength::Fair,
            13 => Strength::Good,
            _ => Strength::Strong,
        }
    }
    fn change_password(&mut self, confirmation: StepUp, new: Password<'_>) -> Result<(), Refused> {
        if !self.redeem(confirmation, Purpose::ChangePassword) {
            return Err(Refused::NotConfirmed);
        }
        // The request carries OPAQUE registration messages and the re-wrapped vault key: the
        // fixture writes their lengths where a real client writes the bytes.
        let _ = new.for_opaque().len();
        self.wire
            .push("POST auth/password opaque-registration-bytes=192 wrapped-vault-key-bytes=60".into());
        self.security.password_changed = "Just now".into();
        Ok(())
    }
    fn two_factor(&self) -> TwoFactor {
        self.two_factor.clone()
    }
    fn begin_two_factor(&mut self, _confirmation: Option<StepUp>) -> Result<Setup, Refused> {
        self.setting_up = true;
        self.wire.push("POST account/two-factor/setup".into());
        Ok(Setup {
            modules: crate::frames::signin::qr::board_modules(),
            key: Secret::new(BOARD_KEY),
        })
    }
    fn turn_on_two_factor(&mut self, code: &str) -> Result<Vec<Secret>, CodeRefused> {
        if !self.setting_up {
            return Err(CodeRefused::Expired);
        }
        Self::code_ok(code)?;
        self.setting_up = false;
        self.two_factor = TwoFactor {
            on: true,
            added: "Today".into(),
            last_used: "Just now".into(),
            codes_left: 8,
            codes_total: 8,
        };
        self.wire.push("POST account/two-factor/enable code".into());
        Ok(board_codes())
    }
    fn new_backup_codes(&mut self, code: &str) -> Result<Vec<Secret>, CodeRefused> {
        Self::code_ok(code)?;
        self.two_factor.codes_left = 8;
        self.wire.push("POST account/two-factor/backup-codes code".into());
        Ok(board_codes())
    }
    fn check_code(&mut self, code: &str) -> Result<(), CodeRefused> {
        Self::code_ok(code)
    }
    fn turn_off_two_factor(&mut self, code: &str) -> Result<(), CodeRefused> {
        Self::code_ok(code)?;
        self.two_factor.on = false;
        self.two_factor.codes_left = 0;
        self.wire.push("DELETE account/two-factor code".into());
        Ok(())
    }
    fn passkeys(&self) -> Vec<Passkey> {
        self.passkeys.clone()
    }
    fn add_passkey(&mut self) -> Result<(), Refused> {
        // The board: Windows Hello adds a passkey saved on this PC, listed as Windows Hello.
        self.passkeys.push(Passkey {
            id: format!("pk-hello-{}", self.passkeys.len()),
            name: "Windows Hello".into(),
            added: "Today".into(),
            can_unlock: true,
        });
        self.wire.push("POST account/passkeys attestation".into());
        Ok(())
    }
    fn rename_passkey(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        match self.passkeys.iter_mut().find(|p| p.id == id) {
            Some(passkey) if !name.is_empty() => {
                passkey.name = name.to_string();
                true
            }
            _ => false,
        }
    }
    fn remove_passkey(&mut self, id: &str) -> bool {
        let before = self.passkeys.len();
        self.passkeys.retain(|p| p.id != id);
        self.passkeys.len() < before
    }
    fn activity(&self) -> Option<Vec<(String, Vec<Event>)>> {
        if !self.activity {
            return None;
        }
        let event = |kind, what: &str, when_where: &str| Event {
            kind,
            what: what.into(),
            when_where: when_where.into(),
        };
        // The board's own rows (DW-A77).
        Some(vec![
            (
                "Today".into(),
                vec![event(
                    EventKind::SignIn,
                    "New sign-in on Anna-Laptop",
                    "09:12 · Netherlands · 203.0.113.24",
                )],
            ),
            (
                "October".into(),
                vec![
                    event(
                        EventKind::PasskeyAdded,
                        "Passkey added",
                        "28 Oct · This iPhone · 198.51.100.7",
                    ),
                    event(
                        EventKind::DeviceApproved,
                        "Device approved: Anna's iPad",
                        "19 Oct · This iPhone · 198.51.100.7",
                    ),
                    event(
                        EventKind::WrongPassword,
                        "Wrong password, 3 tries",
                        "4 Oct · Germany · 192.0.2.61",
                    ),
                ],
            ),
            (
                "Earlier".into(),
                vec![
                    event(
                        EventKind::PasswordChanged,
                        "Password changed",
                        "14 Mar · This iPhone · 198.51.100.7",
                    ),
                    event(
                        EventKind::AccountCreated,
                        "Account created",
                        "2 Feb · Netherlands · 198.51.100.7",
                    ),
                ],
            ),
        ])
    }
    fn start_activity(&mut self) {
        self.activity = true;
        self.wire.push("PUT account/activity-tracking on".into());
    }
    fn stop_activity(&mut self, code: &str) -> Result<(), CodeRefused> {
        Self::code_ok(code)?;
        self.activity = false;
        self.wire.push("PUT account/activity-tracking off code".into());
        Ok(())
    }
    fn deletion(&self) -> Deletion {
        self.deletion.clone()
    }
    fn request_deletion(&mut self, confirmation: StepUp, code: &str) -> Result<(), DeletionRefused> {
        if code != BOARD_CODE && code != BOARD_BACKUP_CODE {
            return Err(DeletionRefused {
                why: CodeRefused::Wrong,
                confirmation: Some(confirmation),
            });
        }
        if !self.redeem(confirmation, Purpose::DeleteAccount) {
            return Err(DeletionRefused {
                why: CodeRefused::NotConfirmed,
                confirmation: None,
            });
        }
        self.deletion = Deletion::LinkSent;
        self.wait = 60;
        self.wire.push("POST account/deletion step-up-token code".into());
        Ok(())
    }
    fn cancel_deletion_request(&mut self) {
        self.deletion = Deletion::None;
        self.wire.push("DELETE account/deletion".into());
    }
    fn deletion_link_used(&mut self) -> bool {
        // The fixture's person opened the link (the board's next frame, DW-A82).
        if self.deletion == Deletion::LinkSent {
            self.deletion = Deletion::Scheduled {
                on: "8 November at 19:42".into(),
            };
        }
        matches!(self.deletion, Deletion::Scheduled { .. })
    }
    fn undo_deletion(&mut self) -> bool {
        let scheduled = matches!(self.deletion, Deletion::Scheduled { .. });
        if scheduled {
            self.deletion = Deletion::None;
            self.wire.push("POST account/deletion/undo".into());
        }
        scheduled
    }
}

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
