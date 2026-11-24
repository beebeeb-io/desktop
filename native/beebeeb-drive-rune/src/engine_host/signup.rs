//! Creating an account on the real engine (DW-A31 to DW-A44): the sign-up half of [`EngineSignInSource`]. Every
//! step is an engine call (`beebeeb_engine::app::signup_api`, which is core's `SignupFlow`), run on the sign-in
//! source's one worker and answered through the same channel, so an answer the person left is dropped, and a step
//! that finishes after they left is undone (`undo_left_step`).
//!
//! What this file adds is only the mapping: the engine's kinds to the screen's events, the once-a-second wait for
//! a new code, and the computer's name.
//!
//! Secrets: the password and the three typed words are copied ONCE into wiping buffers that move to the worker
//! and into the engine; none is logged or put in an event. The twelve words are never held here: `with_sign_up_phrase`
//! lends them from the engine for one draw. While the server still refuses sign-up from this app (403
//! `signup_web_only`) the email step says so with the page the server named; nothing is stored.
//!
//! Must not: log an email, a code, a password, a word, a token or a key.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use zeroize::Zeroizing;

use beebeeb_engine::app::CredentialStores;
use beebeeb_engine::app::Engine;
use beebeeb_engine::app::signup_api::{
    SignupCodeProblem, SignupCreateProblem, SignupHint, SignupPasswordProblem, SignupSendProblem, word_suggestions,
};

use super::{Answer, EngineSignInSource, WAITS, Wait, secret};
use crate::frames::signin::create_source::{
    CodeProblem, CodeSent, CreateProblem, PasswordProblem, PasswordVerdict, SendProblem, SignUpEvent,
};
use crate::frames::signin::source::SignInEvent;

/// How many words are offered under what is typed.
const SUGGESTIONS: usize = 4;

/// The longest name a device may have (the screen's own rule, `DEVICE_NAME_MAX`).
const DEVICE_NAME_FALLBACK: &str = "This computer";

fn event(event: SignUpEvent) -> SignInEvent {
    SignInEvent::SignUp(event)
}

fn send_problem(problem: SignupSendProblem) -> SendProblem {
    match problem {
        SignupSendProblem::RateLimited { .. } => SendProblem::RateLimited,
        SignupSendProblem::Offline => SendProblem::Offline,
        SignupSendProblem::WebOnly => SendProblem::WebOnly,
        SignupSendProblem::Other | SignupSendProblem::Left => SendProblem::Other,
    }
}

fn code_problem(problem: SignupCodeProblem) -> CodeProblem {
    match problem {
        SignupCodeProblem::WrongOrExpired => CodeProblem::WrongOrExpired,
        SignupCodeProblem::TooManyWrong => CodeProblem::TooManyWrong,
        SignupCodeProblem::Offline => CodeProblem::Offline,
        SignupCodeProblem::Other | SignupCodeProblem::Left => CodeProblem::Other,
    }
}

fn password_problem(problem: SignupPasswordProblem) -> PasswordProblem {
    match problem {
        SignupPasswordProblem::Breached => PasswordProblem::Breached,
        SignupPasswordProblem::CheckUnreachable => PasswordProblem::CheckUnreachable,
        SignupPasswordProblem::Other | SignupPasswordProblem::Left => PasswordProblem::Other,
    }
}

fn create_problem(problem: SignupCreateProblem) -> CreateProblem {
    match problem {
        SignupCreateProblem::NotFinishedHere => CreateProblem::NotFinishedHere,
        SignupCreateProblem::CodeExpired => CreateProblem::CodeExpired,
        SignupCreateProblem::NothingStored | SignupCreateProblem::Left => CreateProblem::NothingStored,
    }
}

/// The waits that belong to sign-up: leaving it ends every one of them.
pub(super) const SIGN_UP_WAITS: [Wait; 6] = [
    Wait::SignUpCode,
    Wait::SignUpVerify,
    Wait::SignUpPassword,
    Wait::SignUpConfirm,
    Wait::SignUpCreate,
    Wait::SignUpTick,
];

/// The words the password's strength is told in (the board's, for its two drawings, and a word for each level between).
fn verdict_words(level: u8, meets_minimum: bool, min_length: u32, missing: u32) -> String {
    if !meets_minimum {
        return format!("Needs at least {min_length} characters, {missing} more.");
    }
    match level {
        4 => "Strong.".to_string(),
        3 => "Good.".to_string(),
        _ => "Fair.".to_string(),
    }
}

fn hint_words(hint: SignupHint) -> Option<String> {
    match hint {
        SignupHint::None => None,
        SignupHint::MoreCharacters | SignupHint::MixCaseAndNumberOrSymbol => {
            Some("Mix upper and lowercase, and add a number or symbol.".to_string())
        }
        SignupHint::MixCase => Some("Mix upper and lowercase.".to_string()),
        SignupHint::NumberOrSymbol => Some("Add a number or symbol.".to_string()),
    }
}

/// The computer's name as the system has it (Windows: `COMPUTERNAME`), at most the screen's 40 characters.
fn computer_name() -> String {
    let named = ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|name| name.trim().to_string())
        .find(|name| !name.is_empty());
    named
        .unwrap_or_else(|| DEVICE_NAME_FALLBACK.to_string())
        .chars()
        .take(40)
        .collect()
}

impl<C: CredentialStores> EngineSignInSource<C> {
    pub(super) fn signup_available(&self) -> bool {
        true
    }

    pub(super) fn signup_begin_code(&mut self, email: &str) {
        let (attempt, email) = (self.signup.clone(), email.to_string());
        self.ask(
            Wait::SignUpCode,
            event(SignUpEvent::CodeSent(Err(SendProblem::Other))),
            move |engine| {
                let sent = match engine.signup_send_code(&attempt, &email) {
                    Ok(sent) => Ok(CodeSent {
                        digits: sent.digits,
                        resend_seconds: sent.resend_seconds,
                    }),
                    Err(problem) => Err(send_problem(problem)),
                };
                event(SignUpEvent::CodeSent(sent))
            },
        );
    }

    pub(super) fn signup_begin_verify(&mut self, code: &str) {
        let (attempt, code) = (self.signup.clone(), secret(code));
        self.ask(
            Wait::SignUpVerify,
            event(SignUpEvent::Verified(Err(CodeProblem::Other))),
            move |engine| {
                let verified = engine.signup_verify_code(&attempt, &code).map_err(code_problem);
                event(SignUpEvent::Verified(verified))
            },
        );
    }

    /// Core's evaluator with the server's minimum: local, at once, on every key press.
    pub(super) fn signup_password_verdict(&self, password: &str) -> PasswordVerdict {
        let verdict = self.signup.verdict(password);
        PasswordVerdict {
            bars: verdict.level,
            words: verdict_words(
                verdict.level,
                verdict.meets_minimum,
                verdict.min_length,
                verdict.missing,
            ),
            hint: hint_words(verdict.hint),
            long_enough: verdict.meets_minimum,
        }
    }

    pub(super) fn signup_begin_password(&mut self, password: &str) {
        let (attempt, password) = (self.signup.clone(), secret(password));
        self.ask(
            Wait::SignUpPassword,
            event(SignUpEvent::PasswordChecked(Err(PasswordProblem::Other))),
            move |engine| {
                let checked = engine.signup_set_password(&attempt, password).map_err(password_problem);
                event(SignUpEvent::PasswordChecked(checked))
            },
        );
    }

    pub(super) fn signup_begin_confirm(&mut self, words: &[&str]) {
        let attempt = self.signup.clone();
        let words: Vec<Zeroizing<String>> = words.iter().map(|word| secret(word)).collect();
        self.ask(
            Wait::SignUpConfirm,
            event(SignUpEvent::PhraseConfirmed(false)),
            move |engine| {
                let typed: Vec<&str> = words.iter().map(|word| word.as_str()).collect();
                event(SignUpEvent::PhraseConfirmed(
                    engine.signup_confirm_phrase(&attempt, &typed),
                ))
            },
        );
    }

    pub(super) fn signup_suggestions(&self, typed: &str) -> Vec<&'static str> {
        word_suggestions(typed, SUGGESTIONS)
    }

    pub(super) fn signup_device_name(&self) -> String {
        computer_name()
    }

    pub(super) fn signup_begin_create(&mut self, device_name: &str) {
        let (attempt, device) = (self.signup.clone(), device_name.to_string());
        self.ask(
            Wait::SignUpCreate,
            event(SignUpEvent::Created(Err(CreateProblem::NothingStored))),
            move |engine| {
                let created = engine.signup_create(&attempt, &device).map_err(create_problem);
                event(SignUpEvent::Created(created))
            },
        );
    }

    /// The person left sign-up. From this call on no answer of it is handed out, a step that has not started does
    /// not start, and one that is running ends with `Left` (and a session it made is signed out again). The engine
    /// then forgets the password, the phrase and the key it held, in its turn on the worker.
    pub(super) fn signup_cancel(&mut self) {
        for wait in SIGN_UP_WAITS {
            self.next_attempt(wait);
        }
        self.signup.leave();
        let attempt = self.signup.clone();
        let job: super::Job<C> = Box::new(move |engine| {
            attempt.cancel();
            engine.wake_host();
        });
        if !self.jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok()) {
            // No worker to wipe in: wipe here, waiting for a step that holds the flow.
            self.signup.cancel();
        }
    }
}

/// Count the wait for a new code down: `ResendTick` once a `tick`, `0` when it is over. It stops when a newer wait
/// began or sign-up was left. Called AFTER the `CodeSent` answer it counts down from is in the channel.
pub(super) fn count_down_resend<C: CredentialStores>(
    engine: &Arc<Engine<C>>,
    answers: &mpsc::Sender<Answer>,
    attempts: &Arc<[AtomicU64; WAITS]>,
    seconds: u32,
    tick: Duration,
) {
    let wait = Wait::SignUpTick;
    let attempt = attempts[wait as usize].fetch_add(1, Ordering::SeqCst) + 1;
    if seconds == 0 {
        return;
    }
    let (engine_in, answers_in, attempts_in) = (engine.clone(), answers.clone(), attempts.clone());
    let (answers_end, engine_end) = (answers.clone(), engine.clone());
    let started = thread::Builder::new().name("signup-wait".into()).spawn(move || {
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
                event: event(SignUpEvent::ResendTick { seconds: left }),
            };
            if answers_in.send(tick).is_err() {
                return;
            }
            engine_in.wake_host();
        }
    });
    if started.is_err() {
        // A wait that cannot be counted ends at once: a new code may be asked for, and the server says if it is too soon.
        let end = Answer {
            wait,
            attempt,
            event: event(SignUpEvent::ResendTick { seconds: 0 }),
        };
        let _ = answers_end.send(end);
        engine_end.wake_host();
    }
}

#[cfg(test)]
#[path = "signup_tests.rs"]
mod tests;
