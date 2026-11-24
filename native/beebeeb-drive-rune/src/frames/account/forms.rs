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
use super::source::{AccountSource, CallKind, NameAnswer, NameWhy, Password, Purpose, Refused};
use super::{AccountUi, icons};

/// A 28 pt button of a form's foot, and whether it has a drawn border.
pub(super) fn button(index: usize, label: &str, look: Look, dimmed: bool, t: &Theme) -> (El, bool) {
    (
        set::button(&format!("btn-{index}"), label, None, look, Size::Medium, dimmed, t),
        look == Look::Plain,
    )
}

pub(super) fn refused(why: Refused) -> Problem {
    match why {
        Refused::WrongPassword | Refused::NotConfirmed => Problem::WrongPassword,
        Refused::Locked => Problem::Locked,
        Refused::Offline => Problem::Offline,
        // Beebeeb's own refusal of the new password (the same as now, too weak, found in a breach): said in words. The
        // check on this PC before the press stays `Weak` (the strength meter says why).
        Refused::Weak => Problem::NotAccepted,
        Refused::Unconfirmed => Problem::Unconfirmed,
    }
}

pub(super) const OFFLINE: &str = "Beebeeb can't be reached. Nothing was changed.";
/// Change email while its code is being sent (this press, or one from an earlier visit).
pub(super) const STILL_SENDING: &str = "A code is still being sent. Wait for it to finish.";
/// A code confirm whose call is still out.
pub(super) const STILL_OUT: &str = "Your earlier request is still being made. Wait for it to finish.";
/// Beebeeb did not take the new password, whichever of its reasons (the same as now, too weak, found in a breach).
pub(super) const NOT_ACCEPTED: &str =
    "Beebeeb did not accept the new password. Nothing was changed. Choose a different one.";
/// A password change that may have been made (review of desktop#423, P2 #9): never "nothing was changed".
pub(super) const UNCONFIRMED: &str =
    "We couldn't confirm the change. Try signing in with your new password; if that fails, use your old one.";

/// Name (DW-A64): one name for every product. Enter saves, Esc cancels.
pub fn name(f: &Form, t: &Theme) -> El {
    let message = match f.problem {
        Some(Problem::EmptyName) => Some("A name can't be empty."),
        Some(Problem::Offline) => Some(OFFLINE),
        Some(Problem::NameRefused) => Some("Beebeeb did not take the new name. Nothing was changed."),
        _ => None,
    };
    let (mut cancel, cancel_b) = button(0, "Cancel", Look::Plain, f.saving_name, t);
    let (mut save, save_b) = button(1, "Save", Look::Primary, f.saving_name, t);
    // While the name is out the page takes no press (a second save, a way out): nothing is drawn as saved before
    // Beebeeb answered, and its answer finds the page it is for.
    if !f.saving_name {
        cancel = cancel.on_press(|s: &mut State| super::go(s, "DW-A51"));
        save = save.default_button().on_press(super::act(save_name));
    }
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
            fm::actions(vec![(cancel, cancel_b), (save, save_b)]),
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
    match ui.source.as_deref_mut()?.save_name(name) {
        NameAnswer::Saved => Some("DW-A51"),
        NameAnswer::NotSaved(why) => {
            ui.form.problem = Some(not_saved(why));
            None
        }
        // The answer is taken in by the window's poll (`take_in_name`).
        NameAnswer::Pending => {
            ui.form.problem = None;
            ui.form.saving_name = true;
            None
        }
    }
}

fn not_saved(why: NameWhy) -> Problem {
    match why {
        NameWhy::Offline => Problem::Offline,
        NameWhy::Refused => Problem::NameRefused,
    }
}

/// The poll's side of a save that was `Pending`: what Beebeeb answered, when it has. `Some(frame)`: the save
/// worked and the page goes back to Account. The answer settles the page only while the Name page still shows.
pub(super) fn take_in_name(ui: &mut AccountUi) -> Option<&'static str> {
    if !ui.form.saving_name {
        return None;
    }
    let answer = ui.source.as_deref_mut()?.name_result()?;
    ui.form.saving_name = false;
    let showing = ui.pane == super::Pane::Name;
    match answer {
        NameAnswer::Saved => showing.then_some("DW-A51"),
        NameAnswer::NotSaved(why) => {
            if showing {
                ui.form.problem = Some(not_saved(why));
            }
            None
        }
        NameAnswer::Pending => {
            ui.form.saving_name = true;
            None
        }
    }
}

/// Change email (DW-A65): the new address, and the password to confirm it is the person. While its code is being sent
/// (this visit's press, or one from an earlier visit) the fields are frozen, Send code is off and the first button
/// leaves the page: its answer is that press's, and DW-A66 names the address Beebeeb sent the code to.
pub fn email(f: &Form, t: &Theme) -> El {
    let out = f.waiting == Some(CallKind::StartEmail);
    let address = match f.problem {
        _ if out => Some(STILL_SENDING),
        Some(Problem::EmailInUse) => Some("This email is already in use."),
        Some(Problem::NotAnAddress) => Some("Enter an email address."),
        Some(Problem::Offline) => Some(OFFLINE),
        _ => None,
    };
    let wrong = f.problem == Some(Problem::WrongPassword) && !out;
    let (cancel, cancel_b) = button(0, if out { "Leave this page" } else { "Cancel" }, Look::Plain, false, t);
    let (mut send, send_b) = button(1, "Send code", Look::Primary, out, t);
    if !out {
        send = send.default_button().on_press(super::act(send_code));
    }
    let frozen = |field: El| {
        if out {
            field.disabled(true).focusable(false)
        } else {
            field
        }
    };
    set::pane(
        vec![
            set::heading(
                "Change email",
                Some("We send a code to the new address. Your sign-in email changes once you enter it."),
                None,
                t,
            ),
            frozen(fm::field(
                0,
                &Field {
                    label: "New email",
                    icon: Some(&icons::MAIL),
                    message: address,
                    error: address.is_some() && !out,
                    input_key: form::EMAIL_KEY,
                },
                &f.email,
                move |s: &mut State| {
                    if out {
                        &mut s.account.form.sink
                    } else {
                        &mut s.account.form.email
                    }
                },
                t,
            )),
            frozen(fm::field(
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
                move |s: &mut State| {
                    if out {
                        &mut s.account.form.sink
                    } else {
                        &mut s.account.form.current
                    }
                },
                t,
            )),
            fm::actions(vec![
                (cancel.on_press(|s: &mut State| super::go(s, "DW-A51")), cancel_b),
                (send, send_b),
            ]),
        ],
        t,
    )
}

fn send_code(ui: &mut AccountUi) -> Option<&'static str> {
    ui.source.as_deref()?;
    if ui.form.waiting.is_some() {
        return None;
    }
    let address = ui.form.email.text.trim().to_string();
    if !address.contains('@') {
        ui.form.problem = Some(Problem::NotAnAddress);
        form::focus(&mut ui.form, Some(form::EMAIL_KEY));
        return None;
    }
    // Every call starts through `calls::start`, which records the press the answer belongs to. The password leaves
    // the field with the press (a wiping copy for the one call); the field is empty from here on.
    let typed = zeroize::Zeroizing::new(ui.form.current.text.clone());
    ui.form.current = form::secure();
    if super::calls::start(ui, CallKind::StartEmail, &address, &typed) == super::calls::Start::NotStarted {
        // No open session: nothing was sent, and nothing else is asked (no step-up stands in).
        ui.form.problem = Some(Problem::Offline);
        form::focus(&mut ui.form, Some(form::EMAIL_KEY));
    }
    None
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
pub(super) fn send_again(source: &dyn AccountSource, email: bool, t: &Theme) -> El {
    // The email's code can always be sent again (Beebeeb has the route); the deletion's only where a source says so.
    let offered = email || source.can_send_again_deletion();
    if !offered {
        // Nothing sends it again: no link that does nothing.
        return rune::el().key("resend-none");
    }
    let wait = source.resend_wait();
    let link = fm::link(0, "Send again", fm::CAP, wait > 0, t);
    let link = if wait > 0 {
        link
    } else {
        link.on_press(move |s: &mut State| {
            // The email's code is sent on the source's worker (the answer settles on the page); with no open session
            // nothing is sent and the page says so. The deletion's code uses `send_again`.
            if email {
                if super::calls::start(&mut s.account, CallKind::ResendEmail, "", "") == super::calls::Start::NotStarted
                {
                    s.account.form.problem = Some(Problem::Offline);
                }
                return;
            }
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
    // The address Beebeeb sent the code to (its own word), never what a field holds now.
    let sent_to = match source.email_code_sent_to() {
        Some(address) => format!("We sent an 8-digit code to {address}."),
        None => "We sent an 8-digit code to your new address.".to_string(),
    };
    let (back, back_b) = button(0, "Back", Look::Plain, false, t);
    let (confirm, confirm_b) = button(1, "Confirm", Look::Primary, !complete, t);
    let confirm = if complete {
        confirm.default_button().on_press(super::act(confirm_email))
    } else {
        confirm
    };
    let mut children = vec![
        set::heading("Check your new address", Some(&sent_to), None, t),
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
    let words = match f.problem {
        None => None,
        Some(Problem::Offline) => Some(OFFLINE.to_string()),
        // The wait counts down on the source's clock; at 0 there is nothing left to say.
        Some(Problem::ResendWait) => {
            let wait = source.resend_wait();
            (wait > 0).then(|| format!("Wait {wait} s before asking again."))
        }
        Some(Problem::ResendLimit) => Some("No more codes for this change. Start again.".to_string()),
        Some(Problem::StillOut) if f.waiting == Some(CallKind::ResendEmail) => Some(STILL_SENDING.to_string()),
        Some(Problem::StillOut) => Some("Your code is still being checked. Wait for it to finish.".to_string()),
        Some(_) => Some("That code is wrong or has expired. Check it, or send a new one.".to_string()),
    };
    if let Some(words) = words {
        children.push(code_error(&words, t));
    }
    children.push(send_again(source, true, t));
    children.push(fm::actions(vec![
        (back.on_press(|s: &mut State| super::go(s, "DW-A65")), back_b),
        (confirm, confirm_b),
    ]));
    set::pane(children, t)
}

fn confirm_email(ui: &mut AccountUi) -> Option<&'static str> {
    let code = form::code(&ui.form, 8);
    if ui.form.waiting.is_none()
        && super::calls::start(ui, CallKind::ConfirmEmail, &code, "") == super::calls::Start::NotStarted
    {
        ui.form.problem = Some(Problem::Offline);
    }
    None
}

/// Change password (DW-A67), and the same page after a wrong current password (DW-A68: the
/// new password is kept, only the current one is retyped).
/// What the Change password page says while a change is out (from this visit or an earlier one).
pub(super) const CHANGING: &str = "A password change is still being made. Wait for it to finish.";

/// A change that finished while the person was on another page.
pub(super) const CHANGED_AWAY: &str =
    "Your password was changed while you were away. Use the new one the next time you sign in.";

pub fn password(source: &dyn AccountSource, f: &Form, t: &Theme) -> El {
    let out = f.waiting == Some(CallKind::ChangePassword);
    let current = match f.problem {
        _ if out => Some(CHANGING),
        Some(Problem::WrongPassword) => Some("Current password is incorrect. Nothing was changed."),
        Some(Problem::Locked) => Some("Unlock first, then change your password."),
        Some(Problem::Offline) => Some(OFFLINE),
        Some(Problem::Unconfirmed) => Some(UNCONFIRMED),
        Some(Problem::Changed) => Some(CHANGED_AWAY),
        _ => None,
    };
    let weak = matches!(f.problem, Some(Problem::Weak | Problem::NotAccepted));
    let not_accepted = (f.problem == Some(Problem::NotAccepted)).then_some(NOT_ACCEPTED);
    let mismatch = f.problem == Some(Problem::Mismatch);
    // While a change is out (from this visit or an earlier one) nothing is drawn as changed before Beebeeb answered:
    // the fields are frozen and Change is off. The first button leaves the page and says so; it does not cancel the
    // change. An answer that lands after the person left is said wherever they are (`late`), and here in full.
    let (cancel, cancel_b) = button(0, if out { "Leave this page" } else { "Cancel" }, Look::Plain, false, t);
    let (mut change, change_b) = button(1, "Change password", Look::Primary, out, t);
    let cancel = cancel.on_press(|s: &mut State| super::go(s, "DW-A52"));
    if !out {
        change = change.default_button().on_press(super::act(change_password));
    }
    let frozen = |field: El| {
        if out {
            field.disabled(true).focusable(false)
        } else {
            field
        }
    };
    let lock = Some(&icons::PADLOCK);
    let mut new = rune::col()
        .key("new-password")
        .w_full()
        .gap(crate::theme::scale::space::S6)
        .child(frozen(fm::field(
            1,
            &Field {
                label: "New password",
                icon: lock,
                message: not_accepted,
                error: weak,
                input_key: form::NEW_KEY,
            },
            &f.new,
            move |s: &mut State| {
                if out {
                    &mut s.account.form.sink
                } else {
                    &mut s.account.form.new
                }
            },
            t,
        )));
    if !f.new.text.is_empty() {
        // Read after the new password: how strong core finds it.
        let strength = source.password_strength(&Password::from_field(&f.new.text));
        new = new.child(fm::strength(1, strength.bars(), strength.says(), t));
    }
    let mut keys = Keys::new();
    set::pane(
        vec![
            set::heading("Change password", None, None, t),
            frozen(fm::field(
                0,
                &Field {
                    label: "Current password",
                    icon: lock,
                    message: current,
                    error: current.is_some() && !out && f.problem != Some(Problem::Changed),
                    input_key: form::CURRENT_KEY,
                },
                &f.current,
                move |s: &mut State| {
                    if out {
                        &mut s.account.form.sink
                    } else {
                        &mut s.account.form.current
                    }
                },
                t,
            )),
            new,
            frozen(fm::field(
                2,
                &Field {
                    label: "New password again",
                    icon: lock,
                    message: mismatch.then_some("Passwords do not match"),
                    error: mismatch,
                    input_key: form::AGAIN_KEY,
                },
                &f.again,
                move |s: &mut State| {
                    if out {
                        &mut s.account.form.sink
                    } else {
                        &mut s.account.form.again
                    }
                },
                t,
            )),
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
            fm::actions(vec![(cancel, cancel_b), (change, change_b)]),
        ],
        t,
    )
}

fn change_password(ui: &mut AccountUi) -> Option<&'static str> {
    if ui.form.waiting.is_some() {
        return None;
    }
    let source = ui.source.as_deref_mut()?;
    // A change from an earlier visit of this page is still out: this press is refused with nothing sent, and the
    // page shows that wait. Its answer is that change's, and is drawn as that (never as a second press's).
    if let Some(mark) = source.call_mark(CallKind::ChangePassword).filter(|mark| mark.out) {
        ui.form.waiting_serial = mark.serial;
        ui.form.waiting = Some(CallKind::ChangePassword);
        ui.form.problem = None;
        return None;
    }
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
    let stepped = source.step_up(Password::from_field(&f.current.text), Purpose::ChangePassword);
    let outcome = match stepped {
        Ok(confirmation) => {
            // A source that asks Beebeeb on a worker takes the change here: the page waits for its answer
            // (`calls::settle`). One that answers at once is asked as before.
            let new = zeroize::Zeroizing::new(f.new.text.clone());
            if super::calls::start(ui, CallKind::ChangePassword, &new, "") != super::calls::Start::NotStarted {
                return None;
            }
            let (Some(source), f) = (ui.source.as_deref_mut(), &mut ui.form) else {
                return None;
            };
            source.change_password(confirmation, Password::from_field(&f.new.text))
        }
        Err(why) => Err(why),
    };
    let f = &mut ui.form;
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
