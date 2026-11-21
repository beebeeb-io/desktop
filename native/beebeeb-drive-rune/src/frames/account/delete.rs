//! Delete the account (DW-A58, DW-A80 to DW-A83): four steps, each with the board's own words.
//! What goes, the time to undo, the password; the second factor; the email link; the
//! schedule and how to undo it; and the screen that says the undo worked.
//!
//! The password goes into the OPAQUE step-up; its confirmation waits in the form between step
//! 1 and step 2 and is consumed by `request_deletion`. Nothing is scheduled until the link in
//! the email is used.
//!
//! Flagged deviations from the board (`FRAMES.md`): Cancel deletion (DW-A81) and Undo deletion
//! (DW-A82) ask the account password first, in a dialog over the page, because core and the
//! server require the step-up for a signed-in cancel (D-P40-11). The time to undo is Beebeeb's
//! (the date the source names), never a length of time written here.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_form::{self as fm, Field};
use crate::ui::account_set::{self as set, Lead, Look, Row};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, Keys, Tone};
use crate::ui::kit;
use rune::{Cx, El, col};

use super::form::{self, Form, Problem};
use super::forms::{button, code_error, refused, send_again};
use super::source::{AccountSource, CodeRefused, Deletion, Password, Purpose, Refused};
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

/// Step 1 of 4 (DW-A58): what goes, the time to undo, the password.
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
            // The line about the time to undo is read before the list. How long it is, is Beebeeb's: the date is
            // shown once the deletion is scheduled (DW-A82).
            set::heading(
                "Delete your Beebeeb account?",
                Some(
                    "After you confirm by email, you can still undo it until a date we show you. After that, it can't be undone.",
                ),
                None,
                t,
            ),
            fm::steps(1, 4, "What goes", t),
            super::numbered_list(
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
    match f.problem {
        Some(Problem::WrongCode) => children.push(code_error("Incorrect code. Nothing was deleted.", t)),
        Some(Problem::Offline) => children.push(code_error("Beebeeb can't be reached. Nothing was deleted.", t)),
        // Anything else is said by the source over the page (a toast), or is not about this page.
        _ => {}
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
        Err(not_asked) => match not_asked.confirmation {
            // The proof of the password still stands: only the code is retyped.
            Some(confirmation) => {
                f.confirmation = Some(confirmation);
                f.problem = match not_asked.why {
                    CodeRefused::Wrong => Some(Problem::WrongCode),
                    CodeRefused::Offline => Some(Problem::Offline),
                    // Not the code: the source says why over the page.
                    CodeRefused::Expired | CodeRefused::NotConfirmed => None,
                };
                f.code = rune::TextEdit::new("").secure().revealed(true);
                form::focus(f, Some(form::CODE_KEY));
                None
            }
            // Beebeeb has a deletion scheduled already: its schedule, and the way to undo it.
            None if matches!(source.deletion(), Deletion::Scheduled { .. }) => Some("DW-A82"),
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
    children.push(send_again(source, false, t));
    children.push(fm::actions(vec![
        (
            opened.on_press(super::act(|ui: &mut AccountUi| {
                let source = ui.source.as_deref_mut()?;
                if source.deletion_link_used() {
                    return Some("DW-A82");
                }
                // "Not used" only on Beebeeb's own answer: a check that failed is said by the source (a toast).
                let answered = !source.notice_failed();
                ui.form.problem = answered.then_some(Problem::LinkNotUsed);
                None
            })),
            opened_b,
        ),
        (
            // The password first (D-P40-11): the dialog over this page does the cancel.
            cancel.on_press(|s: &mut State| super::ask(s, Dialog::DeletionPassword(DeletionAsk::Cancel))),
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
            // The date is read in full. It is the last moment it can be undone, as Beebeeb says it; the account
            // is deleted after it.
            set::heading(
                "Deletion scheduled",
                Some(&format!("Your account will be deleted after {on}.")),
                None,
                t,
            ),
            fm::steps(4, 4, "Scheduled", t),
            au::banner(
                &mut keys,
                Tone::Neutral,
                &icons::CLOCK,
                Some("You can undo it until then"),
                // Beebeeb sends no second email at the confirm: the link it sent first undoes it too.
                Some(&format!(
                    "The link in the email we sent to {email} undoes it too. You can also undo it here or by signing in. Until then everything works as usual."
                )),
                false,
                t,
            ),
            kit::label(
                "note-0",
                "After that, every product's data is deleted and shredded. Then nothing can bring it back, not you, not Beebeeb.",
                set::SUB,
                t.text_tertiary,
            )
            .wrap()
            .w_full(),
            fm::actions(vec![
                (
                    // The password first (D-P40-11): the dialog over this page does the undo.
                    undo.default_button()
                        .on_press(|s: &mut State| super::ask(s, Dialog::DeletionPassword(DeletionAsk::Undo))),
                    undo_b,
                ),
                (
                    // Signs out of this app; signing in again before that date offers the undo.
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

/// Which change of a pending deletion the password is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletionAsk {
    /// Cancel deletion (DW-A81).
    Cancel,
    /// Undo deletion (DW-A82).
    Undo,
}

/// The password asked over DW-A81 and DW-A82 (`Dialog::DeletionPassword`); else nothing.
pub fn overlay(dialog: Option<&Dialog>, f: &Form, t: &Theme) -> Option<El> {
    match dialog {
        Some(Dialog::DeletionPassword(ask)) => Some(password_ask(*ask, f, t)),
        _ => None,
    }
}

/// The ask: the account password, then the change. The board draws no password here (D-P40-11, `FRAMES.md`).
fn password_ask(ask: DeletionAsk, f: &Form, t: &Theme) -> El {
    let message = match f.problem {
        Some(Problem::WrongPassword) => Some("Incorrect password. Nothing changed."),
        Some(Problem::Locked) => Some("Unlock first. Nothing changed."),
        Some(Problem::Offline) => Some("Beebeeb can't be reached. Nothing changed."),
        Some(Problem::Unconfirmed) => Some("We couldn't confirm it went through. Look again in a moment."),
        _ => None,
    };
    let (title, action, rest) = match ask {
        DeletionAsk::Cancel => (
            "Cancel the deletion?",
            "Cancel deletion",
            "If the link in the email was used, the deletion is cancelled. If not, nothing is scheduled: ignore the email and the link expires.",
        ),
        DeletionAsk::Undo => (
            "Undo the deletion?",
            "Undo deletion",
            "Your account stays as it is, and nothing is deleted.",
        ),
    };
    let field = fm::field(
        0,
        &Field {
            label: "Account password",
            icon: None,
            message,
            error: message.is_some(),
            input_key: form::CURRENT_KEY,
        },
        &f.current,
        |s: &mut State| &mut s.account.form.current,
        t,
    );
    let paragraph = fm::lead_paragraph("Enter your password to confirm it's you.", rest, t);
    // Keys of their own: the page under the dialog has btn-0 and btn-1.
    let (go, go_b) = button(4, action, Look::Primary, false, t);
    let (back, back_b) = button(5, "Back", Look::Plain, false, t);
    let mut go = go.on_press(super::act(confirm_ask));
    if !f.current.text.is_empty() {
        go = go.default_button();
    }
    let close = |s: &mut State| {
        s.account.dialog = None;
        form::arrive(&mut s.account);
    };
    set::dialog_with(
        title,
        440.0,
        title,
        paragraph,
        vec![field],
        vec![(go, go_b), (back.on_press(close), back_b)],
        t,
    )
    .modal(false, close)
}

/// The ask's press: a real step-up through the source (as step 1 makes one), then the cancel or the undo with it.
/// A wrong password changes nothing and keeps the dialog open; the typed password goes with the press either way.
fn confirm_ask(ui: &mut AccountUi) -> Option<&'static str> {
    let Some(Dialog::DeletionPassword(ask)) = ui.dialog else {
        return None;
    };
    let f = &mut ui.form;
    if f.current.text.is_empty() {
        f.problem = Some(Problem::WrongPassword);
        form::focus(f, Some(form::CURRENT_KEY));
        return None;
    }
    let source = ui.source.as_deref_mut()?;
    let answer = match source.step_up(Password::from_field(&f.current.text), Purpose::DeleteAccount) {
        Ok(confirmation) => match ask {
            DeletionAsk::Cancel => source.cancel_deletion_request(confirmation),
            DeletionAsk::Undo => source.undo_deletion(confirmation),
        },
        Err(why) => Err(why),
    };
    f.current = form::secure();
    match answer {
        Ok(Deletion::None) => {
            ui.dialog = None;
            Some(match ask {
                DeletionAsk::Cancel => "DW-A52",
                DeletionAsk::Undo => "DW-A83",
            })
        }
        // Nothing was scheduled, so nothing was cancelled: the link still works until it expires, and the page
        // says that nothing is scheduled.
        Ok(Deletion::LinkSent) => {
            ui.dialog = None;
            form::focus(&mut ui.form, None);
            if ui.pane == Pane::DeleteLink {
                ui.form.problem = Some(Problem::LinkNotUsed);
                None
            } else {
                Some("DW-A81")
            }
        }
        // Not undone (the source says why): the schedule stands.
        Ok(Deletion::Scheduled { .. }) => {
            ui.dialog = None;
            form::focus(&mut ui.form, None);
            (ui.pane != Pane::DeleteScheduled).then_some("DW-A82")
        }
        Err(why) => {
            ui.form.problem = Some(refused(why));
            form::focus(&mut ui.form, Some(form::CURRENT_KEY));
            None
        }
    }
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
