//! Deleting the account from this PC (DW-A58, DW-A80 to DW-A83). Part of
//! [`EngineAccountSource`] (`account.rs`): the step-up of [`Purpose::DeleteAccount`] (one line there routes it here),
//! the deletion's panes, its state, the request, the link's use, the cancel and the undo. The engine's calls are
//! `beebeeb_engine::app::bindings::account_deletion` (core's flow, Beebeeb's routes).
//!
//! - The step-up (DW-A58, and the password asked over DW-A81 and DW-A82) proves the password at once
//!   ([`Engine::step_up_for`]), so a wrong one is said on the page that asked it. Core's request runs its OWN step-up
//!   (Beebeeb consumes a step-up's token before it reads the second factor, so a token would be gone after a wrong
//!   code): the typed password is KEPT for the request, as `account_password.rs` keeps it for a change, together
//!   with the step-up's token for a cancel or an undo.
//! - What is kept is for ONE confirmation (its handle and purpose), under the session and account it was typed in,
//!   for [`KEPT_FOR`] at most. It is dropped at once when that session or account moves on (a lock, a sign-out,
//!   another account: [`watch`] looks every [`WATCH_EVERY`], also while no account page is drawn), when the request
//!   was asked, when it is used for a cancel or an undo, and on a new step-up. It is wiped when dropped
//!   ([`Zeroizing`]; the token by core).
//! - A wrong code keeps it (the same handle, the same four minutes) and hands the confirmation back: the person
//!   retypes the code, not the password.
//! - The state is Beebeeb's (`GET …/deletion`), read when the person says the link was used, after a request Beebeeb
//!   refused as already scheduled, and after a cancel it did not take. What was read is kept for the session and
//!   account it was read under and is never shown under another. A scheduled deletion is named by the last moment it
//!   can be undone (Beebeeb's `cancel_until`), never by a length of time this app assumes.
//! - Beebeeb cancels only a SCHEDULED deletion; a link that was not used yet cannot be stopped (no route does it; it
//!   expires by itself): a cancel then answers [`Deletion::LinkSent`] and the page says nothing is scheduled.
//!
//! Thread rule: the trait's deletion methods answer at once, so each engine call is made on the calling thread and
//! is bounded by the engine (`DELETION_WITHIN`), as the step-up of `account_actions.rs` is. Every press stamps the
//! session and the account; an answer that lands under another one is dropped (nothing kept, nothing said).
//!
//! Must not: log a password, a code, a token, an email or a server's words; keep the password past [`KEPT_FOR`].

use std::sync::Mutex;

use beebeeb_engine::app::account_actions_api::{ActionError, StepUpGrant};
use beebeeb_engine::app::bindings::account_deletion::{DeletionError, DeletionState};
use beebeeb_engine::signin::gate::GateState;
use chrono::{DateTime, Datelike, Local};
use zeroize::Zeroizing;

use super::account_approve::keeps_late;
use super::*;

/// The typed password is kept for the request this long at most.
const KEPT_FOR: Duration = Duration::from_secs(4 * 60);
/// How often [`watch`] looks whether what is kept still belongs to the session and account it was typed in.
const WATCH_EVERY: Duration = Duration::from_millis(100);
/// This slice's confirmations are numbered from here: never the number of a confirmation `account_actions.rs` holds
/// for another purpose (its `spend` looks a confirmation up by its number first).
const DELETION_HANDLES: u64 = 1 << 32;

/// The toasts of a request Beebeeb refused for a reason that is not the code. Nothing changed in any of them.
pub(super) const PLAN_FIRST: &str = "Cancel your plan first. No link was sent.";
pub(super) const LINK_TOO_SOON: &str =
    "A link was sent less than a minute ago. Check your email, or try again in a minute.";
pub(super) const EMAIL_NOT_SENT: &str =
    "The email with the link could not be sent. Nothing was scheduled. Try again later.";
/// A cancel or an undo Beebeeb did not take, and the deletion still stands.
pub(super) const TOO_LATE_TO_UNDO: &str = "It can no longer be undone. Nothing changed.";
/// A scheduled deletion whose time Beebeeb did not say.
const NO_DATE: &str = "the date in the email we sent";
/// The link went out while the screen locked: said after the unlock. Nothing is scheduled until the link is used.
pub(super) const LINK_SENT_LATE: &str =
    "The deletion link was sent to your email. Nothing is deleted until you open it.";
/// "I've opened the link" when Beebeeb could not be asked: never said as "not used".
pub(super) const NOT_CHECKED: &str = "Couldn't check with Beebeeb. Nothing changed. Try again.";

/// How a read of Beebeeb's state ended.
enum Read {
    /// Beebeeb answered (kept for the session and account of the press).
    Known(Deletion),
    /// The read failed: nothing is known, and nothing may be said as known.
    NotKnown,
    /// The session or the account moved on while it was out: dropped, nothing said.
    Moved,
}

/// The password and the step-up's token kept for one confirmation (see the module's doc).
struct Kept {
    password: Zeroizing<String>,
    /// For a cancel or an undo; the request does not use it (core proves the password again).
    grant: Option<StepUpGrant>,
    pressed: Stamp,
    handle: u64,
    made: Instant,
}

/// What this file keeps between a press and its answer. The adapter holds one (`EngineAccountSource::account_deletion`).
#[derive(Default)]
pub(super) struct DeletionCalls {
    /// Shared with the [`watch`] of the step-up that made it.
    kept: Arc<Mutex<Option<Kept>>>,
    /// The handles made so far (one per step-up).
    made: u64,
    /// The deletion as Beebeeb last answered it, with the session and account it was asked under.
    known: Option<(Stamp, Deletion)>,
    /// The request whose link WENT OUT while the session turned (a lock during the round trip): the stamp it was made
    /// under. Told once after the unlock ([`account_deletion_tell_late`]); wiped unsaid for a sign-out, a closed vault
    /// or another account (`keeps_late`, as `account_approve.rs` keeps a late approval).
    late: Option<Stamp>,
}

fn held<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn refused(why: CodeRefused, confirmation: Option<StepUp>) -> DeletionRefused {
    DeletionRefused { why, confirmation }
}

/// Drops what is kept under `handle` the moment its session or account moves on, or [`KEPT_FOR`] has passed; ends
/// when it was used, dropped or replaced. `false`: no watch could start (then nothing may be kept).
fn watch<C: CredentialStores + 'static>(engine: Arc<Engine<C>>, kept: Arc<Mutex<Option<Kept>>>, handle: u64) -> bool {
    std::thread::Builder::new()
        .name("account-deletion-kept".into())
        .spawn(move || {
            loop {
                std::thread::sleep(WATCH_EVERY);
                // Read before the lock: no lock of the engine is ever taken while this one is held.
                let now = stamp(&engine);
                let mut slot = held(&kept);
                let Some(this) = slot.as_ref().filter(|this| this.handle == handle) else {
                    return;
                };
                if this.made.elapsed() >= KEPT_FOR || this.pressed != now {
                    *slot = None;
                    return;
                }
            }
        })
        .is_ok()
}

/// The deletion as the pages say it.
fn shown(state: &DeletionState) -> Deletion {
    match state {
        DeletionState::None => Deletion::None,
        DeletionState::Requested => Deletion::LinkSent,
        DeletionState::Scheduled {
            shred_after,
            cancel_until,
        } => Deletion::Scheduled {
            // The last moment it can be undone; the account is deleted right after it.
            on: (*cancel_until)
                .or(*shred_after)
                .map_or_else(|| NO_DATE.to_string(), on_words),
        },
    }
}

/// "14 December at 18:42" on this PC's clock; the year only when it is not this one.
fn on_words(at: DateTime<Utc>) -> String {
    let local = at.with_timezone(&Local);
    if local.year() == Local::now().year() {
        local.format("%-d %B at %H:%M").to_string()
    } else {
        local.format("%-d %B %Y at %H:%M").to_string()
    }
}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    /// The deletion's panes: ready while a session is open (they read nothing before a press).
    pub(super) fn account_deletion_pane_state(&self, pane: Pane) -> Option<PaneState> {
        if !matches!(
            pane,
            Pane::Delete | Pane::DeleteFactor | Pane::DeleteLink | Pane::DeleteScheduled | Pane::DeleteUndone
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

    /// The step-up of [`Purpose::DeleteAccount`]: the password is proven now and kept (see the module's doc).
    pub(super) fn account_deletion_step_up(&mut self, password: Password<'_>) -> Result<StepUp, Refused> {
        self.account_deletion_forget();
        if !self.engine.has_session() {
            return Err(Refused::Offline);
        }
        let Ok(text) = std::str::from_utf8(password.for_opaque()) else {
            return Err(Refused::WrongPassword);
        };
        if text.is_empty() {
            return Err(Refused::WrongPassword);
        }
        let pressed = stamp(&self.engine);
        let grant = self
            .engine
            .step_up_for(pressed.0, Zeroizing::new(text.to_string()))
            .map_err(|error| {
                log(format!("account: the password was not confirmed ({})", error.words()));
                match error {
                    ActionError::WrongPassword => Refused::WrongPassword,
                    // Nothing was confirmed, and nothing changed: said as "could not be reached".
                    _ => Refused::Offline,
                }
            })?;
        // The session or the account moved on while the password was proven: nothing is kept.
        if stamp(&self.engine) != pressed {
            return Err(Refused::Offline);
        }
        self.account_deletion.made += 1;
        let handle = DELETION_HANDLES + self.account_deletion.made;
        *held(&self.account_deletion.kept) = Some(Kept {
            password: Zeroizing::new(text.to_string()),
            grant: Some(grant),
            pressed,
            handle,
            made: Instant::now(),
        });
        if !watch(self.engine.clone(), self.account_deletion.kept.clone(), handle) {
            self.account_deletion_forget();
            return Err(Refused::Offline);
        }
        Ok(StepUp::granted(Purpose::DeleteAccount, handle))
    }

    /// Whatever is kept is dropped (and wiped).
    pub(super) fn account_deletion_forget(&mut self) {
        *held(&self.account_deletion.kept) = None;
    }

    /// What was kept for `confirmation`, taken: only for its own handle and purpose, under the session and account
    /// it was typed in, within [`KEPT_FOR`]. Anything else is dropped.
    fn account_deletion_take(&mut self, confirmation: &StepUp) -> Option<Kept> {
        let now = stamp(&self.engine);
        let kept = held(&self.account_deletion.kept).take()?;
        (confirmation.purpose() == Purpose::DeleteAccount
            && confirmation.handle() == kept.handle
            && kept.made.elapsed() < KEPT_FOR
            && kept.pressed == now)
            .then_some(kept)
    }

    /// The request was not asked for and the proof of the password still stands: kept again (the same handle, the
    /// same four minutes), and the confirmation handed back, so the person retypes the code and not the password.
    fn account_deletion_back(&mut self, kept: Kept, confirmation: StepUp, why: CodeRefused) -> DeletionRefused {
        let handle = kept.handle;
        *held(&self.account_deletion.kept) = Some(kept);
        if watch(self.engine.clone(), self.account_deletion.kept.clone(), handle) {
            refused(why, Some(confirmation))
        } else {
            self.account_deletion_forget();
            refused(why, None)
        }
    }

    /// Beebeeb's state of the deletion, kept for the session and account of this press.
    fn account_deletion_read(&mut self) -> Read {
        let pressed = stamp(&self.engine);
        let answer = self.engine.account_deletion_for(pressed.0);
        if stamp(&self.engine) != pressed {
            return Read::Moved;
        }
        match answer {
            Ok(state) => {
                let deletion = shown(&state);
                self.account_deletion.known = Some((pressed, deletion.clone()));
                Read::Known(deletion)
            }
            Err(error) => {
                log(format!("account: the deletion was not read ({})", error.words()));
                Read::NotKnown
            }
        }
    }

    /// What Beebeeb last answered, for the session and account that are the engine's now; else nothing asked.
    pub(super) fn account_deletion_deletion(&self) -> Deletion {
        match &self.account_deletion.known {
            Some((at, deletion)) if *at == stamp(&self.engine) => deletion.clone(),
            _ => Deletion::None,
        }
    }

    /// The request (DW-A80): core's step-up with the kept password, then the second factor.
    pub(super) fn account_deletion_request_deletion(
        &mut self,
        confirmation: StepUp,
        code: &str,
    ) -> Result<(), DeletionRefused> {
        let Some(kept) = self.account_deletion_take(&confirmation) else {
            return Err(refused(CodeRefused::NotConfirmed, None));
        };
        let pressed = kept.pressed.clone();
        let answer =
            self.engine
                .request_account_deletion_for(pressed.0, Zeroizing::new(kept.password.to_string()), code);
        // A late answer: the session or the account moved on while it was out. Nothing is kept; if the link went out
        // under a turn of this same account that stays open (a lock), it is told after the unlock.
        let now = stamp(&self.engine);
        if now != pressed || answer == Err(DeletionError::SentThenMoved) {
            if matches!(answer, Ok(()) | Err(DeletionError::SentThenMoved)) && keeps_late(&pressed, &now, self.open()) {
                self.account_deletion.late = Some(pressed);
            }
            return Err(refused(CodeRefused::NotConfirmed, None));
        }
        let error = match answer {
            Ok(()) => {
                // Asked: what was kept goes with `kept`; nothing is scheduled until the link is used.
                self.account_deletion.known = Some((pressed, Deletion::LinkSent));
                return Ok(());
            }
            Err(error) => error,
        };
        log(format!("account: the deletion was not asked for ({})", error.words()));
        Err(match error {
            DeletionError::WrongCode | DeletionError::SecondFactorRequired => {
                self.account_deletion_back(kept, confirmation, CodeRefused::Wrong)
            }
            DeletionError::Offline
            | DeletionError::Unconfirmed
            | DeletionError::Failed
            | DeletionError::TooManyTries => self.account_deletion_back(kept, confirmation, CodeRefused::Offline),
            DeletionError::TooSoon => {
                self.say(Said::failed(LINK_TOO_SOON, None, false));
                self.account_deletion_back(kept, confirmation, CodeRefused::NotConfirmed)
            }
            DeletionError::EmailNotSent => {
                self.say(Said::failed(EMAIL_NOT_SENT, None, false));
                self.account_deletion_back(kept, confirmation, CodeRefused::NotConfirmed)
            }
            // Nothing the code can change: what was kept is dropped, and the page says why.
            DeletionError::SubscriptionActive => {
                self.say(Said::failed(PLAN_FIRST, None, false));
                refused(CodeRefused::NotConfirmed, None)
            }
            // The page moves to the schedule Beebeeb has (`delete.rs`).
            DeletionError::AlreadyScheduled => {
                self.account_deletion_read();
                refused(CodeRefused::NotConfirmed, None)
            }
            DeletionError::WrongPassword
            | DeletionError::SentThenMoved
            | DeletionError::NotConfirmed
            | DeletionError::NotScheduled
            | DeletionError::NotSignedIn
            | DeletionError::SessionEnded => refused(CodeRefused::NotConfirmed, None),
        })
    }

    /// A link that went out while the screen locked: wiped for another account, a sign-out or a closed vault; said once,
    /// as a toast, when the window is open (the gate is Ready) on an account page. Every window poll (`pump`) asks.
    pub(super) fn account_deletion_tell_late(&mut self) {
        let now = stamp(&self.engine);
        let open = self.open();
        let Some(made_for) = self.account_deletion.late.as_ref() else {
            return;
        };
        if !keeps_late(made_for, &now, open) {
            self.account_deletion.late = None;
            return;
        }
        if self.engine.gate() != GateState::Ready || self.showing.is_none() {
            return;
        }
        self.account_deletion.late = None;
        self.say(Said::done(LINK_SENT_LATE));
    }

    /// "I've opened the link" (DW-A81): `true` only when Beebeeb says the deletion is scheduled. A check that failed is
    /// said as one (a toast), so the page never says "not used" for an answer it did not get.
    pub(super) fn account_deletion_deletion_link_used(&mut self) -> bool {
        match self.account_deletion_read() {
            Read::Known(deletion) => matches!(deletion, Deletion::Scheduled { .. }),
            Read::NotKnown => {
                self.say(Said::failed(NOT_CHECKED, None, false));
                false
            }
            Read::Moved => false,
        }
    }

    /// Cancel deletion (DW-A81), with the password asked over the page.
    pub(super) fn account_deletion_cancel_deletion_request(
        &mut self,
        confirmation: StepUp,
    ) -> Result<Deletion, Refused> {
        self.account_deletion_cancel(confirmation)
    }

    /// Undo deletion (DW-A82), with the password asked over the page.
    pub(super) fn account_deletion_undo_deletion(&mut self, confirmation: StepUp) -> Result<Deletion, Refused> {
        self.account_deletion_cancel(confirmation)
    }

    /// The signed-in cancel with the step-up's token. `Ok`: how the deletion stands after it, as Beebeeb says.
    fn account_deletion_cancel(&mut self, confirmation: StepUp) -> Result<Deletion, Refused> {
        let Some(mut kept) = self.account_deletion_take(&confirmation) else {
            return Err(Refused::NotConfirmed);
        };
        let pressed = kept.pressed.clone();
        let grant = kept.grant.take();
        // A cancel needs no password: it is wiped now.
        drop(kept);
        let Some(grant) = grant else {
            return Err(Refused::NotConfirmed);
        };
        let answer = self.engine.cancel_account_deletion_for(pressed.0, grant);
        // A late answer: nothing is kept or said for a session or an account that is over.
        if stamp(&self.engine) != pressed {
            return Err(Refused::NotConfirmed);
        }
        let error = match answer {
            Ok(()) => {
                self.account_deletion.known = Some((pressed, Deletion::None));
                return Ok(Deletion::None);
            }
            Err(error) => error,
        };
        log(format!("account: the deletion was not cancelled ({})", error.words()));
        match error {
            // Nothing Beebeeb may still cancel, or a cancel whose end is not known: what stands is read again.
            DeletionError::NotScheduled | DeletionError::Unconfirmed => match self.account_deletion_read() {
                // Beebeeb said it takes nothing, and the schedule stands: the time to undo it has passed.
                Read::Known(deletion @ Deletion::Scheduled { .. }) if error == DeletionError::NotScheduled => {
                    self.say(Said::failed(TOO_LATE_TO_UNDO, None, false));
                    Ok(deletion)
                }
                // The cancel's answer was lost and the schedule stands: it did not land. Nothing changed; the dialog
                // stays for another try (never "it can no longer be undone").
                Read::Known(Deletion::Scheduled { .. }) => Err(Refused::Offline),
                Read::Known(deletion) => Ok(deletion),
                Read::NotKnown if error == DeletionError::Unconfirmed => Err(Refused::Unconfirmed),
                Read::NotKnown => Err(Refused::Offline),
                Read::Moved => Err(Refused::NotConfirmed),
            },
            DeletionError::NotConfirmed | DeletionError::WrongPassword => Err(Refused::NotConfirmed),
            _ => Err(Refused::Offline),
        }
    }
}

#[cfg(test)]
#[path = "account_deletion_tests.rs"]
mod tests;
