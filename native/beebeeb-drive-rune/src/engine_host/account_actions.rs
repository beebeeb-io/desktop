//! What the account sections DO on the real engine, beside reading: sign this app out (Sign-in and security, DW-S02),
//! prove the password for a guarded change (`step_up`), and change the name (DW-A64). Each is the engine's own call
//! ([`Engine::sign_out_for`], [`Engine::step_up_for`], [`Engine::set_display_name_for`]), session-bound.
//!
//! Shape of every call that waits on Beebeeb: the press stamps the session and the account; the engine call runs
//! on a worker of its own and sends its answer down a channel the window's poll takes in; an answer that is not
//! for the session and account the window is on now is dropped. Nothing is drawn as done before the engine
//! answered. The exception is a SUCCESSFUL sign-out: it moves the session itself, so it is the one answer that is
//! handed over although the session it was made for is over (it is the reason it is over).
//!
//! The password of a step-up is borrowed from the field for one call, copied once into a wiping buffer, and gone
//! when the call returns. The confirmation it yields is held here, by an opaque handle, for one purpose, one
//! session and account and a few minutes ([`Actions::spend`]); it is never text, never logged.
//!
//! Must not: log a name, a password, a token or a server's words; keep what was typed.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_core::auth_flow::step_up::ConfirmToken;
use beebeeb_engine::app::account_actions_api::ActionError;
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use super::{Stamp, account_of, log, stamp};
use crate::frames::account::source::{NameAnswer, NameWhy, Password, Purpose, Refused, SignOutAnswer, StepUp};

/// A confirmation of the server is good for this long here (the server's own is shorter or equal).
const CONFIRMATION_LASTS: Duration = Duration::from_secs(4 * 60);

/// What the Sign out confirm says when the sign-out was not done and Beebeeb or the engine could not finish it.
pub(super) const NOT_SIGNED_OUT_APP: &str = "Drive could not sign out. You are still signed in. Try again.";
/// The window's press was made under a session that is over (another sign-in, another account).
pub(super) const SESSION_MOVED: &str = "Your session changed. Nothing was signed out.";
/// No session at all: nothing to sign out.
pub(super) const NO_SESSION: &str = "You are not signed in. Nothing was signed out.";

/// One answer on its way home: what a worker found, and the stamp it was asked under.
struct Ended {
    stamp: Stamp,
    result: Result<(), ActionError>,
}

/// A confirmation of a step-up, until it is spent.
struct Confirmation {
    handle: u64,
    purpose: Purpose,
    stamp: Stamp,
    made: Instant,
    token: ConfirmToken,
}

/// The things this file does for the account window.
pub(super) struct Actions<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    /// The sign-out that is out, by the stamp it was pressed under. One at a time.
    signing_out: Option<Stamp>,
    sign_out_end: (mpsc::Sender<Ended>, mpsc::Receiver<Ended>),
    /// The name save that is out, by its stamp.
    naming: Option<Stamp>,
    name_end: (mpsc::Sender<Ended>, mpsc::Receiver<Ended>),
    confirmations: Vec<Confirmation>,
    next_handle: u64,
    /// Tests: how far the confirmations' clock is ahead of the real one.
    #[cfg(test)]
    pub(super) ahead: Duration,
}

/// Run `work` on a worker named `name`; its answer (or a failure if it panicked) goes down `to`, and the window
/// wakes. `false`: no worker could start.
fn spawn<C: CredentialStores + 'static>(
    name: &str,
    engine: &Arc<Engine<C>>,
    to: &mpsc::Sender<Ended>,
    asked_as: Stamp,
    work: impl FnOnce(&Engine<C>) -> Result<(), ActionError> + Send + 'static,
) -> bool {
    let (engine, to) = (engine.clone(), to.clone());
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&engine)))
                .unwrap_or(Err(ActionError::Failed));
            let _ = to.send(Ended {
                stamp: asked_as,
                result,
            });
            engine.wake_host();
        })
        .is_ok()
}

impl<C: CredentialStores + 'static> Actions<C> {
    pub(super) fn new(engine: Arc<Engine<C>>) -> Self {
        Self {
            engine,
            signing_out: None,
            sign_out_end: mpsc::channel(),
            naming: None,
            name_end: mpsc::channel(),
            confirmations: Vec::new(),
            next_handle: 0,
            #[cfg(test)]
            ahead: Duration::ZERO,
        }
    }

    /// Whether a stamp is the engine's session and account now.
    fn is_now(&self, stamp_of: &Stamp) -> bool {
        self.engine.has_session() && stamp_of.0 == self.engine.session_turn() && stamp_of.1 == account_of(&self.engine)
    }

    fn now(&self) -> Instant {
        #[cfg(test)]
        return Instant::now() + self.ahead;
        #[cfg(not(test))]
        Instant::now()
    }

    /// The window's poll: what belongs to a session that is over goes (a confirmation, a wait).
    pub(super) fn pump(&mut self) -> bool {
        let before = self.confirmations.len();
        let now = self.now();
        let (session, account) = (self.engine.session_turn(), account_of(&self.engine));
        let open = self.engine.has_session();
        self.confirmations.retain(|c| {
            open && c.stamp.0 == session && c.stamp.1 == account && now.duration_since(c.made) < CONFIRMATION_LASTS
        });
        // A wait for a session that is over is not cleared here: its answer is still taken in, and dropped, by
        // `sign_out_result` and `name_result` (a sign-out that worked is the one answer handed over).
        before != self.confirmations.len()
    }

    /// Something is out: the window keeps looking.
    pub(super) fn busy(&self) -> bool {
        self.naming.as_ref().is_some_and(|waiting| self.is_now(waiting))
    }

    // ---- Sign out of Drive ----------------------------------------------------------------------------------

    /// DW-S02: sign THIS app out. `Pending` once sent; the answer comes through [`Actions::sign_out_result`].
    pub(super) fn sign_out_this_app(&mut self) -> SignOutAnswer {
        // One that is out for this session and account is waited for; one of a session that is over is forgotten
        // (its answer, when it comes, is for nobody).
        match &self.signing_out {
            Some(waiting) if self.is_now(waiting) => return SignOutAnswer::Pending,
            _ => self.signing_out = None,
        }
        if !self.engine.has_session() {
            return SignOutAnswer::Refused(NO_SESSION);
        }
        let asked_as = stamp(&self.engine);
        let session = asked_as.0;
        let started = spawn(
            "account-sign-out-app",
            &self.engine,
            &self.sign_out_end.0,
            asked_as.clone(),
            move |engine| engine.sign_out_for(session),
        );
        if !started {
            log("account: could not start a thread for the sign-out");
            return SignOutAnswer::Refused(NOT_SIGNED_OUT_APP);
        }
        self.signing_out = Some(asked_as);
        SignOutAnswer::Pending
    }

    /// How the sign-out that was sent ended, once. A refusal of a session that moved on is dropped; a sign-out
    /// that WORKED is handed over whatever the session is now: it is why the session is over.
    pub(super) fn sign_out_result(&mut self) -> Option<SignOutAnswer> {
        while let Ok(end) = self.sign_out_end.1.try_recv() {
            let Some(waiting) = self.signing_out.clone() else {
                continue;
            };
            if end.stamp != waiting {
                continue;
            }
            self.signing_out = None;
            return match end.result {
                Ok(()) => Some(SignOutAnswer::Done),
                // What was refused is for the window that asked: not the one of another session or account.
                Err(_) if !self.is_now(&end.stamp) => None,
                Err(ActionError::NotSignedIn) => Some(SignOutAnswer::Refused(SESSION_MOVED)),
                Err(error) => {
                    log(format!("account: the sign-out was not done ({})", error.words()));
                    Some(SignOutAnswer::Refused(NOT_SIGNED_OUT_APP))
                }
            };
        }
        None
    }

    // ---- The name -------------------------------------------------------------------------------------------

    /// DW-A64: save the name the account shows. `Pending` once sent ([`Actions::name_result`]); `NotSaved` when
    /// nothing could be sent.
    pub(super) fn save_name(&mut self, name: &str) -> NameAnswer {
        match &self.naming {
            Some(waiting) if self.is_now(waiting) => return NameAnswer::Pending,
            _ => self.naming = None,
        }
        if !self.engine.has_session() || self.engine.account_session_refused() {
            return NameAnswer::NotSaved(NameWhy::Refused);
        }
        let asked_as = stamp(&self.engine);
        let (session, name) = (asked_as.0, name.to_string());
        let started = spawn(
            "account-set-name",
            &self.engine,
            &self.name_end.0,
            asked_as.clone(),
            move |engine| engine.set_display_name_for(session, &name),
        );
        if !started {
            log("account: could not start a thread for the name");
            return NameAnswer::NotSaved(NameWhy::Offline);
        }
        self.naming = Some(asked_as);
        NameAnswer::Pending
    }

    /// How the save that was sent ended, once; an answer for a session or account that is over is dropped. The
    /// second part says Beebeeb has the new name: the host reads the name again, and never draws it before.
    pub(super) fn name_result(&mut self) -> Option<(NameAnswer, bool)> {
        while let Ok(end) = self.name_end.1.try_recv() {
            let Some(waiting) = self.naming.clone() else {
                continue;
            };
            if end.stamp != waiting {
                continue;
            }
            self.naming = None;
            if !self.is_now(&end.stamp) {
                return None;
            }
            return Some(match end.result {
                Ok(()) => (NameAnswer::Saved, true),
                Err(ActionError::Offline) => (NameAnswer::NotSaved(NameWhy::Offline), false),
                Err(error) => {
                    log(format!("account: the name was not saved ({})", error.words()));
                    (NameAnswer::NotSaved(NameWhy::Refused), false)
                }
            });
        }
        None
    }

    // ---- Step-up ----------------------------------------------------------------------------------------------

    /// Prove the password for `purpose`: core's step-up on this session. The proof is waited for here, for at
    /// most [`beebeeb_engine::app::account_actions_api::STEP_UP_WITHIN`] (see the module's PR record: the trait's
    /// `step_up` answers at once, so the pages that call it wait for the proof).
    pub(super) fn step_up(&mut self, password: Password<'_>, purpose: Purpose) -> Result<StepUp, Refused> {
        if !self.engine.has_session() {
            return Err(Refused::Offline);
        }
        let asked_as = stamp(&self.engine);
        // One copy of the password, in a buffer that wipes itself; the engine takes it and wipes it too.
        let typed = Zeroizing::new(String::from_utf8_lossy(password.for_opaque()).into_owned());
        let token = self.engine.step_up_for(asked_as.0, typed).map_err(|error| {
            log(format!("account: the password was not confirmed ({})", error.words()));
            match error {
                ActionError::WrongPassword => Refused::WrongPassword,
                // Nothing was confirmed, and nothing changed: said as "could not be reached".
                _ => Refused::Offline,
            }
        })?;
        // The proof is for the session and account it was made under: it is spent only under the same ones.
        if !self.is_now(&asked_as) {
            return Err(Refused::Offline);
        }
        self.next_handle += 1;
        let handle = self.next_handle;
        let made = self.now();
        self.confirmations.push(Confirmation {
            handle,
            purpose,
            stamp: asked_as,
            made,
            token,
        });
        Ok(StepUp::granted(purpose, handle))
    }

    /// Spend a confirmation: the token of `confirmation`, once, for `purpose`, under the session and account it
    /// was made for. `None`: it was for something else, was spent, expired, or its session is over.
    pub(super) fn spend(&mut self, confirmation: &StepUp, purpose: Purpose) -> Option<ConfirmToken> {
        let position = self
            .confirmations
            .iter()
            .position(|c| c.handle == confirmation.handle())?;
        let held = self.confirmations.remove(position);
        let fresh = self.now().duration_since(held.made) < CONFIRMATION_LASTS;
        (held.purpose == purpose && confirmation.purpose() == purpose && fresh && self.is_now(&held.stamp))
            .then_some(held.token)
    }

    /// How many confirmations are held (tests).
    #[cfg(test)]
    pub(super) fn held(&self) -> usize {
        self.confirmations.len()
    }
}
