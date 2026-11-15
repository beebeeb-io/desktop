//! Every account call the window can start, pressed from its real control against a source that holds the call as
//! out: its answer (Done AND a refusal) must settle the page and draw its result. A call that was started without
//! the one path that records which press it belongs to would hang the page silently (review of desktop#427).

use super::form::{self, Problem};
use super::source::{
    Answered, BOARD_PASSWORD, BoardAccount, CallKind, CodeRefused, EmailRefused, HeldCalls, Refused, Resend, Secret,
    Setup,
};
use super::tests::{click, says, settle, window};
use super::{CodeFor, Dialog, Pane};
use crate::{State, init};
use rune::Runtime;

struct Rig {
    rt: Runtime<State>,
    held: HeldCalls,
}

fn rig(frame: &str) -> Rig {
    let held = HeldCalls::default();
    // The source is in place before the scene opens, so a page that starts a call on arrival starts it here.
    let mut state = init();
    state.account.source = Some(Box::new(BoardAccount::new().holding_calls(held.clone())));
    assert!(super::scene(frame, &mut state), "{frame} is a built frame");
    let mut rt = window(state);
    settle(&mut rt);
    Rig { rt, held }
}

impl Rig {
    fn started(&self) -> Vec<CallKind> {
        self.held.started.borrow().clone()
    }
    fn form(&mut self, f: impl FnOnce(&mut form::Form)) {
        self.rt.update(|s, _| f(&mut s.account.form));
        settle(&mut self.rt);
    }
    /// The answer lands between frames, as a worker's does.
    fn answer(&mut self, answered: Answered) {
        *self.held.answer.borrow_mut() = Some(answered);
        settle(&mut self.rt);
    }
    fn pane(&self) -> Pane {
        self.rt.state.account.pane
    }
    fn waiting(&self) -> bool {
        self.rt.state.account.form.waiting.is_some()
    }
    fn problem(&self) -> Option<Problem> {
        self.rt.state.account.form.problem
    }
}

const SETUP_DONE: fn() -> Answered = || {
    Answered::Setup(Setup {
        modules: vec![vec![true; 21]; 21],
        key: Secret::new("JBSW Y3DP EHPK 3PXP"),
    })
};
const CODES: fn() -> Answered = || Answered::Codes(vec![Secret::new("48271590")]);

/// Opens a code confirm over the Two-factor page and types a full code.
fn code_confirm(for_: CodeFor) -> Rig {
    let mut r = rig("DW-A71");
    r.rt.update(|s, _| s.account.dialog = Some(Dialog::Code(for_)));
    settle(&mut r.rt);
    r.form(|f| form::type_in(&mut f.code, "514208"));
    click(&mut r.rt, "btn-0");
    r
}

fn change_password() -> Rig {
    let mut r = rig("DW-A67");
    r.form(|f| {
        form::type_in(&mut f.current, BOARD_PASSWORD);
        form::type_in(&mut f.new, "a new long passphrase 42");
        form::type_in(&mut f.again, "a new long passphrase 42");
    });
    click(&mut r.rt, "btn-1");
    r
}

fn start_email() -> Rig {
    let mut r = rig("DW-A65");
    r.form(|f| {
        form::type_in(&mut f.email, "new.address@example.eu");
        form::type_in(&mut f.current, BOARD_PASSWORD);
    });
    click(&mut r.rt, "btn-1");
    r
}

fn email_code() -> Rig {
    rig("DW-A66")
}

fn confirm_email() -> Rig {
    let mut r = email_code();
    r.form(|f| form::type_in(&mut f.code, "48271590"));
    click(&mut r.rt, "btn-1");
    r
}

fn resend_email() -> Rig {
    let mut r = email_code();
    click(&mut r.rt, "link-0-box");
    r
}

/// The set-up page, opened: it asks Beebeeb for a key; the key is delivered, then the first code is typed.
fn turn_on() -> Rig {
    let mut r = rig("DW-A69");
    r.answer(SETUP_DONE());
    r.form(|f| form::type_in(&mut f.code, "123456"));
    click(&mut r.rt, "btn-1");
    r
}

#[test]
fn every_account_call_settles_and_draws_its_result_for_a_done_and_for_a_refusal() {
    // (the call, where it starts, what Done leaves, the refusal and what it draws)
    let mut seen = Vec::new();

    // Change password.
    let mut r = change_password();
    assert_eq!(r.started(), [CallKind::ChangePassword]);
    assert!(r.waiting());
    r.answer(Answered::Done);
    assert_eq!(r.pane(), Pane::Security, "ChangePassword Done");
    let mut r = change_password();
    r.answer(Answered::Refused(Refused::WrongPassword));
    assert!(!r.waiting(), "ChangePassword refusal settles");
    assert!(
        says(&r.rt, "field-0-msg").contains("incorrect"),
        "ChangePassword refusal is drawn"
    );
    seen.push(CallKind::ChangePassword);

    // Change email, step 1.
    let mut r = start_email();
    assert_eq!(r.started(), [CallKind::StartEmail], "StartEmail is started");
    r.answer(Answered::Done);
    assert_eq!(r.pane(), Pane::EmailCode, "StartEmail Done");
    let mut r = start_email();
    r.answer(Answered::Email(EmailRefused::Offline));
    assert!(!r.waiting(), "StartEmail refusal settles");
    assert_eq!(r.problem(), Some(Problem::Offline), "StartEmail refusal is drawn");
    assert!(says(&r.rt, "field-0-msg").contains("can't be reached"));
    seen.push(CallKind::StartEmail);

    // Change email, step 2: the code.
    let mut r = confirm_email();
    assert_eq!(r.started(), [CallKind::ConfirmEmail]);
    r.answer(Answered::Done);
    assert_eq!(r.pane(), Pane::Account, "ConfirmEmail Done");
    let mut r = confirm_email();
    r.answer(Answered::Code(CodeRefused::Wrong));
    assert!(!r.waiting(), "ConfirmEmail refusal settles");
    assert!(
        says(&r.rt, "code-msg").contains("wrong or has expired"),
        "ConfirmEmail refusal is drawn"
    );
    seen.push(CallKind::ConfirmEmail);

    // Send again.
    let mut r = resend_email();
    assert_eq!(r.started(), [CallKind::ResendEmail]);
    assert!(r.waiting());
    r.answer(Answered::Done);
    assert!(!r.waiting(), "ResendEmail Done settles");
    let mut r = resend_email();
    r.answer(Answered::Resend(Resend::Limit));
    assert!(!r.waiting(), "ResendEmail refusal settles");
    assert!(
        says(&r.rt, "code-msg").contains("No more codes"),
        "ResendEmail refusal is drawn"
    );
    seen.push(CallKind::ResendEmail);

    // Two-factor set-up: the key.
    let mut r = rig("DW-A69");
    assert_eq!(r.started(), [CallKind::BeginTwoFactor]);
    r.answer(SETUP_DONE());
    assert!(
        r.rt.state.account.form.setup.is_some(),
        "BeginTwoFactor Done draws the key"
    );
    let mut r = rig("DW-A69");
    r.answer(Answered::Refused(Refused::Offline));
    assert!(!r.waiting(), "BeginTwoFactor refusal settles");
    assert_eq!(r.problem(), Some(Problem::Offline), "BeginTwoFactor refusal is drawn");
    seen.push(CallKind::BeginTwoFactor);

    // Two-factor set-up: the first code.
    let mut r = turn_on();
    assert_eq!(r.started(), [CallKind::BeginTwoFactor, CallKind::TurnOnTwoFactor]);
    r.answer(CODES());
    assert_eq!(r.pane(), Pane::BackupCodes, "TurnOnTwoFactor Done");
    let mut r = turn_on();
    r.answer(Answered::Code(CodeRefused::Wrong));
    assert!(!r.waiting(), "TurnOnTwoFactor refusal settles");
    assert_eq!(
        r.problem(),
        Some(Problem::WrongCode),
        "TurnOnTwoFactor refusal is drawn"
    );
    seen.push(CallKind::TurnOnTwoFactor);

    // The three code confirms.
    for (for_, kind, done, frame) in [
        (
            CodeFor::TurnOffTwoFactor,
            CallKind::TurnOffTwoFactor,
            Answered::Done,
            Pane::Security,
        ),
        (
            CodeFor::MoveAuthenticator,
            CallKind::CheckCode,
            Answered::Done,
            Pane::TwoFactorSetup,
        ),
        (
            CodeFor::NewBackupCodes,
            CallKind::NewBackupCodes,
            CODES(),
            Pane::BackupCodes,
        ),
    ] {
        let mut r = code_confirm(for_);
        assert_eq!(r.started(), [kind], "{kind:?} is started");
        r.answer(done);
        assert_eq!(r.pane(), frame, "{kind:?} Done");
        let mut r = code_confirm(for_);
        r.answer(Answered::Code(CodeRefused::Wrong));
        assert!(!r.waiting(), "{kind:?} refusal settles");
        assert_eq!(r.problem(), Some(Problem::WrongCode), "{kind:?} refusal is drawn");
        seen.push(kind);
    }

    // Every call kind the window can start is in the table (a new one must be added here).
    for kind in [
        CallKind::ChangePassword,
        CallKind::StartEmail,
        CallKind::ConfirmEmail,
        CallKind::ResendEmail,
        CallKind::BeginTwoFactor,
        CallKind::TurnOnTwoFactor,
        CallKind::TurnOffTwoFactor,
        CallKind::CheckCode,
        CallKind::NewBackupCodes,
    ] {
        assert!(seen.contains(&kind), "{kind:?} is not covered");
    }
    // A match with no wildcard: adding a kind stops the build here until it has a row above.
    for kind in &seen {
        match kind {
            CallKind::ChangePassword
            | CallKind::StartEmail
            | CallKind::ConfirmEmail
            | CallKind::ResendEmail
            | CallKind::BeginTwoFactor
            | CallKind::TurnOnTwoFactor
            | CallKind::TurnOffTwoFactor
            | CallKind::CheckCode
            | CallKind::NewBackupCodes => {}
        }
    }
}
