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
//! - `two_factor()`: Beebeeb's status when it was read for this session (`account_twofactor_status.rs`: added, last
//!   used, backup codes left); before that, or when the read failed, only whether it is on (the profile's mark).
//!
//! Session rule: a worker is stamped with the session and account of the press; an answer, and a held key, that is
//! not the engine's session and account now is dropped (a held key is wiped). A press while the same call is out
//! sends nothing new. Must not: log a code, a key, a backup code or an address.
//!
//! Backup codes across a screen lock (review of desktop#424): codes Beebeeb made (the old ones stopped working) are
//! never wiped unseen while the same account stays signed in. Codes whose answer lands after the session's turn moved
//! (a screen lock moves it), and codes a page was showing when a lock ended it ([`EngineAccountSource::a_keep_codes`]),
//! are kept here, outside the page, until the screen is unlocked; then the window shows them once
//! ([`EngineAccountSource::a_late_codes`]). A sign-out, a closed vault, a session Beebeeb no longer accepts or another
//! account drops (wipes) them. They are never handed over while the screen is locked.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::account_email_resend_api::EmailResendError;
use beebeeb_engine::app::account_twofactor_api::{AccountFlowFailure, PendingEnrollment};
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use super::{EngineAccountSource, Stamp, account_of, stamp};
use crate::frames::account::Pane;
use crate::frames::account::source::{
    Answered, CallKind, CallMark, CodeRefused, EmailRefused, PaneState, Refused, Resend, Secret, Setup, TwoFactor,
};

/// A key made for one session and account, waiting for its first code.
type Held = Arc<Mutex<Option<(Stamp, PendingEnrollment)>>>;

/// The address Beebeeb named for the email change that waits for its code, for the session and account it was made
/// for.
type Pending = Arc<Mutex<Option<(Stamp, String)>>>;

/// One call's answer: the session and account it was made for, the turn the call's OWN write moved to (a changed
/// password stores its new session token: the generation the engine's claim produced, handed back), and the answer.
type Done = (Stamp, Option<Stamp>, CallKind, Answered);

/// Backup codes kept for after a screen lock, with the session and account they were made for (wiped on drop).
struct Unseen {
    made_for: Stamp,
    kind: CallKind,
    codes: Vec<Secret>,
}

pub(super) struct Calls {
    /// The calls that are out: a second press of the same call sends nothing.
    out: Vec<CallKind>,
    /// How many calls were started.
    counter: u64,
    /// The latest call of each kind: its serial, the session and account its press was made under (set at the
    /// start), and the turn the call's own write moved to (a changed password; a confirmed email's address at the
    /// drain), never a turn a lock or a sign-in moved to. A late answer is current only while the engine's session and
    /// account are still one of those two.
    latest: Vec<(CallKind, u64, Stamp, Option<Stamp>)>,
    held: Held,
    pending_email: Pending,
    /// A wait Beebeeb asked for before the email's code is sent again.
    resend: super::email::Waiting,
    answers: mpsc::Sender<Done>,
    inbox: mpsc::Receiver<Done>,
    /// Codes a screen lock kept from the page, until the unlock.
    unseen: Option<Unseen>,
    /// The last codes handed to the page: what they were made for, so a lock that ends the page can keep them.
    handed: Option<(Stamp, CallKind)>,
}

impl Calls {
    pub(super) fn new() -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            out: Vec::new(),
            counter: 0,
            latest: Vec::new(),
            held: Arc::default(),
            pending_email: Arc::default(),
            resend: super::email::Waiting::default(),
            answers,
            inbox,
            unseen: None,
            handed: None,
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

fn remember(pending: &Pending, made_for: Stamp, address: String) {
    if let Ok(mut held) = pending.lock() {
        *held = Some((made_for, address));
    }
}

/// The work of one call, on its worker: its answer, and the generation its own write moved to (a changed password
/// only: the engine hands back what its claim produced).
fn work<C: CredentialStores>(
    engine: &Engine<C>,
    held: &Held,
    pending_email: &Pending,
    made_for: Stamp,
    kind: CallKind,
    input: Zeroizing<String>,
    password: Zeroizing<String>,
) -> (Answered, Option<SessionGeneration>) {
    let session = made_for.0;
    let code = input.as_str();
    // The generation the call's own write moved to: a changed password's only.
    let mut own = None;
    let answered = match kind {
        CallKind::StartEmail => match engine.email_change_start(session, code, password.as_str()) {
            Ok(address) => {
                remember(pending_email, made_for, address);
                Answered::Done
            }
            Err(why) => Answered::Email(email_why(why)),
        },
        CallKind::ResendEmail => match engine.email_change_resend(session) {
            Ok(sent) => {
                // The server's own word for where the code went.
                remember(pending_email, made_for, sent.pending_email);
                Answered::Done
            }
            Err(why) => Answered::Resend(match why {
                EmailResendError::Wait(seconds) => Resend::Wait(seconds.min(u64::from(u32::MAX)) as u32),
                EmailResendError::Limit => Resend::Limit,
                EmailResendError::Gone => Resend::Gone,
                EmailResendError::NotSignedIn | EmailResendError::SessionEnded | EmailResendError::Offline => {
                    Resend::Offline
                }
                EmailResendError::Failed => Resend::Offline,
            }),
        },
        // The current password is the one the step-up kept, handed in as `password` by `start_a_call`.
        CallKind::ChangePassword => match engine.change_password_for(session, password, input) {
            Ok(moved_to) => {
                own = Some(moved_to);
                Answered::Done
            }
            Err(why) => Answered::Refused(super::password_change::refusal(why)),
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
                return (Answered::Setup(setup), None);
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
                return (Answered::Code(CodeRefused::Wrong), None);
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
    };
    (answered, own)
}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    /// See [`crate::frames::account::source::AccountSource::call_mark`].
    pub(super) fn a_call_mark(&self, kind: CallKind) -> Option<CallMark> {
        let (_, serial, made_for, answered_under) = self.calls.latest.iter().find(|(k, _, _, _)| *k == kind)?;
        let out = self.calls.out.contains(&kind);
        let now = stamp(&self.engine);
        Some(CallMark {
            serial: *serial,
            out,
            // Per kind and per press (desktop#427, the lead's follow-up): one slot for every kind let another kind's
            // answer make this one current again, and the turn of the drain let a same-account sign-in say it. Only
            // the call's OWN write extends it (#440 review): the generation the engine's claim handed back.
            current: out || (self.open() && (*made_for == now || answered_under.as_ref() == Some(&now))),
        })
    }

    /// The turn `kind`'s call made by its own write: its press is current there too.
    fn restamp(&mut self, kind: CallKind, made_for: &Stamp, own: Stamp) {
        if let Some(entry) = self
            .calls
            .latest
            .iter_mut()
            .find(|(k, _, made, _)| *k == kind && made == made_for)
        {
            entry.3 = Some(own);
        }
    }

    /// See [`crate::frames::account::source::AccountSource::start_call`].
    pub(super) fn start_a_call(&mut self, kind: CallKind, input: &str, password: &str) -> bool {
        if !self.open() {
            return false;
        }
        if self.calls.out.contains(&kind) {
            if kind == CallKind::ChangePassword {
                // A second press while a change is out is never "started", and never shares the first one's
                // answer: refused, and the password its step-up kept is wiped (review of desktop#427).
                self.password.forget();
                return false;
            }
            return true;
        }
        let made_for = stamp(&self.engine);
        let made_for_mark = made_for.clone();
        let (engine, held, answers) = (self.engine.clone(), self.calls.held.clone(), self.calls.answers.clone());
        let pending = self.calls.pending_email.clone();
        let input = Zeroizing::new(input.to_string());
        let password = if kind == CallKind::ChangePassword {
            // The password the step-up of this press kept, for this session and account: used once, here. Nothing
            // kept (or kept for another session): not started, and the sync method refuses it as not confirmed.
            match self.password.take_kept_for(&made_for) {
                Some(kept) => kept,
                None => return false,
            }
        } else {
            Zeroizing::new(password.to_string())
        };
        let started = std::thread::Builder::new()
            .name("account-call".into())
            .spawn(move || {
                let (answered, own) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(&engine, &held, &pending, made_for.clone(), kind, input, password)
                }))
                .unwrap_or((Answered::Refused(Refused::Offline), None));
                // The turn the call's own write moved to (never one read afterwards: a lock or a sign-in in between
                // would pass for it), for the same account as the press.
                let own = own.map(|moved_to| (moved_to, made_for.1.clone()));
                let _ = answers.send((made_for, own, kind, answered));
                engine.wake_host();
            })
            .is_ok();
        if started {
            self.calls.out.push(kind);
            self.calls.counter += 1;
            self.calls.latest.retain(|(k, _, _, _)| *k != kind);
            self.calls.latest.push((kind, self.calls.counter, made_for_mark, None));
        }
        started
    }

    /// Whether the engine's screen is locked now (the session stays in memory under it).
    fn screen_locked(&self) -> bool {
        self.engine.gate() == beebeeb_engine::signin::gate::GateState::ScreenLocked
    }

    /// Codes kept for after a lock stay only while their account is signed in with a session Beebeeb accepts;
    /// otherwise they are wiped here.
    fn forget_unseen_of_another_session(&mut self) {
        let account_now = stamp(&self.engine).1;
        let still = self.open();
        let keep = |made_for: &Stamp| still && made_for.1.is_some() && made_for.1 == account_now;
        if self.calls.unseen.as_ref().is_some_and(|unseen| !keep(&unseen.made_for)) {
            self.calls.unseen = None;
        }
        if self.calls.handed.as_ref().is_some_and(|(made_for, _)| !keep(made_for)) {
            self.calls.handed = None;
        }
    }

    /// What changed on the account is read again once its answer reaches the page, so the pages say Beebeeb's.
    fn read_again_after(&mut self, kind: CallKind, answered: &Answered) {
        let changed = matches!(
            (&kind, answered),
            (CallKind::ConfirmEmail, Answered::Done)
                | (CallKind::TurnOffTwoFactor, Answered::Done)
                | (CallKind::TurnOnTwoFactor, Answered::Codes(_))
        );
        if changed {
            crate::frames::account::source::AccountSource::reload(self, Pane::Account);
            if !matches!(kind, CallKind::ConfirmEmail) {
                // Two-factor was changed: what the status holds is out of date.
                self.two_factor_status.stale();
            }
        }
    }

    /// See [`crate::frames::account::source::AccountSource::late_codes`].
    pub(super) fn a_late_codes(&mut self) -> Option<(CallKind, Vec<Secret>)> {
        self.forget_unseen_of_another_session();
        if self.calls.unseen.is_none() || self.screen_locked() {
            return None;
        }
        let Unseen { made_for, kind, codes } = self.calls.unseen.take()?;
        let answered = Answered::Codes(codes);
        self.read_again_after(kind, &answered);
        let Answered::Codes(codes) = answered else {
            return None;
        };
        self.calls.handed = Some((made_for, kind));
        Some((kind, codes))
    }

    /// See [`crate::frames::account::source::AccountSource::keep_codes`].
    pub(super) fn a_keep_codes(&mut self, codes: Vec<Secret>) {
        self.forget_unseen_of_another_session();
        let Some((made_for, kind)) = self.calls.handed.take() else {
            return;
        };
        if self.screen_locked() && self.calls.unseen.is_none() {
            self.calls.unseen = Some(Unseen { made_for, kind, codes });
        }
    }

    /// See [`crate::frames::account::source::AccountSource::call_answer`].
    pub(super) fn a_call_answer(&mut self) -> Option<(CallKind, Answered)> {
        self.forget_unseen_of_another_session();
        while let Ok((made_for, own, kind, answered)) = self.calls.inbox.try_recv() {
            self.calls.out.retain(|k| *k != kind);
            // The turn may have moved since the press (a screen lock moves it) while the request was out and Beebeeb
            // may already have changed the account: an answer for the SAME account still reaches the pane, once (new
            // backup codes are never wiped unseen). Another account's, or an ended session's, is dropped (wiped) here.
            let now = stamp(&self.engine);
            if now.1 != made_for.1 || !self.open() {
                self.calls.drop_held();
                continue;
            }
            // A call whose own write moved the turn (a changed password) is still that press's under exactly that
            // turn; nothing else extends the mark (a lock and an unlock, or a sign-in, during the call never do).
            if let Some(own) = own {
                self.restamp(kind, &made_for, own);
            }
            if now != made_for {
                // What is held under the old turn (a key that waits) is the old turn's: wiped. The answer stays.
                self.calls.drop_held();
                // Codes Beebeeb made while the turn moved (a screen lock moves it): the page that asked may have been
                // ended by the lock. They are kept here and handed over after the unlock (`late_codes`).
                if let Answered::Codes(codes) = answered {
                    self.calls.unseen = Some(Unseen { made_for, kind, codes });
                    continue;
                }
            }
            // A wait Beebeeb asked for counts down from now; a new change or a confirmed one starts clean.
            match (&kind, &answered) {
                (CallKind::ResendEmail, Answered::Resend(Resend::Wait(seconds))) => {
                    self.calls.resend.start(made_for.clone(), *seconds);
                }
                (CallKind::StartEmail | CallKind::ConfirmEmail, Answered::Done) => self.calls.resend.clear(),
                _ => {}
            }
            // The code was taken: the address Beebeeb named replaces the one this engine mirrors. After the stamp was
            // checked above (the mirror moves the session on); the page reads the account again below.
            if let (CallKind::ConfirmEmail, Answered::Done) = (&kind, &answered) {
                let address = self
                    .calls
                    .pending_email
                    .lock()
                    .ok()
                    .and_then(|mut held| held.take())
                    .filter(|(held_for, _)| *held_for == made_for)
                    .map(|(_, address)| address);
                if let Some(address) = address
                    && let Some(moved_to) = self.engine.email_changed_for(made_for.0, &address)
                {
                    // The drain's own write moved the turn (and the address the stamp names): the answer is still
                    // this press's under exactly that turn.
                    self.restamp(kind, &made_for, (moved_to, account_of(&self.engine)));
                }
            }
            // What changed on the account is read again, so what the pages say is Beebeeb's.
            self.read_again_after(kind, &answered);
            if matches!(answered, Answered::Codes(_)) {
                self.calls.handed = Some((made_for, kind));
            }
            return Some((kind, answered));
        }
        None
    }

    /// The address Beebeeb named for the email change that waits for its code, while the session and account are the
    /// ones it was named for.
    pub(super) fn pending_address(&self) -> Option<String> {
        let now = stamp(&self.engine);
        let held = self.calls.pending_email.lock().ok()?;
        held.as_ref()
            .filter(|(made_for, _)| *made_for == now)
            .map(|(_, address)| address.clone())
    }

    /// Whole seconds before "Send again" may be pressed (Beebeeb's `Retry-After`, counted down); 0: now.
    pub(super) fn resend_left(&self) -> u32 {
        self.calls.resend.left(&stamp(&self.engine))
    }

    /// While a wait counts down the window looks again every second (never from a draw).
    pub(super) fn resend_tick(&self) -> Option<Duration> {
        (self.resend_left() > 0).then_some(Duration::from_secs(1))
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

    /// Two-factor as far as it is known: Beebeeb's own status when it was read for this session (added, last used,
    /// backup codes), else on or off from the profile's mark, and nothing invented.
    pub(super) fn two_factor_known(&self) -> TwoFactor {
        if let Some(status) = self.two_factor_status.known() {
            return status;
        }
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
