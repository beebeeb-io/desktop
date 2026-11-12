//! Sign-in and security (DW-A52): everything about getting in, who else is in, and getting
//! out; its two sign-out confirms (DW-A57, DW-A57a); and the security checkup (DW-A84, DW-A84a).

use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::account_set::{self as set, Lead, Look, Row, Size, Tail};
use crate::ui::kit;
use crate::ui::settings_list as lists;
use rune::{El, col, el, row};

use super::source::{AccountSource, Factor, FactorKind, score};
use super::{Dialog, icons};

fn on_off(on: bool) -> String {
    if on { "On".into() } else { "Off".into() }
}

/// The pane, and the confirm over it when one is open.
pub fn pane(source: &dyn AccountSource, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let sec = source.security();
    let (done, of) = score(&source.factors());
    // A button on the heading line, not a row: the pane is full (DW-A52's rule).
    let checkup = set::button(
        "btn-0",
        &format!("Security checkup · {done} of {of}"),
        Some(&icons::SHIELD_CHECK),
        Look::Plain,
        Size::Small,
        false,
        t,
    )
    .on_press(|s: &mut State| super::go(s, "DW-A84"));

    // App lock is Drive's own row on this page (the board's `securityExtra` slot, DW-0716).
    let lock_on = sec.app_lock;
    let mut lock_controls = Vec::new();
    if lock_on {
        lock_controls.push(
            el().key("item-0-0-pop-button")
                .role("combobox")
                .label(format!("Locks again after, {}", sec.lock_after))
                .mr(8.0)
                .radius(6.0)
                .focusable(true)
                .on_press(|s: &mut State| super::go(s, "DW-0716"))
                .child(lists::pop_button("item-0-0", &sec.lock_after, t)),
        );
    }
    lock_controls.push(
        lists::switch("item-0-0", "Lock Beebeeb", lock_on, t).on_press(move |s: &mut State| {
            if let Some(source) = s.account.source.as_mut() {
                source.set_app_lock(!lock_on);
            }
        }),
    );
    let app_lock = lists::list(
        0,
        Some("App lock"),
        vec![set::item(
            0,
            0,
            &Row::new(Lead::Icon(&icons::APP_LOCK), "Lock Beebeeb").sub(if lock_on {
                "Windows Hello · locks again after"
            } else {
                "Off: anyone using this PC can open Beebeeb"
            }),
            lock_controls,
            t,
        )],
        None,
        t,
    );

    let opens = |list: usize, index: usize, it: Row, frame: &'static str| {
        set::item(list, index, &it.link(true), Vec::new(), t).on_press(move |s: &mut State| super::go(s, frame))
    };
    let sign_in = lists::list(
        1,
        Some("Sign-in"),
        vec![
            opens(
                1,
                0,
                Row::new(Lead::Icon(&icons::PADLOCK), "Password").tail(Tail::Value(sec.password_changed.clone())),
                "DW-A67",
            ),
            opens(
                1,
                1,
                Row::new(Lead::Icon(&icons::SHIELD), "Two-factor authentication")
                    .tail(Tail::Value(on_off(sec.two_factor))),
                "DW-A71",
            ),
            opens(
                1,
                2,
                Row::new(Lead::Icon(&icons::PASSKEY), "Passkeys").tail(Tail::Value(sec.passkeys.to_string())),
                "DW-A73",
            ),
            opens(
                1,
                3,
                Row::new(Lead::Icon(&icons::ACTIVITY), "Sign-in activity")
                    .tail(Tail::Value(on_off(sec.sign_in_activity))),
                "DW-A77",
            ),
        ],
        None,
        t,
    );
    let devices = lists::list(
        2,
        Some("Devices"),
        vec![
            opens(
                2,
                0,
                Row::new(Lead::Icon(&icons::DEVICE_PHONE), "Devices and sessions")
                    .tail(Tail::Value(sec.sessions.to_string())),
                "DW-A55",
            ),
            opens(
                2,
                1,
                Row::new(Lead::Icon(&icons::SCAN), "Approve a sign-in").sub("Let another device sign in"),
                "DW-A12",
            ),
        ],
        None,
        t,
    );
    // The two destructive rows open a confirm over the view; neither touches other devices.
    let confirms = |index: usize, it: Row, dialog: Dialog| {
        set::item(3, index, &it.danger().link(false), Vec::new(), t)
            .on_press(move |s: &mut State| s.account.dialog = Some(dialog.clone()))
    };
    let sign_out = lists::list(
        3,
        Some("Sign out"),
        vec![
            confirms(
                0,
                Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of Drive"),
                Dialog::SignOutApp,
            ),
            confirms(
                1,
                Row::new(Lead::Icon(&icons::SIGN_OUT), "Sign out of all Beebeeb apps").sub("On this PC"),
                Dialog::SignOutAllApps,
            ),
        ],
        Some("Signing out of Drive signs out Drive on this PC only. The other Beebeeb apps stay signed in."),
        t,
    );
    let delete = lists::list(
        4,
        Some("Delete"),
        vec![opens(
            4,
            0,
            Row::new(Lead::Icon(&icons::TRASH), "Delete your Beebeeb account").danger(),
            "DW-A58",
        )],
        Some("Deletes every product's data after 72 hours to undo. After that it cannot be undone."),
        t,
    );

    let confirm = match dialog {
        Some(Dialog::SignOutApp) => Some(sign_out_confirm(
            "Sign out of Drive?",
            "This signs out Drive on this PC only. Drive, VPN, Chat and Mail stay signed in. Your data stays encrypted on this PC; signing in again opens it.",
            "Sign out",
            false,
            t,
        )),
        Some(Dialog::SignOutAllApps) => Some(sign_out_confirm(
            "Sign out of all Beebeeb apps on this PC?",
            "Drive, Pass, VPN, Chat and Mail on this PC are signed out. Their data stays encrypted on this PC; signing in again opens it. Your other devices stay signed in.",
            "Sign out of all",
            true,
            t,
        )),
        _ => None,
    };
    let children = vec![
        set::heading("Sign-in and security", None, Some(checkup), t),
        app_lock,
        sign_in,
        devices,
        sign_out,
        delete,
    ];
    (set::pane(children, t), confirm)
}

/// DW-A57 and DW-A57a: the scope sentence is read before the buttons; Enter is Cancel.
fn sign_out_confirm(title: &str, words: &str, action: &str, all: bool, t: &Theme) -> El {
    let foot = vec![
        (
            set::button("btn-1", action, None, Look::Danger, Size::Medium, false, t)
                .label(format!("{action}, destructive"))
                .on_press(move |s: &mut State| {
                    if let Some(source) = s.account.source.as_mut() {
                        if all {
                            source.sign_out_all_apps();
                        } else {
                            source.sign_out_this_app();
                        }
                    }
                    s.account.dialog = None;
                    // Sign in: with the Continue as card after one app (DW-A02), without it
                    // after all of them (DW-A03).
                    super::go(s, if all { "DW-A03" } else { "DW-A02" });
                }),
            false,
        ),
        (
            set::button("btn-2", "Cancel", None, Look::Plain, Size::Medium, false, t)
                .default_button()
                .on_press(|s: &mut State| s.account.dialog = None),
            true,
        ),
    ];
    set::dialog(title, 440.0, title, words, Vec::new(), foot, t).modal(false, |s: &mut State| s.account.dialog = None)
}

fn factor_icon(kind: FactorKind) -> &'static rune::Icon {
    match kind {
        FactorKind::EmailVerified => &icons::MAIL,
        FactorKind::TwoFactor => &icons::SHIELD,
        FactorKind::StrongPassword => &icons::PADLOCK,
        FactorKind::RecoveryPhraseSaved => &icons::KEY,
        FactorKind::Passkeys => &icons::PASSKEY,
        FactorKind::PasswordNotInBreach => &icons::EYE_OFF,
        FactorKind::Sessions => &icons::DEVICE_PHONE,
    }
}

/// Security checkup (DW-A84, DW-A84a): the factors in one list with a score; what is left
/// comes first, with the button that fixes it. No label word: the count says it plainly.
pub fn checkup(source: &dyn AccountSource, t: &Theme) -> El {
    let factors = source.factors();
    let (done, of) = score(&factors);
    let left = of - done;
    let says = match left {
        0 => "Nothing to do.".to_string(),
        1 => "One thing to do.".to_string(),
        n => format!("{n} things to do."),
    };
    // The ring and the number in it are a picture; "N of M done" is the text.
    let summary = row()
        .key("score")
        .center_y()
        .gap(14.0)
        .child(
            el().key("ring-box")
                .a11y_hidden()
                .size(52.0, 52.0)
                .center()
                .child(set::ring("ring", 52.0, 4.0, done as f32 / of.max(1) as f32, t.text, t).absolute(0.0, 0.0))
                .child(kit::label("ring-label", done.to_string(), set::RING_COUNT, t.text)),
        )
        .child(
            col()
                .key("score-words")
                .gap(2.0)
                .child(kit::label(
                    "ring-head",
                    format!("{done} of {of} done"),
                    type_::HEAD,
                    t.text,
                ))
                .child(kit::label("ring-sub", says, set::SUB, t.text_secondary)),
        );

    let mut children = vec![
        set::heading(
            "Security checkup",
            Some("What keeps your account safe. Checked each time you open this page."),
            None,
            t,
        ),
        summary,
    ];
    let todo: Vec<&Factor> = factors.iter().filter(|f| !f.done).collect();
    let mut list = 0;
    if !todo.is_empty() {
        let rows = todo
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let frame = f.kind.fix_frame();
                let fix = set::button(
                    &format!("btn-{i}"),
                    f.kind.fix_label(),
                    None,
                    Look::Plain,
                    Size::Small,
                    false,
                    t,
                )
                .label(format!("{}, {}", f.says, f.kind.title()))
                .on_press(move |s: &mut State| super::go(s, frame));
                set::item(
                    list,
                    i,
                    &Row::new(Lead::Icon(factor_icon(f.kind)), f.kind.title())
                        .sub(f.says.clone())
                        .name(format!("{}, to do: {}", f.kind.title(), f.says)),
                    vec![fix],
                    t,
                )
            })
            .collect();
        children.push(lists::list(list, Some("To do"), rows, None, t));
        list += 1;
    }
    let rows: Vec<El> = factors
        .iter()
        .filter(|f| f.done)
        .enumerate()
        .map(|(i, f)| {
            let frame = f.kind.frame();
            set::item(
                list,
                i,
                &Row::new(Lead::Icon(factor_icon(f.kind)), f.kind.title())
                    .sub(f.says.clone())
                    .tail(Tail::Check)
                    .link(true),
                Vec::new(),
                t,
            )
            .on_press(move |s: &mut State| super::go(s, frame))
        })
        .collect();
    if !rows.is_empty() {
        children.push(lists::list(list, Some("Done"), rows, None, t));
    }
    set::pane(children, t)
}
