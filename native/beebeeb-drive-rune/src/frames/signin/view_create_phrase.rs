//! Create an account, drawn, from the recovery phrase on: the phrase shown once (DW-A39), three
//! of its words typed back (DW-A40, DW-A41), this device's name (DW-A42), the account being
//! made (DW-A43) and "Your account is ready" (DW-A44).
//!
//! The phrase. DW-A39 is the one screen that shows it. The view borrows the twelve words from
//! the source for this one draw and hands each to a secure text node (`au_entry::shown_secret`):
//! no element label, no accessibility text, no shared layout cache, no `String`. The three
//! words typed back are secure fields shown as typed, and the suggestions are secure text too.

use rune::{El, el, row};

use super::create::{DEVICE_KEY, Pictured, WORD_KEYS};
use super::create_kit::{self as ck, Field, cact};
use super::create_source::CreateProblem;
use super::view::actions;
use super::view_create::{head, primary, state};
use super::view_steps::{escape_first, open};
use super::{Intent, Ui, icons};
use crate::State;
use crate::theme::icons::{DOC_GENERIC, FILES};
use crate::theme::tokens::{Theme, type_};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, Keys, Tone};
use crate::ui::au_entry::{self, WORD_WIDTH, type_entry};
use crate::ui::kit;

// ---------------------------------------------------------------------------
// DW-A39
// ---------------------------------------------------------------------------

/// One numbered box of the phrase, read-only (`.au-word`). Keys as `au_entry::word_box`.
fn shown_word(i: usize, word: &str, t: &Theme) -> El {
    let key = format!("word-{i}");
    el().key(key.clone())
        .role("listitem")
        .label(format!("Word {}", i + 1))
        .w(WORD_WIDTH)
        .h(34.0)
        .radius(11.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator))
        .child(
            kit::label(
                format!("{key}-n"),
                (i + 1).to_string(),
                type_entry::WORD_N,
                t.text_tertiary,
            )
            .a11y_hidden()
            .absolute(10.0, 9.0)
            .w(16.0),
        )
        .child(au_entry::shown_secret(&format!("{key}-text"), word, type_entry::WORD, t.text).absolute(32.0, 8.0))
}

/// The twelve words, three to a row, six apart (`.au-pw`). Nothing when the source lends none.
fn phrase_grid(ui: &Ui, t: &Theme) -> El {
    let mut boxes: Vec<El> = Vec::new();
    ui.with_phrase(&mut |words: &[&str]| {
        boxes = words
            .iter()
            .enumerate()
            .map(|(i, word)| shown_word(i, word, t))
            .collect();
    });
    let mut grid = rune::col()
        .key("au-pw")
        .role("list")
        .label("Recovery phrase")
        .w_full()
        .gap(6.0);
    let mut boxes = boxes.into_iter().peekable();
    while boxes.peek().is_some() {
        grid = grid.child(row().gap(6.0).children(boxes.by_ref().take(3).collect::<Vec<_>>()));
    }
    grid
}

pub fn phrase(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let out = |b: El, to: fn(&mut Ui)| b.on_press_tx(cact(to));
    // Copy needs the clipboard, which only a handler's `Tx` has.
    let copy = ck::small_button(k, "Copy", &ck::COPY, t).on_press_tx(|s: &mut State, tx: &mut rune::Tx| {
        if let Some(ui) = s.signin.as_mut() {
            ui.copy_phrase(tx);
        }
    });
    let save = out(ck::small_button(k, "Save as file", &ck::DOWNLOAD, t), |ui| {
        ui.phrase_out(Intent::SavePhraseAsFile)
    });
    let print = out(ck::small_button(k, "Print", &DOC_GENERIC, t), |ui| {
        ui.phrase_out(Intent::PrintPhrase)
    });
    head("Back", 5, "Recovery phrase", t)
        .child(au::heading(
            "Your recovery phrase",
            Some("This phrase protects Drive, Pass, VPN, Chat and Mail. The same 12 words."),
            false,
            t,
        ))
        .child(phrase_grid(ui, t))
        .child(au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some("We can't show it again or reset it"),
            Some("Anyone with these 12 words can open your account. Write them on paper. Don't screenshot them or send them."),
            false,
            t,
        ))
        .child(row().w_full().gap(10.0).center_y().child(copy).child(save).child(print))
        .child(
            ck::checkbox(k, ui.phrase_saved(), "I have saved my recovery phrase offline.", false, t)
                .on_press_tx(cact(Ui::toggle_saved)),
        )
        .child(actions(6.0).child(primary(
            k,
            "I saved it, verify",
            state(false, ui.phrase_saved()),
            Ui::verify_saved,
            t,
        )))
}

// ---------------------------------------------------------------------------
// DW-A40, DW-A41
// ---------------------------------------------------------------------------

/// The typed word's handlers cannot capture which field they belong to (`fn` pointers).
const WORD_CHANGED: [fn(&mut Ui); 3] = [
    |ui| ui.typed_word_changed(0),
    |ui| ui.typed_word_changed(1),
    |ui| ui.typed_word_changed(2),
];
const TAKE: [fn(&mut Ui); 4] = [
    |ui| ui.take_suggestion(0),
    |ui| ui.take_suggestion(1),
    |ui| ui.take_suggestion(2),
    |ui| ui.take_suggestion(3),
];

pub fn confirm(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let c = ui.create_state();
    let mut column = head("Back", 5, "Confirm the phrase", t).child(au::heading(
        "Confirm your phrase",
        Some("Type the words you wrote down."),
        false,
        t,
    ));
    for (i, at) in ui.confirm_positions().iter().enumerate() {
        // The message stands under the middle field and does not say which word is wrong.
        let wrong = ui.words_mismatch() && i == 1;
        let label = format!("Word {}", at + 1);
        let name = format!("Recovery word {}", at + 1);
        column = column.child(ck::field(
            k,
            Field {
                label: &label,
                name: Some(&name),
                icon: None,
                message: wrong.then_some("Those words do not match your recovery phrase."),
                error: wrong,
                mono: true,
                pictured: c.pictured == Some(Pictured::Word(i)),
            },
            &c.words[i],
            WORD_KEYS[i],
            move |s: &mut State| &mut open(s).create.words[i],
            ck::changed(WORD_CHANGED[i]),
            escape_first,
            None,
            None,
            t,
        ));
    }
    let suggestions = ui.suggestions();
    let mut offered = row().gap(10.0).center_y();
    if !suggestions.is_empty() {
        offered = offered.child(kit::label(
            "suggest-label",
            "Suggestions:",
            type_::CAPTION,
            t.text_tertiary,
        ));
        for (i, word) in suggestions.iter().take(TAKE.len()).enumerate() {
            offered = offered.child(ck::chip(i, word, t).on_press_tx(cact(TAKE[i])));
        }
    }
    column
        .child(
            row()
                .key("suggest-row")
                .w_full()
                .h(26.0)
                .center_y()
                .child(offered)
                .child(el().grow(1.0))
                .child(
                    au_entry::link_in(k, "Show the words again", type_::CAPTION, 13.0, 0.875, t)
                        .on_press_tx(cact(Ui::show_words_again)),
                ),
        )
        .child(actions(6.0).child(primary(
            k,
            "Confirm",
            state(ui.confirming(), ui.can_confirm_words()),
            Ui::confirm_words,
            t,
        )))
}

// ---------------------------------------------------------------------------
// DW-A42
// ---------------------------------------------------------------------------

pub fn device(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let c = ui.create_state();
    head("Back", 6, "This device", t)
        .child(au::heading(
            "Name this PC",
            Some("You'll see this name in Devices and sessions, and when you approve a sign-in on another device."),
            false,
            t,
        ))
        .child(ck::field(
            k,
            Field {
                label: "Device name",
                name: None,
                icon: Some(&icons::LAPTOP),
                message: Some("The system name is suggested. You can change it later in Devices and sessions."),
                error: false,
                mono: false,
                pictured: c.pictured == Some(Pictured::Device),
            },
            &c.device,
            DEVICE_KEY,
            |s: &mut State| &mut open(s).create.device,
            ck::changed(Ui::device_name_changed),
            escape_first,
            None,
            None,
            t,
        ))
        .child(actions(6.0).child(primary(
            k,
            "Create my account",
            ButtonState::Enabled,
            Ui::create_account,
            t,
        )))
}

// ---------------------------------------------------------------------------
// DW-A43
// ---------------------------------------------------------------------------

pub fn creating(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let button = |k: &mut Keys, label: &str, to: fn(&mut Ui)| {
        au::button(
            k,
            label,
            None,
            ButtonKind::Primary,
            ButtonSize::Lg,
            ButtonState::Enabled,
            t,
        )
        .on_press_tx(cact(to))
        .default_button()
    };
    match ui.create_problem() {
        // No Back and no Cancel while it runs.
        None | Some(CreateProblem::CodeExpired) => au::column().child(ck::hero_ring(0.55, t)).child(au::heading(
            "Creating your account",
            Some("Setting up account encryption on this PC. This takes a few seconds."),
            true,
            t,
        )),
        Some(CreateProblem::NothingStored) => au::column()
            .child(au::heading("We could not create your account", None, false, t))
            .child(au::banner(
                k,
                Tone::Red,
                &icons::INFO,
                None,
                Some("We could not create your account. Nothing was stored. You can try again."),
                true,
                t,
            ))
            .child(actions(6.0).child(button(k, "Try again", Ui::create_account))),
        Some(CreateProblem::NotFinishedHere) => au::column()
            .child(au::heading("Your account was created", None, false, t))
            .child(au::banner(
                k,
                Tone::Neutral,
                &icons::INFO,
                None,
                Some("Your account was created. We could not finish setting it up on this device. Nothing is lost."),
                true,
                t,
            ))
            .child(actions(6.0).child(button(k, "Sign in", Ui::sign_in_after_create))),
    }
}

// ---------------------------------------------------------------------------
// DW-A44
// ---------------------------------------------------------------------------

/// "The other apps can continue as Anna": the part of the address before the at sign, with a
/// capital, until the account has a name of its own.
fn first_name(email: &str) -> String {
    let name: String = email.chars().take_while(|c| c.is_alphanumeric()).collect();
    let mut letters = name.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

pub fn ready(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let email = ui.email.text.trim();
    let line = format!(
        "{email} works in Drive, Pass, VPN, Chat and Mail. Sign in once on this PC; the other apps can continue as {}.",
        first_name(email)
    );
    au::column()
        .child(au::hero_tile(&icons::CHECK, Tone::Green, 0.0, t))
        .child(au::heading("Your account is ready", Some(&line), true, t))
        .child(ck::next_card(&FILES, "Set up Drive on this PC", t))
        .child(au::banner(
            k,
            Tone::Amber,
            &icons::LOCK,
            Some("Encrypted on this PC"),
            Some("Your recovery phrase was shown once and can't be shown again. It is the way back in if you lose your password and have no other signed-in device."),
            false,
            t,
        ))
        .child(
            actions(6.0).child(
                au::button(
                    k,
                    "Continue",
                    None,
                    ButtonKind::Primary,
                    ButtonSize::Lg,
                    ButtonState::Enabled,
                    t,
                )
                .on_press_tx(cact(Ui::finish_create))
                .default_button(),
            ),
        )
}
