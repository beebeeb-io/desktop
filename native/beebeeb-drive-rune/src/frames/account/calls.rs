//! The calls of the Account pages that Beebeeb answers later (change email, change password, two-factor): the press
//! starts one on the source's worker ([`AccountSource::start_call`]), the page waits, and the window settles the
//! answer ([`settle`]). A press while the same call is out starts nothing and says so; the page waits for that call.
//! A page that was left drops the wait: its answer is then dropped, except a password change's, which is said
//! wherever the person is (`late`). An answer is only ever settled for the press and the turn that asked for it (the
//! call's mark is current), whatever the kind.

use super::form::{self, Problem};
use super::source::{Answered, CallKind, CodeRefused, EmailRefused, Resend};
use super::{AccountUi, Pane};

/// What the start of a press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Start {
    /// This press started the call: the page waits for its answer.
    Started,
    /// The same call was already out: nothing new was started. The page waits for that call, and says so.
    AlreadyOut,
    /// Nothing was started (no open session, nothing that can send it): the page says so; nothing was sent.
    NotStarted,
}

/// Starts `kind` through the source. The serial is read before and after the start: a source that takes a press
/// of a kind that is already out ("taken", nothing new started: the engine's rule) leaves it where it was, and that
/// press is never handed the earlier one's answer as its own.
pub(super) fn start(ui: &mut AccountUi, kind: CallKind, input: &str, password: &str) -> Start {
    let Some(source) = ui.source.as_deref_mut() else {
        return Start::NotStarted;
    };
    let before = source.call_mark(kind).map(|mark| mark.serial);
    if !source.start_call(kind, input, password) {
        return Start::NotStarted;
    }
    let after = source.call_mark(kind).map(|mark| mark.serial);
    ui.form.waiting_serial = after.unwrap_or(0);
    ui.form.waiting = Some(kind);
    if after.is_some() && after != before {
        Start::Started
    } else {
        ui.form.problem = Some(Problem::StillOut);
        Start::AlreadyOut
    }
}

/// The set-up page asks Beebeeb for a key; with no open session it says the set-up could not start.
pub(super) fn ask_for_a_key(ui: &mut AccountUi) {
    if start(ui, CallKind::BeginTwoFactor, "", "") == Start::NotStarted {
        ui.form.problem = Some(Problem::Offline);
    }
}

fn code_problem(why: CodeRefused) -> Problem {
    if why == CodeRefused::Offline {
        Problem::Offline
    } else {
        Problem::WrongCode
    }
}

/// An answer for the call the page waits for: the frame to show next, or `None` (stay; a problem may be set). An
/// answer for a call the page no longer waits for is dropped (a password change's is kept and said: `late`).
pub fn settle(ui: &mut AccountUi, kind: CallKind, answered: Answered) -> Option<&'static str> {
    let mark = ui.source.as_deref().and_then(|s| s.call_mark(kind));
    if ui.form.waiting != Some(kind) {
        if kind == CallKind::ChangePassword {
            super::late::keep(ui, mark, answered);
        }
        return None;
    }
    // An answer belongs to its own press: one for an earlier call never completes a later one.
    if mark.is_some_and(|mark| mark.serial != ui.form.waiting_serial) {
        return None;
    }
    ui.form.waiting = None;
    // What the page said while it waited ("still being sent") goes with the wait.
    ui.form.problem = None;
    // A wait a page adopted (on its return, or through `AlreadyOut`) can belong to a press of another turn of the
    // same account (a lock and an unlock, a sign-in again): its answer is not this turn's to say, for every kind (#440
    // review). The page is live again; the set-up page asks for a key of its own (the earlier one was wiped).
    if mark.is_some_and(|mark| !mark.current) {
        if kind == CallKind::BeginTwoFactor && ui.pane == Pane::TwoFactorSetup && ui.form.setup.is_none() {
            ask_for_a_key(ui);
        }
        return None;
    }
    let f = &mut ui.form;
    match (kind, answered) {
        (CallKind::StartEmail, Answered::Done) => Some("DW-A66"),
        (CallKind::StartEmail, Answered::Email(why)) => {
            f.problem = Some(match why {
                EmailRefused::InUse => Problem::EmailInUse,
                EmailRefused::NotAnAddress => Problem::NotAnAddress,
                EmailRefused::NotConfirmed => Problem::WrongPassword,
                EmailRefused::Offline => Problem::Offline,
            });
            // The password went with the press: the field is empty, and is typed again.
            f.current = form::secure();
            let key = if why == EmailRefused::NotConfirmed {
                form::CURRENT_KEY
            } else {
                form::EMAIL_KEY
            };
            form::focus(f, Some(key));
            None
        }
        (CallKind::ChangePassword, Answered::Done) => Some("DW-A52"),
        // DW-A68: the new password is kept, only the current one is retyped. A new password Beebeeb did not take is
        // wiped with it: it is typed again.
        (CallKind::ChangePassword, Answered::Refused(why)) => {
            let problem = super::forms::refused(why);
            f.problem = Some(problem);
            f.current = form::secure();
            if problem == Problem::NotAccepted {
                (f.new, f.again) = (form::secure(), form::secure());
                form::focus(f, Some(form::NEW_KEY));
            } else {
                form::focus(f, Some(form::CURRENT_KEY));
            }
            None
        }
        (CallKind::ConfirmEmail, Answered::Done) => Some("DW-A51"),
        (CallKind::ResendEmail, Answered::Done) => {
            f.problem = None;
            None
        }
        // The change is gone: back to step 1.
        (CallKind::ResendEmail, Answered::Resend(Resend::Gone)) => {
            f.problem = None;
            Some("DW-A65")
        }
        (CallKind::ResendEmail, Answered::Resend(why)) => {
            f.problem = Some(match why {
                Resend::Wait(_) => Problem::ResendWait,
                Resend::Limit => Problem::ResendLimit,
                _ => Problem::Offline,
            });
            None
        }
        (CallKind::BeginTwoFactor, Answered::Setup(setup)) => {
            f.setup = Some(setup);
            None
        }
        (CallKind::TurnOnTwoFactor, Answered::Codes(codes)) | (CallKind::NewBackupCodes, Answered::Codes(codes)) => {
            f.codes = Some(codes);
            ui.pane = Pane::BackupCodes;
            Some("DW-A70")
        }
        (CallKind::TurnOnTwoFactor, Answered::NewKey(setup)) => {
            f.setup = Some(setup);
            f.key_replaced = true;
            f.problem = Some(Problem::WrongCode);
            f.code = form::fresh_code();
            form::focus(f, Some(form::CODE_KEY));
            None
        }
        (CallKind::TurnOffTwoFactor, Answered::Done) => Some("DW-A52"),
        (CallKind::CheckCode, Answered::Done) => Some("DW-A69"),
        (_, Answered::Code(why)) => {
            f.problem = Some(code_problem(why));
            form::focus(f, Some(form::CODE_KEY));
            None
        }
        // Not taken: the page stays, and says so (an answer of another shape than the call's is a refusal too).
        (_, _) => {
            f.problem = Some(Problem::Offline);
            None
        }
    }
}
