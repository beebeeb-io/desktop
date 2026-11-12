//! Recovery with the phrase, drawn: the email and the twelve words (DW-A47, DW-A48), the new
//! password (DW-A49) and "Password updated" (DW-A50). The files are not touched by any of it;
//! the copy says "recover" and "set a new password", never "reset".

use rune::{El, col, el, row};

use super::source::{PhraseProblem, SignInProblem};
use super::steps::{CONFIRM_KEY, Pictured};
use super::view::{GENERIC_FAILURE, actions};
use super::view_steps::{escape_first, open, primary, then, word_grid};
use super::{EMAIL_KEY, PASSWORD_KEY, Ui, act, icons};
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, FieldSpec, FieldState, Keys, Tone};
use crate::ui::au_entry::{self, FocusedSpec};
use crate::ui::kit;

/// The banner when the phrase did not verify. DW-A48 draws the wrong phrase; the hourly limit's
/// words are the board's rule for it.
fn problem_banner(k: &mut Keys, problem: &PhraseProblem, t: &Theme) -> El {
    match problem {
        PhraseProblem::WrongPhrase { .. } => au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some("That phrase doesn't open this account"),
            Some("Check the words against what you wrote down. Nothing was changed, and your password is as it was."),
            true,
            t,
        ),
        PhraseProblem::TooManyTries => au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            None,
            Some("Too many attempts. For security, wait an hour and try again."),
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

pub fn recover(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let mut c = au::column()
        .child(au::back_link("Recovery options", &icons::BACK, t).on_press_tx(act(Ui::leave_recovery)))
        .child(au::heading(
            "Recover with your phrase",
            Some("Enter your email and the 12-word recovery phrase from your account setup."),
            false,
            t,
        ));
    if let Some(problem) = ui.phrase_problem() {
        c = c.child(problem_banner(k, problem, t));
    }
    c = c
        .child(au::field(
            k,
            FieldSpec {
                label: "Email",
                icon: Some(&icons::MAIL),
                state: FieldState::Rest,
                message: None,
                mono: false,
            },
            &ui.email,
            EMAIL_KEY,
            |s: &mut State| &mut open(s).email,
            None,
            t,
        ))
        .child(word_grid(ui, t));
    let state = if ui.recovering() {
        ButtonState::Busy
    } else if ui.can_verify_phrase() {
        ButtonState::Enabled
    } else {
        ButtonState::Disabled
    };
    let mut foot = actions(6.0).child(primary(
        k,
        "Verify phrase",
        Some(&icons::KEY),
        state,
        Ui::verify_phrase,
        t,
    ));
    // The way to the honest dead end is offered once a phrase did not verify (DW-A48).
    if ui.phrase_problem().is_some() {
        foot = foot.child(
            au::button(
                k,
                "I don't have it",
                None,
                ButtonKind::Ghost,
                ButtonSize::Md,
                ButtonState::Enabled,
                t,
            )
            .on_press_tx(act(|ui| ui.cannot_recover(true))),
        );
    }
    c.child(foot)
}

/// The strength meter under the new password (`.kit-strength`): four bars and the engine's
/// words. One lit bar is red, two are grey, three and four are green.
fn strength(key: &str, ui: &Ui, t: &Theme) -> El {
    let verdict = ui.strength();
    let lit = verdict.as_ref().map_or(0, |v| usize::from(v.bars.min(4)));
    let ink = match lit {
        1 => t.red,
        2 => t.text_tertiary,
        _ => t.green,
    };
    let mut bars = row().grow(1.0).gap(4.0).center_y();
    for i in 0..4 {
        bars = bars.child(
            el().key(format!("{key}-strength-{i}"))
                .grow(1.0)
                .h(4.0)
                .radius(2.0)
                .fill(if i < lit { ink } else { t.separator }),
        );
    }
    let mut meter = row()
        .key(format!("{key}-meter"))
        .role("status")
        .h(17.0)
        .mx(2.0)
        .gap(10.0)
        .center_y()
        .child(bars);
    if let Some(verdict) = verdict {
        meter = meter
            .label(format!("Password strength: {}", verdict.words))
            .child(kit::label(format!("{key}-strength"), verdict.words, type_::STATUS, t.text_tertiary).a11y_hidden());
    }
    meter
}

pub fn new_password(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    // The board gives this step no error of its own: the sign-in form's general words.
    let message = ui.set_problem().map(|_: SignInProblem| GENERIC_FAILURE);
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
    let new = au_entry::focused_field(
        k,
        FocusedSpec {
            label: "New password",
            icon: Some(&icons::LOCK),
            error: message,
            mono: false,
            pictured: ui.pictured() == Some(Pictured::NewPassword),
        },
        &ui.password,
        PASSWORD_KEY,
        |s: &mut State| &mut open(s).password,
        then(|ui| ui.pictured = None),
        escape_first,
        Some(eye),
        t,
    )
    .child(strength("field-0", ui, t));
    let confirm = au::field(
        k,
        FieldSpec {
            label: "Confirm password",
            icon: Some(&icons::LOCK),
            state: FieldState::Rest,
            message: None,
            mono: false,
        },
        &ui.confirm,
        CONFIRM_KEY,
        |s: &mut State| &mut open(s).confirm,
        None,
        t,
    );
    let state = if ui.setting() {
        ButtonState::Busy
    } else if ui.can_set_password() {
        ButtonState::Enabled
    } else {
        ButtonState::Disabled
    };
    au::column()
        .child(au::back_link("Back", &icons::BACK, t).on_press_tx(act(Ui::recover_with_phrase)))
        .child(au::heading(
            "Set a new password",
            Some("Your phrase was verified. Choose a strong password for your account."),
            false,
            t,
        ))
        .child(col().w_full().gap(11.0).child(new).child(confirm))
        .child(actions(6.0).child(primary(k, "Set new password", None, state, Ui::set_password, t)))
}

pub fn password_updated(k: &mut Keys, t: &Theme) -> El {
    au::column()
        .child(au::hero_tile(&icons::CHECK, Tone::Green, 0.0, t))
        .child(au::heading(
            "Password updated",
            Some("Your vault is unlocked with your new password. Your files are unchanged. Other devices were signed out."),
            true,
            t,
        ))
        .child(
            actions(6.0).child(
                au::button(
                    k,
                    "Continue to Drive",
                    None,
                    ButtonKind::Primary,
                    ButtonSize::Lg,
                    ButtonState::Enabled,
                    t,
                )
                .on_press_tx(act(Ui::continue_to_drive))
                .default_button(),
            ),
        )
}
