//! Sign-in activity (DW-A77 to DW-A79): sign-ins and security changes with time, country and
//! address; the account's one switch for recording them, and the confirm before it stops.
//!
//! Countries by name, never cities; never which files or items were opened.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_form as fm;
use crate::ui::account_set::{self as set, Lead, Row, Tail};
use crate::ui::settings_list as lists;
use rune::El;

use super::form::{self, Form, Problem};
use super::forms::button;
use super::source::{AccountSource, EventKind, Password};
use super::{AccountUi, CodeFor, Dialog, icons};
use crate::ui::account_form::Field;
use crate::ui::account_set::Look;

pub(super) fn event_icon(kind: EventKind) -> &'static rune::Icon {
    match kind {
        EventKind::SignIn => &icons::DEVICE_LAPTOP,
        EventKind::PasskeyAdded => &icons::PASSKEY,
        EventKind::DeviceApproved => &icons::CHECK,
        EventKind::WrongPassword | EventKind::PasswordChanged => &icons::PADLOCK,
        EventKind::AccountCreated => &icons::PEOPLE,
    }
}

pub fn pane(source: &dyn AccountSource, f: &Form, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let periods = source.activity();
    let on = periods.is_some();
    let confirm = match dialog {
        // Owner ruling 2026-11-14: a risky action proves the PASSWORD (the board's DW-A78 asks it too): every source
        // gets the password dialog.
        Some(Dialog::Code(CodeFor::StopActivity)) => Some(stop_with_password(f, t)),
        _ => None,
    };
    // Behind the confirm the board draws the heading without its second line (DW-A78).
    let sub = confirm
        .is_none()
        .then_some("Sign-ins and security changes on your account. Never which files or items you open.");
    // A source that cannot stop the recording from here shows it as a value, not as a switch that would not turn.
    let fixed_on = on && !source.can_stop_activity();
    // Turning it off asks first; turning it on starts a new log at once.
    let switch = crate::ui::switch::switch_sized(&format!("{}-switch", "item-0-0"), "", on, true, crate::ui::button::Size::Md, t).label("Record sign-in activity").on_press(move |s: &mut State| {
        if on {
            super::ask(s, Dialog::Code(CodeFor::StopActivity));
            form::focus(&mut s.account.form, Some(form::CURRENT_KEY));
        } else if let Some(source) = s.account.source.as_deref_mut() {
            source.start_activity();
        }
    });
    let mut children = vec![
        set::heading("Sign-in activity", sub, None, t),
        lists::list(
            0,
            None,
            vec![if fixed_on {
                set::item(
                    0,
                    0,
                    &Row::new(Lead::Icon(&icons::ACTIVITY), "Record sign-in activity").tail(Tail::Value("On".into())),
                    Vec::new(),
                    t,
                )
            } else {
                set::item(
                    0,
                    0,
                    &Row::new(Lead::Icon(&icons::ACTIVITY), "Record sign-in activity"),
                    vec![switch],
                    t,
                )
            }],
            Some(if fixed_on {
                "Turning this off is not available in this version yet. Turn it off on the web."
            } else if on {
                "Keeps the address and country of each sign-in and security change, so you can see if someone else got in."
            } else {
                "Off: nothing is recorded. Security alerts still arrive."
            }),
            t,
        ),
    ];
    match periods {
        None => children.push(fm::empty(
            &icons::ACTIVITY,
            "Security logging is off",
            "Nothing new is recorded. What was recorded is deleted within 30 days.",
            t,
        )),
        Some(periods) => {
            let last = periods.len();
            for (p, (title, events)) in periods.iter().enumerate() {
                let rows = events
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        set::item(
                            p + 1,
                            i,
                            // Read as what happened, then when and where.
                            &Row::new(Lead::Icon(event_icon(e.kind)), e.what.clone())
                                .sub_mono(e.when_where.clone())
                                .name(format!("{}, {}", e.what, e.when_where.replace(" · ", ", "))),
                            Vec::new(),
                            t,
                        )
                    })
                    .collect();
                children.push(lists::list(
                    p + 1,
                    Some(title),
                    rows,
                    (p + 1 == last).then_some(
                        "Device names are what each device reports. Something you don't recognise? Change your password and check Devices and sessions.",
                    ),
                    t,
                ));
            }
        }
    }
    (set::pane(children, t), confirm)
}

/// "Turn off security logging?" asking the account password (DW-A78, drawn as the board's frame).
fn stop_with_password(f: &Form, t: &Theme) -> El {
    let wrong = f.problem == Some(Problem::WrongPassword);
    let (turn_off, turn_off_b) = button(0, "Turn off", Look::Danger, false, t);
    let (cancel, cancel_b) = button(1, "Cancel", Look::Plain, false, t);
    // The board's field (DW-A78, owner ruling 2026-11-14: the password): "Account password", no icon, no words under
    // it; the red state and its sentence come only after a refusal.
    let field = fm::field(
        0,
        &Field {
            label: "Account password",
            icon: None,
            message: wrong.then_some("Enter your password to confirm it's you."),
            error: wrong,
            input_key: form::CURRENT_KEY,
        },
        &f.current,
        |s: &mut State| &mut s.account.form.current,
        t,
    );
    let title = "Turn off security logging?";
    let paragraph = fm::lead_paragraph(
        "You may not notice if someone else gets into your account.",
        "Nothing new is recorded, and what was recorded is deleted within 30 days. This can't be undone.",
        t,
    );
    let mut turn_off = turn_off
        .label("Turn off, destructive")
        .on_press(super::act(confirm_stop_with_password));
    if !f.current.text.is_empty() {
        turn_off = turn_off.default_button();
    }
    set::dialog_with(
        title,
        440.0,
        title,
        paragraph,
        vec![field],
        vec![
            (turn_off, turn_off_b),
            (
                cancel.on_press(|s: &mut State| {
                    s.account.dialog = None;
                    form::arrive(&mut s.account);
                }),
                cancel_b,
            ),
        ],
        t,
    )
    .modal(false, |s: &mut State| {
        s.account.dialog = None;
        form::arrive(&mut s.account);
    })
}

/// The press of "Turn off": an empty field keeps the dialog open and asks for the password; otherwise the password
/// moves to the source's worker and the dialog closes, the answer coming as a toast (the switch moves with the
/// log read after it).
fn confirm_stop_with_password(ui: &mut AccountUi) -> Option<&'static str> {
    if ui.form.current.text.is_empty() {
        ui.form.problem = Some(Problem::WrongPassword);
        form::focus(&mut ui.form, Some(form::CURRENT_KEY));
        return None;
    }
    let source = ui.source.as_deref_mut()?;
    source.stop_activity_confirmed(Password::from_field(&ui.form.current.text));
    ui.form.current = form::secure();
    ui.form.problem = None;
    ui.dialog = None;
    None
}
