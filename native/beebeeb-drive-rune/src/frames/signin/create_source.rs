//! Sign-up's part of [`super::source::SignInSource`] (DW-A31 to DW-A44): the answers, the
//! problems, and what the board's fixture says to each call.
//!
//! Where the secrets live. The password and the recovery phrase belong to the ENGINE for the
//! length of sign-up: the window hands the password over once (`begin_sign_up_password`, a
//! borrowed `&str`) and forgets it, and it never holds the phrase at all. The phrase is lent for
//! one draw (`with_sign_up_phrase`) and the three typed words are compared in the engine
//! (`begin_sign_up_confirm`), so none of these types has a secret for a field: every one of
//! them is `Debug` and `Clone`, and may be logged.

// The whole contract is written down, also the problems only the engine can answer with.
#![allow(dead_code)]

use super::source::board;

/// What a sent code allows (the server's sign-up policy; the client reads it, never hard-codes it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeSent {
    /// How many digits the code has: 8.
    pub digits: u8,
    /// The seconds until a new code may be asked for.
    pub resend_seconds: u32,
}

/// Why no code was sent (DW-A31). An address that already has an account is NOT one of these:
/// the reply is the same for any address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendProblem {
    RateLimited,
    Offline,
    /// The server does not take sign-ups from this app (yet): the web page is the way.
    WebOnly,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeProblem {
    /// DW-A33. One answer for a wrong code and an expired one.
    WrongOrExpired,
    /// The guess budget is spent: a new code is needed, and may be asked for at once.
    TooManyWrong,
    Offline,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordProblem {
    /// DW-A38: the password is known from a data breach.
    Breached,
    /// The breach check could not be reached; nobody is let through without it.
    CheckUnreachable,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateProblem {
    /// Registering failed before anything was stored: Try again.
    NothingStored,
    /// The account exists and this device was not finished: the only way on is Sign in.
    NotFinishedHere,
    /// The email code ran out meanwhile: back to Verify. Password and phrase are kept.
    CodeExpired,
}

/// The engine's verdict on a password being chosen, on every keystroke (core's evaluator, so
/// every client agrees). Its words hold numbers, never a character of the password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordVerdict {
    /// 1 to 4.
    pub bars: u8,
    /// "Strong.", "Needs at least 12 characters, 2 more."
    pub words: String,
    /// The grey guidance under the fields, while there is some (DW-A37).
    pub hint: Option<String>,
    /// The one rule that is enforced: the minimum length.
    pub long_enough: bool,
}

/// What a sign-up call answers with, later ([`super::source::SignInEvent::SignUp`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignUpEvent {
    /// The first code, and every new one. The same answer for any address (anti-enumeration).
    CodeSent(Result<CodeSent, SendProblem>),
    /// One second passed on the wait for a new code.
    ResendTick {
        seconds: u32,
    },
    Verified(Result<(), CodeProblem>),
    /// The breach check. On `Ok` the engine keeps the password for the account it will create.
    PasswordChecked(Result<(), PasswordProblem>),
    /// The three typed words, compared in the engine. It does not say which one was wrong.
    PhraseConfirmed(bool),
    Created(Result<(), CreateProblem>),
}

/// How many of the twelve words are asked back (policy `verify_word_count`).
pub const CONFIRM_WORDS: usize = 3;

/// The board's sample values of sign-up.
pub mod sample {
    /// The code the fixture accepts. DW-A32 draws its first four digits.
    pub const CODE: &str = "48291650";
    /// DW-A33's eight digits.
    pub const WRONG_CODE: &str = "48291651";
    pub const CODE_DIGITS: u8 = 8;
    /// "You can ask for a new code in 0:42".
    pub const RESEND_SECONDS: u32 = 42;
    /// Word 3, Word 7 and Word 11 (DW-A40), counted from zero.
    pub const POSITIONS: [usize; super::CONFIRM_WORDS] = [2, 6, 10];
    /// DW-A41's second word.
    pub const WRONG_WORD: &str = "meador";
    pub const MIN_PASSWORD: usize = 12;
    /// DW-A37: ten characters typed.
    pub const SHORT_PASSWORD_LEN: usize = 10;
    /// The words DW-A40 offers under "thun".
    pub const SUGGESTED: [&str; 2] = ["thunder", "thus"];
}

/// What the fixture answers to sign-up's calls. `None`: the step succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignUpScript {
    /// `false`: this source has no sign-up (what a source written before it answers).
    pub available: bool,
    pub send: Option<SendProblem>,
    pub password: Option<PasswordProblem>,
    pub create: Option<CreateProblem>,
    /// `false`: creating the account never answers (DW-A43, drawn while it runs).
    pub create_answers: bool,
}

impl Default for SignUpScript {
    fn default() -> Self {
        SignUpScript {
            available: true,
            send: None,
            password: None,
            create: None,
            create_answers: true,
        }
    }
}

impl SignUpScript {
    pub fn code_sent(&self) -> SignUpEvent {
        SignUpEvent::CodeSent(match self.send {
            Some(problem) => Err(problem),
            None => Ok(CodeSent {
                digits: sample::CODE_DIGITS,
                resend_seconds: sample::RESEND_SECONDS,
            }),
        })
    }

    pub fn verified(code: &str) -> SignUpEvent {
        SignUpEvent::Verified(if code == sample::CODE {
            Ok(())
        } else {
            Err(CodeProblem::WrongOrExpired)
        })
    }

    pub fn password_checked(&self) -> SignUpEvent {
        SignUpEvent::PasswordChecked(self.password.map_or(Ok(()), Err))
    }

    /// The board's words for its two drawings: too short (DW-A37) and long enough (DW-A36).
    pub fn verdict(password: &str) -> PasswordVerdict {
        let typed = password.chars().count();
        if typed < sample::MIN_PASSWORD {
            PasswordVerdict {
                bars: 1,
                words: format!(
                    "Needs at least {} characters, {} more.",
                    sample::MIN_PASSWORD,
                    sample::MIN_PASSWORD - typed
                ),
                hint: Some("Mix upper and lowercase, and add a number or symbol.".into()),
                long_enough: false,
            }
        } else {
            PasswordVerdict {
                bars: 4,
                words: "Strong.".into(),
                hint: None,
                long_enough: true,
            }
        }
    }

    pub fn confirmed(words: &[&str]) -> SignUpEvent {
        let right = words.len() == CONFIRM_WORDS
            && words
                .iter()
                .zip(sample::POSITIONS)
                .all(|(typed, at)| *typed == board::WORDS[at]);
        SignUpEvent::PhraseConfirmed(right)
    }

    pub fn created(&self) -> SignUpEvent {
        SignUpEvent::Created(self.create.map_or(Ok(()), Err))
    }

    /// The board draws "thunder" and "thus" under "thun", and under "thunder" too: the fixture
    /// offers them for anything that starts as they do. The engine's list is core's 2048 words.
    pub fn suggestions(prefix: &str) -> Vec<&'static str> {
        let starts_alike = |start: &str| sample::SUGGESTED.iter().any(|w| w.starts_with(start));
        if prefix.get(..2).is_some_and(starts_alike) {
            sample::SUGGESTED.to_vec()
        } else {
            Vec::new()
        }
    }
}
