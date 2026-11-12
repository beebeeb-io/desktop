//! Delete the account (DW-A58, DW-A80 to DW-A83): four steps, each with the board's own words.
//! What goes, the 72 hours to undo, the password; the second factor; the email link; the
//! schedule and how to undo it; and the screen that says the undo worked.
//!
//! The password goes into the OPAQUE step-up; its confirmation waits in the form between step
//! 1 and step 2 and is consumed by `request_deletion`. Nothing is scheduled until the link in
//! the email is used.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_form::{self as fm, Field};
use crate::ui::account_set::{self as set, Lead, Look, Row};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, Keys, Tone};
use crate::ui::kit;
use crate::ui::settings_list as lists;
use rune::{Cx, El, col};

use super::form::{self, Form, Problem};
use super::forms::{button, code_error, send_again};
use super::source::{AccountSource, Deletion, Password, Purpose, Refused};
use super::{AccountUi, Dialog, Pane, icons};

pub fn pane(pane: Pane, source: &dyn AccountSource, f: &Form, t: &Theme) -> El {
    let email = source.identity().email;
    match (pane, source.deletion()) {
        (Pane::DeleteFactor, _) if f.confirmation.is_some() => second_factor(f, t),
        (Pane::DeleteLink, Deletion::LinkSent) => link_sent(source, f, &email, t),
        (Pane::DeleteScheduled, Deletion::Scheduled { on }) | (Pane::DeleteLink, Deletion::Scheduled { on }) => {
            scheduled(&on, &email, t)
        }
        _ => what_goes(f, t),
    }
}

/// Step 1 of 4 (DW-A58): what goes, the 72 hours, the password.
fn what_goes(f: &Form, t: &Theme) -> El {
    let goes = |index: usize, icon: &'static rune::Icon, title: &str, sub: &str| {
        set::item(0, index, &Row::new(Lead::Icon(icon), title).sub(sub), Vec::new(), t)
    };
    let message = match f.problem {
        Some(Problem::WrongPassword) => (true, "Incorrect password. Nothing was deleted."),
        Some(Problem::Offline) => (true, "Beebeeb can't be reached. Nothing was deleted."),
        Some(Problem::Locked) => (true, "Unlock first. Nothing was deleted."),
        _ => (false, "Enter your password to continue."),
    };
    let (cancel, cancel_b) = button(0, "Cancel", Look::Plain, false, t);
    let (next, next_b) = button(1, "Continue", Look::Danger, false, t);
    set::pane(
        vec![
            // The 72-hour line is read before the list.
            set::heading(
                "Delete your Beebeeb account?",
                Some("After you confirm by email, you have 72 hours to undo it. After that, it can't be undone."),
                None,
                t,
            ),
            fm::steps(1, 4, "What goes", t),
            lists::list(
                0,
                Some("What gets deleted"),
                vec![
                    goes(
                        0,
                        &icons::FOLDER,
                        "Every product's data",
                        "Drive, Pass, VPN, Chat and Mail",
                    ),
                    goes(1, &icons::LINK, "Shared links", "Stop working when it is deleted"),
                    goes(2, &icons::PEOPLE, "Team access", "Memberships revoked"),
                    goes(3, &icons::DEVICE_PHONE, "Every device", "Signed out"),
                ],
                Some("Audit logs are kept for 12 months."),
                t,
            ),
            fm::field(
                0,
                &Field {
                    label: "Password",
                    icon: Some(&icons::PADLOCK),
                    message: Some(message.1),
                    error: message.0,
                    input_key: form::CURRENT_KEY,
                },
                &f.current,
                |s: &mut State| &mut s.account.form.current,
                t,
            ),
            fm::actions(vec![
                (cancel.on_press(|s: &mut State| super::go(s, "DW-A52")), cancel_b),
                (
                    next.label("Continue, destructive")
                        .default_button()
                        .on_press(super::act(check_password)),
                    next_b,
                ),
            ]),
        ],
        t,
    )
}

/// Checks the password by the OPAQUE step-up; the confirmation waits for step 2.
pub(super) fn check_password(ui: &mut AccountUi) -> Option<&'static str> {
    let f = &mut ui.form;
    let source = ui.source.as_deref_mut()?;
    match source.step_up(Password::from_field(&f.current.text), Purpose::DeleteAccount) {
        Ok(confirmation) => {
            f.confirmation = Some(confirmation);
            ui.pane = Pane::DeleteFactor;
            Some("DW-A80")
        }
        Err(why) => {
            f.problem = Some(match why {
                Refused::Locked => Problem::Locked,
                Refused::Offline => Problem::Offline,
                _ => Problem::WrongPassword,
            });
            f.current = rune::TextEdit::new("").secure();
            form::focus(f, Some(form::CURRENT_KEY));
            None
        }
    }
}

/// Step 2 of 4 (DW-A80): the second factor, so a stolen password alone can't delete the account.
fn second_factor(f: &Form, t: &Theme) -> El {
    let count = if f.backup_code { 8 } else { 6 };
    let complete = form::code(f, count).len() == count;
    let (back, back_b) = button(0, "Back", Look::Plain, false, t);
    let (next, next_b) = button(1, "Continue", Look::Danger, !complete, t);
    let next = next.label("Continue, destructive");
    let next = if complete {
        next.default_button().on_press(super::act(send_link))
    } else {
        next
    };
    let other = fm::link(
        0,
        if f.backup_code {
            "Use your authenticator app"
        } else {
            "Use a backup code"
        },
        set::SUB,
        false,
        t,
    )
    .on_press(|s: &mut State| {
        let ui = &mut s.account;
        let backup = !ui.form.backup_code;
        ui.form.code = rune::TextEdit::new("").secure().revealed(true);
        ui.form.backup_code = backup;
        ui.form.problem = None;
        form::focus(&mut ui.form, Some(form::CODE_KEY));
    });
    let mut children = vec![
        set::heading(
            "Confirm it's you",
            Some(if f.backup_code {
                "Enter one of your backup codes."
            } else {
                "Enter a code from your authenticator app."
            }),
            None,
            t,
        ),
        fm::steps(2, 4, "Confirm it's you", t),
        fm::digits(
            0,
            count,
            &f.code,
            form::CODE_KEY,
            if f.backup_code {
                "8-digit backup code"
            } else {
                "6-digit code"
            },
            |s: &mut State| &mut s.account.form.code,
            t,
        ),
    ];
    if f.problem.is_some() {
        children.push(code_error("Incorrect code. Nothing was deleted.", t));
    }
    children.push(rune::row().key("other-way").w_full().child(other));
    children.push(fm::actions(vec![
        (back.on_press(|s: &mut State| super::go(s, "DW-A58")), back_b),
        (next, next_b),
    ]));
    set::pane(children, t)
}

/// Sends the confirmation link: the step-up's confirmation and the second factor together.
fn send_link(ui: &mut AccountUi) -> Option<&'static str> {
    let f = &mut ui.form;
    let code = form::code(f, if f.backup_code { 8 } else { 6 });
    let source = ui.source.as_deref_mut()?;
    let Some(confirmation) = f.confirmation.take() else {
        // The proof of the password was used: step 1 again.
        return Some("DW-A58");
    };
    match source.request_deletion(confirmation, &code) {
        Ok(()) => Some("DW-A81"),
        Err(refused) => match refused.confirmation {
            // A wrong code: the proof of the password still stands, only the code is retyped.
            Some(confirmation) => {
                f.confirmation = Some(confirmation);
                f.problem = Some(Problem::WrongCode);
                f.code = rune::TextEdit::new("").secure().revealed(true);
                form::focus(f, Some(form::CODE_KEY));
                None
            }
            None => Some("DW-A58"),
        },
    }
}

/// Step 3 of 4 (DW-A81): nothing is deleted until the link in the email is used.
fn link_sent(source: &dyn AccountSource, f: &Form, email: &str, t: &Theme) -> El {
    let mut keys = Keys::new();
    let (opened, opened_b) = button(0, "I've opened the link", Look::Plain, false, t);
    let (cancel, cancel_b) = button(1, "Cancel deletion", Look::Quiet, false, t);
    let mut children = vec![
        set::heading(
            "Check your email",
            Some(&format!("We sent a confirmation link to {email}.")),
            None,
            t,
        ),
        fm::steps(3, 4, "Confirm by email", t),
        // Read first after the heading: a status, not an alert.
        au::banner(
            &mut keys,
            Tone::Neutral,
            &icons::MAIL,
            Some("Nothing is deleted until you use that link"),
            Some("If you didn't mean to do this, ignore the email: the link expires and nothing happens."),
            false,
            t,
        ),
    ];
    if f.problem == Some(Problem::LinkNotUsed) {
        children.push(
            kit::label(
                "link-msg",
                "The link has not been used yet. Nothing is scheduled.",
                au::type_au::FIELD_MSG,
                t.text_secondary,
            )
            .role("status"),
        );
    }
    children.push(send_again(source, t));
    children.push(fm::actions(vec![
        (
            opened.on_press(super::act(|ui: &mut AccountUi| {
                if ui.source.as_deref_mut()?.deletion_link_used() {
                    Some("DW-A82")
                } else {
                    ui.form.problem = Some(Problem::LinkNotUsed);
                    None
                }
            })),
            opened_b,
        ),
        (
            cancel.on_press(super::act(|ui: &mut AccountUi| {
                ui.source.as_deref_mut()?.cancel_deletion_request();
                Some("DW-A52")
            })),
            cancel_b,
        ),
    ]));
    set::pane(children, t)
}

/// Step 4 of 4 (DW-A82): scheduled, when, and how to undo it.
fn scheduled(on: &str, email: &str, t: &Theme) -> El {
    let mut keys = Keys::new();
    let (undo, undo_b) = button(0, "Undo deletion", Look::Primary, false, t);
    let (sign_out, sign_out_b) = button(1, "Sign out", Look::Quiet, false, t);
    set::pane(
        vec![
            // The date is read in full.
            set::heading(
                "Deletion scheduled",
                Some(&format!("Your account will be deleted in 72 hours, on {on}.")),
                None,
                t,
            ),
            fm::steps(4, 4, "Scheduled", t),
            au::banner(
                &mut keys,
                Tone::Neutral,
                &icons::CLOCK,
                Some("You can undo it until then"),
                Some(&format!(
                    "We sent a second email to {email} with a link to undo it. You can also undo it here or by signing in. Until then everything works as usual."
                )),
                false,
                t,
            ),
            kit::label(
                "note-0",
                "After 72 hours every product's data is deleted and shredded within 30 days. Then nothing can bring it back, not you, not Beebeeb.",
                set::SUB,
                t.text_tertiary,
            )
            .wrap()
            .w_full(),
            fm::actions(vec![
                (
                    undo.default_button().on_press(super::act(|ui: &mut AccountUi| {
                        ui.source.as_deref_mut()?.undo_deletion().then_some("DW-A83")
                    })),
                    undo_b,
                ),
                (
                    // Signs out of this app; signing in again within 72 hours offers the undo.
                    sign_out.on_press(|s: &mut State| {
                        super::go(s, "DW-A52");
                        super::ask(s, Dialog::SignOutApp);
                    }),
                    sign_out_b,
                ),
            ]),
        ],
        t,
    )
}

/// Deletion cancelled (DW-A83): a screen of its own, in the account window's layout.
pub fn undone(name: &str, t: &Theme, cx: &mut Cx<State>) -> El {
    use crate::frames::signin::icons as si;
    let mut keys = Keys::new();
    let column = au::column()
        .child(au::hero_tile(&si::CHECK, Tone::Green, 0.0, t))
        // The result is announced: the heading is the screen's first element.
        .child(au::heading(
            "Deletion cancelled",
            Some("Your account is as it was. Nothing was deleted. We sent an email to say so."),
            true,
            t,
        ))
        .child(
            col().key("au-actions").w_full().gap(6.0).mt(6.0).child(
                au::button(
                    &mut keys,
                    "Continue to Drive",
                    None,
                    ButtonKind::Primary,
                    ButtonSize::Lg,
                    ButtonState::Enabled,
                    t,
                )
                .default_button()
                .on_press(|s: &mut State| super::go(s, "DW-0401")),
            ),
        );
    au::page(name, "Drive", [&si::LOCK, &si::PEOPLE, &si::GLOBE], column, t, cx).on_key(super::on_key)
}
