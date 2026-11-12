//! The steps after the password, drawn: the two-factor code (DW-A26 to DW-A28), the twelve
//! words (DW-A23, DW-A24) and the vault password after a passkey (DW-A30). Every string is the
//! board's; a state the board gives a rule and no words for uses words the board has elsewhere.

use rune::{El, Key, Tx, col, edit_access, el, row, text};

use super::source::{PhraseProblem, SignInProblem, TwoFactorProblem};
use super::steps::{BACKUP_DIGITS, BACKUP_KEY, CODE_DIGITS, CODE_KEY, Pictured, WORD_KEYS};
use super::view::{GENERIC_FAILURE, actions, back};
use super::{PASSWORD_KEY, Ui, act, icons};
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, Keys, Tone};
use crate::ui::au_entry::{self, DigitsState, FocusedSpec, WordState};
use crate::ui::kit;

fn open(s: &mut State) -> &mut Ui {
    s.signin.as_mut().expect("the sign-in window is open")
}

/// A field's change handler: `f` on the sign-in state, then the engine's ready answers, then
/// the focus the change asked for.
fn then(f: impl Fn(&mut Ui) + 'static) -> impl Fn(&mut State, &mut Tx) + 'static {
    move |s: &mut State, tx: &mut Tx| {
        if let Some(ui) = s.signin.as_mut() {
            f(ui);
            ui.pump();
            ui.settle(tx);
        }
    }
}

/// A focused field takes Escape for itself; on these screens Escape is the back link, so each
/// field hands it to the group's keys first (`super::on_key`).
fn escape_first(s: &mut State, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
    ev.key == Key::Escape && super::on_key(s, ev, tx)
}

/// A link by itself on a line, centred.
fn centered(link: El) -> El {
    el().w_full().h(15.0).center().child(link)
}

/// A small dim sentence of the column (`.t-cap`).
fn caption(k: &mut Keys, words: &str, t: &Theme) -> El {
    kit::label(k.next("note"), words, type_::CAPTION, t.text_tertiary)
        .wrap()
        .w_full()
}

fn primary(
    k: &mut Keys,
    label: &str,
    icon: Option<&'static rune::Icon>,
    state: ButtonState,
    to: fn(&mut Ui),
    t: &Theme,
) -> El {
    let b = au::button(k, label, icon, ButtonKind::Primary, ButtonSize::Lg, state, t);
    if state == ButtonState::Enabled {
        b.on_press_tx(act(to)).default_button()
    } else {
        b
    }
}

// ---------------------------------------------------------------------------
// DW-A26, DW-A27, DW-A28
// ---------------------------------------------------------------------------

/// The banner for a code that did not pass. The board draws the wrong code (DW-A27); the
/// other problems use the sign-in form's words for the same problem.
fn code_banner(k: &mut Keys, problem: TwoFactorProblem, backup: bool, t: &Theme) -> El {
    match problem {
        TwoFactorProblem::WrongCode => au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some("That code isn't right"),
            Some(if backup {
                "Each backup code works once. Nothing was signed in."
            } else {
                "Codes change every 30 seconds. Check the time on this PC, then try the next one. Nothing was signed in."
            }),
            true,
            t,
        ),
        TwoFactorProblem::Offline => au::banner(
            k,
            Tone::Neutral,
            &icons::OFFLINE,
            Some("You're offline"),
            Some("Connect to sign in. Nothing was sent."),
            true,
            t,
        ),
        TwoFactorProblem::RateLimited => au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            None,
            Some("Too many tries from this network. Wait, then try again. Nothing was signed in."),
            true,
            t,
        ),
        TwoFactorProblem::Expired | TwoFactorProblem::Other => {
            au::banner(k, Tone::Red, &icons::INFO, None, Some(GENERIC_FAILURE), true, t)
        }
    }
}

pub fn two_factor(ui: &Ui, backup: bool, k: &mut Keys, t: &Theme) -> El {
    let subtitle = if backup {
        "Enter one of the 8-digit backup codes you saved when you turned on two-factor."
    } else {
        "Your account is protected with two-factor. Enter the 6-digit code from your authenticator app."
    };
    let mut c = au::column()
        .child(back(t))
        .child(au::heading("Two-factor authentication", Some(subtitle), false, t));
    let problem = ui.code_problem();
    if backup {
        c = c.child(au_entry::focused_field(
            k,
            FocusedSpec {
                label: "Backup code",
                icon: None,
                error: None,
                mono: true,
                pictured: ui.pictured() == Some(Pictured::BackupCode),
            },
            &ui.backup_code,
            BACKUP_KEY,
            |s: &mut State| &mut open(s).backup_code,
            then(Ui::backup_changed),
            escape_first,
            None,
            t,
        ));
        c = match problem {
            Some(problem) => c.child(code_banner(k, problem, true, t)),
            None => c.child(caption(k, "Each backup code works once.", t)),
        };
    } else {
        let state = if ui.verifying() {
            DigitsState::Checking
        } else if problem == Some(TwoFactorProblem::WrongCode) {
            DigitsState::Wrong
        } else {
            DigitsState::Entering
        };
        // One input, labelled for assistive technology; the boxes are its picture.
        let input = au_entry::code_input(
            &ui.code,
            CODE_KEY,
            "6-digit verification code",
            CODE_DIGITS,
            |s: &mut State| &mut open(s).code,
        )
        .on_input_key(|s: &mut State, ev: &rune::KeyEvent, tx: &mut Tx| {
            open(s).before_code_key(matches!(ev.key, Key::Text(_)));
            escape_first(s, ev, tx)
        })
        .on_input_change(then(Ui::code_changed));
        c = c.child(
            el().key("au-code")
                .self_center()
                .size(au_entry::digits_width(CODE_DIGITS), 52.0)
                .child(au_entry::digits(&ui.code.text, CODE_DIGITS, state, t))
                .child(input),
        );
        c = match problem {
            Some(problem) => c.child(code_banner(k, problem, false, t)),
            None => c.child(caption(
                k,
                "The code is checked as soon as you type the sixth digit.",
                t,
            )),
        };
    }
    let other = if backup {
        au::link(k, "Use the authenticator code", None, t).on_press_tx(act(|ui| ui.switch_code(false)))
    } else {
        au::link(k, "Use a backup code", None, t).on_press_tx(act(|ui| ui.switch_code(true)))
    };
    let label = match (ui.verifying(), backup) {
        (true, _) => "Verifying\u{2026}",
        (false, true) => "Verify backup code",
        (false, false) => "Verify",
    };
    let state = if ui.verifying() {
        ButtonState::Busy
    } else if ui.can_verify() {
        ButtonState::Enabled
    } else {
        ButtonState::Disabled
    };
    debug_assert!(BACKUP_DIGITS > CODE_DIGITS);
    c.child(centered(other))
        .child(actions(6.0).child(primary(k, label, None, state, Ui::verify_code, t)))
}

// ---------------------------------------------------------------------------
// DW-A23, DW-A24
// ---------------------------------------------------------------------------

/// DW-A23's hint: a sentence that ends in a link, on the board's two lines.
fn paste_hint(k: &mut Keys, t: &Theme) -> El {
    const LINE_1: &str = "Your password is right. Enter your 12 words (paste all 12 into any";
    const LINE_2: &str = "box), or ";
    const LINK: &str = "approve from another device";
    let note = k.next("note");
    let note_box = format!("{note}-box");
    kit::note_text(
        &note,
        &note_box,
        &format!("{LINE_1} {LINE_2}{LINK}."),
        type_::CAPTION,
        t.text_tertiary,
    );
    let words = |s: &str| text(s).font(type_::CAPTION).color(t.text_tertiary);
    // 11px Inter on a 14px line: the baseline is 11 down, the underline 2 below it.
    let link_el = au_entry::link_in(k, LINK, type_::CAPTION, 13.0, 0.8, t)
        // The other device hands the keys over: it replaces this step (DW-A23's rule).
        .on_press_tx(act(Ui::ask_other_device));
    col().key(note_box).w_full().child(words(LINE_1)).child(
        row()
            .child(words(LINE_2).w(rune::text::advance(LINE_2, type_::CAPTION)))
            .child(link_el)
            .child(words(".")),
    )
}

/// The banner over the boxes when the words did not restore the keys. The board draws the
/// wrong phrase (DW-A24) and gives the words for too many tries in its rule.
fn phrase_banner(k: &mut Keys, problem: &PhraseProblem, t: &Theme) -> El {
    match problem {
        PhraseProblem::WrongPhrase { .. } => au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some("That phrase doesn't open this account"),
            Some("Check the words against what you wrote down. Nothing was restored."),
            true,
            t,
        ),
        PhraseProblem::TooManyTries => au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            None,
            Some("Too many tries. Wait an hour."),
            true,
            t,
        ),
        PhraseProblem::Offline => au::banner(
            k,
            Tone::Neutral,
            &icons::OFFLINE,
            Some("You're offline"),
            Some("Connect to sign in. Nothing was sent."),
            true,
            t,
        ),
        PhraseProblem::Other => au::banner(k, Tone::Red, &icons::INFO, None, Some(GENERIC_FAILURE), true, t),
    }
}

pub fn phrase(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let mut c = au::column().child(back(t)).child(au::heading(
        "Set up this device",
        Some("This PC doesn't have your encryption keys yet. Restore them to continue."),
        false,
        t,
    ));
    c = match ui.phrase_problem() {
        Some(problem) => c.child(phrase_banner(k, problem, t)),
        None => c.child(paste_hint(k, t)),
    };
    // Only a word that is not in the word list is marked; a valid word in the wrong place is not.
    let not_a_word = match ui.phrase_problem() {
        Some(PhraseProblem::WrongPhrase { bad_word }) => *bad_word,
        _ => None,
    };
    let state_of = |i: usize| {
        if not_a_word == Some(i) {
            WordState::NotAWord
        } else if ui.pictured() == Some(Pictured::Word(i)) {
            WordState::Pictured
        } else {
            WordState::Rest
        }
    };
    // The twelve words sit in ONE element: a capture of this frame blanks its whole box.
    let mut grid = col()
        .key("secure-words")
        .role("group")
        .label("Recovery phrase")
        .w_full()
        .gap(6.0);
    for line in 0..4 {
        let boxes = line * 3..line * 3 + 3;
        let h = boxes
            .clone()
            .map(|i| au_entry::word_height(!ui.words[i].text.is_empty(), state_of(i)))
            .fold(0.0, f32::max);
        let mut r = row().gap(6.0);
        for i in boxes {
            let input = au_entry::word_input(&ui.words[i], WORD_KEYS[i], i, not_a_word == Some(i), t)
                .on_edit(edit_access(move |s: &mut State| &mut open(s).words[i]))
                .on_input_change(then(move |ui| ui.word_changed(i)))
                .on_input_key(escape_first)
                // Twelve words pasted into any box fill them all.
                .on_input_paste(move |s: &mut State, content: &rune::ClipboardContent, tx: &mut Tx| {
                    let ui = open(s);
                    let took = ui.paste_words(content.text());
                    ui.settle(tx);
                    took
                });
            r = r.child(au_entry::word_box(i, &ui.words[i], input, state_of(i), h, t));
        }
        grid = grid.child(r);
    }
    let state = if ui.restoring() {
        ButtonState::Busy
    } else if ui.can_restore() {
        ButtonState::Enabled
    } else {
        ButtonState::Disabled
    };
    let lost = au::link(k, "I've lost my recovery phrase", None, t).on_press_tx(act(|ui| ui.lost_phrase(true)));
    // The button comes first in the board's markup, so it takes `btn-0` before the link's key.
    c.child(grid).child(
        actions(6.0)
            .child(primary(k, "Restore keys", Some(&icons::KEY), state, Ui::restore, t))
            .child(centered(lost).mt(2.0).mb(2.0)),
    )
}

// ---------------------------------------------------------------------------
// DW-A30
// ---------------------------------------------------------------------------

pub fn unlock(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    // The board gives this step no error of its own: the sign-in form's general words.
    let message = ui.unlock_problem().map(|_: SignInProblem| GENERIC_FAILURE);
    let eye = au::field_action(
        "field-0",
        if ui.password.is_revealed() {
            "Hide password"
        } else {
            "Show password"
        },
        &icons::EYE,
        t,
    )
    .on_press_tx(act(Ui::toggle_password));
    let (label, state) = if ui.unlocking() {
        ("Unlocking vault\u{2026}", ButtonState::Busy)
    } else if ui.can_unlock() {
        ("Unlock", ButtonState::Enabled)
    } else {
        ("Unlock", ButtonState::Disabled)
    };
    au::column()
        .child(back(t))
        .child(au::heading(
            "Unlock your vault",
            Some("Passkey verified. Enter your password to decrypt your data on this PC."),
            false,
            t,
        ))
        .child(au::banner(
            k,
            Tone::Green,
            &icons::CHECK,
            Some("Identity confirmed with your passkey"),
            Some("Your vault still needs your password to unlock here."),
            false,
            t,
        ))
        .child(au_entry::focused_field(
            k,
            FocusedSpec {
                label: "Password",
                icon: Some(&icons::LOCK),
                error: message,
                mono: false,
                pictured: ui.pictured() == Some(Pictured::VaultPassword),
            },
            &ui.password,
            PASSWORD_KEY,
            |s: &mut State| &mut open(s).password,
            then(|ui| ui.pictured = None),
            escape_first,
            Some(eye),
            t,
        ))
        .child(actions(6.0).child(primary(k, label, None, state, Ui::unlock, t)))
}
