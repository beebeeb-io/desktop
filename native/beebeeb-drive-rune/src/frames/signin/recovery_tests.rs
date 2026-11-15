//! The recovery with the twelve words (DW-A47 to DW-A50): leaving its screens, and what each answer of the new
//! password says. The engine keeps a recovery between its two steps (the key the words derived, the server's recovery
//! token): leaving either screen has it forgotten at once (`discard_partial_sign_in`), never at the next verify or when
//! the engine is dropped. Every account, word and password is invented or the board's.

use std::cell::RefCell;
use std::rc::Rc;

use rune::Runtime;

use super::source::{FixtureSignInSource, NewPasswordProblem, Script, SignInEvent, SignInSource, board};
use super::tests::{click, runtime_of, ui};
use super::view::GENERIC_FAILURE;
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

/// The new password's screen (DW-A49) after the words verified, both fields filled in, and the call list.
fn at_new_password() -> (Runtime<State>, Calls) {
    let (mut rt, calls) = at_recover(Script::default());
    ui(&mut rt).verify_phrase();
    rt.frame(0.0);
    assert_eq!(ui(&mut rt).screen, Screen::NewPassword);
    let new = "x".repeat(board::PASSWORD_LEN);
    ui(&mut rt).password.replace_text(&new);
    ui(&mut rt).confirm.replace_text(&new);
    rt.frame(0.0);
    (rt, calls)
}

/// The engine answered the new password with `problem` (as `poll` hands it over while the step runs). Through
/// `Runtime::update`, so the window is drawn again from the new state.
fn answered(rt: &mut Runtime<State>, problem: NewPasswordProblem) {
    rt.update(|s, _| {
        let ui_ = s.signin.as_mut().expect("the sign-in window is open");
        ui_.setting = true;
        ui_.apply(SignInEvent::PasswordSet(Err(problem)));
    });
    rt.frame(0.0);
}

fn tree(rt: &Runtime<State>) -> String {
    rt.accessibility_json().to_string()
}

/// Press the control whose accessible name is `label`.
fn press_labelled(rt: &mut Runtime<State>, label: &str) {
    rt.settle_all();
    rt.frame(0.0);
    let r = rt
        .elements()
        .into_iter()
        .find(|e| e.label.as_deref() == Some(label) && !e.exiting)
        .unwrap_or_else(|| panic!("no control named {label}"))
        .rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

/// Row 3.5. Each answer of the new password is said in its own words, never the sign-in form's "We couldn't sign you
/// in. Nothing was unlocked." Mutant: `password_words` answers `None` for `TooShort` (its words are gone).
#[test]
fn every_answer_of_the_new_password_is_said_in_its_own_words() {
    let cases = [
        (
            NewPasswordProblem::TooShort,
            "Too short for your account. Nothing was changed",
        ),
        (
            NewPasswordProblem::Breached,
            "appears in known data breaches. Nothing was changed",
        ),
        (
            NewPasswordProblem::Offline,
            "couldn't check this password against known breaches. Try again. If your new password was already set",
        ),
        (
            NewPasswordProblem::RateLimited { seconds: 30 },
            "Too many tries. Wait a moment",
        ),
        (NewPasswordProblem::Ended, "This recovery has ended"),
        (
            NewPasswordProblem::Other,
            "That didn't finish, and nothing was signed in",
        ),
    ];
    for (problem, words) in cases {
        let (mut rt, _calls) = at_new_password();
        answered(&mut rt, problem);
        assert_eq!(ui(&mut rt).screen, Screen::NewPassword, "{problem:?}: the screen stays");
        let tree = tree(&rt);
        assert!(tree.contains(words), "{problem:?}: {words:?} is not said");
        assert!(!tree.contains(GENERIC_FAILURE), "{problem:?}: the sign-in form's words");
        let retry = problem != NewPasswordProblem::Ended;
        assert_eq!(ui(&mut rt).can_set_password(), retry, "{problem:?}: Set new password");
    }
}

/// An ended recovery sets nothing: "Set new password" is off, and "Sign in" leads to the form with the address kept.
#[test]
fn an_ended_recovery_offers_the_sign_in_and_sets_nothing() {
    let (mut rt, calls) = at_new_password();
    answered(&mut rt, NewPasswordProblem::Ended);
    ui(&mut rt).set_password();
    assert_eq!(
        calls.borrow().iter().filter(|call| **call == "set_password").count(),
        0,
        "nothing is sent for an ended recovery"
    );
    press_labelled(&mut rt, "Sign in");
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(ui(&mut rt).email.text, board::EMAIL);
    assert!(
        ui(&mut rt).password.text.is_empty(),
        "the new password is not the form's"
    );
}

/// Row 3.5: the two-factor account after recovery. The password WAS changed and nothing was signed in: the window says
/// so (never "Your vault is unlocked", never the form's failure) and its one button leads to the form. Mutant: the
/// `SignInRequired` arm in `apply_step` removed (it then stays on the new password as a problem).
#[test]
fn a_changed_password_with_a_sign_in_to_do_is_said_and_leads_to_the_form() {
    let (mut rt, _calls) = at_new_password();
    answered(&mut rt, NewPasswordProblem::SignInRequired);
    assert_eq!(ui(&mut rt).screen, Screen::PasswordChanged);
    assert!(ui(&mut rt).password.text.is_empty() && ui(&mut rt).confirm.text.is_empty());
    let tree = tree(&rt);
    assert!(tree.contains("Your password was changed. Sign in with your new password"));
    assert!(!tree.contains("Your vault is unlocked"), "nothing was unlocked");
    assert!(!tree.contains(GENERIC_FAILURE));
    assert!(!ui(&mut rt).can_set_password());

    press_labelled(&mut rt, "Sign in");
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(ui(&mut rt).email.text, board::EMAIL, "the address stays");
    assert!(ui(&mut rt).password.text.is_empty());
}
