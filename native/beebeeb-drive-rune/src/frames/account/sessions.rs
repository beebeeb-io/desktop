//! Devices and sessions (DW-A55): every device signed in to the account, across the products;
//! and the confirm before one is signed out (DW-A56).

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_set::{self as set, Lead, Look, Row, Size, Tail};
use crate::ui::au::Tone;
use crate::ui::settings_list as lists;
use rune::{El, row};

use super::source::{AccountSource, DeviceKind, Session};
use super::{Dialog, icons};

fn device_icon(kind: DeviceKind) -> &'static rune::Icon {
    match kind {
        DeviceKind::Laptop => &icons::DEVICE_LAPTOP,
        DeviceKind::Phone => &icons::DEVICE_PHONE,
        DeviceKind::CommandLine => &icons::TERMINAL,
        DeviceKind::Browser => &icons::GLOBE,
    }
}

/// The pane, and the confirm over it when one is open.
pub fn pane(source: &dyn AccountSource, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let sessions = source.sessions();
    let confirm = match dialog {
        Some(Dialog::SignOutDevice(id)) => sessions
            .iter()
            .find(|s| s.id == *id && !s.this_device)
            .map(|s| device_confirm(s, &sessions, t)),
        Some(Dialog::SignOutElsewhere) => Some(elsewhere_confirm(&sessions, t)),
        _ => None,
    };
    let mut button = 0;
    let rows = sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let mut it = Row::new(Lead::Icon(device_icon(session.kind)), session.name.clone())
                .sub(format!("{} · {}", session.apps, session.last_active))
                .name(format!(
                    "{}, {}, last active {}",
                    session.name, session.apps, session.last_active
                ))
                .tail(Tail::Mono(session.country.clone()));
            let mut extra = Vec::new();
            if session.this_device {
                // This device has no sign-out here: that is on Sign-in and security.
                it = it.tail(Tail::Pill {
                    label: "This PC".into(),
                    tone: Tone::Green,
                    dot: true,
                });
            } else {
                let id = session.id.clone();
                extra.push(
                    set::button(
                        &format!("btn-{button}"),
                        "Sign out",
                        None,
                        Look::Quiet,
                        Size::Small,
                        false,
                        t,
                    )
                    .label(format!("Sign out {}", session.name))
                    .on_press(move |s: &mut State| super::confirm_device(s, id.clone())),
                );
                button += 1;
            }
            set::item(0, i, &it, extra, t)
        })
        .collect();
    // DW-A56 draws the pane behind the confirm without its second line and without the button
    // under the list (the PR's Decision line quotes both frames).
    let behind = confirm.is_some();
    let mut children = vec![
        set::heading(
            "Devices and sessions",
            (!behind).then_some("Every device that is signed in to your account."),
            None,
            t,
        ),
        lists::list(0, None, rows, None, t),
    ];
    if !behind && sessions.iter().any(|s| !s.this_device) {
        children.push(
            row().key("actions").child(
                set::button(
                    &format!("btn-{button}"),
                    "Sign out everywhere else",
                    None,
                    Look::Plain,
                    Size::Medium,
                    false,
                    t,
                )
                .on_press(|s: &mut State| s.account.dialog = Some(Dialog::SignOutElsewhere)),
            ),
        );
    }
    (set::pane(children, t), confirm)
}

/// The dialog's two buttons, numbered after the rows' own.
fn foot(
    first: usize,
    action: &str,
    name: String,
    t: &Theme,
    confirmed: impl Fn(&mut State) + 'static,
) -> Vec<(El, bool)> {
    vec![
        (
            set::button(
                &format!("btn-{first}"),
                action,
                None,
                Look::Danger,
                Size::Medium,
                false,
                t,
            )
            // Announced with what it ends, and as destructive.
            .label(format!("{name}, destructive"))
            .on_press(confirmed),
            false,
        ),
        (
            // Enter is Cancel: the safe choice.
            set::button(
                &format!("btn-{}", first + 1),
                "Cancel",
                None,
                Look::Plain,
                Size::Medium,
                false,
                t,
            )
            .default_button()
            .on_press(|s: &mut State| s.account.dialog = None),
            true,
        ),
    ]
}

/// Sign out one device (DW-A56). Honest about timing: a revoked session ends the next time
/// the app talks to the server, not instantly on the device.
fn device_confirm(session: &Session, all: &[Session], t: &Theme) -> El {
    let others = all.iter().filter(|s| !s.this_device).count();
    let apps = if session.apps_sentence.contains(" and ") {
        session.apps_sentence.clone()
    } else {
        "The Beebeeb apps".into()
    };
    let id = session.id.clone();
    set::dialog(
        &format!("Sign out {}?", session.name),
        440.0,
        &format!("Sign out {}?", session.name),
        &format!(
            "{apps} on that device are signed out the next time they contact Beebeeb. Their encrypted data stays on that device until it is removed there."
        ),
        Vec::new(),
        foot(others, "Sign out", format!("Sign out {}", session.name), t, move |s: &mut State| {
            if let Some(source) = s.account.source.as_mut() {
                source.sign_out_device(&id);
            }
            s.account.dialog = None;
        }),
        t,
    )
    .modal(false, |s: &mut State| s.account.dialog = None)
}

/// Sign out every device but this one: the same confirm, in the plural.
fn elsewhere_confirm(all: &[Session], t: &Theme) -> El {
    let others = all.iter().filter(|s| !s.this_device).count();
    set::dialog(
        "Sign out everywhere else?",
        440.0,
        "Sign out everywhere else?",
        "The Beebeeb apps on your other devices are signed out the next time they contact Beebeeb. Their encrypted data stays on those devices until it is removed there.",
        Vec::new(),
        foot(others, "Sign out", "Sign out everywhere else".into(), t, |s: &mut State| {
            if let Some(source) = s.account.source.as_mut() {
                source.sign_out_everywhere_else();
            }
            s.account.dialog = None;
        }),
        t,
    )
    .modal(false, |s: &mut State| s.account.dialog = None)
}
