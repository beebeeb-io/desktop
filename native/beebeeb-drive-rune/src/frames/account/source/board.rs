//! The board's sample account (the fixture source of the account panes): moved out of `source.rs` unchanged when
//! that file passed the size limit. `source.rs` re-exports every public item, so paths are the same.

use super::*;

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
    /// Approving a sign-in: the answers on their way.
    approvals: super::super::request::BoardApprovals,
    /// What the Security page is told instead of everything (a source that knows only some parts).
    facts: Option<fn(SecurityFacts) -> SecurityFacts>,
    /// Only the three sections answer; every page under them is unwired (as on the real engine today).
    sections_only: bool,
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
    /// Windows Hello's prompt stays up: no answer comes (the scene of DW-A14).
    pub fn holding_hello(mut self) -> Self {
        self.approvals.hold();
        self
    }
    /// A cancel comes too late to stop an answer that is on its way.
    #[cfg(test)]
    pub fn late_answers(mut self) -> Self {
        self.approvals.late();
        self
    }
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
            approvals: super::super::request::BoardApprovals::default(),
            facts: None,
            sections_only: false,
        }
    }

    /// A source that knows only part of the Security page: `change` takes what the board knows and hands
    /// back what this source says.
    pub fn knowing(mut self, change: fn(SecurityFacts) -> SecurityFacts) -> Self {
        self.facts = Some(change);
        self
    }
    /// A source that answers for the three sections and for no page under them.
    pub fn sections_only(mut self) -> Self {
        self.sections_only = true;
        self
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
    fn pane_state(&self, pane: super::super::Pane) -> PaneState {
        use super::super::Pane;
        if self.sections_only && !matches!(pane, Pane::Account | Pane::Security | Pane::Sessions) {
            PaneState::Unwired
        } else {
            PaneState::Ready
        }
    }
    fn security_facts(&self) -> SecurityFacts {
        let all = all_known(self);
        match self.facts {
            Some(change) => change(all),
            None => all,
        }
    }
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
            day_label: "Renews",
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
    #[cfg(test)]
    fn sent(&self) -> Vec<String> {
        self.wire.clone()
    }
    fn find_sign_in_request(&mut self, code: &str) -> Lookup {
        // What reaches the server is the request for a lookup, never the code in a log line.
        self.wire.push("POST /cli-auth/lookup".into());
        super::super::approve::board_lookup(code)
    }
    fn waiting_sign_in_request(&self, request: &str) -> Option<SignInRequest> {
        super::super::request::BoardApprovals::waiting(request)
    }
    fn begin_approval(&mut self, request: &str, _code: &str) {
        self.wire.push(format!("POST /cli-auth/approve {request}"));
        self.approvals.begin(request);
    }
    fn approval_answer(&mut self) -> Option<(String, Approval)> {
        self.approvals.answer()
    }
    fn cancel_approval(&mut self, request: &str) {
        self.wire.push(format!("cancel approval {request}"));
        self.approvals.cancel(request);
    }
    fn deny_sign_in(&mut self, request: &str) -> Denial {
        self.wire.push(format!("POST /cli-auth/deny {request}"));
        self.approvals.deny(request)
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
