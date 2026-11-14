//! Sign-in on the real engine: [`EngineSignInSource`], the `SignInSource` of the running app.
//!
//! What it binds, and to which engine step:
//! - the password ([`Engine::sign_in_password`]), the code of an authenticator app or a backup code
//!   ([`Engine::two_factor`]: core tells the two apart), the twelve words on a computer that holds no key
//!   ([`Engine::unlock_with_phrase`]), a locked vault's password ([`Engine::unlock_with_password`]), and leaving a
//!   half-done sign-in ([`Engine::discard_partial_sign_in`]);
//! - the wait after "too many tries" (DW-A21): counted down here, once a second, from the seconds the server named.
//!
//! What it does not bind answers AT ONCE with the refusal its screen draws, never with silence: Windows Hello and the
//! Continue-as card (no shared session store and no Hello prompt exist for this app yet), a device request, a
//! passkey, "Forgot password?" with the phrase (core's `recovery_flow` has no engine step yet) and sign-up (the
//! trait's defaults: `sign_up_available` stays `false`).
//!
//! Threads: nothing here waits for Beebeeb on the window's thread. Every step is a job on ONE worker, run in the
//! order the person asked (leaving a step, then signing in again, happen in that order); its answer goes down a
//! channel that [`SignInSource::poll`] drains, and the worker wakes the window AFTER the answer is in the channel.
//! Every answer carries the attempt it answers: an answer of a step the person left is dropped, and a job of such a
//! step that has not started yet does not run.
//!
//! Secrets: a password, a code and the twelve words arrive as `&str` borrowed from a secure field; each is copied
//! ONCE into a `Zeroizing` buffer made at its full size, which moves to the worker and into the engine call, and
//! is wiped there. None is logged, formatted, compared or put in an event or an error. An event is a kind
//! (`SignInProblem`, `TwoFactorProblem`, `PhraseProblem`): fixed words, never a server's body, and the same kind for
//! a wrong password and an address nobody has.
//!
//! Must not: log an email, a password, a code, a word, a token or a key; start a thread per key press.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use beebeeb_engine::app::{CredentialStores, Engine};

use super::{log, map_gate_signin, map_next, map_problem, phrase_problem, two_factor_problem};
use crate::frames::signin::source::{
    CodeKind, GateState, HelloOutcome, NextAfterSignIn, PasskeyOutcome, PhraseProblem, SharedSession, SignInEvent,
    SignInProblem, SignInSource,
};

/// The waits an answer can belong to. An attempt of one is over when the person leaves its screen.
#[derive(Clone, Copy)]
enum Wait {
    SignIn,
    TwoFactor,
    Restore,
    Unlock,
    /// A passkey sign-in: the system's dialog, on a thread of its own.
    Passkey,
    /// The countdown after "too many tries".
    Retry,
    /// A call this source does not bind: answered at once, never stale.
    Unbound,
}

const WAITS: usize = 7;

/// One answer: the wait it belongs to, which attempt of it, and the event for the screen.
struct Answer {
    wait: Wait,
    attempt: u64,
    event: SignInEvent,
}

type Job<C> = Box<dyn FnOnce(&Arc<Engine<C>>) + Send>;

/// `SignInSource` backed by the real engine. See the module's doc.
pub struct EngineSignInSource<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The current attempt of each wait, shared with the worker: a job of an older attempt does not start.
    attempts: Arc<[AtomicU64; WAITS]>,
    /// The one worker's queue. `None`: the worker could not be started, and every step answers its refusal.
    jobs: Option<mpsc::Sender<Job<C>>>,
    /// One step of the countdown (a second; a test shortens it).
    tick: Duration,
}

impl<C: CredentialStores> EngineSignInSource<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self::with_tick(engine, Duration::from_secs(1))
    }

    pub(super) fn with_tick(engine: Arc<Engine<C>>, tick: Duration) -> Self {
        let (answers, inbox) = mpsc::channel();
        let (jobs, queue) = mpsc::channel::<Job<C>>();
        let for_worker = engine.clone();
        // The worker ends when this source is dropped (its queue closes).
        let started = thread::Builder::new().name("signin".into()).spawn(move || {
            for job in queue {
                job(&for_worker);
            }
        });
        if started.is_err() {
            log("sign-in: the worker did not start");
        }
        Self {
            engine,
            answers,
            inbox,
            attempts: Arc::new(std::array::from_fn(|_| AtomicU64::new(0))),
            jobs: started.is_ok().then_some(jobs),
            tick,
        }
    }

    /// A new attempt of `wait`: every answer of an earlier one is stale from here on.
    fn next_attempt(&self, wait: Wait) -> u64 {
        self.attempts[wait as usize].fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Run `work` on the worker as a new attempt of `wait`; its event is the answer. `failed` is the answer when
    /// the worker is not there. A job whose attempt was left before it started does nothing (and wipes what it
    /// holds by dropping it).
    fn ask(
        &mut self,
        wait: Wait,
        failed: SignInEvent,
        work: impl FnOnce(&Arc<Engine<C>>) -> SignInEvent + Send + 'static,
    ) {
        let attempt = self.next_attempt(wait);
        let (answers, attempts, tick) = (self.answers.clone(), self.attempts.clone(), self.tick);
        let job: Job<C> = Box::new(move |engine| {
            if attempts[wait as usize].load(Ordering::SeqCst) != attempt {
                return;
            }
            let event = work(engine);
            // The person left this step while it ran (`discard_partial_sign_in` moved the attempt on): its answer
            // is dropped by `poll`, and what it did is undone here, before the next job. Back wins.
            if attempts[wait as usize].load(Ordering::SeqCst) != attempt {
                undo_left_step(engine, &answers, &event);
                engine.wake_host();
                return;
            }
            let wait_seconds = match &event {
                SignInEvent::SignedIn(Err(SignInProblem::RateLimited { seconds })) => Some(*seconds),
                _ => None,
            };
            let _ = answers.send(Answer { wait, attempt, event });
            // After the answer is in the channel: no tick can come before the problem it counts down.
            if let Some(seconds) = wait_seconds {
                count_down(engine, &answers, &attempts, seconds, tick);
            }
            engine.wake_host();
        });
        let queued = self.jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok());
        if !queued {
            let _ = self.answers.send(Answer {
                wait,
                attempt,
                event: failed,
            });
        }
    }

    /// The refusal of a call this source does not bind: in the channel at once, so the screen draws it.
    fn not_bound(&mut self, what: &'static str, event: SignInEvent) {
        log(format!("NotWired({what})"));
        let _ = self.answers.send(Answer {
            wait: Wait::Unbound,
            attempt: 0,
            event,
        });
    }
}

/// A secret as the engine takes it: one wiping buffer, made at its full size so it never moves.
fn secret(text: &str) -> Zeroizing<String> {
    let mut buffer = Zeroizing::new(String::with_capacity(text.len()));
    buffer.push_str(text);
    buffer
}

/// The twelve words as one phrase, a space between: one wiping buffer, made at its full size.
fn phrase_of(words: &[&str]) -> Zeroizing<String> {
    let size = words.iter().map(|word| word.len() + 1).sum();
    let mut phrase = Zeroizing::new(String::with_capacity(size));
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            phrase.push(' ');
        }
        phrase.push_str(word);
    }
    phrase
}

/// A step the person left finished anyway (it held the engine's turn): undo what it did. A sign-in that the code
/// or the twelve words completed is signed out (the local files and state stay); a vault the password opened is
/// locked again. A step that was refused did nothing. A sign-out that does not finish is said to the window
/// (`StillSignedIn`), never only logged.
fn undo_left_step<C: CredentialStores>(engine: &Arc<Engine<C>>, answers: &mpsc::Sender<Answer>, event: &SignInEvent) {
    match event {
        SignInEvent::TwoFactor(Ok(_))
        | SignInEvent::PhraseRestored(Ok(_))
        | SignInEvent::Passkey(PasskeyOutcome::Succeeded(_)) => {
            if engine.sign_out_keeping_local_data().is_err() {
                still_signed_in(answers);
            }
        }
        SignInEvent::VaultUnlocked(Ok(_)) => {
            let _ = engine.lock();
        }
        _ => {}
    }
}

/// Tell the window that leaving a step did not end its session.
fn still_signed_in(answers: &mpsc::Sender<Answer>) {
    log("sign-in: leaving the step did not sign out; still signed in");
    let _ = answers.send(Answer {
        wait: Wait::Unbound,
        attempt: 0,
        event: SignInEvent::StillSignedIn,
    });
}

/// Count the wait down: `RetryAfter` once a `tick`, `0` when it is over (DW-A21). It stops when a newer countdown
/// began or the source is gone. A wait of no seconds, and a thread that cannot start, end the wait at once: the
/// form works again, and the server answers the next try as it sees fit. Called AFTER the answer it counts down
/// is in the channel.
fn count_down<C: CredentialStores>(
    engine: &Arc<Engine<C>>,
    answers: &mpsc::Sender<Answer>,
    attempts: &Arc<[AtomicU64; WAITS]>,
    seconds: u32,
    tick: Duration,
) {
    let attempt = attempts[Wait::Retry as usize].fetch_add(1, Ordering::SeqCst) + 1;
    let over = || {
        let _ = answers.send(Answer {
            wait: Wait::Retry,
            attempt,
            event: SignInEvent::RetryAfter { seconds: 0 },
        });
    };
    if seconds == 0 {
        return over();
    }
    let (engine_in, answers_in, attempts_in) = (engine.clone(), answers.clone(), attempts.clone());
    let started = thread::Builder::new().name("signin-wait".into()).spawn(move || {
        let mut left = seconds;
        while left > 0 {
            thread::sleep(tick);
            if attempts_in[Wait::Retry as usize].load(Ordering::SeqCst) != attempt {
                return;
            }
            left -= 1;
            let event = SignInEvent::RetryAfter { seconds: left };
            let tick = Answer {
                wait: Wait::Retry,
                attempt,
                event,
            };
            if answers_in.send(tick).is_err() {
                return;
            }
            engine_in.wake_host();
        }
    });
    if started.is_err() {
        over();
    }
}

impl<C: CredentialStores> SignInSource for EngineSignInSource<C> {
    /// The gate from the handle's view: read without waiting, on every frame.
    fn gate(&self) -> GateState {
        map_gate_signin(self.engine.view().gate)
    }

    /// No other Beebeeb app's session is read on this computer, and no Windows Hello prompt exists for this app
    /// yet: the Continue-as card is never offered, so `begin_hello` is never reached from a screen.
    fn shared_session(&self) -> Option<SharedSession> {
        None
    }

    fn begin_sign_in(&mut self, email: &str, password: &str) {
        let (email, password) = (email.to_string(), secret(password));
        // A wait the server names is counted down by `ask`, after this answer is in the channel.
        self.ask(
            Wait::SignIn,
            SignInEvent::SignedIn(Err(SignInProblem::Other)),
            move |engine| match engine.sign_in_password(&email, password) {
                Ok(next) => SignInEvent::SignedIn(Ok(map_next(next))),
                Err(problem) => SignInEvent::SignedIn(Err(map_problem(problem))),
            },
        );
    }

    fn begin_hello(&mut self) {
        self.not_bound("begin_hello", SignInEvent::Hello(HelloOutcome::Cancelled));
    }

    fn cancel_hello(&mut self) {}

    fn begin_device_request(&mut self) {
        self.not_bound(
            "begin_device_request",
            SignInEvent::DeviceRequested(Err(SignInProblem::Other)),
        );
    }

    fn cancel_device_request(&mut self) {}

    /// Both kinds go to the same engine step: core reads six digits as an authenticator code and eight as a
    /// backup code, and the screen lets nothing else through.
    fn begin_two_factor(&mut self, code: &str, _kind: CodeKind) {
        let code = secret(code);
        self.ask(
            Wait::TwoFactor,
            SignInEvent::TwoFactor(Err(crate::frames::signin::source::TwoFactorProblem::Other)),
            move |engine| match engine.two_factor(code) {
                Ok(next) => SignInEvent::TwoFactor(Ok(map_next(next))),
                Err(words) => SignInEvent::TwoFactor(Err(two_factor_problem(words))),
            },
        );
    }

    fn begin_restore(&mut self, words: &[&str]) {
        let phrase = phrase_of(words);
        self.ask(
            Wait::Restore,
            SignInEvent::PhraseRestored(Err(PhraseProblem::Other)),
            move |engine| match engine.unlock_with_phrase(phrase) {
                Ok(()) => SignInEvent::PhraseRestored(Ok(NextAfterSignIn::Drive)),
                Err(words) => SignInEvent::PhraseRestored(Err(phrase_problem(words))),
            },
        );
    }

    /// Core's passkey sign-in starts from an address: [`SignInSource::begin_passkey_for`] is the call a screen makes.
    fn begin_passkey(&mut self) {
        self.begin_passkey_for("");
    }

    fn passkey_available(&self) -> bool {
        self.engine.passkeys_supported()
    }

    /// The system's passkey dialog for the address typed, on a thread of its own (it stays up for as long as the
    /// person takes; the shared worker keeps serving the other steps). The person leaving ([`Self::cancel_passkey`])
    /// moves the attempt on: its answer is dropped, and a sign-in it completed is undone.
    fn begin_passkey_for(&mut self, email: &str) {
        let attempt = self.next_attempt(Wait::Passkey);
        let (engine, answers, attempts) = (self.engine.clone(), self.answers.clone(), self.attempts.clone());
        let email = email.to_string();
        let started = thread::Builder::new().name("signin-passkey".into()).spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::passkeys::sign_in_outcome(&engine, &email)
            }))
            .unwrap_or(PasskeyOutcome::Unavailable);
            let event = SignInEvent::Passkey(outcome);
            if attempts[Wait::Passkey as usize].load(Ordering::SeqCst) != attempt {
                undo_left_step(&engine, &answers, &event);
            } else {
                let _ = answers.send(Answer {
                    wait: Wait::Passkey,
                    attempt,
                    event,
                });
            }
            engine.wake_host();
        });
        if started.is_err() {
            log("sign-in: the passkey thread did not start");
            let _ = self.answers.send(Answer {
                wait: Wait::Passkey,
                attempt,
                event: SignInEvent::Passkey(PasskeyOutcome::Unavailable),
            });
        }
    }

    /// The person left while the dialog was up. The system's dialog cannot be closed from here: it ends when they
    /// close it, and nothing it did is kept (an answer of an attempt that was left is undone, never drawn).
    fn cancel_passkey(&mut self) {
        self.next_attempt(Wait::Passkey);
    }

    /// A locked vault's password: proven by the server, then the key this computer holds is loaded
    /// ([`Engine::unlock_with_password`]).
    fn begin_unlock(&mut self, password: &str) {
        let password = secret(password);
        self.ask(
            Wait::Unlock,
            SignInEvent::VaultUnlocked(Err(SignInProblem::Other)),
            move |engine| match engine.unlock_with_password(password) {
                Ok(()) => SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::Drive)),
                Err(problem) => SignInEvent::VaultUnlocked(Err(map_problem(problem))),
            },
        );
    }

    fn begin_recovery_verify(&mut self, _email: &str, _words: &[&str]) {
        self.not_bound(
            "begin_recovery_verify",
            SignInEvent::RecoveryPhraseVerified(Err(PhraseProblem::Other)),
        );
    }

    fn begin_set_password(&mut self, _password: &str) {
        self.not_bound(
            "begin_set_password",
            SignInEvent::PasswordSet(Err(SignInProblem::Other)),
        );
    }

    /// The next answer that is still waited for; an answer of a step the person left is dropped here.
    fn poll(&mut self) -> Option<SignInEvent> {
        while let Ok(answer) = self.inbox.try_recv() {
            let current = match answer.wait {
                Wait::Unbound => 0,
                wait => self.attempts[wait as usize].load(Ordering::SeqCst),
            };
            if answer.attempt == current {
                return Some(answer.event);
            }
        }
        None
    }

    /// The person went back from the code, the twelve words or the vault password. From this call on no answer of
    /// those steps is handed out, a step of them that has not started does not start, and one that is running
    /// has its result undone when it ends (`undo_left_step`). The engine then forgets what was half done, in its
    /// turn on the worker ([`Engine::discard_partial_sign_in`]). A session that could not be ended is said to the
    /// window (`SignInEvent::StillSignedIn`).
    fn discard_partial_sign_in(&mut self) {
        for wait in [Wait::TwoFactor, Wait::Restore, Wait::Unlock] {
            self.next_attempt(wait);
        }
        let answers = self.answers.clone();
        let job: Job<C> = Box::new(move |engine| {
            if engine.discard_partial_sign_in().is_err() {
                still_signed_in(&answers);
            }
            engine.wake_host();
        });
        if !self.jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok()) {
            log("sign-in: leaving the step could not be passed to the engine");
        }
    }
}

#[cfg(test)]
#[path = "signin_tests.rs"]
mod tests;
