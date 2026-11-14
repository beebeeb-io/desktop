//! Tests of the steps after the password and of the honest exits (DW-A23 to DW-A30, DW-A45,
//! DW-A46, DW-A63): every control from the keyboard, names and roles, the engine's answers, and
//! that a code or a recovery word is in no tree a tool or assistive technology can read.

use rune::{Key, Runtime};

use super::source::{
    FixtureSignInSource, NextAfterSignIn, PasskeyOutcome, PhraseProblem, Script, SignInAnswer, TwoFactorProblem, board,
};
use super::steps::{BACKUP_KEY, CODE_KEY, CONFIRM_KEY, Pictured, WORD_KEYS};
use super::tests::{click, focused, has, info, press, readable, runtime, runtime_of, space, tab_to, ui};
use super::{EMAIL_KEY, FRAMES, Intent, PASSWORD_KEY, Screen, TITLE_KEY, Ui, scene};
use crate::State;
use crate::ui::kit;

const BUILT: [(&str, &str); 15] = [
    ("DW-A23", "Set up this device"),
    ("DW-A24", "Set up this device"),
    ("DW-A25", "We can't restore your keys"),
    ("DW-A26", "Two-factor authentication"),
    ("DW-A27", "Two-factor authentication"),
    ("DW-A28", "Two-factor authentication"),
    ("DW-A29", "Sign in to Drive"),
    ("DW-A30", "Unlock your vault"),
    ("DW-A45", "Trouble signing in?"),
    ("DW-A46", "We cannot recover your account"),
    ("DW-A47", "Recover with your phrase"),
    ("DW-A48", "Recover with your phrase"),
    ("DW-A49", "Set a new password"),
    ("DW-A50", "Password updated"),
    ("DW-A63", "Sign in to Drive"),
];

type Calls = std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>;

/// The sign-in form filled in, on a fixture that answers as `script` says, and its call list.
fn form(script: Script) -> (Runtime<State>, Calls) {
    let source = FixtureSignInSource::new(script);
    let calls = source.calls.clone();
    let mut ui_ = Ui::new(Box::new(source));
    ui_.email.replace_text(board::EMAIL);
    ui_.password.replace_text("x".repeat(board::PASSWORD_LEN));
    let rt = runtime_of(ui_, false);
    (rt, calls)
}

/// The password was right: Enter in its field, as a person signs in.
fn signed_in(script: Script) -> (Runtime<State>, Calls) {
    let (mut rt, calls) = form(script);
    click(&mut rt, PASSWORD_KEY);
    press(&mut rt, Key::Enter);
    (rt, calls)
}

fn type_(rt: &mut Runtime<State>, text: &str) {
    press(rt, Key::Text(text.into()));
}

fn a11y(rt: &Runtime<State>) -> String {
    rt.accessibility_json().to_string()
}

fn phrase_script() -> Script {
    Script {
        sign_in: SignInAnswer::Next(NextAfterSignIn::RecoveryPhrase),
        ..Script::default()
    }
}

#[test]
fn every_frame_has_a_scene_its_heading_and_fits_the_window_in_both_themes() {
    assert_eq!(FRAMES.len(), BUILT.len());
    for (frame, title) in BUILT {
        assert!(FRAMES.iter().any(|(id, _)| *id == frame), "{frame} is a scene");
        for dark in [false, true] {
            let rt = runtime_of(scene(frame).unwrap_or_else(|| panic!("no scene {frame}")), dark);
            assert_eq!(info(&rt, TITLE_KEY).text.as_deref(), Some(title), "{frame}");
            assert_eq!(info(&rt, TITLE_KEY).role, "heading", "{frame}");
            for e in rt.elements().iter().filter(|e| e.key.is_some() && !e.exiting) {
                let [x, y, w, h] = e.rect;
                assert!(
                    x >= -3.5 && y >= 0.0 && x + w <= 958.0 + 3.5 && y + h <= 638.0,
                    "{frame}: {:?} is cut off at {:?}",
                    e.key,
                    e.rect
                );
            }
        }
    }
}

#[test]
fn every_scene_keeps_the_products_secure_fields() {
    for (frame, _) in BUILT {
        let ui = scene(frame).expect("a scene");
        assert!(ui.code.is_secure(), "{frame}: the code's field is secure");
        assert!(ui.backup_code.is_secure(), "{frame}: the backup code's field is secure");
        assert!(
            ui.words.iter().all(|w| w.is_secure()),
            "{frame}: every word box is secure"
        );
        assert!(
            ui.password.is_secure() && !ui.password.is_revealed(),
            "{frame}: the password"
        );
        assert!(
            ui.confirm.is_secure() && !ui.confirm.is_revealed(),
            "{frame}: its confirmation"
        );
    }
    let running = super::start().expect("a state");
    assert!(running.code.is_secure() && running.backup_code.is_secure());
    assert!(running.words.iter().all(|w| w.is_secure()));
    assert_eq!(running.pictured(), None, "the running app never pictures a focus");
}

// ---------------------------------------------------------------------------
// Two-factor
// ---------------------------------------------------------------------------

#[test]
fn the_code_is_typed_into_one_named_input_and_checked_at_the_sixth_digit() {
    let (mut rt, calls) = signed_in(Script::default());
    assert_eq!(ui(&mut rt).screen, Screen::TwoFactor { backup: false });
    assert_eq!(
        focused(&rt).as_deref(),
        Some(CODE_KEY),
        "the code's input has the keyboard"
    );
    assert_eq!(info(&rt, CODE_KEY).label.as_deref(), Some("6-digit verification code"));
    assert!(ui(&mut rt).password.text.is_empty(), "the password did its work");
    type_(&mut rt, "4820");
    assert!(has(&rt, "digit-3-text-box") && !has(&rt, "digit-4-text-box"));
    assert_eq!(*calls.borrow(), ["sign_in"], "four digits ask nothing of the engine");
    assert!(!info(&rt, "btn-0").focusable, "Verify waits for six digits");
    type_(&mut rt, "1");
    type_(&mut rt, "9");
    assert_eq!(*calls.borrow(), ["sign_in", "two_factor"], "the sixth digit verifies");
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
    assert!(
        ui(&mut rt).code.text.is_empty(),
        "the code is gone once it did its work"
    );
}

#[test]
fn a_code_takes_digits_only_six_at_most_and_a_pasted_one() {
    let (mut rt, calls) = signed_in(Script {
        hold: false,
        two_factor_problem: Some(TwoFactorProblem::WrongCode),
        ..Script::default()
    });
    type_(&mut rt, "4a 8-");
    assert_eq!(ui(&mut rt).code.text, "48", "letters, spaces and signs are not kept");
    rt.paste("201 955");
    rt.frame(0.0);
    assert_eq!(ui(&mut rt).code.text, "482019", "six at most");
    assert_eq!(*calls.borrow(), ["sign_in", "two_factor"], "a pasted code is checked");
}

#[test]
fn a_wrong_code_is_an_alert_and_the_next_keystroke_starts_over() {
    let (mut rt, calls) = signed_in(Script {
        two_factor_problem: Some(TwoFactorProblem::WrongCode),
        ..Script::default()
    });
    type_(&mut rt, board::TOTP);
    assert_eq!(ui(&mut rt).code_problem(), Some(TwoFactorProblem::WrongCode));
    assert_eq!(info(&rt, "banner-0").role, "alert");
    assert_eq!(
        info(&rt, "banner-0-title").text.as_deref(),
        Some("That code isn't right")
    );
    assert!(ui(&mut rt).take_intents().is_empty(), "nothing was signed in");
    assert_eq!(focused(&rt).as_deref(), Some(CODE_KEY), "the input keeps the keyboard");
    // Verify is there for the same six digits (the board draws it at full strength).
    tab_to(&mut rt, "btn-0");
    space(&mut rt);
    assert_eq!(calls.borrow().iter().filter(|c| **c == "two_factor").count(), 2);
    // "The boxes clear on the next keystroke."
    click(&mut rt, CODE_KEY);
    type_(&mut rt, "7");
    assert_eq!(ui(&mut rt).code.text, "7");
    assert_eq!(ui(&mut rt).code_problem(), None);
    assert!(!has(&rt, "banner-0"));
}

#[test]
fn the_backup_code_has_its_own_field_and_button_and_the_way_back() {
    let (mut rt, calls) = signed_in(Script::default());
    tab_to(&mut rt, "link-0-box");
    press(&mut rt, Key::Enter);
    assert_eq!(ui(&mut rt).screen, Screen::TwoFactor { backup: true });
    assert_eq!(focused(&rt).as_deref(), Some(BACKUP_KEY));
    assert_eq!(info(&rt, BACKUP_KEY).label.as_deref(), Some("Backup code"));
    type_(&mut rt, "4820-13");
    assert_eq!(ui(&mut rt).backup_code.text, "482013");
    assert!(
        !info(&rt, "btn-0").focusable,
        "Verify backup code waits for eight digits"
    );
    type_(&mut rt, "95");
    assert_eq!(*calls.borrow(), ["sign_in"], "a backup code is checked by its button");
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["sign_in", "two_factor"]);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);

    let (mut rt, _) = signed_in(Script::default());
    click(&mut rt, "link-0-box");
    type_(&mut rt, "12");
    tab_to(&mut rt, "link-0-box");
    space(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::TwoFactor { backup: false });
    assert!(ui(&mut rt).backup_code.text.is_empty(), "what was typed is forgotten");
    assert_eq!(focused(&rt).as_deref(), Some(CODE_KEY));
}

#[test]
fn back_from_the_code_discards_the_partial_sign_in_and_a_late_answer_changes_nothing() {
    let (mut rt, calls) = signed_in(Script::default());
    type_(&mut rt, "48");
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(ui(&mut rt).code.text.is_empty());
    assert_eq!(*calls.borrow(), ["sign_in", "discard_partial_sign_in"]);

    // An answer of a check that was left behind: the screen it would open is not opened.
    let mut ui_ = scene("DW-A26").expect("a scene");
    ui_.screen = Screen::SignIn;
    ui_.apply(super::source::SignInEvent::TwoFactor(Ok(NextAfterSignIn::Drive)));
    ui_.apply(super::source::SignInEvent::PhraseRestored(Ok(NextAfterSignIn::Drive)));
    ui_.apply(super::source::SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::Drive)));
    ui_.apply(super::source::SignInEvent::Passkey(PasskeyOutcome::NeedsPassword));
    ui_.apply(super::source::SignInEvent::RecoveryPhraseVerified(Ok(())));
    ui_.apply(super::source::SignInEvent::PasswordSet(Ok(())));
    assert!(ui_.take_intents().is_empty());
    assert_eq!(ui_.screen, Screen::SignIn);
}

// ---------------------------------------------------------------------------
// The twelve words
// ---------------------------------------------------------------------------

#[test]
fn twelve_named_boxes_take_lowercase_letters_and_tab_moves_word_to_word() {
    let (mut rt, calls) = signed_in(phrase_script());
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
    assert_eq!(focused(&rt).as_deref(), Some(WORD_KEYS[0]));
    for (i, key) in WORD_KEYS.iter().enumerate() {
        assert_eq!(info(&rt, key).label, Some(format!("Recovery word {}", i + 1)));
    }
    type_(&mut rt, "Har-bor 1");
    assert_eq!(ui(&mut rt).words[0].text, "harbor", "lowercased, letters only");
    press(&mut rt, Key::Tab);
    assert_eq!(focused(&rt).as_deref(), Some(WORD_KEYS[1]));
    assert!(!info(&rt, "btn-0").focusable, "Restore keys waits for twelve words");
    for (i, word) in board::WORDS.iter().enumerate().skip(1) {
        tab_to(&mut rt, WORD_KEYS[i]);
        type_(&mut rt, word);
    }
    assert!(ui(&mut rt).can_restore());
    assert_eq!(*calls.borrow(), ["sign_in"]);
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["sign_in", "restore"]);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
    assert!(
        ui(&mut rt).words.iter().all(|w| w.text.is_empty()),
        "the words are gone once they did their work"
    );
}

#[test]
fn twelve_words_pasted_into_any_box_fill_them_all() {
    let (mut rt, _) = signed_in(phrase_script());
    click(&mut rt, WORD_KEYS[4]);
    rt.paste(format!("  {}\n", board::WORDS.join("  ").to_uppercase()));
    rt.frame(0.0);
    for (i, word) in board::WORDS.iter().enumerate() {
        assert_eq!(ui(&mut rt).words[i].text, *word, "box {}", i + 1);
    }
    assert_eq!(focused(&rt).as_deref(), Some(WORD_KEYS[11]));
    // One word pasted is the box's own paste.
    click(&mut rt, WORD_KEYS[2]);
    assert!(!ui(&mut rt).paste_words("quiet"));
}

#[test]
fn a_wrong_phrase_is_an_alert_and_only_a_word_that_is_not_in_the_list_is_marked() {
    let mut rt = runtime("DW-A24");
    assert_eq!(info(&rt, "banner-0").role, "alert");
    assert_eq!(
        ui(&mut rt).phrase_problem(),
        Some(&PhraseProblem::WrongPhrase { bad_word: Some(2) })
    );
    let tree = a11y(&rt);
    assert_eq!(tree.matches("not a recovery word").count(), 1, "one box is announced");
    assert!(ui(&mut rt).take_intents().is_empty(), "nothing was restored");
    // Typing in a box takes the mark away: it was about the phrase that was tried.
    click(&mut rt, WORD_KEYS[2]);
    type_(&mut rt, "t");
    assert_eq!(ui(&mut rt).phrase_problem(), None);
    assert!(!a11y(&rt).contains("not a recovery word"));
}

#[test]
fn the_lost_phrase_is_reachable_from_the_keyboard_and_no_other_device_door_is_drawn() {
    let (mut rt, _) = signed_in(phrase_script());
    assert!(
        !a11y(&rt).contains("another device"),
        "approving from another device is the web's: no door to it here"
    );
    let (mut rt, _) = signed_in(phrase_script());
    tab_to(&mut rt, "link-0-box");
    space(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::NoPhrase);
    assert_eq!(focused(&rt).as_deref(), Some(TITLE_KEY), "focus moves to the heading");
    // The three exits are buttons; the destructive ones say so.
    for (row, name, intent) in [
        ("item-0", "Cancel billing", Intent::CancelBilling),
        ("item-1", "Erase all files", Intent::EraseAllFiles),
        ("item-2", "Delete the account", Intent::DeleteAccount),
    ] {
        assert_eq!(info(&rt, row).role, "button");
        assert_eq!(info(&rt, row).label.as_deref(), Some(name));
        tab_to(&mut rt, row);
        press(&mut rt, Key::Enter);
        assert_eq!(ui(&mut rt).take_intents(), vec![intent]);
    }
    assert_eq!(a11y(&rt).matches("Destructive.").count(), 2);
    tab_to(&mut rt, "btn-0");
    space(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::Phrase, "I found my phrase");
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

/// A code and recovery words typed into the PRODUCT's fields (`Ui::new`) are in no element
/// text, no label, no accessibility tree and no `Debug`, although the screen shows them as
/// typed. The same search finds what is not secret (a box's name), so it can find.
#[test]
fn a_typed_code_and_typed_words_are_in_no_tree_and_no_label() {
    const DIGITS: &str = "9135";
    const WORD: &str = "xylophone";
    let (mut rt, _) = signed_in(Script::default());
    type_(&mut rt, DIGITS);
    assert_eq!(ui(&mut rt).code.text, DIGITS, "the field took the keys");
    assert!(has(&rt, "digit-3-text-box"), "and the boxes show them");
    let mut dumps = readable(&rt).to_vec();
    dumps.push(("Debug", format!("{:?}", ui(&mut rt).code)));
    for (name, dump) in &dumps {
        assert!(!dump.contains(DIGITS), "the code is in the {name}");
    }
    for i in 0..4 {
        let digit = info(&rt, &format!("digit-{i}-text-box"));
        assert_eq!((digit.text, digit.label), (None, None), "digit {i} is a secure node");
    }
    assert!(
        dumps[1].1.contains("6-digit verification code"),
        "the search finds a name"
    );

    click(&mut rt, "link-0-box");
    type_(&mut rt, "77120934");
    let mut dumps = readable(&rt).to_vec();
    dumps.push(("Debug", format!("{:?}", ui(&mut rt).backup_code)));
    for (name, dump) in &dumps {
        assert!(!dump.contains("77120934"), "the backup code is in the {name}");
    }

    let (mut rt, _) = signed_in(phrase_script());
    type_(&mut rt, WORD);
    assert_eq!(ui(&mut rt).words[0].text, WORD);
    let mut dumps = readable(&rt).to_vec();
    dumps.push(("Debug", format!("{:?}", ui(&mut rt).words)));
    for (name, dump) in &dumps {
        assert!(!dump.contains(WORD), "the word is in the {name}");
    }
    assert!(dumps[1].1.contains("Recovery word 1"), "the search finds a name");
    assert_eq!(info(&rt, WORD_KEYS[0]).text, None, "a secure field reports no text");
}

/// The frame gate is told a shown code or word only while its export records (a fixture
/// scene); a view that runs in the app notes nothing.
#[test]
fn the_gate_is_told_shown_secrets_only_while_it_records() {
    let rt = runtime("DW-A24");
    assert!(kit::take().is_empty(), "no note outside a gate export");
    drop(rt);
    kit::record();
    let _rt = runtime("DW-A24");
    let notes = kit::take();
    assert_eq!(
        notes.get("word-2-text").and_then(|n| n.text.as_deref()),
        Some(board::WRONG_WORD),
        "the export of the board's fixture is"
    );
}

// ---------------------------------------------------------------------------
// A passkey
// ---------------------------------------------------------------------------

#[test]
fn a_passkey_asks_windows_once_and_its_answers_are_frames() {
    // Windows' dialog is up: the form waits, and nothing else starts.
    let (mut rt, calls) = form(Script {
        hold: true,
        ..Script::default()
    });
    tab_to(&mut rt, "link-1-box");
    press(&mut rt, Key::Enter);
    assert!(ui(&mut rt).passkey_waiting());
    click(&mut rt, "link-1-box");
    click(&mut rt, "link-0-box");
    click(&mut rt, "btn-0");
    click(&mut rt, "btn-1");
    assert_eq!(
        *calls.borrow(),
        ["passkey"],
        "one dialog, and no other way in beside it"
    );
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);

    // Cancel: back to the form, unchanged.
    let (mut rt, _) = form(Script {
        passkey: PasskeyOutcome::Cancelled,
        ..Script::default()
    });
    click(&mut rt, "link-1-box");
    assert!(!ui(&mut rt).passkey_waiting());
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(ui(&mut rt).problem, None);

    // The passkey carries the unlock key: on to the product.
    let (mut rt, _) = form(Script {
        passkey: PasskeyOutcome::Succeeded(NextAfterSignIn::Drive),
        ..Script::default()
    });
    click(&mut rt, "link-1-box");
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
}

#[test]
fn after_a_passkey_the_vault_password_is_asked_with_focus_in_its_field() {
    let (mut rt, calls) = form(Script::default());
    click(&mut rt, "link-1-box");
    assert_eq!(ui(&mut rt).screen, Screen::UnlockVault);
    assert_eq!(
        focused(&rt).as_deref(),
        Some(PASSWORD_KEY),
        "focus lands on the password field"
    );
    assert!(
        ui(&mut rt).password.text.is_empty(),
        "the form's password is not this step's"
    );
    assert_eq!(info(&rt, "banner-0").role, "status");
    assert!(!info(&rt, "btn-0").focusable, "Unlock waits for a password");
    type_(&mut rt, "short");
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["passkey", "unlock"]);
    assert!(ui(&mut rt).unlock_problem().is_some());
    assert_eq!(info(&rt, "field-0-msg").role, "alert");
    assert!(ui(&mut rt).take_intents().is_empty(), "nothing was unlocked");
    assert_eq!(focused(&rt).as_deref(), Some(PASSWORD_KEY));
    press(&mut rt, Key::Text("x".repeat(board::PASSWORD_LEN - 5).into()));
    press(&mut rt, Key::Enter);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
    assert!(ui(&mut rt).password.text.is_empty());

    let (mut rt, calls) = form(Script::default());
    click(&mut rt, "link-1-box");
    type_(&mut rt, "abc");
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(
        ui(&mut rt).password.text.is_empty(),
        "a password typed to unlock does not stay"
    );
    assert_eq!(*calls.borrow(), ["passkey", "discard_partial_sign_in"]);
}

// ---------------------------------------------------------------------------
// Forgot password, the deleted account
// ---------------------------------------------------------------------------

#[test]
fn forgot_password_offers_the_web_page_and_the_honest_exit_from_the_keyboard() {
    let (mut rt, calls) = form(Script::default());
    click(&mut rt, "link-0-box");
    assert_eq!(ui(&mut rt).screen, Screen::Forgot);
    assert_eq!(focused(&rt).as_deref(), Some(TITLE_KEY));
    let tree = a11y(&rt).to_lowercase();
    assert!(!tree.contains("reset"), "the copy never says reset");
    assert!(
        !tree.contains("sign in with another device"),
        "no door to approving from another device"
    );
    for (card, title) in [
        ("card-0", "I have my recovery phrase or a signed-in device"),
        ("card-1", "I have neither"),
    ] {
        assert_eq!(info(&rt, card).label.as_deref(), Some(title));
    }
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Enter);
    assert_eq!(
        ui(&mut rt).take_intents(),
        vec![Intent::OpenRecoveryPage],
        "the web's recovery page opens"
    );
    assert_eq!(ui(&mut rt).screen, Screen::Forgot, "the window stays where it was");
    tab_to(&mut rt, "btn-1");
    space(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::CannotRecover);
    assert_eq!(
        info(&rt, TITLE_KEY).text.as_deref(),
        Some("We cannot recover your account")
    );
    tab_to(&mut rt, "link-0-box");
    assert_eq!(info(&rt, "link-0-box").role, "link");
    press(&mut rt, Key::Enter);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::WriteToSupport]);
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Enter);
    assert_eq!(ui(&mut rt).screen, Screen::Forgot, "Back to recovery options");
    click(&mut rt, "btn-1");
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::Forgot, "Escape is the back link");
    assert!(calls.borrow().is_empty(), "nothing was asked of the engine");
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
}

#[test]
fn a_deleted_account_is_said_once_under_the_password_and_only_after_it_was_right() {
    let (mut rt, calls) = signed_in(Script {
        sign_in: SignInAnswer::AccountDeleted,
        ..Script::default()
    });
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(ui(&mut rt).deleted());
    assert_eq!(
        info(&rt, "field-1-msg").text.as_deref(),
        Some("This account has been deleted. We can't recover it.")
    );
    assert_eq!(info(&rt, "field-1-msg").role, "alert");
    assert_eq!(
        focused(&rt).as_deref(),
        Some(PASSWORD_KEY),
        "the field with the problem keeps focus"
    );
    assert!(ui(&mut rt).take_intents().is_empty());
    assert!(
        !a11y(&rt).to_lowercase().contains("restore"),
        "there is no restore action"
    );
    // Sign in tries again; the message is not kept for the next answer.
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["sign_in", "sign_in"]);
    // A wrong password gets the general failure, never this message.
    let (mut rt, _) = signed_in(Script {
        sign_in: SignInAnswer::Problem(super::source::SignInProblem::Invalid),
        ..Script::default()
    });
    assert!(!ui(&mut rt).deleted());
    assert!(!a11y(&rt).contains("deleted"));
}

// ---------------------------------------------------------------------------
// The pictured focus of a scene
// ---------------------------------------------------------------------------

#[test]
fn a_scenes_pictured_focus_goes_with_the_first_key_or_press() {
    for (frame, pictured, caret) in [
        ("DW-A23", Pictured::Word(5), "word-5-caret"),
        ("DW-A28", Pictured::BackupCode, "field-0-caret"),
        ("DW-A30", Pictured::VaultPassword, "field-0-caret"),
        ("DW-A47", Pictured::Word(8), "word-8-caret"),
        ("DW-A49", Pictured::NewPassword, "field-0-caret"),
    ] {
        let mut rt = runtime(frame);
        assert_eq!(ui(&mut rt).pictured(), Some(pictured), "{frame}");
        assert!(has(&rt, caret), "{frame} draws the board's caret");
        let field = match pictured {
            Pictured::Word(i) => WORD_KEYS[i],
            Pictured::BackupCode => BACKUP_KEY,
            Pictured::VaultPassword | Pictured::NewPassword => PASSWORD_KEY,
        };
        click(&mut rt, field);
        type_(&mut rt, "a1");
        assert_eq!(ui(&mut rt).pictured(), None, "{frame}: typing clears it");
        assert!(!has(&rt, caret), "{frame}: Rune's own caret is the only one");
    }
    let mut rt = runtime("DW-A23");
    click(&mut rt, "back-link");
    assert_eq!(ui(&mut rt).pictured(), None, "a press clears it");
}

// ---------------------------------------------------------------------------
// Recovery with the phrase
// ---------------------------------------------------------------------------

#[test]
fn recovery_proves_the_phrase_then_sets_a_new_password_from_the_keyboard() {
    const NEW: &str = "Moss-trail-9xQ";
    assert_eq!(NEW.chars().count(), board::PASSWORD_LEN);
    let (mut rt, calls) = form(Script::default());
    click(&mut rt, "link-0-box");
    // The twelve-word recovery screens are kept but have no door in this build (recovery is the web's).
    ui(&mut rt).recover_with_phrase();
    rt.invalidate();
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(ui(&mut rt).screen, Screen::Recover);
    assert_eq!(info(&rt, EMAIL_KEY).label.as_deref(), Some("Email"));
    assert_eq!(
        focused(&rt).as_deref(),
        Some(WORD_KEYS[0]),
        "the email is there: the first word is next"
    );
    assert!(!info(&rt, "btn-0").focusable, "Verify phrase waits for twelve words");
    // "Enter moves to the next word."
    type_(&mut rt, board::WORDS[0]);
    press(&mut rt, Key::Enter);
    assert_eq!(focused(&rt).as_deref(), Some(WORD_KEYS[1]));
    assert!(calls.borrow().is_empty(), "and asks nothing of the engine");
    rt.paste(board::WORDS.join(" "));
    rt.frame(0.0);
    assert!(ui(&mut rt).can_verify_phrase());
    assert_eq!(focused(&rt).as_deref(), Some(WORD_KEYS[11]));
    // In the last box Enter is the screen's button.
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["recovery_verify"]);
    assert_eq!(ui(&mut rt).screen, Screen::NewPassword);
    assert!(
        ui(&mut rt).words.iter().all(|w| w.text.is_empty()),
        "the words did their work"
    );
    assert_eq!(focused(&rt).as_deref(), Some(PASSWORD_KEY));
    assert!(!has(&rt, "field-0-strength"), "no verdict before a password is typed");

    type_(&mut rt, NEW);
    assert_eq!(info(&rt, "field-0-strength").text.as_deref(), Some("Strong."));
    assert_eq!(
        info(&rt, "field-0-meter").label.as_deref(),
        Some("Password strength: Strong.")
    );
    assert!(
        !info(&rt, "btn-0").focusable,
        "Set new password waits for the confirmation"
    );
    tab_to(&mut rt, CONFIRM_KEY);
    assert_eq!(info(&rt, CONFIRM_KEY).label.as_deref(), Some("Confirm password"));
    type_(&mut rt, "Moss-trail-9x");
    assert!(!ui(&mut rt).can_set_password(), "the two differ");
    type_(&mut rt, "Q");
    // Neither password is in a tree, typed into the product's own fields.
    let mut dumps = readable(&rt).to_vec();
    dumps.push(("Debug", {
        let ui = ui(&mut rt);
        format!("{:?}{:?}", ui.password, ui.confirm)
    }));
    for (name, dump) in &dumps {
        assert!(!dump.contains("trail"), "the new password is in the {name}");
    }
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["recovery_verify", "set_password"]);
    assert_eq!(ui(&mut rt).screen, Screen::PasswordUpdated);
    assert_eq!(
        focused(&rt).as_deref(),
        Some(TITLE_KEY),
        "success is announced: focus is on the heading"
    );
    assert!(ui(&mut rt).password.text.is_empty() && ui(&mut rt).confirm.text.is_empty());
    press(&mut rt, Key::Enter);
    assert_eq!(
        ui(&mut rt).take_intents(),
        vec![Intent::Next(NextAfterSignIn::Drive)],
        "Continue to Drive"
    );
}

#[test]
fn a_phrase_that_does_not_verify_changes_nothing_and_offers_the_dead_end() {
    let mut rt = runtime("DW-A48");
    assert_eq!(ui(&mut rt).screen, Screen::Recover);
    assert_eq!(info(&rt, "banner-0").role, "alert");
    assert_eq!(a11y(&rt).matches("not a recovery word").count(), 1);
    assert!(a11y(&rt).contains("your password is as it was"));
    tab_to(&mut rt, "btn-1");
    assert_eq!(info(&rt, "btn-1").label.as_deref(), Some("I don't have it"));
    space(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::CannotRecover);
    assert!(
        ui(&mut rt).words.iter().all(|w| w.text.is_empty()),
        "the words are forgotten"
    );
    // Back from a new password is recovery's first step, with nothing typed left behind.
    let mut rt = runtime("DW-A49");
    click(&mut rt, "back-link");
    assert_eq!(ui(&mut rt).screen, Screen::Recover);
    assert!(ui(&mut rt).password.text.is_empty() && ui(&mut rt).confirm.text.is_empty());
}

/// The session's gate says "signed in, no key on this computer" (a sign-in whose answer is still on its way, or
/// a start with such a session): the twelve words come in front, what was typed is forgotten, and the session is
/// NOT discarded (it is why the screen shows). Any other gate, and a screen already there, change nothing.
#[test]
fn the_gate_that_needs_the_phrase_puts_the_twelve_words_in_front_and_discards_nothing() {
    use crate::shell::app::GateState;
    let (mut rt, calls) = form(Script::default());
    // The first gate seen, signed out, and the same gate again: a window that is on its form stays as it is.
    for _ in 0..2 {
        assert!(!ui(&mut rt).follow_gate(GateState::SignedOut));
    }
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(!ui(&mut rt).password.text.is_empty(), "the form is as it was");

    ui(&mut rt).busy = true;
    assert!(ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
    assert!(
        ui(&mut rt).password.text.is_empty(),
        "the password does not stay behind the words"
    );
    assert!(!ui(&mut rt).busy);
    assert_eq!(
        calls
            .borrow()
            .iter()
            .filter(|call| **call == "discard_partial_sign_in")
            .count(),
        0,
        "the session is kept"
    );

    // Already there: the words being typed stay, and "I've lost my recovery phrase" stays open.
    ui(&mut rt).words[0].replace_text("harbor");
    assert!(!ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(ui(&mut rt).words[0].text, "harbor");
    ui(&mut rt).lost_phrase(true);
    assert!(!ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(ui(&mut rt).screen, Screen::NoPhrase);
}

/// Back from the twelve words asks the engine to end the session, which takes a round trip: while the gate still
/// says "needs the phrase" the form stays in front (the words are not forced back), and a later arrival at that
/// gate (the next sign-in) shows the words again.
#[test]
fn going_back_from_the_words_stays_on_the_form_while_the_gate_has_not_moved_yet() {
    use crate::shell::app::GateState;
    let (mut rt, calls) = form(Script::default());
    assert!(ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    ui(&mut rt).open_sign_in();
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(
        *calls.borrow(),
        ["discard_partial_sign_in"],
        "the engine is asked to end it"
    );
    for _ in 0..3 {
        assert!(!ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
        assert_eq!(ui(&mut rt).screen, Screen::SignIn, "not bounced back to the words");
    }
    // The sign-out landed; the next sign-in arrives at the same gate: the words again.
    assert!(!ui(&mut rt).follow_gate(GateState::SignedOut));
    assert!(ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
}

/// On the twelve words, with the session's gate at "needs the phrase", as after a password on a new computer.
fn on_the_words() -> (Runtime<State>, Calls) {
    use crate::shell::app::GateState;
    let (mut rt, calls) = form(Script::default());
    assert!(ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
    (rt, calls)
}

fn discards(calls: &Calls) -> usize {
    calls
        .borrow()
        .iter()
        .filter(|call| **call == "discard_partial_sign_in")
        .count()
}

/// Back must win over a step that is on its way, so while one runs there is no way off its screen (as during the
/// password check): not Back, not "Forgot password?", not the other device. Nothing is discarded under a running
/// step; once it answered, Back works and discards.
#[test]
fn a_running_step_cannot_be_left_and_nothing_is_discarded_under_it() {
    // The twelve words being restored.
    let (mut rt, calls) = on_the_words();
    ui(&mut rt).restoring = true;
    press(&mut rt, Key::Escape);
    ui(&mut rt).ask_other_device();
    ui(&mut rt).forgot();
    assert_eq!(
        ui(&mut rt).screen,
        Screen::Phrase,
        "the restore is on its way: its screen stays"
    );
    assert_eq!(discards(&calls), 0);
    ui(&mut rt).restoring = false;
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(discards(&calls), 1, "left with nothing running: discarded");

    // A code being checked, and a vault password being proven.
    for (screen, running) in [
        (Screen::TwoFactor { backup: false }, 0),
        (Screen::TwoFactor { backup: true }, 0),
        (Screen::UnlockVault, 1),
    ] {
        let (mut rt, calls) = form(Script::default());
        ui(&mut rt).screen = screen.clone();
        match running {
            0 => ui(&mut rt).verifying = true,
            _ => ui(&mut rt).unlocking = true,
        }
        assert!(ui(&mut rt).step_running());
        press(&mut rt, Key::Escape);
        ui(&mut rt).open_sign_in();
        assert_eq!(ui(&mut rt).screen, screen, "{screen:?}");
        assert_eq!(discards(&calls), 0, "{screen:?}");
    }
}

/// "approve from another device" on the twelve words keeps the session that waits behind them (the other device
/// hands its keys to THAT session): no discard on the way in, on the way back, or when the request is refused.
#[test]
fn the_other_device_link_on_the_words_keeps_the_session_and_comes_back_to_the_words() {
    use super::source::{SignInEvent, SignInProblem};
    let (mut rt, calls) = on_the_words();
    ui(&mut rt).words[0].replace_text("harbor");
    ui(&mut rt).ask_other_device();
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false });
    assert!(ui(&mut rt).request().is_some(), "the request is shown");
    assert!(
        ui(&mut rt).words[0].text.is_empty(),
        "the words typed so far are forgotten"
    );
    // "Get a new code" closes the old request and keeps the session too.
    ui(&mut rt).ask_other_device();
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false });
    // Back: the words again, the request closed.
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
    assert!(ui(&mut rt).request().is_none());
    assert_eq!(discards(&calls), 0, "the session was never discarded");
    assert!(calls.borrow().contains(&"cancel_device_request"));

    // A request that is refused (a source that cannot ask one): back on the words, and said there.
    ui(&mut rt).ask_other_device();
    ui(&mut rt).request = None;
    ui(&mut rt).apply(SignInEvent::DeviceRequested(Err(SignInProblem::Other)));
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
    assert_eq!(ui(&mut rt).phrase_problem(), Some(&PhraseProblem::Other));
    assert_eq!(discards(&calls), 0);
    // From the words, Back still discards (the one way to sign in as someone else).
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(discards(&calls), 1);
}

/// The gate is followed whenever it moves, not only when it arrives at the words: a session that ended under the
/// words leaves the form (never the words of a session that is gone), and a session that opened or locked ends
/// every step, so the window starts from its form the next time it shows. The engine is told nothing.
#[test]
fn the_window_follows_the_gate_whenever_it_moves() {
    use crate::shell::app::GateState;
    // The words, then the session is gone (signed out elsewhere, or by the engine).
    let (mut rt, calls) = on_the_words();
    ui(&mut rt).words[3].replace_text("lantern");
    assert!(ui(&mut rt).follow_gate(GateState::SignedOut));
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(ui(&mut rt).words[3].text.is_empty(), "no word stays behind");
    assert_eq!(discards(&calls), 0);

    // Back from the words, a new password being typed, then the sign-out lands: the form is not touched.
    let (mut rt, _) = on_the_words();
    press(&mut rt, Key::Escape);
    ui(&mut rt).password.replace_text("being typed");
    assert!(!ui(&mut rt).follow_gate(GateState::SignedOut));
    assert_eq!(ui(&mut rt).password.text, "being typed");

    // The vault opened (the words were right) or locked: the steps are over.
    for gate in [GateState::Ready, GateState::Locked, GateState::NoPlan] {
        let (mut rt, calls) = on_the_words();
        ui(&mut rt).words[0].replace_text("harbor");
        ui(&mut rt).restoring = true;
        assert!(ui(&mut rt).follow_gate(gate), "{gate:?}");
        assert_eq!(ui(&mut rt).screen, Screen::SignIn, "{gate:?}");
        assert!(ui(&mut rt).words[0].text.is_empty(), "{gate:?}");
        assert!(!ui(&mut rt).step_running(), "{gate:?}");
        assert_eq!(discards(&calls), 0, "{gate:?}");
    }

    // A window that opens on a step under a signed-out gate (the first gate seen) is left alone.
    let (mut rt, _) = form(Script::default());
    ui(&mut rt).screen = Screen::Phrase;
    assert!(!ui(&mut rt).follow_gate(GateState::SignedOut));
    assert_eq!(ui(&mut rt).screen, Screen::Phrase);
}

/// Leaving the words did not end the session (the engine could not sign out): the words come back in front on
/// the next poll, because that is the state; the form that could only refuse does not stay.
#[test]
fn a_sign_out_that_did_not_finish_brings_the_words_back() {
    use super::source::SignInEvent;
    use crate::shell::app::GateState;
    let (mut rt, calls) = on_the_words();
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(
        !ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase),
        "the form stays while it is not known"
    );
    ui(&mut rt).apply(SignInEvent::StillSignedIn);
    assert!(ui(&mut rt).follow_gate(GateState::NeedsRecoveryPhrase));
    assert_eq!(ui(&mut rt).screen, Screen::Phrase, "still signed in: the words");
    // Back tries again.
    press(&mut rt, Key::Escape);
    assert_eq!(discards(&calls), 2);
    // Had the sign-out worked after all, the same event changes nothing.
    ui(&mut rt).apply(SignInEvent::StillSignedIn);
    assert!(!ui(&mut rt).follow_gate(GateState::SignedOut));
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
}

#[test]
fn the_passkey_door_is_drawn_only_where_the_source_can_sign_in_with_one() {
    // A source without passkey sign-in (the engine's default): no door, and no call even from the keyboard.
    let (mut rt, calls) = form(Script {
        passkey_available: false,
        ..Script::default()
    });
    assert!(!a11y(&rt).contains("Use a passkey"), "no dead passkey door");
    ui(&mut rt).use_passkey();
    assert!(!ui(&mut rt).passkey_waiting());
    assert!(calls.borrow().is_empty(), "the engine was not asked");
    // Where it can, the door is there (the board's frames).
    let (rt, _) = form(Script::default());
    assert!(a11y(&rt).contains("Use a passkey"));
}
