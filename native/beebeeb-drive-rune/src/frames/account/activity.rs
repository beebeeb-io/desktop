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

use super::form::Form;
use super::source::{AccountSource, EventKind};
use super::{CodeFor, Dialog, icons};

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
        Some(Dialog::Code(CodeFor::StopActivity)) => Some(super::two_factor::code_dialog(CodeFor::StopActivity, f, t)),
        _ => None,
    };
    // Behind the confirm the board draws the heading without its second line (DW-A78).
    let sub = confirm
        .is_none()
        .then_some("Sign-ins and security changes on your account. Never which files or items you open.");
    // A source that cannot stop the recording from here shows it as a value, not as a switch that would not turn.
    let fixed_on = on && !source.can_stop_activity();
    // Turning it off asks first; turning it on starts a new log at once.
    let switch = lists::switch("item-0-0", "Record sign-in activity", on, t).on_press(move |s: &mut State| {
        if on {
            super::ask(s, Dialog::Code(CodeFor::StopActivity));
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
