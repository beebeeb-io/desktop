//! Name, email and password (DW-A64 to DW-A68): the pages under Account and Security that
//! change who the account says it is and how it gets in.
//!
//! The current password goes into `AccountSource::step_up` and nowhere else; what comes back
//! is an opaque confirmation that the guarded call consumes.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_form::{self as fm, Field};
use crate::ui::account_set::{self as set, Look, Size};
use crate::ui::au::{self, Keys, Tone};
use rune::El;

use super::form::{self, Form, Problem};
use super::source::{AccountSource, CodeRefused, EmailRefused, Password, Purpose, Refused};
use super::{AccountUi, icons};

/// A 28 pt button of a form's foot, and whether it has a drawn border.
pub(super) fn button(index: usize, label: &str, look: Look, dimmed: bool, t: &Theme) -> (El, bool) {
    (
        set::button(&format!("btn-{index}"), label, None, look, Size::Medium, dimmed, t),
        look == Look::Plain,
    )
}

fn refused(why: Refused) -> Problem {
    match why {
        Refused::WrongPassword | Refused::NotConfirmed => Problem::WrongPassword,
        Refused::Locked => Problem::Locked,
        Refused::Offline => Problem::Offline,
    }
}

const OFFLINE: &str = "Beebeeb can't be reached. Nothing was changed.";

/// Name (DW-A64): one name for every product. Enter saves, Esc cancels.
pub fn name(f: &Form, t: &Theme) -> El {
    let message = match f.problem {
        Some(Problem::EmptyName) => Some("A name can't be empty."),
        Some(Problem::Offline) => Some(OFFLINE),
        _ => None,
    };
    let (cancel, cancel_b) = button(0, "Cancel", Look::Plain, false, t);
    let (save, save_b) = button(1, "Save", Look::Primary, false, t);
    set::pane(
        vec![
            set::heading(
                "Name",
                Some("Shown to people you share with, in every Beebeeb app."),
                None,
                t,
            ),
            fm::field(
                0,
                &Field {
                    label: "Name",
                    message,
                    error: message.is_some(),
                    input_key: form::NAME_KEY,
                    ..Field::default()
                },
                &f.name,
                |s: &mut State| &mut s.account.form.name,
                t,
            ),
            fm::actions(vec![
                (cancel.on_press(|s: &mut State| super::go(s, "DW-A51")), cancel_b),
                (save.default_button().on_press(super::act(save_name)), save_b),
            ]),
        ],
        t,
    )
}

fn save_name(ui: &mut AccountUi) -> Option<&'static str> {
    let name = ui.form.name.text.trim();
    if name.is_empty() || name.chars().count() > 64 {
        ui.form.problem = Some(Problem::EmptyName);
        return None;
    }
    if ui.source.as_deref_mut()?.set_name(name) {
        Some("DW-A51")
    } else {
        ui.form.problem = Some(Problem::Offline);
        None
    }
}

/// Change email (DW-A65): the new address, and the password to confirm it is the person.
pub fn email(f: &Form, t: &Theme) -> El {
    let address = match f.problem {
        Some(Problem::EmailInUse) => Some("This email is already in use."),
        Some(Problem::NotAnAddress) => Some("Enter an email address."),
        Some(Problem::Offline) => Some(OFFLINE),
        _ => None,
    };
    let wrong = f.problem == Some(Problem::WrongPassword);
    let (cancel, cancel_b) = button(0, "Cancel", Look::Plain, false, t);
    let (send, send_b) = button(1, "Send code", Look::Primary, false, t);
    set::pane(
        vec![
            set::heading(
                "Change email",
                Some("We send a code to the new address. Your sign-in email changes once you enter it."),
                None,
                t,
            ),
            fm::field(
                0,
                &Field {
                    label: "New email",
                    icon: Some(&icons::MAIL),
                    message: address,
                    error: address.is_some(),
                    input_key: form::EMAIL_KEY,
                },
                &f.email,
                |s: &mut State| &mut s.account.form.email,
                t,
            ),
            fm::field(
                1,
                &Field {
                    label: "Password",
                    icon: Some(&icons::PADLOCK),
                    message: Some(if wrong {
                        "Incorrect password. Nothing was changed."
                    } else {
                        "To confirm it's you."
                    }),
                    error: wrong,
                    input_key: form::CURRENT_KEY,
                },
                &f.current,
                |s: &mut State| &mut s.account.form.current,
                t,
            ),
            fm::actions(vec![
                (cancel.on_press(|s: &mut State| super::go(s, "DW-A51")), cancel_b),
                (send.default_button().on_press(super::act(send_code)), send_b),
            ]),
        ],
        t,
    )
}

fn send_code(ui: &mut AccountUi) -> Option<&'static str> {
    let source = ui.source.as_deref_mut()?;
    let f = &mut ui.form;
    let address = f.email.text.trim().to_string();
    if !address.contains('@') {
        f.problem = Some(Problem::NotAnAddress);
        form::focus(f, Some(form::EMAIL_KEY));
        return None;
    }
    let confirmation = match source.step_up(Password::from_field(&f.current.text), Purpose::ChangeEmail) {
        Ok(confirmation) => confirmation,
        Err(why) => {
            f.problem = Some(refused(why));
            form::focus(f, Some(form::CURRENT_KEY));
            return None;
        }
    };
    match source.start_email_change(confirmation, &address) {
        Ok(()) => Some("DW-A66"),
        Err(why) => {
            f.problem = Some(match why {
                EmailRefused::InUse => Problem::EmailInUse,
                EmailRefused::NotAnAddress => Problem::NotAnAddress,
                EmailRefused::NotConfirmed => Problem::WrongPassword,
                EmailRefused::Offline => Problem::Offline,
            });
            form::focus(f, Some(form::EMAIL_KEY));
            None
        }
    }
}

/// The words under a row of code boxes when the code was not taken.
pub(super) fn code_error(words: &str, t: &Theme) -> El {
    rune::el().key("code-msg-box").w_full().center().child(
        crate::ui::kit::label("code-msg", words, au::type_au::FIELD_MSG, au::red_text(t))
            .role("alert")
            .wrap(),
    )
}

/// "Didn't get it? Send again in 42 s" (DW-A66, DW-A81).
pub(super) fn send_again(source: &dyn AccountSource, t: &Theme) -> El {
    let wait = source.resend_wait();
    let link = fm::link(0, "Send again", fm::CAP, wait > 0, t);
    let link = if wait > 0 {
        link
    } else {
        link.on_press(|s: &mut State| {
            if let Some(source) = s.account.source.as_deref_mut() {
                source.send_again();
            }
        })
    };
    fm::resend(0, link, wait, t)
}

/// Check your new address (DW-A66): the 8-digit code. Digits can be pasted.
pub fn email_code(source: &dyn AccountSource, f: &Form, t: &Theme) -> El {
    let complete = form::code(f, 8).len() == 8;
    let (back, back_b) = button(0, "Back", Look::Plain, false, t);
    let (confirm, confirm_b) = button(1, "Confirm", Look::Primary, !complete, t);
    let confirm = if complete {
        confirm.default_button().on_press(super::act(confirm_email))
    } else {
        confirm
    };
    let mut children = vec![
        set::heading(
            "Check your new address",
            Some(&format!("We sent an 8-digit code to {}.", f.email.text.trim())),
            None,
            t,
        ),
        fm::digits(
            0,
            8,
            &f.code,
            form::CODE_KEY,
            "8-digit code",
            |s: &mut State| &mut s.account.form.code,
            t,
        ),
    ];
    if f.problem.is_some() {
        children.push(code_error(
            if f.problem == Some(Problem::Offline) {
                OFFLINE
            } else {
                "That code is wrong or has expired. Check it, or send a new one."
            },
            t,
        ));
    }
    children.push(send_again(source, t));
    children.push(fm::actions(vec![
        (back.on_press(|s: &mut State| super::go(s, "DW-A65")), back_b),
        (confirm, confirm_b),
    ]));
    set::pane(children, t)
}

fn confirm_email(ui: &mut AccountUi) -> Option<&'static str> {
    let code = form::code(&ui.form, 8);
    match ui.source.as_deref_mut()?.confirm_email_change(&code) {
        Ok(()) => Some("DW-A51"),
        Err(why) => {
            ui.form.problem = Some(if why == CodeRefused::Offline {
                Problem::Offline
            } else {
                Problem::WrongCode
            });
            None
        }
    }
}

/// Change password (DW-A67), and the same page after a wrong current password (DW-A68: the
/// new password is kept, only the current one is retyped).
pub fn password(source: &dyn AccountSource, f: &Form, t: &Theme) -> El {
    let current = match f.problem {
        Some(Problem::WrongPassword) => Some("Current password is incorrect. Nothing was changed."),
        Some(Problem::Locked) => Some("Unlock first, then change your password."),
        Some(Problem::Offline) => Some(OFFLINE),
        _ => None,
    };
    let weak = f.problem == Some(Problem::Weak);
    let mismatch = f.problem == Some(Problem::Mismatch);
    let (cancel, cancel_b) = button(0, "Cancel", Look::Plain, false, t);
    let (change, change_b) = button(1, "Change password", Look::Primary, false, t);
    let lock = Some(&icons::PADLOCK);
    let mut new = rune::col().key("new-password").w_full().gap(6.0).child(fm::field(
        1,
        &Field {
            label: "New password",
            icon: lock,
            message: None,
            error: weak,
            input_key: form::NEW_KEY,
        },
        &f.new,
        |s: &mut State| &mut s.account.form.new,
        t,
    ));
    if !f.new.text.is_empty() {
        // Read after the new password: how strong core finds it.
        let strength = source.password_strength(&Password::from_field(&f.new.text));
        new = new.child(fm::strength(1, strength.bars(), strength.says(), t));
    }
    let mut keys = Keys::new();
    set::pane(
        vec![
            set::heading("Change password", None, None, t),
            fm::field(
                0,
                &Field {
                    label: "Current password",
                    icon: lock,
                    message: current,
                    error: current.is_some(),
                    input_key: form::CURRENT_KEY,
                },
                &f.current,
                |s: &mut State| &mut s.account.form.current,
                t,
            ),
            new,
            fm::field(
                2,
                &Field {
                    label: "New password again",
                    icon: lock,
                    message: mismatch.then_some("Passwords do not match"),
                    error: mismatch,
                    input_key: form::AGAIN_KEY,
                },
                &f.again,
                |s: &mut State| &mut s.account.form.again,
                t,
            ),
            au::banner(
                &mut keys,
                Tone::Neutral,
                &icons::PADLOCK,
                Some("Your data stays encrypted as it is"),
                Some(
                    "Devices that never unlocked your data are signed out. The others stay signed in and get an alert.",
                ),
                false,
                t,
            ),
            fm::actions(vec![
                (cancel.on_press(|s: &mut State| super::go(s, "DW-A52")), cancel_b),
                (change.default_button().on_press(super::act(change_password)), change_b),
            ]),
        ],
        t,
    )
}

fn change_password(ui: &mut AccountUi) -> Option<&'static str> {
    let source = ui.source.as_deref_mut()?;
    let f = &mut ui.form;
    // The new password first: a wrong current password must not be asked twice for a typo.
    if !source.password_strength(&Password::from_field(&f.new.text)).accepted() {
        f.problem = Some(Problem::Weak);
        form::focus(f, Some(form::NEW_KEY));
        return None;
    }
    if f.new.text != f.again.text {
        f.problem = Some(Problem::Mismatch);
        form::focus(f, Some(form::AGAIN_KEY));
        return None;
    }
    let outcome = source
        .step_up(Password::from_field(&f.current.text), Purpose::ChangePassword)
        .and_then(|confirmation| source.change_password(confirmation, Password::from_field(&f.new.text)));
    match outcome {
        Ok(()) => Some("DW-A52"),
        Err(why) => {
            // DW-A68: the new password is kept, only the current one is retyped.
            f.problem = Some(refused(why));
            f.current = rune::TextEdit::new("").secure();
            form::focus(f, Some(form::CURRENT_KEY));
            None
        }
    }
}
