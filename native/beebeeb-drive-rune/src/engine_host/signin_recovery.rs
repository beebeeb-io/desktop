//! The in-app recovery on the real engine (DW-A45 to DW-A50): the recovery half of [`EngineSignInSource`]. Both steps
//! are engine calls (`beebeeb_engine::app::bindings::recovery`, which is core's `PhraseRecovery`), run on the sign-in
//! source's one worker and answered through the same channel.
//!
//! What this file adds is only the binding: the session turn read at the press, the words joined into ONE wiping
//! buffer, and the engine's kinds mapped to the screen's events. The rules are the engine's and core's: a word that
//! is not in the word list is marked and nothing is sent; the words never leave this computer (core sends a check
//! derived from the key); the held recovery (the key, the server's recovery token) stays in the engine, never here.
//!
//! The answers belong to the twelve-word wait (`Wait::Restore`): the recovery's screens and the twelve-word unlock
//! are never on screen together, and a newer press (or leaving the twelve words) makes an older answer stale, so
//! `poll` drops it. A press whose session turn moved before the engine answered is refused by the engine
//! (`Overtaken`), which stores nothing. Leaving a half-done sign-in (`discard_partial_sign_in`) has the engine forget
//! the recovery it holds, after any step already on the worker.
//!
//! Secrets: the twelve words and the password arrive as `&str` borrowed from the window's fields; each is copied ONCE
//! into a wiping buffer made at its full size, which moves to the worker and into the engine, and is wiped there.
//! None is kept, logged, formatted or put in an event. Must not: log an email, a word, a password, a token or a key.

use beebeeb_engine::app::CredentialStores;
use beebeeb_engine::app::bindings::recovery::{RecoveryDone, RecoveryRefused};

use super::{EngineSignInSource, Wait, phrase_of, secret};
use crate::frames::signin::source::{NewPasswordProblem, PasswordStrength, PhraseProblem, SignInEvent};

/// The wait the recovery's answers belong to (see the module's doc).
const RECOVERY: Wait = Wait::Restore;

/// The engine's refusal of the words, as the recovery screen draws it (DW-A48).
fn phrase_problem(refused: RecoveryRefused) -> PhraseProblem {
    match refused {
        RecoveryRefused::WrongPhrase => PhraseProblem::WrongPhrase { bad_word: None },
        // The screen marks the first word that is not in the list, and its focus goes there.
        RecoveryRefused::UnknownWords(positions) => PhraseProblem::WrongPhrase {
            bad_word: positions.first().map(|at| *at as usize),
        },
        RecoveryRefused::RateLimited { .. } => PhraseProblem::TooManyTries,
        RecoveryRefused::Offline => PhraseProblem::Offline,
        RecoveryRefused::Weak | RecoveryRefused::Breached | RecoveryRefused::Overtaken | RecoveryRefused::Other => {
            PhraseProblem::Other
        }
    }
}

/// The engine's answer to the new password, as the new-password screen takes it (DW-A49): each kind keeps its own words
/// there. Every kind is named: a new one must be given its words, never fall into another's.
fn password_set(done: Result<RecoveryDone, RecoveryRefused>) -> Result<(), NewPasswordProblem> {
    match done {
        Ok(RecoveryDone::SignedIn) => Ok(()),
        // The password WAS changed and nothing was signed in (two-factor: the server wants a normal sign-in). `Ok` would
        // draw "Your vault is unlocked", which is not true: the window shows the password updated and the way to sign in.
        Ok(RecoveryDone::SignInRequired) => Err(NewPasswordProblem::SignInRequired),
        Err(RecoveryRefused::Weak) => Err(NewPasswordProblem::TooShort),
        Err(RecoveryRefused::Breached) => Err(NewPasswordProblem::Breached),
        Err(RecoveryRefused::Offline) => Err(NewPasswordProblem::Offline),
        Err(RecoveryRefused::RateLimited { seconds }) => Err(NewPasswordProblem::RateLimited { seconds }),
        // Left, ended by the server (its recovery token refused), overtaken, or nothing held: start again from the words.
        Err(RecoveryRefused::Overtaken) => Err(NewPasswordProblem::Ended),
        // The words are not checked at this step; a refusal of them here is a step that did not finish.
        Err(RecoveryRefused::WrongPhrase | RecoveryRefused::UnknownWords(_) | RecoveryRefused::Other) => {
            Err(NewPasswordProblem::Other)
        }
    }
}

/// The words the meter tells the strength in: the same as sign-up's (`signup.rs`), so both screens agree.
fn strength_words(level: u8, meets_minimum: bool, min_length: u32, missing: u32) -> String {
    if !meets_minimum {
        return format!("Needs at least {min_length} characters, {missing} more.");
    }
    match level {
        4 => "Strong.".to_string(),
        3 => "Good.".to_string(),
        _ => "Fair.".to_string(),
    }
}

impl<C: CredentialStores> EngineSignInSource<C> {
    /// "Verify master key" (DW-A47): the email and the twelve words, for the session turn of this press.
    pub(super) fn recovery_begin_verify(&mut self, email: &str, words: &[&str]) {
        let turn = self.engine.session_turn();
        // Each box's word without the spaces around it, so the joined words split back into the boxes one to one and
        // a position the engine marks is the box's (a box typed as "word " would shift every later mark by one).
        let words: Vec<&str> = words.iter().map(|word| word.trim()).collect();
        let (email, phrase) = (email.to_string(), phrase_of(&words));
        self.ask(
            RECOVERY,
            SignInEvent::RecoveryPhraseVerified(Err(PhraseProblem::Other)),
            move |engine| {
                let verified = engine.recovery_verify_for(turn, &email, phrase).map_err(phrase_problem);
                SignInEvent::RecoveryPhraseVerified(verified)
            },
        );
    }

    /// "Set new password" (DW-A49): the password the window already compared with its confirmation, for the session
    /// turn of this press. `Ok`: signed in and the vault is open (DW-A50).
    pub(super) fn recovery_begin_set_password(&mut self, password: &str) {
        let turn = self.engine.session_turn();
        let password = secret(password);
        self.ask(
            RECOVERY,
            SignInEvent::PasswordSet(Err(NewPasswordProblem::Other)),
            move |engine| SignInEvent::PasswordSet(password_set(engine.recovery_set_password_for(turn, password))),
        );
    }

    /// The meter under the new password: the engine's verdict, by sign-up's rules and the server's minimum. Local and
    /// at once (every key press): no job, no request.
    pub(super) fn recovery_strength(&self, password: &str) -> Option<PasswordStrength> {
        if password.is_empty() {
            return None;
        }
        let verdict = self.engine.recovery_password_verdict(password);
        Some(PasswordStrength {
            bars: verdict.level.clamp(1, 4),
            words: strength_words(
                verdict.level,
                verdict.meets_minimum,
                verdict.min_length,
                verdict.missing,
            ),
        })
    }
}

#[cfg(test)]
#[path = "signin_recovery_tests.rs"]
mod tests;
