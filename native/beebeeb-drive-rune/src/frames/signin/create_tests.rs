//! Tests of sign-up (DW-A31 to DW-A44): every frame's scene, the whole flow from the keyboard,
//! Back from every step, the engine's answers, and that the password, the recovery phrase, the
//! email code and the words typed back are in no tree a tool or assistive technology can read.

use rune::{Key, Runtime};

use super::create::{CODE_KEY, DEVICE_KEY, FRAME_IDS, Step, WORD_KEYS, scene};
use super::create_source::{
    CodeProblem, CreateProblem, PasswordProblem, SendProblem, SignUpEvent, SignUpScript, sample,
};
use super::source::{FixtureSignInSource, NextAfterSignIn, Script, SignInEvent, board};
use super::steps::CONFIRM_KEY;
use super::tests::{click, focused, has, info, press, readable, runtime, runtime_of, space, tab_to, ui};
use super::{EMAIL_KEY, Intent, PASSWORD_KEY, Screen, TITLE_KEY, Ui};
use crate::State;

const HEADINGS: [&str; 8] = [
    "Create your Beebeeb account",
    "Check your email",
    "Check your email",
    "Check your email",
    "Terms and privacy",
    "Set a password",
    "Set a password",
    "Set a password",
];

type Calls = std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>;

/// The words an element shows (a text) or is named by (a control).
fn said(rt: &Runtime<State>, key: &str) -> Option<String> {
    let element = info(rt, key);
    element.text.or(element.label)
}

fn type_(rt: &mut Runtime<State>, text: &str) {
    press(rt, Key::Text(text.into()));
}

fn at(rt: &mut Runtime<State>) -> Step {
    match ui(rt).screen {
        Screen::Create(step) => step,
        ref other => panic!("not a sign-up screen: {other:?}"),
    }
}

/// The first screen of a PC with no session, on a fixture that answers as `sign_up` says.
fn welcome(sign_up: SignUpScript) -> (Runtime<State>, Calls) {
    let source = FixtureSignInSource::new(Script {
        shared_session: false,
        sign_up,
        ..Script::default()
    });
    let calls = source.calls.clone();
    (runtime_of(Ui::new(Box::new(source)), false), calls)
}

/// What `shell::host::poll` does every frame with what the screens asked for.
fn shell_poll(rt: &mut Runtime<State>) {
    rt.update(|s, tx| {
        crate::shell::host::drain_signin_intents(s);
        super::poll(s, tx);
    });
    rt.frame(0.0);
}

/// "Create a Beebeeb account", pressed, and the shell's next poll.
fn started(sign_up: SignUpScript) -> (Runtime<State>, Calls) {
    let (mut rt, calls) = welcome(sign_up);
    click(&mut rt, "btn-0");
    shell_poll(&mut rt);
    (rt, calls)
}

/// As a person gets there: from the first screen, by keys, to `step`.
fn walked_to(step: Step, sign_up: SignUpScript) -> (Runtime<State>, Calls) {
    let (mut rt, calls) = started(sign_up);
    let order = [
        Step::Email,
        Step::Verify,
        Step::Terms,
        Step::Password,
        Step::Phrase,
        Step::Confirm,
        Step::Device,
    ];
    for now in order {
        if now == step {
            break;
        }
        match now {
            Step::Email => {
                click(&mut rt, EMAIL_KEY);
                type_(&mut rt, board::EMAIL);
                press(&mut rt, Key::Enter);
            }
            Step::Verify => {
                type_(&mut rt, sample::CODE);
                press(&mut rt, Key::Enter);
            }
            Step::Terms => {
                tab_to(&mut rt, "check-0-row");
                space(&mut rt);
                tab_to(&mut rt, "check-1-row");
                space(&mut rt);
                tab_to(&mut rt, "btn-0");
                press(&mut rt, Key::Enter);
            }
            Step::Password => {
                type_(&mut rt, &"x".repeat(board::PASSWORD_LEN));
                click(&mut rt, CONFIRM_KEY);
                type_(&mut rt, &"x".repeat(board::PASSWORD_LEN));
                press(&mut rt, Key::Enter);
            }
            Step::Phrase => {
                tab_to(&mut rt, "check-0-row");
                space(&mut rt);
                tab_to(&mut rt, "btn-3");
                press(&mut rt, Key::Enter);
            }
            Step::Confirm => {
                for (i, position) in sample::POSITIONS.iter().enumerate() {
                    click(&mut rt, WORD_KEYS[i]);
                    type_(&mut rt, board::WORDS[*position]);
                }
                press(&mut rt, Key::Enter);
            }
            _ => unreachable!(),
        }
        assert_ne!(at(&mut rt), now, "the keys did not leave {now:?}");
    }
    assert_eq!(at(&mut rt), step);
    (rt, calls)
}

#[test]
fn every_frame_has_a_scene_its_heading_and_fits_the_window_in_both_themes() {
    for (frame, heading) in FRAME_IDS.iter().zip(HEADINGS) {
        for dark in [false, true] {
            let rt = runtime_of(scene(frame).unwrap_or_else(|| panic!("no scene {frame}")), dark);
            assert_eq!(said(&rt, TITLE_KEY).as_deref(), Some(heading), "{frame}");
            let column = info(&rt, "au-col").rect;
            assert!(
                column[1] >= 0.0 && column[1] + column[3] <= 638.0,
                "{frame} is cut off: {column:?}"
            );
        }
    }
    assert_eq!(super::create::FRAMES.len(), 8);
}

#[test]
fn create_an_account_opens_sign_up_from_both_first_screens_and_back_returns_there() {
    let (mut rt, calls) = started(SignUpScript::default());
    assert_eq!(at(&mut rt), Step::Email);
    assert_eq!(
        focused(&rt).as_deref(),
        Some(EMAIL_KEY),
        "the email field has the keyboard"
    );
    assert!(ui(&mut rt).take_intents().is_empty(), "the shell took the intent");
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::Welcome);
    assert_eq!(*calls.borrow(), ["cancel_sign_up"]);

    // "Create an account" under the sign-in form (DW-A03): Back goes to the form.
    let mut rt = runtime("DW-A03");
    ui(&mut rt).password.replace_text("x".repeat(board::PASSWORD_LEN));
    tab_to(&mut rt, "link-2-box");
    press(&mut rt, Key::Enter);
    shell_poll(&mut rt);
    assert_eq!(at(&mut rt), Step::Email);
    assert!(
        ui(&mut rt).password.text.is_empty(),
        "the sign-in form's password is not the new account's"
    );
    assert_eq!(said(&rt, "back").as_deref(), Some("Sign in"));
    click(&mut rt, "back-link");
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
}

#[test]
fn a_source_without_sign_up_keeps_the_first_screen_and_makes_no_call() {
    let (mut rt, calls) = started(SignUpScript {
        available: false,
        ..SignUpScript::default()
    });
    assert_eq!(ui(&mut rt).screen, Screen::Welcome);
    assert!(calls.borrow().is_empty());
    assert!(!ui(&mut rt).start_create());
}

#[test]
fn the_whole_flow_runs_from_the_keyboard_and_ends_at_the_products_first_step() {
    let (mut rt, calls) = walked_to(Step::Device, SignUpScript::default());
    assert_eq!(
        ui(&mut rt).create.device.text,
        board::DEVICE,
        "prefilled with the system name"
    );
    assert_eq!(focused(&rt).as_deref(), Some(DEVICE_KEY));
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Ready);
    assert_eq!(focused(&rt).as_deref(), Some(TITLE_KEY), "the result is announced");
    let line = said(&rt, "subtitle").unwrap_or_default();
    assert!(
        line.starts_with(board::EMAIL) && line.ends_with("continue as Anna."),
        "{line}"
    );
    press(&mut rt, Key::Enter);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
    assert_eq!(
        *calls.borrow(),
        [
            "sign_up_code",
            "sign_up_verify",
            "sign_up_password",
            "sign_up_phrase_left",
            "sign_up_confirm",
            "sign_up_create"
        ]
    );
}

#[test]
fn back_and_escape_go_one_step_back_and_forget_what_was_typed() {
    let (mut rt, _) = walked_to(Step::Device, SignUpScript::default());
    press(&mut rt, Key::Escape);
    assert_eq!(at(&mut rt), Step::Confirm);
    assert!(ui(&mut rt).create.words.iter().all(|w| w.text.is_empty()));
    click(&mut rt, "back-link");
    assert_eq!(at(&mut rt), Step::Phrase);
    assert!(!ui(&mut rt).phrase_saved(), "the box is unticked again");
    press(&mut rt, Key::Escape);
    assert_eq!(at(&mut rt), Step::Password);
    assert!(ui(&mut rt).password.text.is_empty() && ui(&mut rt).confirm.text.is_empty());
    press(&mut rt, Key::Escape);
    assert_eq!(at(&mut rt), Step::Terms);
    press(&mut rt, Key::Escape);
    assert_eq!(
        at(&mut rt),
        Step::Email,
        "a verified code is spent: Back is a different email"
    );
    assert_eq!(
        ui(&mut rt).email.text,
        board::EMAIL,
        "the address stays to be corrected"
    );

    let (mut rt, _) = walked_to(Step::Verify, SignUpScript::default());
    type_(&mut rt, "4829");
    tab_to(&mut rt, "link-0-box");
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Email, "Use a different email");
    assert!(ui(&mut rt).create.code.text.is_empty());
}

#[test]
fn the_email_is_checked_when_it_is_sent_and_each_problem_has_the_boards_words() {
    let (mut rt, calls) = started(SignUpScript::default());
    type_(&mut rt, "anna");
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Email);
    assert_eq!(
        said(&rt, "field-0-msg").as_deref(),
        Some("Enter a valid email address.")
    );
    assert!(calls.borrow().is_empty(), "nothing was sent");
    type_(&mut rt, "@northwind.studio");
    assert_eq!(
        said(&rt, "field-0-msg").as_deref(),
        Some("We'll send an 8-digit code to this address."),
        "typing clears the message"
    );
    for (problem, words) in [
        (
            SendProblem::RateLimited,
            "Too many requests for this address. Try again later.",
        ),
        (
            SendProblem::Offline,
            "We could not send the email. Check your connection and try again.",
        ),
    ] {
        let (mut rt, _) = started(SignUpScript {
            send: Some(problem),
            ..SignUpScript::default()
        });
        type_(&mut rt, board::EMAIL);
        press(&mut rt, Key::Enter);
        assert_eq!(at(&mut rt), Step::Email);
        assert_eq!(said(&rt, "field-0-msg").as_deref(), Some(words));
    }
}

#[test]
fn the_code_takes_eight_digits_and_a_wrong_one_is_said_without_saying_which() {
    let (mut rt, calls) = walked_to(Step::Verify, SignUpScript::default());
    assert_eq!(focused(&rt).as_deref(), Some(CODE_KEY));
    assert_eq!(said(&rt, "field-0-label").as_deref(), Some("8-digit code"));
    type_(&mut rt, "48 29-16x51999");
    assert_eq!(
        ui(&mut rt).create.code.text,
        sample::WRONG_CODE,
        "digits only, eight at most"
    );
    let mut dumps = readable(&rt).to_vec();
    dumps.push(("Debug", format!("{:?}", ui(&mut rt).create.code)));
    for (name, dump) in &dumps {
        assert!(!dump.contains(sample::WRONG_CODE), "the code is in the {name}");
    }
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Verify);
    assert_eq!(
        said(&rt, "field-0-msg").as_deref(),
        Some("That code is not right, or it has expired.")
    );
    press(&mut rt, Key::Backspace);
    assert!(
        !has(&rt, "field-0-msg"),
        "the message was about the code that was tried"
    );
    press(&mut rt, Key::Enter);
    assert_eq!(
        calls.borrow().iter().filter(|c| **c == "sign_up_verify").count(),
        1,
        "seven digits verify nothing"
    );
}

#[test]
fn a_new_code_can_be_asked_for_when_the_countdown_ends_or_the_guesses_are_spent() {
    let (mut rt, calls) = walked_to(Step::Verify, SignUpScript::default());
    assert_eq!(said(&rt, "resend-time").as_deref(), Some("0:42"));
    type_(&mut rt, "4829");
    ui(&mut rt).resend_code();
    assert_eq!(
        calls.borrow().iter().filter(|c| **c == "sign_up_code").count(),
        1,
        "not before the countdown ends"
    );
    ui(&mut rt).apply_sign_up(SignUpEvent::ResendTick { seconds: 0 });
    rt.invalidate();
    rt.frame(0.0);
    assert!(!has(&rt, "resend-time"));
    tab_to(&mut rt, "link-0-box");
    assert_eq!(said(&rt, "link-0-box").as_deref(), Some("Send a new code"));
    press(&mut rt, Key::Enter);
    assert!(
        ui(&mut rt).create.code.text.is_empty(),
        "the field is cleared when a new code is sent"
    );
    assert_eq!(
        said(&rt, "note-0").as_deref(),
        Some("If this address can be used, a new email is on its way.")
    );
    assert_eq!(
        said(&rt, "resend-time").as_deref(),
        Some("0:42"),
        "the countdown restarted"
    );

    ui(&mut rt).create.code.replace_text(sample::CODE);
    ui(&mut rt).create.verifying = true;
    ui(&mut rt).apply_sign_up(SignUpEvent::Verified(Err(CodeProblem::TooManyWrong)));
    rt.invalidate();
    rt.frame(0.0);
    assert_eq!(
        said(&rt, "field-0-msg").as_deref(),
        Some("Too many wrong codes. Ask for a new one.")
    );
    assert!(
        !ui(&mut rt).can_verify_email() && ui(&mut rt).can_resend(),
        "a new code at once, no more guesses"
    );
}

#[test]
fn both_boxes_must_be_ticked_and_the_pages_open_outside_the_app() {
    let (mut rt, _) = walked_to(Step::Terms, SignUpScript::default());
    let boxes = [info(&rt, "check-0-row"), info(&rt, "check-1-row")];
    assert!(boxes.iter().all(|b| b.role == "checkbox"));
    assert_eq!(
        boxes[1].label.as_deref(),
        Some("I understand that Beebeeb cannot recover my account if I lose both my password and my recovery phrase.")
    );
    tab_to(&mut rt, "check-0-row");
    space(&mut rt);
    ui(&mut rt).accept_terms();
    assert_eq!(at(&mut rt), Step::Terms, "one box is not enough");
    tab_to(&mut rt, "item-0");
    press(&mut rt, Key::Enter);
    tab_to(&mut rt, "item-1");
    press(&mut rt, Key::Enter);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::OpenTerms, Intent::OpenPrivacy]);
    tab_to(&mut rt, "check-1-row");
    space(&mut rt);
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Password);
    assert_eq!(focused(&rt).as_deref(), Some(PASSWORD_KEY));
}

#[test]
fn the_password_needs_its_length_its_twin_and_the_breach_check() {
    let (mut rt, calls) = walked_to(
        Step::Password,
        SignUpScript {
            password: Some(PasswordProblem::Breached),
            ..SignUpScript::default()
        },
    );
    assert!(!has(&rt, "field-0-meter"), "no verdict on an empty field");
    type_(&mut rt, &"x".repeat(sample::SHORT_PASSWORD_LEN));
    assert_eq!(
        said(&rt, "field-0-meter").as_deref(),
        Some("Password strength: Needs at least 12 characters, 2 more.")
    );
    click(&mut rt, CONFIRM_KEY);
    type_(&mut rt, &"x".repeat(sample::SHORT_PASSWORD_LEN));
    assert!(!ui(&mut rt).can_set_new_password(), "too short, though the two match");
    click(&mut rt, PASSWORD_KEY);
    type_(&mut rt, "xxxx");
    assert!(!ui(&mut rt).can_set_new_password(), "long enough, and the two differ");
    click(&mut rt, CONFIRM_KEY);
    type_(&mut rt, "xxxy");
    assert_eq!(said(&rt, "field-1-msg").as_deref(), Some("Does not match yet."));
    press(&mut rt, Key::Backspace);
    type_(&mut rt, "x");
    assert!(ui(&mut rt).can_set_new_password());
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Password, "a known password is refused");
    assert_eq!(
        said(&rt, "field-0-msg").as_deref(),
        Some("This password appears in known data breaches. Choose a different one.")
    );
    assert_eq!(
        said(&rt, "field-0-meter").as_deref(),
        Some("Password strength: Strong in length, but it's known.")
    );
    assert!(
        !ui(&mut rt).can_set_new_password(),
        "Continue stays dimmed until the password changes"
    );
    assert_eq!(focused(&rt).as_deref(), Some(PASSWORD_KEY));
    type_(&mut rt, "z");
    assert!(!has(&rt, "field-0-msg"), "the error goes as soon as the text changes");
    assert_eq!(*calls.borrow().last().unwrap(), "sign_up_password");

    // No answer from the check: nobody is let through.
    ui(&mut rt).confirm.replace_text("x".repeat(board::PASSWORD_LEN) + "z");
    ui(&mut rt).create.checking = true;
    ui(&mut rt).apply_sign_up(SignUpEvent::PasswordChecked(Err(PasswordProblem::CheckUnreachable)));
    rt.invalidate();
    rt.frame(0.0);
    assert_eq!(at(&mut rt), Step::Password);
    assert!(
        said(&rt, "field-0-msg")
            .unwrap_or_default()
            .starts_with("We could not check this password")
    );
}

/// The password is typed into the group's one secure field, and is gone from this window the
/// moment the engine has it.
#[test]
fn the_password_is_in_no_tree_and_does_not_stay_behind_the_next_screens() {
    const SECRET: &str = "Vx7-quartz-lamp";
    let (mut rt, _) = walked_to(Step::Password, SignUpScript::default());
    type_(&mut rt, SECRET);
    click(&mut rt, CONFIRM_KEY);
    type_(&mut rt, SECRET);
    assert_eq!(ui(&mut rt).password.text, SECRET, "the field took the keys");
    for shown in [false, true] {
        ui(&mut rt).password.set_revealed(shown);
        rt.invalidate();
        rt.frame(0.0);
        let mut dumps = readable(&rt).to_vec();
        let state = ui(&mut rt);
        dumps.push(("Debug", format!("{:?}{:?}", state.password, state.confirm)));
        for (name, dump) in &dumps {
            assert!(!dump.contains(SECRET), "the password is in the {name} (shown: {shown})");
        }
    }
    ui(&mut rt).check_password();
    assert_eq!(at(&mut rt), Step::Phrase);
    assert!(ui(&mut rt).password.text.is_empty() && ui(&mut rt).confirm.text.is_empty());
    assert!(
        !ui(&mut rt).password.is_revealed(),
        "and the next password starts hidden"
    );
}

/// The phrase is drawn, and is in no element text, no label and no accessibility tree; the
/// window's state has no field that could hold it.
#[test]
fn the_shown_phrase_is_in_no_tree_and_no_label() {
    let (rt, _) = walked_to(Step::Phrase, SignUpScript::default());
    for i in 0..12 {
        let word = info(&rt, &format!("word-{i}-text-box"));
        assert_eq!((word.text, word.label), (None, None), "word {} is a secure node", i + 1);
    }
    let dumps = readable(&rt);
    for word in board::WORDS {
        for (name, dump) in &dumps {
            assert!(!dump.contains(word), "{word} is in the {name}");
        }
    }
    assert!(
        dumps[1].1.contains("Recovery phrase") && dumps[1].1.contains("Word 12"),
        "the list and its places have names"
    );
    // Left, the screen draws no word: the source is told, and the words are asked for again only there.
    let (mut rt, calls) = walked_to(Step::Confirm, SignUpScript::default());
    assert!(calls.borrow().contains(&"sign_up_phrase_left"));
    assert!(!has(&rt, "word-0-text-box"));
    let mut lent = false;
    ui(&mut rt).with_phrase(&mut |_| lent = true);
    assert!(!lent, "the phrase is lent on its own screen only");
}

#[test]
fn copy_puts_the_phrase_on_the_clipboard_until_sign_up_ends_and_save_and_print_carry_no_word() {
    let (mut rt, _) = walked_to(Step::Phrase, SignUpScript::default());
    assert!(ui(&mut rt).create.phrase_copy.is_none());
    tab_to(&mut rt, "btn-0");
    space(&mut rt);
    assert!(ui(&mut rt).create.phrase_copy.is_some(), "Copy copied");
    tab_to(&mut rt, "btn-1");
    space(&mut rt);
    tab_to(&mut rt, "btn-2");
    space(&mut rt);
    let intents = ui(&mut rt).take_intents();
    assert_eq!(intents, vec![Intent::SavePhraseAsFile, Intent::PrintPhrase]);
    let said = format!("{intents:?}");
    assert!(board::WORDS.iter().all(|w| !said.contains(w)));
    // Back out of sign-up altogether: the clipboard's copy is cleared with it.
    for _ in 0..4 {
        press(&mut rt, Key::Escape);
    }
    assert_eq!(ui(&mut rt).screen, Screen::Welcome);
    assert!(ui(&mut rt).create.phrase_copy.is_none());
}

#[test]
fn three_words_are_typed_back_into_named_secure_fields_and_a_suggestion_fills_one() {
    const WORD: &str = "xylophone";
    let (mut rt, _) = walked_to(Step::Confirm, SignUpScript::default());
    assert_eq!(focused(&rt).as_deref(), Some(WORD_KEYS[0]));
    assert_eq!(said(&rt, "field-2-label").as_deref(), Some("Word 11"));
    type_(&mut rt, "Xylo phone9");
    assert_eq!(ui(&mut rt).create.words[0].text, WORD, "lowercase letters only");
    let mut dumps = readable(&rt).to_vec();
    dumps.push(("Debug", format!("{:?}", ui(&mut rt).create.words)));
    for (name, dump) in &dumps {
        assert!(!dump.contains(WORD), "the word is in the {name}");
    }
    assert!(dumps[1].1.contains("Recovery word 3"), "the field has a name");
    assert!(
        !has(&rt, "chip-0"),
        "nothing is offered for a word the list does not know"
    );
    assert!(!ui(&mut rt).can_confirm_words());

    click(&mut rt, WORD_KEYS[2]);
    type_(&mut rt, "thun");
    let chip = info(&rt, "chip-0");
    assert_eq!((chip.role, chip.label.as_deref()), ("button", Some("Use suggestion 1")));
    for (name, dump) in &readable(&rt) {
        assert!(
            !dump.contains("thun"),
            "what is typed, or what completes it, is in the {name}"
        );
    }
    click(&mut rt, "chip-0");
    assert_eq!(ui(&mut rt).create.words[2].text, "thunder");
}

#[test]
fn words_that_do_not_match_are_said_without_saying_which_and_the_words_can_be_shown_again() {
    let (mut rt, calls) = walked_to(Step::Confirm, SignUpScript::default());
    for (i, word) in [board::WORDS[2], sample::WRONG_WORD, board::WORDS[10]]
        .iter()
        .enumerate()
    {
        click(&mut rt, WORD_KEYS[i]);
        type_(&mut rt, word);
    }
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Confirm);
    assert_eq!(info(&rt, "field-1-msg").role, "alert");
    assert_eq!(
        said(&rt, "field-1-msg").as_deref(),
        Some("Those words do not match your recovery phrase.")
    );
    assert!(!has(&rt, "field-0-msg") && !has(&rt, "field-2-msg"));
    // The same place whichever word is wrong.
    ui(&mut rt).create.words[1].replace_text(board::WORDS[6]);
    ui(&mut rt).create.words[2].replace_text("thus");
    ui(&mut rt).confirm_words();
    rt.invalidate();
    rt.frame(0.0);
    assert!(has(&rt, "field-1-msg") && !has(&rt, "field-2-msg"));
    tab_to(&mut rt, "link-0-box");
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Phrase);
    assert!(!ui(&mut rt).phrase_saved());
    assert!(has(&rt, "word-0-text-box"), "the words are on screen again");
    assert_eq!(calls.borrow().iter().filter(|c| **c == "sign_up_confirm").count(), 2);
}

#[test]
fn creating_has_no_way_back_and_each_failure_has_its_one_way_on() {
    let hold = SignUpScript {
        create_answers: false,
        ..SignUpScript::default()
    };
    let (mut rt, calls) = walked_to(Step::Device, hold);
    // An empty name is not allowed: it falls back to the system name.
    ui(&mut rt).create.device.replace_text("   ");
    press(&mut rt, Key::Enter);
    assert_eq!(at(&mut rt), Step::Creating);
    assert_eq!(ui(&mut rt).create.device.text, board::DEVICE);
    assert!(
        !has(&rt, "back-link") && !has(&rt, "btn-0"),
        "no Back and no Cancel while it runs"
    );
    press(&mut rt, Key::Escape);
    ui(&mut rt).create_account();
    assert_eq!(at(&mut rt), Step::Creating);
    assert_eq!(calls.borrow().iter().filter(|c| **c == "sign_up_create").count(), 1);

    let answer = |rt: &mut Runtime<State>, problem| {
        ui(rt).apply_sign_up(SignUpEvent::Created(Err(problem)));
        rt.invalidate();
        rt.frame(0.0);
    };
    answer(&mut rt, CreateProblem::NothingStored);
    assert_eq!(said(&rt, "btn-0").as_deref(), Some("Try again"));
    assert!(
        said(&rt, "banner-0-text")
            .unwrap_or_default()
            .contains("Nothing was stored.")
    );
    click(&mut rt, "btn-0");
    assert_eq!(calls.borrow().iter().filter(|c| **c == "sign_up_create").count(), 2);
    assert!(!has(&rt, "btn-0"), "running again");

    answer(&mut rt, CreateProblem::NotFinishedHere);
    assert_eq!(said(&rt, "btn-0").as_deref(), Some("Sign in"));
    assert!(
        !has(&rt, "btn-1"),
        "the only way forward is Sign in, never register again"
    );
    ui(&mut rt).create_account();
    assert_eq!(calls.borrow().iter().filter(|c| **c == "sign_up_create").count(), 2);
    click(&mut rt, "btn-0");
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(*calls.borrow().last().unwrap(), "cancel_sign_up");

    let (mut rt, _) = walked_to(Step::Device, hold);
    press(&mut rt, Key::Enter);
    answer(&mut rt, CreateProblem::CodeExpired);
    assert_eq!(at(&mut rt), Step::Verify);
    assert!(
        said(&rt, "field-0-msg")
            .unwrap_or_default()
            .ends_with("Your password and recovery phrase are kept.")
    );
    assert!(ui(&mut rt).can_resend(), "a new code can be asked for at once");
}

/// An answer is applied only by the wait it answers.
#[test]
fn an_answer_nobody_waits_for_changes_nothing() {
    let (mut rt, _) = walked_to(Step::Terms, SignUpScript::default());
    for event in [
        SignUpEvent::CodeSent(Err(SendProblem::Offline)),
        SignUpEvent::Verified(Err(CodeProblem::WrongOrExpired)),
        SignUpEvent::PasswordChecked(Ok(())),
        SignUpEvent::PhraseConfirmed(true),
        SignUpEvent::Created(Ok(())),
    ] {
        rt.update(|s, _| s.signin.as_mut().unwrap().apply(SignInEvent::SignUp(event.clone())));
        assert_eq!(at(&mut rt), Step::Terms, "{event:?}");
    }
    // Signed out of sign-up, a late answer of it opens nothing.
    press(&mut rt, Key::Escape);
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::Welcome);
    rt.update(|s, _| {
        s.signin
            .as_mut()
            .unwrap()
            .apply(SignInEvent::SignUp(SignUpEvent::Created(Ok(()))))
    });
    assert_eq!(ui(&mut rt).screen, Screen::Welcome);
}

#[test]
fn a_scenes_pictured_focus_goes_with_the_first_key_or_press() {
    let mut rt = runtime("DW-A31");
    assert!(has(&rt, "field-0-caret"));
    click(&mut rt, EMAIL_KEY);
    type_(&mut rt, "x");
    assert!(!has(&rt, "field-0-caret"), "a real field again");
    let mut rt = runtime("DW-A35");
    tab_to(&mut rt, "check-0-row");
    space(&mut rt);
    assert_eq!(
        ui(&mut rt).terms_accepted(),
        (false, true),
        "Space unticks the focused box"
    );
}

#[test]
fn only_the_password_and_phrase_steps_are_named_secret_bearing() {
    let secret: Vec<Step> = [
        Step::Email,
        Step::Verify,
        Step::Terms,
        Step::Password,
        Step::Phrase,
        Step::Confirm,
        Step::Device,
        Step::Creating,
        Step::Ready,
    ]
    .into_iter()
    .filter(|step| step.shows_secret())
    .collect();
    assert_eq!(secret, [Step::Password, Step::Phrase, Step::Confirm]);
}
