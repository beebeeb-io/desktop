//! What the account panes show and ask for: `AccountSource`, and the fixture that holds exactly
//! the board's sample content (DW-A51 to DW-A84a).
//!
//! The engine lane implements the trait over core; each method names the call it stands for.
//!
//! **The password never leaves the device.** Changing the password, changing two-factor and
//! deleting the account go through core's OPAQUE step-up. The trait's shape is the guard:
//! [`AccountSource::step_up`] is the ONLY method that takes a [`Password`], and what comes back
//! is a [`StepUp`]: an opaque confirmation with no text in it. Every guarded action takes that
//! confirmation BY VALUE (used once), never the password, so there is no call a pane can make
//! that puts a password next to something that is posted. `Password` cannot be printed, copied
//! or compared; it gives its bytes up only to `Password::for_opaque`, the one place an
//! implementation reads them (to feed core's OPAQUE client, which sends a blinded message).

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
}

/// Counts what is done: "7 of 7".
pub fn score(factors: &[Factor]) -> (usize, usize) {
    (factors.iter().filter(|f| f.done).count(), factors.len())
}

/// The board's sample account: Anna Visser, anna@northwind.studio.
pub struct BoardAccount {
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
            security: Security {
                app_lock: true,
                lock_after: "5 min".into(),
                password_changed: "3 weeks ago".into(),
                two_factor: true,
                passkeys: 2,
                sign_in_activity: true,
                sessions: 4,
            },
            // The board's own words, row by row (DW-A55, DW-A56).
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
            accepts: "correct horse battery staple",
            granted: Vec::new(),
            next_handle: 1,
            signed_out: None,
        }
    }

    /// DW-A84a: no passkey yet.
    pub fn without_passkeys(mut self) -> Self {
        self.security.passkeys = 0;
        self
    }

    /// Uses up a confirmation: `true` once, for the purpose it was granted for.
    #[cfg_attr(not(test), allow(dead_code))] // the guarded actions (password, two-factor, delete) are not built yet
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
            name: "Anna Visser".into(),
            initials: "AV".into(),
            email: "anna@northwind.studio".into(),
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
