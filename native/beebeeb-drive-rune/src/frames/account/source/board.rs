//! The board's sample account (the fixture source of the account panes): moved out of `source.rs` unchanged when
//! that file passed the size limit. `source.rs` re-exports every public item, so paths are the same.

use super::*;

/// The board's sample account: Anna Visser, anna@northwind.studio.
pub struct BoardAccount {
    #[cfg(test)]
    held: Option<HeldCalls>,
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
    /// How this fixture answers a sign-out of this app.
    sign_out_mode: SignOutMode,
    /// The answers of calls that were started, handed over on the next poll; the serial of the latest call of each
    /// kind.
    queued: std::collections::VecDeque<(CallKind, Answered)>,
    serial: u64,
    marks: Vec<(CallKind, u64)>,
    /// A sign-out that answered `Pending`: all apps or this one, and the polls until its answer.
    signing_out: Option<(bool, u32)>,
}

/// How the fixture answers a sign-out of this app.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SignOutMode {
    /// Done in the call (the board's frames).
    AtOnce,
    /// Refused: unsent changes.
    #[cfg_attr(not(test), allow(dead_code))]
    Refused,
    /// Refused: an open editor holds unsaved changes (until `sign_out_discarding_unsaved`, or the mode is reset).
    #[cfg_attr(not(test), allow(dead_code))]
    Unsaved,
    /// On its way: answered after this many polls (`u32::MAX`: never, in a test's lifetime).
    After(u32),
}

/// What the fixture says when it refuses a sign-out.
pub const BOARD_SIGN_OUT_REFUSED: &str = "Some changes are not sent yet. Nothing was signed out.";

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

/// A test's control over the calls the fixture holds as out (a worker's call that has not answered): every
/// `start_call` is taken, and its answer comes only when the test sets one.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct HeldCalls {
    /// The answer to deliver to the call that is out.
    pub answer: std::rc::Rc<std::cell::RefCell<Option<Answered>>>,
    /// The session or the account moved on after the answer was taken (a sign-out and sign-in, another account).
    pub moved: std::rc::Rc<std::cell::Cell<bool>>,
    /// Every call that was started, in order.
    pub started: std::rc::Rc<std::cell::RefCell<Vec<CallKind>>>,
    /// A press of the kind that is already out is "taken" and starts nothing (the engine's rule), instead of refused.
    pub repeat_out: std::rc::Rc<std::cell::Cell<bool>>,
    /// No session is open: every start is refused (`start_call` is `false`).
    pub refuse: std::rc::Rc<std::cell::Cell<bool>>,
    out: Option<CallKind>,
    /// What the call that is out was given (the address of a change of email).
    input: String,
    marks: Vec<(CallKind, u64)>,
}

impl BoardAccount {
    /// Every account call is asked through `start_call` and answers only when `held.answer` is set.
    #[cfg(test)]
    pub fn holding_calls(mut self, held: HeldCalls) -> Self {
        self.held = Some(held);
        // No countdown over "Send again": it is pressable, so its call can be started.
        self.wait = 0;
        self
    }
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
            #[cfg(test)]
            held: None,
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
                    unlock_known: true,
                },
                Passkey {
                    id: "pk-1password".into(),
                    name: "1Password".into(),
                    added: "2 Nov".into(),
                    can_unlock: false,
                    unlock_known: true,
                },
            ],
            activity: true,
            deletion: Deletion::None,
            wait: 42,
            approvals: super::super::request::BoardApprovals::default(),
            facts: None,
            sections_only: false,
            sign_out_mode: SignOutMode::AtOnce,
            signing_out: None,
            queued: std::collections::VecDeque::new(),
            serial: 0,
            marks: Vec::new(),
        }
    }

    /// A source that knows only part of the Security page: `change` takes what the board knows and hands
    /// back what this source says.
    pub fn knowing(mut self, change: fn(SecurityFacts) -> SecurityFacts) -> Self {
        self.facts = Some(change);
        self
    }
    /// How a sign-out of this app is answered (tests, and a scene of a sign-out on its way).
    pub fn answering_sign_out(mut self, mode: SignOutMode) -> Self {
        self.sign_out_mode = mode;
        self
    }
    /// What a sign-out answers before anything is done, when it is not done in the call.
    fn sign_out_held(&mut self, all: bool) -> Option<SignOutAnswer> {
        match self.sign_out_mode {
            SignOutMode::AtOnce => None,
            SignOutMode::Unsaved => Some(SignOutAnswer::Unsaved),
            SignOutMode::Refused => Some(SignOutAnswer::Refused(BOARD_SIGN_OUT_REFUSED)),
            SignOutMode::After(polls) => {
                self.signing_out = Some((all, polls));
                Some(SignOutAnswer::Pending)
            }
        }
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

#[cfg(test)]
impl HeldCalls {
    fn mark(&self, kind: CallKind) -> Option<CallMark> {
        let (_, serial) = self.marks.iter().find(|(k, _)| *k == kind)?;
        Some(CallMark {
            serial: *serial,
            out: self.out == Some(kind),
            current: self.out == Some(kind) || !self.moved.get(),
        })
    }
}

#[cfg(test)]
impl BoardAccount {
    fn held_start(&mut self, kind: CallKind, input: &str) -> bool {
        match self.held.as_mut() {
            Some(held) if held.refuse.get() => false,
            Some(held) if held.out == Some(kind) && held.repeat_out.get() => true,
            Some(held) if held.out.is_none() => {
                held.out = Some(kind);
                held.input = input.to_string();
                held.started.borrow_mut().push(kind);
                let serial = held.started.borrow().len() as u64;
                held.marks.retain(|(k, _)| *k != kind);
                held.marks.push((kind, serial));
                true
            }
            _ => false,
        }
    }
    fn held_answer(&mut self) -> Option<(CallKind, Answered)> {
        let held = self.held.as_mut()?;
        let kind = held.out?;
        let answered = held.answer.borrow_mut().take()?;
        held.out = None;
        // The address Beebeeb names for the code is the one the call sent.
        if let (CallKind::StartEmail, Answered::Done) = (kind, &answered) {
            self.new_email = Some(std::mem::take(&mut held.input));
        }
        Some((kind, answered))
    }
}

/// The Security page's facts of the board: it knows every part.
fn all_known(board: &BoardAccount) -> SecurityFacts {
    let sec = board.security();
    let two = board.two_factor.clone();
    SecurityFacts {
        backup_codes: sec.two_factor.then_some((two.codes_left, two.codes_total)),
        app_lock: Some((sec.app_lock, sec.lock_after)),
        app_lock_password: None,
        password_changed: Some(sec.password_changed),
        two_factor: Some(sec.two_factor),
        passkeys: Some(sec.passkeys),
        sessions: Count::Known(sec.sessions),
        activity: Some(
            board
                .activity()
                .map(|periods| periods.into_iter().flat_map(|(_, events)| events).collect()),
        ),
        factors: Some(board.factors()),
        can_sign_out: true,
        can_sign_out_all: true,
    }
}

/// The board's own answers (what its sample server would say), given at once.
impl BoardAccount {
    /// The account's security summary, as the board draws it.
    pub fn security(&self) -> Security {
        Security {
            sessions: self.sessions.len(),
            two_factor: self.two_factor.on,
            passkeys: self.passkeys.len(),
            sign_in_activity: self.activity,
            ..self.security.clone()
        }
    }
    /// The security score's factors, as the board draws them.
    pub fn factors(&self) -> Vec<Factor> {
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

    /// One call's answer, as the board's sample server gives it.
    fn answer_now(&mut self, kind: CallKind, input: &str, password: &str) -> Answered {
        let code = input;
        match kind {
            CallKind::StartEmail => match self.start_email_change(password, input) {
                Ok(()) => Answered::Done,
                Err(why) => Answered::Email(why),
            },
            CallKind::ConfirmEmail => match self.confirm_email_change(code) {
                Ok(()) => Answered::Done,
                Err(why) => Answered::Code(why),
            },
            CallKind::ResendEmail => {
                if self.wait > 0 {
                    return Answered::Resend(Resend::Wait(self.wait));
                }
                self.wait = 60;
                self.wire.push("POST send-again".into());
                Answered::Done
            }
            CallKind::BeginTwoFactor => {
                self.setting_up = true;
                self.wire.push("POST account/two-factor/setup".into());
                Answered::Setup(Setup {
                    modules: crate::frames::signin::qr::board_modules(),
                    key: Secret::new(BOARD_KEY),
                })
            }
            CallKind::TurnOnTwoFactor => match self.turn_on_two_factor(code) {
                Ok(codes) => Answered::Codes(codes),
                Err(why) => Answered::Code(why),
            },
            CallKind::NewBackupCodes => match Self::code_ok(code) {
                Ok(()) => {
                    self.two_factor.codes_left = 8;
                    self.wire.push("POST account/two-factor/backup-codes code".into());
                    Answered::Codes(board_codes())
                }
                Err(why) => Answered::Code(why),
            },
            CallKind::CheckCode => match Self::code_ok(code) {
                Ok(()) => Answered::Done,
                Err(why) => Answered::Code(why),
            },
            CallKind::TurnOffTwoFactor => match Self::code_ok(code) {
                Ok(()) => {
                    self.two_factor.on = false;
                    self.two_factor.codes_left = 0;
                    self.wire.push("DELETE account/two-factor code".into());
                    Answered::Done
                }
                Err(why) => Answered::Code(why),
            },
            // The board takes a password change through `change_password` (`start_call` refuses it).
            CallKind::ChangePassword => Answered::Refused(Refused::NotConfirmed),
        }
    }
    /// The new address and the password: the password is proved first (an OPAQUE step-up: what reaches the wire is
    /// its message, never the password), then the change is asked.
    fn start_email_change(&mut self, password: &str, new_email: &str) -> Result<(), EmailRefused> {
        let confirmation = self
            .step_up(Password::from_field(password), Purpose::ChangeEmail)
            .map_err(|_| EmailRefused::NotConfirmed)?;
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
    fn sign_out_this_app(&mut self) -> SignOutAnswer {
        if let Some(answer) = self.sign_out_held(false) {
            return answer;
        }
        self.signed_out = Some("this app");
        SignOutAnswer::Done
    }
    fn sign_out_discarding_unsaved(&mut self) -> SignOutAnswer {
        self.wire.push("sign out, discarding unsaved changes".into());
        self.sign_out_mode = SignOutMode::AtOnce;
        self.sign_out_this_app()
    }
    fn sign_out_all_apps(&mut self) -> SignOutAnswer {
        if let Some(answer) = self.sign_out_held(true) {
            return answer;
        }
        self.signed_out = Some(concat!("all apps on ", beebeeb_engine::device_word!()));
        for product in &mut self.products {
            product.1 = ProductState::NotSignedIn;
        }
        SignOutAnswer::Done
    }
    fn sign_out_result(&mut self) -> Option<SignOutAnswer> {
        let (all, polls) = self.signing_out?;
        if polls > 0 {
            self.signing_out = Some((all, polls - 1));
            return None;
        }
        // The answer arrived: only now is anything signed out.
        self.signing_out = None;
        self.sign_out_mode = SignOutMode::AtOnce;
        Some(if all {
            self.sign_out_all_apps()
        } else {
            self.sign_out_this_app()
        })
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

    fn save_name(&mut self, name: &str) -> NameAnswer {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return NameAnswer::NotSaved(NameWhy::Refused);
        }
        self.name = name.to_string();
        self.wire.push("PATCH account display_name".into());
        NameAnswer::Saved
    }
    fn email_code_sent_to(&self) -> Option<String> {
        self.new_email.clone()
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
    /// The board answers every call at once, and hands the answer over on the window's next poll (as a worker's
    /// would come): a press of a call that is already queued starts nothing (the engine's rule). A password change is
    /// not one of them: the board takes it through `change_password`.
    fn start_call(&mut self, kind: CallKind, input: &str, password: &str) -> bool {
        #[cfg(test)]
        if self.held.is_some() {
            return self.held_start(kind, input);
        }
        if kind == CallKind::ChangePassword {
            return false;
        }
        if self.queued.iter().any(|(k, _)| *k == kind) {
            return true;
        }
        let answered = self.answer_now(kind, input, password);
        self.serial += 1;
        self.marks.retain(|(k, _)| *k != kind);
        self.marks.push((kind, self.serial));
        self.queued.push_back((kind, answered));
        true
    }
    fn call_answer(&mut self) -> Option<(CallKind, Answered)> {
        #[cfg(test)]
        if self.held.is_some() {
            return self.held_answer();
        }
        self.queued.pop_front()
    }
    fn call_mark(&self, kind: CallKind) -> Option<CallMark> {
        #[cfg(test)]
        if let Some(held) = self.held.as_ref() {
            return held.mark(kind);
        }
        let (_, serial) = self.marks.iter().find(|(k, _)| *k == kind)?;
        let out = self.queued.iter().any(|(k, _)| *k == kind);
        // One sample account, one session: an answer is always this session's.
        Some(CallMark {
            serial: *serial,
            out,
            current: true,
        })
    }
    fn pump(&mut self) -> Pumped {
        Pumped {
            busy: !self.queued.is_empty(),
            ..Pumped::default()
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
    fn passkeys(&self) -> Vec<Passkey> {
        self.passkeys.clone()
    }
    /// The board: Windows Hello adds a passkey saved on this PC, listed as Windows Hello (no password is asked).
    fn add_passkey(&mut self, _password: Option<Password<'_>>) -> Result<(), Refused> {
        self.passkeys.push(Passkey {
            id: format!("pk-hello-{}", self.passkeys.len()),
            name: "Windows Hello".into(),
            added: "Today".into(),
            can_unlock: true,
            unlock_known: true,
        });
        self.wire.push("POST account/passkeys attestation".into());
        Ok(())
    }
    fn remove_passkey(&mut self, id: &str, _password: Option<Password<'_>>) -> bool {
        let before = self.passkeys.len();
        self.passkeys.retain(|p| p.id != id);
        self.passkeys.len() < before
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
    fn stop_activity_confirmed(&mut self, password: Password<'_>) -> bool {
        if password.for_opaque() != self.accepts.as_bytes() {
            return false;
        }
        self.activity = false;
        self.wire.push("PUT account/activity-tracking off password".into());
        true
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
    fn cancel_deletion_request(&mut self, confirmation: StepUp) -> Result<Deletion, Refused> {
        if !self.redeem(confirmation, Purpose::DeleteAccount) {
            return Err(Refused::NotConfirmed);
        }
        // The board's flow: a cancel before the link is used stops it.
        self.deletion = Deletion::None;
        self.wire.push("POST auth/account/delete-cancel step-up-token".into());
        Ok(Deletion::None)
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
    fn begin_approval(&mut self, request: &str, _code: &str, _password: Password<'_>) {
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
    fn undo_deletion(&mut self, confirmation: StepUp) -> Result<Deletion, Refused> {
        if !self.redeem(confirmation, Purpose::DeleteAccount) {
            return Err(Refused::NotConfirmed);
        }
        if matches!(self.deletion, Deletion::Scheduled { .. }) {
            self.deletion = Deletion::None;
            self.wire.push("POST auth/account/delete-cancel step-up-token".into());
        }
        Ok(self.deletion.clone())
    }
}
