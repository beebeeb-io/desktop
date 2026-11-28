//! Sign-in on the real engine: [`EngineSignInSource`], the `SignInSource` of the running app.
//!
//! What it binds, and to which engine step:
//! - the password ([`Engine::sign_in_password`]), the code of an authenticator app or a backup code
//!   ([`Engine::two_factor`]: core tells the two apart), the twelve words on a computer that holds no key
//!   ([`Engine::unlock_with_phrase`]), and leaving a half-done sign-in ([`Engine::discard_partial_sign_in`]);
//! - the two-level lock (core's rules, through the engine's `lock_api`): the lock screen's Windows Hello prompt
//!   (`hello::HelloPrompter`), whose yes is handed to [`Engine::unlock_screen_for`] with core's claim on the lock
//!   AS READ AT THE PRESS (the prompt is taken away once that lock ended, `end_hello_of_a_lock_that_ended`, and
//!   when this source goes), and the account's password for a closed vault or a locked screen
//!   ([`Engine::unlock_vault_with_password`]: proven to the server in a new sign-in, the code step after it
//!   when the account has one);
//! - the wait after "too many tries" (DW-A21): counted down here, once a second, from the seconds the server named;
//! - "Sign in with another device" (`signin_device.rs`: the engine's device request, on a thread of its own), the
//!   passkey sign-in (`passkeys.rs`: the system's dialog, on a thread of its own), and "Forgot password?" with the
//!   twelve words and a new password (`signin_recovery.rs`: core's phrase recovery through the engine).
//!
//! What it does not bind answers AT ONCE with the refusal its screen draws, never with silence: the Continue-as
//! card (no shared session store exists for this app yet, so Windows Hello is asked for the lock screen only).
//! Creating an account is bound in `signup.rs` (owner ruling 2026-11-14: sign-up in any app).
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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use beebeeb_engine::app::lock_api::{LockRefused, LockState, ScreenUnlockAsk};
use beebeeb_engine::app::lock_password_api::ScreenUnlock;
use beebeeb_engine::app::{CredentialStores, Engine};

use super::hello::{HelloCancel, HelloDone, HelloPrompter, HelloResult, NoHello};
use super::{log, map_gate_signin, map_next, map_problem, phrase_problem, two_factor_problem};
use beebeeb_engine::app::signup_api::SignupAttempt;

#[path = "signin_device.rs"]
mod signin_device;
#[path = "signin_recovery.rs"]
mod signin_recovery;
#[path = "signup.rs"]
mod signup;
use crate::frames::signin::create_source::{PasswordVerdict, SignUpEvent};
use crate::frames::signin::source::{
    CodeKind, GateState, HelloOutcome, NextAfterSignIn, PasskeyOutcome, PhraseProblem, ScreenPasswordOutcome,
    SharedSession, SignInEvent, SignInProblem, SignInSource,
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
    /// The screen lock's password check, and the countdown of the wait after it.
    ScreenPassword,
    ScreenCountdown,
    /// The lock screen's Windows Hello prompt.
    Hello,
    /// Sign-up's steps and the wait for a new code (`signup.rs`).
    SignUpCode,
    SignUpVerify,
    SignUpPassword,
    SignUpConfirm,
    SignUpCreate,
    SignUpTick,
    /// A request to be signed in by another device (`signin_device.rs`): its code, ticks and ending.
    Device,
    /// A call this source does not bind: answered at once, never stale.
    Unbound,
}

const WAITS: usize = Wait::Unbound as usize + 1;

/// The lock screen's Windows Hello prompt that is up: the lock it was asked for (core's claim), the way to take it
/// away, and whether it answered (an answered prompt is over: nothing of it is taken away any more).
struct HelloUp {
    asked_for: ScreenUnlockAsk,
    cancel: HelloCancel,
    answered: Arc<AtomicBool>,
}

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
    /// The Windows Hello prompt, and the one that is up: the lock it was asked for, and the way to take it away.
    hello: Box<dyn HelloPrompter>,
    hello_up: Option<HelloUp>,
    /// The lock a password is opening, as it was when the password was sent; shared with the worker, which
    /// puts that lock back when the person left the step and it finished anyway. `None`: a sign-in.
    unlocking: Arc<Mutex<Option<LockState>>>,
    /// The account being created, if any: owned here, wiped when sign-up is left (`signup.rs`).
    signup: Arc<SignupAttempt>,
    /// The request to be signed in by another device that runs, if any (`signin_device.rs`).
    device: signin_device::DeviceSeat,
}

impl<C: CredentialStores> EngineSignInSource<C> {
    /// The no-plan screen's sign-out: the session as it is now, signed out by the engine's own call (which refuses
    /// an open editor's unsaved changes unless `discard`).
    fn sign_out_gate_as(&mut self, discard: bool) {
        let job: Job<C> = Box::new(move |engine| {
            if engine.has_session() {
                let session = engine.session_turn();
                let done = if discard {
                    engine.sign_out_discarding_unsaved_for(session)
                } else {
                    engine.sign_out_for(session)
                };
                if done.is_err() {
                    log("sign-in: the sign-out from the gate was not done");
                }
            }
            engine.wake_host();
        });
        if self.jobs.as_ref().is_none_or(|jobs| jobs.send(job).is_err()) {
            log("sign-in: the sign-out could not be passed to the engine");
        }
    }

    /// Without a Windows Hello prompt: the lock screen's Unlock answers "not available" and offers the password.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self::with_tick(engine, Duration::from_secs(1))
    }

    /// The running app's: `hello` shows the prompt for the lock screen.
    pub fn with_hello(engine: Arc<Engine<C>>, hello: Box<dyn HelloPrompter>) -> Self {
        let mut source = Self::new(engine);
        source.hello = hello;
        source
    }

    pub(super) fn with_tick(engine: Arc<Engine<C>>, tick: Duration) -> Self {
        let (answers, inbox) = mpsc::channel();
        let (jobs, queue) = mpsc::channel::<Job<C>>();
        let for_worker = engine.clone();
        // The worker ends when this source is dropped (its queue closes).
        let started = thread::Builder::new().name("signin".into()).spawn(move || {
            for job in queue {
                job(&for_worker);
                super::finder_setup::session_changed();
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
            hello: Box::new(NoHello),
            hello_up: None,
            unlocking: Arc::new(Mutex::new(None)),
            signup: Arc::new(SignupAttempt::new()),
            device: signin_device::DeviceSeat::default(),
        }
    }

    /// A new attempt of `wait`: every answer of an earlier one is stale from here on.
    fn next_attempt(&self, wait: Wait) -> u64 {
        self.attempts[wait as usize].fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The lock screen's prompt is up, unanswered, for a lock that ended (core's word, `screen_unlock_ask_holds`: the
    /// screen opened by any means, locked again, the vault closed, a sign-out, another account): it is taken away,
    /// whatever it still answers is dropped, and the screen hears `Cancelled`, so its Unlock asks again. On every
    /// poll, whatever the window shows: the window's own follow of the lock sees only the lock it reads next.
    fn end_hello_of_a_lock_that_ended(&mut self) {
        let ended = self.hello_up.as_ref().is_some_and(|up| {
            !up.answered.load(Ordering::SeqCst) && !self.engine.screen_unlock_ask_holds(up.asked_for)
        });
        if !ended {
            return;
        }
        if let Some(up) = self.hello_up.take() {
            (up.cancel)();
        }
        let attempt = self.next_attempt(Wait::Hello);
        let _ = self.answers.send(Answer {
            wait: Wait::Hello,
            attempt,
            event: SignInEvent::Hello(HelloOutcome::Cancelled),
        });
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
        let unlocking = self.unlocking.clone();
        let job: Job<C> = Box::new(move |engine| {
            if attempts[wait as usize].load(Ordering::SeqCst) != attempt {
                return;
            }
            let event = work(engine);
            // The person left this step while it ran (`discard_partial_sign_in` moved the attempt on): its answer
            // is dropped by `poll`, and what it did is undone here, before the next job. Back wins.
            if attempts[wait as usize].load(Ordering::SeqCst) != attempt {
                undo_left_step(engine, &answers, &event, *lock(&unlocking));
                engine.wake_host();
                return;
            }
            let wait_seconds = match &event {
                SignInEvent::SignedIn(Err(SignInProblem::RateLimited { seconds })) => Some(*seconds),
                _ => None,
            };
            let screen_wait_seconds = match &event {
                SignInEvent::ScreenPassword(
                    ScreenPasswordOutcome::Wait { wait_ms } | ScreenPasswordOutcome::Wrong { wait_ms },
                ) => Some(seconds_up(*wait_ms)),
                _ => None,
            };
            let resend_seconds = match &event {
                SignInEvent::SignUp(SignUpEvent::CodeSent(Ok(sent))) => Some(sent.resend_seconds),
                _ => None,
            };
            let _ = answers.send(Answer { wait, attempt, event });
            if let Some(seconds) = resend_seconds {
                signup::count_down_resend(engine, &answers, &attempts, seconds, tick);
            }
            // After the answer is in the channel: no tick can come before the problem it counts down.
            if let Some(seconds) = wait_seconds {
                count_down(engine, &answers, &attempts, seconds, tick);
            }
            if let Some(seconds) = screen_wait_seconds.filter(|seconds| *seconds > 0) {
                count_down_as(
                    engine,
                    &answers,
                    &attempts,
                    seconds,
                    tick,
                    Wait::ScreenCountdown,
                    |seconds| SignInEvent::ScreenWaitLeft { seconds },
                );
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
}

/// The engine's answer for the window.
fn map_screen_unlock(answer: Result<ScreenUnlock, LockRefused>) -> ScreenPasswordOutcome {
    match answer {
        Ok(ScreenUnlock::Opened) => ScreenPasswordOutcome::Opened,
        Ok(ScreenUnlock::Wrong { wait_ms }) => ScreenPasswordOutcome::Wrong { wait_ms },
        Ok(ScreenUnlock::Wait { wait_ms }) => ScreenPasswordOutcome::Wait { wait_ms },
        Ok(ScreenUnlock::ServerLimit { wait_ms }) => ScreenPasswordOutcome::ServerLimit { wait_ms },
        Ok(ScreenUnlock::LocalCheckOff) => ScreenPasswordOutcome::LocalCheckOff,
        Ok(ScreenUnlock::OfflineNoRecord) => ScreenPasswordOutcome::OfflineNoRecord,
        Ok(ScreenUnlock::AccountUpgradeNeeded) => ScreenPasswordOutcome::AccountUpgradeNeeded,
        Ok(ScreenUnlock::SessionEnded) => ScreenPasswordOutcome::SessionEnded,
        Err(LockRefused::Overtaken | LockRefused::NotAtThisLevel | LockRefused::NeedsThePassword) => {
            ScreenPasswordOutcome::Overtaken
        }
        Err(LockRefused::NothingTyped | LockRefused::AlreadyChecking | LockRefused::CouldNotCheck) => {
            ScreenPasswordOutcome::Failed
        }
    }
}

/// Whole seconds of a wait in milliseconds, rounded up (a wait of 1 ms is a second).
fn seconds_up(wait_ms: u64) -> u32 {
    u32::try_from(wait_ms.div_ceil(1_000)).unwrap_or(u32::MAX)
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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A step the person left finished anyway (it held the engine's turn): undo what it did. A sign-in that the code
/// or the twelve words completed is signed out (the local files and state stay). A lock that the password (or
/// the code after it) opened is put back at the level it had (`unlocking`): the screen lock for a locked screen,
/// the vault lock for a closed vault. A step that was refused did nothing. A sign-out that does not finish is
/// said to the window (`StillSignedIn`), never only logged.
fn undo_left_step<C: CredentialStores>(
    engine: &Arc<Engine<C>>,
    answers: &mpsc::Sender<Answer>,
    event: &SignInEvent,
    unlocking: Option<LockState>,
) {
    let opened = matches!(
        event,
        SignInEvent::TwoFactor(Ok(NextAfterSignIn::Drive))
            | SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::Drive))
            | SignInEvent::PhraseRestored(Ok(_))
    );
    match (event, unlocking) {
        (_, Some(LockState::ScreenLocked)) if opened => {
            let _ = engine.lock_screen();
        }
        // A screen password the person left finished anyway and opened the screen: locked again.
        (SignInEvent::ScreenPassword(ScreenPasswordOutcome::Opened), _) => {
            let _ = engine.lock_screen();
        }
        (_, Some(_)) if opened => {
            let _ = engine.lock_vault();
        }
        // A left password step that ended at the code step opened and stored nothing: it locks nothing (the queued
        // discard forgets the half-done step). Every other `Ok` (the keyless case: a session WAS stored) locks.
        (SignInEvent::VaultUnlocked(Ok(next)), _) if *next != NextAfterSignIn::TwoFactor => {
            let _ = engine.lock_vault();
        }
        (
            SignInEvent::TwoFactor(Ok(_))
            | SignInEvent::PhraseRestored(Ok(_))
            | SignInEvent::SignUp(SignUpEvent::Created(Ok(_)))
            | SignInEvent::Passkey(PasskeyOutcome::Succeeded(_)),
            None,
        ) if engine.sign_out_keeping_local_data().is_err() => {
            still_signed_in(answers);
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
    count_down_as(engine, answers, attempts, seconds, tick, Wait::Retry, |seconds| {
        SignInEvent::RetryAfter { seconds }
    });
}

/// [`count_down`] for the wait `wait`, its event made by `event`.
fn count_down_as<C: CredentialStores>(
    engine: &Arc<Engine<C>>,
    answers: &mpsc::Sender<Answer>,
    attempts: &Arc<[AtomicU64; WAITS]>,
    seconds: u32,
    tick: Duration,
    wait: Wait,
    event: fn(u32) -> SignInEvent,
) {
    let attempt = attempts[wait as usize].fetch_add(1, Ordering::SeqCst) + 1;
    let over = || {
        let _ = answers.send(Answer {
            wait,
            attempt,
            event: event(0),
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
            if attempts_in[wait as usize].load(Ordering::SeqCst) != attempt {
                return;
            }
            left -= 1;
            let tick = Answer {
                wait,
                attempt,
                event: event(left),
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

impl<C: CredentialStores> Drop for EngineSignInSource<C> {
    /// The source goes (the window that owns it closes, the app quits): a prompt it asked for is taken away, never
    /// left on screen with nobody to answer, and an answer it decided before that opens nothing.
    fn drop(&mut self) {
        self.next_attempt(Wait::Hello);
        if let Some(up) = self.hello_up.take() {
            (up.cancel)();
        }
    }
}

impl<C: CredentialStores> SignInSource for EngineSignInSource<C> {
    /// The gate from the handle's view: read without waiting, on every frame.
    fn gate(&self) -> GateState {
        map_gate_signin(self.engine.view().gate)
    }

    /// w16-gates: the gate read once more and published, on the worker (the credential store may be slow to answer).
    fn recheck_gate(&mut self) {
        let job: Job<C> = Box::new(|engine| {
            engine.recheck_gate();
            engine.wake_host();
        });
        if self.jobs.as_ref().is_none_or(|jobs| jobs.send(job).is_err()) {
            log("sign-in: the gate re-check could not be passed to the engine");
        }
    }

    /// w16-gates: the no-plan screen's sign-out: the session as it is now, signed out by the engine's own call.
    fn sign_out_from_gate(&mut self) {
        self.sign_out_gate_as(false);
    }

    fn sign_out_from_gate_discarding_unsaved(&mut self) {
        self.sign_out_gate_as(true);
    }

    /// No other Beebeeb app's session is read on this computer, and no Windows Hello prompt exists for this app
    /// yet: the Continue-as card is never offered, so `begin_hello` is never reached from a screen.
    fn shared_session(&self) -> Option<SharedSession> {
        None
    }

    fn begin_sign_in(&mut self, email: &str, password: &str) {
        *lock(&self.unlocking) = None;
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

    /// The lock screen's Unlock. The session's turn is read HERE, at the press: a yes that outlives a lock, an
    /// unlock, a sign-out or another account's sign-in opens nothing (the engine refuses a turn that moved on,
    /// under the session's lock; every one of those moves it, so the turn names the account too). Only a
    /// verified prompt reaches the engine.
    fn begin_hello(&mut self) {
        let attempt = self.next_attempt(Wait::Hello);
        // A prompt this screen asked for before is taken away, never just forgotten (it would stay on screen).
        if let Some(up) = self.hello_up.take() {
            (up.cancel)();
        }
        let answer = move |event| Answer {
            wait: Wait::Hello,
            attempt,
            event: SignInEvent::Hello(event),
        };
        // The lock the prompt is for, as core holds it now (the session's turn and the screen lock, in one read).
        let Some(asked_for) = self.engine.ask_screen_unlock() else {
            // Nothing a prompt could open: no prompt is shown.
            let _ = self.answers.send(answer(HelloOutcome::Cancelled));
            return;
        };
        let (engine, answers, attempts) = (self.engine.clone(), self.answers.clone(), self.attempts.clone());
        let answered = Arc::new(AtomicBool::new(false));
        let answering = answered.clone();
        let done: HelloDone = Box::new(move |result| {
            answering.store(true, Ordering::SeqCst);
            let outcome = match result {
                HelloResult::Verified => {
                    let still_asked = attempts[Wait::Hello as usize].load(Ordering::SeqCst) == attempt;
                    if still_asked && engine.unlock_screen_for(asked_for).is_ok() {
                        // Touch ID is no sign-in job: report the session here so the Finder hold lifts.
                        super::finder_setup::session_changed();
                        HelloOutcome::Succeeded(NextAfterSignIn::Drive)
                    } else {
                        // Late, or refused by core's rule: nothing opened, and the button works again.
                        HelloOutcome::Cancelled
                    }
                }
                HelloResult::Cancelled => HelloOutcome::Cancelled,
                HelloResult::Failed => HelloOutcome::NotRecognized,
                HelloResult::Unavailable => HelloOutcome::Unavailable,
            };
            let _ = answers.send(answer(outcome));
            engine.wake_host();
        });
        match self.hello.start(done) {
            Ok(cancel) => {
                self.hello_up = Some(HelloUp {
                    asked_for,
                    cancel,
                    answered,
                })
            }
            Err(()) => {
                let _ = self.answers.send(answer(HelloOutcome::Unavailable));
            }
        }
    }

    /// The person left the lock screen's prompt: no answer of it is handed out, and the prompt is taken away.
    fn cancel_hello(&mut self) {
        self.next_attempt(Wait::Hello);
        if let Some(up) = self.hello_up.take() {
            (up.cancel)();
        }
    }

    fn begin_device_request(&mut self) {
        self.device_begin();
    }

    fn cancel_device_request(&mut self) {
        self.device_cancel();
    }

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
                undo_left_step(&engine, &answers, &event, None);
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

    /// The account's password for a lock: a closed vault, or a locked screen whose Windows Hello cannot answer.
    /// Core's strong unlock through the engine ([`Engine::unlock_vault_with_password`]): the password is proven
    /// to the server in a new sign-in for the account whose session is stored; `TwoFactor` asks the code next.
    /// Needs the network: offline it answers `Offline` and opens nothing.
    fn begin_unlock(&mut self, password: &str) {
        let password = secret(password);
        *lock(&self.unlocking) = Some(self.engine.lock_state());
        self.ask(
            Wait::Unlock,
            SignInEvent::VaultUnlocked(Err(SignInProblem::Other)),
            move |engine| match engine.unlock_vault_with_password(password) {
                Ok(next) => SignInEvent::VaultUnlocked(Ok(map_next(next))),
                Err(problem) => SignInEvent::VaultUnlocked(Err(map_problem(problem))),
            },
        );
    }

    /// The screen lock's password: counted tries and the server's proof that makes no session
    /// ([`Engine::unlock_screen_with_password`]). The session's turn is read HERE, at the press: an answer that
    /// outlived a lock, a sign-out or another sign-in is dropped (`Overtaken`), and a press that was superseded
    /// has its answer dropped by `poll`. A password the person left behind that opened the screen locks it again
    /// (`undo_left_step`). `AccountUpgradeNeeded` closes the vault so the strong unlock's screen follows.
    fn begin_screen_password(&mut self, password: &str) {
        let password = secret(password);
        let asked_at = self.engine.session_turn();
        *lock(&self.unlocking) = Some(LockState::ScreenLocked);
        self.ask(
            Wait::ScreenPassword,
            SignInEvent::ScreenPassword(ScreenPasswordOutcome::Failed),
            move |engine| {
                if engine.session_turn() != asked_at {
                    return SignInEvent::ScreenPassword(ScreenPasswordOutcome::Overtaken);
                }
                let outcome = map_screen_unlock(engine.unlock_screen_with_password(password));
                if outcome == ScreenPasswordOutcome::AccountUpgradeNeeded {
                    let _ = engine.lock_vault();
                }
                SignInEvent::ScreenPassword(outcome)
            },
        );
    }

    /// The in-app recovery with the twelve words (`signin_recovery.rs`), and the device request: both doors are bound.
    fn in_app_doors_available(&self) -> bool {
        true
    }

    fn begin_recovery_verify(&mut self, email: &str, words: &[&str]) {
        self.recovery_begin_verify(email, words);
    }

    fn begin_set_password(&mut self, password: &str) {
        self.recovery_begin_set_password(password);
    }

    fn password_strength(&self, password: &str) -> Option<crate::frames::signin::source::PasswordStrength> {
        self.recovery_strength(password)
    }

    fn sign_up_available(&self) -> bool {
        self.signup_available()
    }

    fn sign_up_web_address(&self) -> Option<String> {
        self.signup.web_address()
    }

    fn begin_sign_up_code(&mut self, email: &str) {
        self.signup_begin_code(email);
    }

    fn begin_sign_up_verify(&mut self, code: &str) {
        self.signup_begin_verify(code);
    }

    fn sign_up_password(&self, password: &str) -> Option<PasswordVerdict> {
        Some(self.signup_password_verdict(password))
    }

    fn begin_sign_up_password(&mut self, password: &str) {
        self.signup_begin_password(password);
    }

    fn with_sign_up_phrase(&self, show: &mut dyn FnMut(&[&str])) {
        self.signup.with_phrase(show);
    }

    /// The window holds none of the words, so leaving the screen that shows them has nothing to wipe here.
    fn sign_up_phrase_left(&mut self) {}

    fn sign_up_confirm_positions(&self) -> Option<[usize; crate::frames::signin::create_source::CONFIRM_WORDS]> {
        self.signup.confirm_positions()
    }

    fn sign_up_suggestions(&self, typed: &str) -> Vec<&'static str> {
        self.signup_suggestions(typed)
    }

    fn begin_sign_up_confirm(&mut self, words: &[&str]) {
        self.signup_begin_confirm(words);
    }

    fn sign_up_device_name(&self) -> String {
        self.signup_device_name()
    }

    fn begin_sign_up_create(&mut self, device_name: &str) {
        self.signup_begin_create(device_name);
    }

    fn cancel_sign_up(&mut self) {
        self.signup_cancel();
    }

    /// The next answer that is still waited for; an answer of a step the person left is dropped here.
    fn poll(&mut self) -> Option<SignInEvent> {
        self.end_hello_of_a_lock_that_ended();
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
            // The engine forgets every half-done step's copies itself (the code challenge, the stretch, a held recovery).
            if engine.discard_partial_sign_in().is_err() {
                still_signed_in(&answers);
            }
            engine.wake_host();
        });
        if self.jobs.as_ref().is_none_or(|jobs| jobs.send(job).is_err()) {
            log("sign-in: leaving the step could not be passed to the engine");
        }
    }
}

/// Opens one of our own addresses by the same OS call a file opens with: the address is one argument, never a
/// command line. Only five schemes open (`https://`, `http://`, `mailto:`, `ms-settings:`, `x-apple.systempreferences:`);
/// anything else is refused in
/// every build. The addresses come from `shell::web_pages` only, never from the server, a file or a typed string.
pub fn open_web_page(url: &str) -> Result<(), ()> {
    if !is_allowed_address(url) {
        return Err(());
    }
    // A test run opens nothing on the machine it runs on; it records what it was asked to open.
    #[cfg(test)]
    {
        OPENED.with(|o| o.borrow_mut().push(url.to_string()));
        Ok(())
    }
    // A Mac opens a web address through `NSWorkspace openURL` (`open_path` there is the Finder-location opener, which
    // cannot open an address: Settings' web rows did nothing).
    #[cfg(all(not(test), target_os = "macos"))]
    {
        crate::shell::platform::macos_open_url::open_url(url)
    }
    #[cfg(all(not(test), not(target_os = "macos")))]
    open_on_the_launch_worker(url, super::open_path)
}

/// The one launch worker the pages open on: the window's thread never waits for the launch lock. A
/// page has no session (the sign-in page opens signed out), so its start compares none.
#[cfg_attr(any(test, target_os = "macos"), allow(dead_code))]
fn open_on_the_launch_worker(url: &str, start: super::files_launch::Start) -> Result<(), ()> {
    static PAGES: std::sync::LazyLock<super::launcher::Launcher> = std::sync::LazyLock::new(Default::default);
    PAGES.open(
        start,
        std::path::PathBuf::from(url),
        || true,
        |answer| {
            if answer.is_err() {
                super::log("sign-in: a page did not open");
            }
        },
    )
}

#[cfg(test)]
thread_local! {
    /// The addresses `open_web_page` accepted on this thread, oldest first (tests only).
    pub(crate) static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Whether `url` starts with one of the five schemes the opener allows (https and http, mail, Windows Settings,
/// macOS System Settings).
pub fn is_allowed_address(url: &str) -> bool {
    [
        "https://",
        "http://",
        "mailto:",
        "ms-settings:",
        "x-apple.systempreferences:",
    ]
    .iter()
    .any(|scheme| url.starts_with(scheme))
}

#[cfg(test)]
#[path = "signin_tests.rs"]
mod tests;
