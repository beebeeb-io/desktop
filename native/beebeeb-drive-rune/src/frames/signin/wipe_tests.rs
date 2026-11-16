//! Tests of the window's close during sign-in (`wipe.rs`): every typed secret goes, the email stays, a half-done step
//! that holds a proof in the engine is left as Back leaves it (the engine is told), and a step on its way, or the
//! twelve words of a session waiting for its key, stay where they are.

use rune::{Key, Runtime};

use super::source::{FixtureSignInSource, Script, board};
use super::tests::{click, press, runtime_of, ui};
use super::{PASSWORD_KEY, Screen, Ui};
use crate::State;
use crate::shell::app::GateState;

type Calls = std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>;

/// The sign-in form filled in, on the board's fixture, and its call list.
fn form() -> (Runtime<State>, Calls) {
    let source = FixtureSignInSource::new(Script::default());
    let calls = source.calls.clone();
    let mut ui_ = Ui::new(Box::new(source));
    ui_.email.replace_text(board::EMAIL);
    ui_.password.replace_text("x".repeat(board::PASSWORD_LEN));
    (runtime_of(ui_, false), calls)
}

/// The password was right and the code is due (the fixture's default answer).
fn at_the_code() -> (Runtime<State>, Calls) {
    let (mut rt, calls) = form();
    click(&mut rt, PASSWORD_KEY);
    press(&mut rt, Key::Enter);
    assert!(matches!(ui(&mut rt).screen, Screen::TwoFactor { .. }));
    (rt, calls)
}

fn close(rt: &mut Runtime<State>) {
    super::wipe::on_window_closed(&mut rt.state);
}

fn discards(calls: &Calls) -> usize {
    calls
        .borrow()
        .iter()
        .filter(|call| **call == "discard_partial_sign_in")
        .count()
}

fn nothing_typed(ui: &Ui) -> bool {
    ui.password.text.is_empty()
        && ui.confirm.text.is_empty()
        && ui.code.text.is_empty()
        && ui.backup_code.text.is_empty()
        && ui.words.iter().all(|word| word.text.is_empty())
}

#[test]
fn signin_wipe_closing_the_window_at_the_code_leaves_the_step_and_wipes_what_was_typed() {
    let (mut rt, calls) = at_the_code();
    press(&mut rt, Key::Text("48".into()));
    assert!(!ui(&mut rt).code.text.is_empty(), "two digits typed");
    close(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn, "left as Back leaves it");
    assert_eq!(
        discards(&calls),
        1,
        "the engine forgets the proven password's challenge"
    );
    assert!(nothing_typed(ui(&mut rt)));
    assert_eq!(ui(&mut rt).email.text, board::EMAIL, "the email stays");
}

#[test]
fn signin_wipe_closing_the_window_on_the_form_wipes_the_typed_password() {
    let (mut rt, calls) = form();
    close(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(
        nothing_typed(ui(&mut rt)),
        "the typed password does not wait in a hidden window"
    );
    assert_eq!(ui(&mut rt).email.text, board::EMAIL);
    assert_eq!(discards(&calls), 0, "nothing was half done");
}

#[test]
fn signin_wipe_closing_the_window_on_the_new_password_leaves_the_recovery() {
    let (mut rt, calls) = form();
    let ui_ = ui(&mut rt);
    ui_.screen = Screen::NewPassword;
    ui_.password.replace_text("Moss-trail-9xQ");
    ui_.confirm.replace_text("Moss-trail-9xQ");
    close(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(
        discards(&calls),
        1,
        "the engine forgets the verified words' key and the recovery token"
    );
    assert!(nothing_typed(ui(&mut rt)));
}

#[test]
fn signin_wipe_closing_the_window_on_the_twelve_words_wipes_them_and_keeps_the_session() {
    let (mut rt, calls) = form();
    assert!(ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
    for (word, typed) in ui(&mut rt).words.iter_mut().zip(board::WORDS) {
        word.replace_text(typed);
    }
    close(&mut rt);
    assert!(nothing_typed(ui(&mut rt)), "the typed words are wiped");
    assert_eq!(
        ui(&mut rt).screen,
        Screen::Phrase,
        "the session waiting for its key is the account's: it stays"
    );
    assert_eq!(discards(&calls), 0);
}

#[test]
fn signin_wipe_closing_the_window_under_a_running_step_leaves_it_to_its_answer() {
    let (mut rt, calls) = at_the_code();
    ui(&mut rt).verifying = true;
    close(&mut rt);
    assert!(matches!(ui(&mut rt).screen, Screen::TwoFactor { .. }));
    assert_eq!(discards(&calls), 0, "a step on its way is not left");
}
