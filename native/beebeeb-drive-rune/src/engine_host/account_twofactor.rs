//! Two-factor and the calls that Beebeeb answers later, on the real engine: the worker, the answer's stamp and the
//! key a set-up holds between its two steps. Part of [`EngineAccountSource`] (`account.rs`).
//!
//! - `begin_two_factor`: a new authenticator key from Beebeeb (`two_factor_begin`), drawn as a QR code and the key
//!   in groups of four. The key and core's backup codes stay in [`PendingEnrollment`], held here for this session
//!   and account only, until the first code is taken or the pane is left.
//! - `turn_on_two_factor`: the first code. Core spends the key when a code is refused, so a wrong code ends with a
//!   NEW key (`Answered::NewKey`) and the page says to scan again; the codes it returns are shown once.
//! - `check_code` (moving to a new authenticator): a current code starts the set-up of the new key
//!   (`two_factor_begin` with the code); the key waits here for `begin_two_factor`.
//! - `new_backup_codes`, `turn_off_two_factor`: a current code each.
//! - `two_factor()`: only whether it is on (the profile's mark). When it was added, when it was last used and how
//!   many backup codes are left are NOT known: Beebeeb has no route that says (see the PR record).
//!
//! Session rule: a worker is stamped with the session and account of the press; an answer, and a held key, that is
//! not the engine's session and account now is dropped (a held key is wiped). A press while the same call is out
//! sends nothing new. Must not: log a code, a key, a backup code or an address.

use std::sync::{Arc, Mutex, mpsc};

use beebeeb_engine::app::account_twofactor_api::{AccountFlowFailure, PendingEnrollment};
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use super::{EngineAccountSource, Stamp, stamp};
use crate::frames::account::Pane;
use crate::frames::account::source::{
    Answered, CallKind, CodeRefused, EmailRefused, PaneState, Refused, Secret, Setup, TwoFactor,
};

/// A key made for one session and account, waiting for its first code.
type Held = Arc<Mutex<Option<(Stamp, PendingEnrollment)>>>;

/// One call's answer, with the session and account it was made for.
type Done = (Stamp, CallKind, Answered);

pub(super) struct Calls {
    /// The calls that are out: a second press of the same call sends nothing.
    out: Vec<CallKind>,
    held: Held,
    answers: mpsc::Sender<Done>,
    inbox: mpsc::Receiver<Done>,
}

impl Calls {
    pub(super) fn new() -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            out: Vec::new(),
            held: Arc::default(),
            answers,
            inbox,
        }
    }

    /// The pane is left: the key it held is wiped.
    pub(super) fn drop_held(&mut self) {
        if let Ok(mut held) = self.held.lock() {
            *held = None;
        }
    }
}

/// The QR code of a link: dark modules, row by row. `None` when it does not fit a code.
fn modules_of(link: &str) -> Option<Vec<Vec<bool>>> {
    let code = qrcode::QrCode::new(link.as_bytes()).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    Some(
        colors
            .chunks(width)
            .map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect())
            .collect(),
    )
}

/// The key in groups of four: "JBSW Y3DP EHPK 3PXP".
fn grouped(key: &str) -> String {
    key.as_bytes()
        .chunks(4)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn setup_of(pending: &PendingEnrollment) -> Option<Setup> {
    Some(Setup {
        modules: modules_of(pending.otpauth_uri())?,
        key: Secret::new(grouped(pending.key())),
    })
}

fn code_why(failure: AccountFlowFailure) -> CodeRefused {
    match failure {
        AccountFlowFailure::WrongCode => CodeRefused::Wrong,
        AccountFlowFailure::NeedsConfirmation => CodeRefused::NotConfirmed,
        _ => CodeRefused::Offline,
    }
}

fn email_why(failure: AccountFlowFailure) -> EmailRefused {
    match failure {
        AccountFlowFailure::EmailInUse => EmailRefused::InUse,
        AccountFlowFailure::NotAnAddress => EmailRefused::NotAnAddress,
        AccountFlowFailure::WrongPassword | AccountFlowFailure::NeedsConfirmation => EmailRefused::NotConfirmed,
        _ => EmailRefused::Offline,
    }
}

fn secrets(codes: Zeroizing<Vec<String>>) -> Vec<Secret> {
    codes.iter().map(|code| Secret::new(code.clone())).collect()
}

/// A key made and held for `stamp`, as the set-up page draws it.
fn hold(held: &Held, stamp: Stamp, pending: PendingEnrollment) -> Option<Setup> {
    let setup = setup_of(&pending)?;
    *held.lock().ok()? = Some((stamp, pending));
    Some(setup)
}

/// The work of one call, on its worker.
fn work<C: CredentialStores>(
    engine: &Engine<C>,
    held: &Held,
    made_for: Stamp,
    kind: CallKind,
    input: Zeroizing<String>,
    password: Zeroizing<String>,
) -> Answered {
    let session = made_for.0;
    let code = input.as_str();
    match kind {
        CallKind::StartEmail => match engine.email_change_start(session, code, password.as_str()) {
            Ok(_) => Answered::Done,
            Err(why) => Answered::Email(email_why(why)),
        },
        CallKind::ConfirmEmail => match engine.email_change_confirm(session, code) {
            Ok(()) => Answered::Done,
            Err(why) => Answered::Code(code_why(why)),
        },
        CallKind::BeginTwoFactor => {
            // A key that waits (from `check_code`) is the one to show; else a new key from Beebeeb.
            let waiting = held.lock().ok().and_then(|held| {
                held.as_ref()
                    .filter(|(held_for, _)| *held_for == made_for)
                    .and_then(|(_, pending)| setup_of(pending))
            });
            if let Some(setup) = waiting {
                return Answered::Setup(setup);
            }
            match engine.two_factor_begin(session, None) {
                Ok(pending) => {
                    hold(held, made_for, pending).map_or(Answered::Refused(Refused::Offline), Answered::Setup)
                }
                Err(AccountFlowFailure::NeedsConfirmation) => Answered::Refused(Refused::NotConfirmed),
                Err(_) => Answered::Refused(Refused::Offline),
            }
        }
        CallKind::CheckCode => match engine.two_factor_begin(session, Some(code)) {
            Ok(pending) => {
                if hold(held, made_for, pending).is_some() {
                    Answered::Done
                } else {
                    Answered::Code(CodeRefused::Offline)
                }
            }
            Err(why) => Answered::Code(code_why(why)),
        },
        CallKind::TurnOnTwoFactor => {
            let pending = held.lock().ok().and_then(|mut held| match held.take() {
                Some((held_for, pending)) if held_for == made_for => Some(pending),
                _ => None,
            });
            let Some(pending) = pending else {
                return Answered::Code(CodeRefused::Wrong);
            };
            match engine.two_factor_turn_on(session, pending, code) {
                Ok(codes) => Answered::Codes(secrets(codes)),
                Err(AccountFlowFailure::WrongCode) => {
                    // The key was spent with the code (core's rule): the page gets a new one.
                    match engine.two_factor_begin(session, None) {
                        Ok(pending) => {
                            hold(held, made_for, pending).map_or(Answered::Code(CodeRefused::Wrong), Answered::NewKey)
                        }
                        Err(_) => Answered::Code(CodeRefused::Wrong),
                    }
                }
                Err(why) => Answered::Code(code_why(why)),
            }
        }
        CallKind::TurnOffTwoFactor => match engine.two_factor_turn_off(session, code) {
            Ok(()) => Answered::Done,
            Err(why) => Answered::Code(code_why(why)),
        },
        CallKind::NewBackupCodes => match engine.two_factor_new_backup_codes(session, code) {
            Ok(codes) => Answered::Codes(secrets(codes)),
            Err(why) => Answered::Code(code_why(why)),
        },
    }
}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    /// See [`crate::frames::account::source::AccountSource::start_call`].
    pub(super) fn start_a_call(&mut self, kind: CallKind, input: &str, password: &str) -> bool {
        if !self.open() {
            return false;
        }
        if self.calls.out.contains(&kind) {
            return true;
        }
        let made_for = stamp(&self.engine);
        let (engine, held, answers) = (self.engine.clone(), self.calls.held.clone(), self.calls.answers.clone());
        let (input, password) = (Zeroizing::new(input.to_string()), Zeroizing::new(password.to_string()));
        let started = std::thread::Builder::new()
            .name("account-call".into())
            .spawn(move || {
                let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(&engine, &held, made_for.clone(), kind, input, password)
                }))
                .unwrap_or(Answered::Refused(Refused::Offline));
                let _ = answers.send((made_for, kind, answered));
                engine.wake_host();
            })
            .is_ok();
        if started {
            self.calls.out.push(kind);
        }
        started
    }

    /// See [`crate::frames::account::source::AccountSource::call_answer`].
    pub(super) fn a_call_answer(&mut self) -> Option<(CallKind, Answered)> {
        while let Ok((made_for, kind, answered)) = self.calls.inbox.try_recv() {
            self.calls.out.retain(|k| *k != kind);
            if stamp(&self.engine) != made_for || !self.open() {
                // A session or an account that is over: its answer (and a code it holds) is wiped here.
                self.calls.drop_held();
                continue;
            }
            // What changed on the account is read again, so what the pages say is Beebeeb's.
            let changed = match (&kind, &answered) {
                (CallKind::ConfirmEmail, Answered::Done) | (CallKind::TurnOffTwoFactor, Answered::Done) => true,
                (CallKind::TurnOnTwoFactor, Answered::Codes(_)) => true,
                _ => false,
            };
            if changed {
                crate::frames::account::source::AccountSource::reload(self, Pane::Account);
            }
            return Some((kind, answered));
        }
        None
    }

    /// The panes of this file: ready while a session is open. Two-factor's own status needs the profile's mark.
    pub(super) fn wired_pane(&self, pane: Pane) -> Option<PaneState> {
        if !matches!(
            pane,
            Pane::Email | Pane::EmailCode | Pane::TwoFactorSetup | Pane::BackupCodes
        ) {
            return None;
        }
        Some(if !self.engine.has_session() {
            PaneState::SignedOut
        } else if self.refused() {
            PaneState::SessionEnded
        } else {
            PaneState::Ready
        })
    }

    /// Two-factor as far as it is known: on or off from the profile's mark, and nothing invented.
    pub(super) fn two_factor_known(&self) -> TwoFactor {
        let on = self
            .current(&self.overview)
            .and_then(|overview| overview.two_factor)
            .unwrap_or(false);
        TwoFactor {
            on,
            added: String::new(),
            last_used: String::new(),
            codes_left: 0,
            codes_total: 0,
        }
    }
}

#[cfg(test)]
#[path = "account_twofactor_tests.rs"]
mod tests;
