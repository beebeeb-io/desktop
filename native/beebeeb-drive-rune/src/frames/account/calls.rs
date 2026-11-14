//! The calls of the Account pages that Beebeeb answers later (change email, two-factor): the press starts one on the
//! source's worker ([`AccountSource::start_call`]), the page waits, and the window settles the answer
//! ([`settle`]) the way the press settled it when the source answered at once. A press while one is out does
//! nothing; a page that was left drops the wait and the answer with it.

use super::form::{self, Problem};
use super::source::{Answered, CallKind, CodeRefused, EmailRefused};
use super::{AccountUi, Pane};

/// Starts `kind`. `true`: it is out (the page waits for its answer); `false`: the source answers the sync methods.
pub(super) fn start(ui: &mut AccountUi, kind: CallKind, input: &str, password: &str) -> bool {
    let Some(source) = ui.source.as_deref_mut() else {
        return false;
    };
    if source.start_call(kind, input, password) {
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
        (CallKind::ConfirmEmail, Answered::Done) => Some("DW-A51"),
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
