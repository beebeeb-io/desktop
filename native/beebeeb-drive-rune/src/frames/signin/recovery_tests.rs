//! The recovery with the twelve words (DW-A47 to DW-A50): leaving its screens, and what each answer of the new
//! password says. The engine keeps a recovery between its two steps (the key the words derived, the server's recovery
//! token): leaving either screen has it forgotten at once (`discard_partial_sign_in`), never at the next verify or when
//! the engine is dropped. Every account, word and password is invented or the board's.

use std::cell::RefCell;
use std::rc::Rc;

use rune::Runtime;

use super::source::{FixtureSignInSource, Script, SignInEvent, SignInSource, board};
use super::tests::{click, runtime_of, ui};
use super::{Screen, Ui};
use crate::State;

type Calls = Rc<RefCell<Vec<&'static str>>>;

/// How many times the window asked the source to forget a half-done step.
fn discards(calls: &Calls) -> usize {
    calls
        .borrow()
        .iter()
        .filter(|call| **call == "discard_partial_sign_in")
        .count()
}

/// Recovery's first step (DW-A47) with the board's address and all twelve words typed, and the call list.
fn at_recover(script: Script) -> (Runtime<State>, Calls) {
    let source = FixtureSignInSource::new(script);
    let calls = source.calls.clone();
    let mut ui_ = Ui::new(Box::new(source));
    ui_.email.replace_text(board::EMAIL);
    ui_.forgot();
    ui_.recover_with_phrase();
    for (i, word) in board::WORDS.iter().enumerate() {
        ui_.words[i].replace_text(*word);
    }
    let rt = runtime_of(ui_, false);
    assert_eq!(
        rt.state.signin.as_ref().map(|ui| ui.screen.clone()),
        Some(Screen::Recover)
    );
    (rt, calls)
}

/// Row 3.6 (security). Mutant: drop `Screen::Recover | Screen::NewPassword` from `drop_steps`' partial screens; every
/// count below stays 0. The engine's half (the forget runs in the source's discard) is
/// `engine_host::signin::signin_recovery::tests::signin_recovery_discard_forgets_the_words`.
#[test]
fn leaving_the_recovery_has_the_engine_forget_the_recovery_it_holds() {
    let (mut rt, calls) = at_recover(Script::default());
    assert_eq!(discards(&calls), 0, "opening the recovery forgets nothing");

    // Back from the words.
    click(&mut rt, "back-link");
    assert_eq!(ui(&mut rt).screen, Screen::Forgot);
    assert_eq!(discards(&calls), 1, "Back from the words");

    // Again, the words verify, then Back from the new password.
    ui(&mut rt).recover_with_phrase();
    assert_eq!(
        discards(&calls),
        1,
        "entering the words from the options forgets nothing"
    );
    for (i, word) in board::WORDS.iter().enumerate() {
        ui(&mut rt).words[i].replace_text(*word);
    }
    ui(&mut rt).verify_phrase();
    rt.frame(0.0);
    assert_eq!(ui(&mut rt).screen, Screen::NewPassword);
    click(&mut rt, "back-link");
    assert_eq!(ui(&mut rt).screen, Screen::Recover);
    assert_eq!(discards(&calls), 2, "Back from the new password");

    // "I don't have it" from the words.
    ui(&mut rt).cannot_recover(true);
    assert_eq!(ui(&mut rt).screen, Screen::CannotRecover);
    assert_eq!(discards(&calls), 3, "the dead end from the words");
}

/// The fixture follows the real source: an answer of the recovery that is still on its way when the person leaves is
/// never handed out.
#[test]
fn an_answer_of_the_recovery_after_leaving_it_is_never_handed_out() {
    let mut source = FixtureSignInSource::new(Script {
        hold: true,
        ..Script::default()
    });
    source.begin_recovery_verify(board::EMAIL, &board::WORDS);
    source.begin_set_password("an invented new password");
    source.discard_partial_sign_in();
    source.release();
    assert_eq!(source.poll(), None);

    // The control: without the discard, both answers are handed out.
    let mut control = FixtureSignInSource::new(Script::default());
    control.begin_recovery_verify(board::EMAIL, &board::WORDS);
    control.begin_set_password("an invented new password");
    assert_eq!(control.poll(), Some(SignInEvent::RecoveryPhraseVerified(Ok(()))));
    assert_eq!(control.poll(), Some(SignInEvent::PasswordSet(Ok(()))));
}
