//! The calls of the Account pages that Beebeeb answers later (change email, two-factor): the press starts one on the
//! source's worker ([`AccountSource::start_call`]), the page waits, and the window settles the answer
//! ([`settle`]) the way the press settled it when the source answered at once. A press while one is out does
//! nothing; a page that was left drops the wait and the answer with it.

use super::form::{self, Problem};
use super::source::{Answered, CallKind, CodeRefused, EmailRefused, Refused, Resend};
use super::{AccountUi, Pane};

/// Starts `kind`. `true`: it is out (the page waits for its answer); `false`: the source answers the sync methods.
pub(super) fn start(ui: &mut AccountUi, kind: CallKind, input: &str, password: &str) -> bool {
    let Some(source) = ui.source.as_deref_mut() else {
        return false;
    };
    if source.start_call(kind, input, password) {
        ui.form.waiting_serial = source.call_mark(kind).map_or(0, |mark| mark.serial);
        ui.form.waiting = Some(kind);
        true
    } else {
        false
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
/// answer for a call the page no longer waits for is dropped.
pub fn settle(ui: &mut AccountUi, kind: CallKind, answered: Answered) -> Option<&'static str> {
    if ui.form.waiting != Some(kind) {
        // A password change that was answered while its page was not open is kept, and said when the page is
        // opened again (`form::arrive`): never dropped, Unconfirmed included.
        if kind == CallKind::ChangePassword {
            ui.form.late_change = Some(match answered {
                Answered::Done => Ok(()),
                Answered::Refused(why) => Err(why),
                _ => Err(Refused::Offline),
            });
        }
        return None;
    }
    // An answer belongs to its own press: one for an earlier call never completes a later one.
    let serial = ui
        .source
        .as_deref()
        .and_then(|s| s.call_mark(kind))
        .map(|mark| mark.serial);
    if serial.is_some_and(|serial| serial != ui.form.waiting_serial) {
        return None;
    }
    ui.form.waiting = None;
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
            let key = if why == EmailRefused::NotConfirmed {
                form::CURRENT_KEY
            } else {
                form::EMAIL_KEY
            };
            form::focus(f, Some(key));
            None
        }
        (CallKind::ChangePassword, Answered::Done) => Some("DW-A52"),
        // DW-A68: the new password is kept, only the current one is retyped.
        (CallKind::ChangePassword, Answered::Refused(why)) => {
            f.problem = Some(super::forms::refused(why));
            f.current = rune::TextEdit::new("").secure();
            form::focus(f, Some(form::CURRENT_KEY));
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
