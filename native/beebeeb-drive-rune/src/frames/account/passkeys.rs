//! Passkeys in settings (DW-A73 to DW-A76): what each passkey can do, adding one, the menu on
//! one (Rename, Remove) and the confirm before one is removed.
//!
//! Adding a passkey is the system's own dialog (Windows Hello and the Windows WebAuthn API,
//! behind `AccountSource::add_passkey`): no web view, and nothing of it is drawn here.

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_form::{self as fm, Field};
use crate::ui::account_set::{self as set, Lead, Look, Row, Size};
use crate::ui::kit;
use crate::ui::settings_list as lists;
use rune::{El, el};

use super::form::{self, Form, Problem};
use super::forms::button;
use super::source::{AccountSource, Passkey, Password, Refused};
use super::{AccountUi, Dialog, icons};

const LINE: &str =
    "An extra way to confirm it's you, and to unlock your data on a new device. It doesn't replace your 12 words.";

fn add_button(index: usize, t: &Theme) -> (El, bool) {
    (
        set::button(
            &format!("btn-{index}"),
            "Add a passkey",
            Some(&icons::PASSKEY),
            Look::Primary,
            Size::Medium,
            false,
            t,
        )
        .on_press(super::act(|ui: &mut AccountUi| {
            let source = ui.source.as_deref_mut()?;
            let f = &mut ui.form;
            let started = if source.passkeys_need_password() {
                if f.current.text.is_empty() {
                    f.problem = Some(Problem::WrongPassword);
                    form::focus(f, Some(form::CURRENT_KEY));
                    return None;
                }
                let started = source.add_passkey_confirmed(Password::from_field(&f.current.text));
                // The password has left the field once it is handed over.
                f.current = form::secure();
                started
            } else {
                source.add_passkey()
            };
            match started {
                Ok(()) => f.problem = None,
                Err(Refused::WrongPassword | Refused::NotConfirmed) => f.problem = Some(Problem::WrongPassword),
                Err(_) => f.problem = Some(Problem::Offline),
            }
            None
        })),
        false,
    )
}

/// "Your password" above Add and in the remove confirm, for a source that asks it (core's step-up): drawn only then,
/// so the board's own frames stay as drawn.
fn password_field(index: usize, f: &Form, t: &Theme) -> El {
    let wrong = f.problem == Some(Problem::WrongPassword);
    fm::field(
        index,
        &Field {
            label: "Password",
            icon: Some(&icons::PADLOCK),
            message: Some(if wrong {
                "Enter your password to confirm it's you."
            } else {
                "To confirm it's you."
            }),
            error: wrong,
            input_key: form::CURRENT_KEY,
        },
        &f.current,
        |s: &mut State| &mut s.account.form.current,
        t,
    )
}

pub fn pane(source: &dyn AccountSource, f: &Form, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let passkeys = source.passkeys();
    let (needs_password, can_add) = (source.passkeys_need_password(), source.can_add_passkey());
    if passkeys.is_empty() {
        // DW-A74: none yet.
        let mut children = vec![
            set::heading("Passkeys", None, None, t),
            fm::empty(&icons::PASSKEY, "No passkeys yet", LINE, t),
        ];
        if can_add {
            if needs_password {
                children.push(password_field(1, f, t));
            }
            children.push(fm::actions(vec![add_button(0, t)]));
        } else {
            children.push(no_dialog(t));
        }
        return (set::pane(children, t), None);
    }
    let about = match dialog {
        Some(Dialog::PasskeyMenu(id) | Dialog::RemovePasskey(id)) => Some(id.as_str()),
        _ => None,
    };
    let rows = passkeys
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let id = p.id.clone();
            let more = el()
                .key(format!("item-0-{i}-more"))
                .role("button")
                .label(format!("More for {}", p.name))
                .size(28.0, 28.0)
                .radius(6.0)
                .center()
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .child(kit::glyph(
                    format!("item-0-{i}-more-icon"),
                    &icons::MORE,
                    18.0,
                    t.text_tertiary,
                ))
                .on_press(move |s: &mut State| super::ask(s, Dialog::PasskeyMenu(id.clone())));
            set::item(
                0,
                i,
                &Row::new(Lead::Icon(&icons::PASSKEY), p.name.clone())
                    .sub(format!("{} · {}", p.added, p.does()))
                    .name(format!("{}, added {}, {}", p.name, p.added, p.does()))
                    .selected(about == Some(p.id.as_str())),
                vec![more],
                t,
            )
        })
        .collect();
    let mut list = rune::col()
        .key("passkeys")
        .w_full()
        .child(lists::list(0, None, rows, None, t).w_full());
    let mut overlay = None;
    match dialog {
        Some(Dialog::PasskeyMenu(id)) => {
            if let Some(at) = passkeys.iter().position(|p| &p.id == id) {
                list = list.child(menu(&passkeys[at], at, source.can_rename_passkey(), t));
            }
        }
        Some(Dialog::RemovePasskey(id)) => {
            overlay = passkeys
                .iter()
                .find(|p| &p.id == id)
                .map(|p| remove_confirm(p, &passkeys, needs_password.then(|| password_field(1, f, t)), t));
        }
        _ => {}
    }
    let mut children = vec![set::heading("Passkeys", Some(LINE), None, t), list];
    if let Some(id) = &f.renaming {
        children.push(rename(id, f, t));
    } else if dialog.is_none() {
        // Behind its menu and its confirm the board draws the page without the button.
        if !can_add {
            children.push(no_dialog(t));
        } else {
            if needs_password {
                children.push(password_field(1, f, t));
            }
            children.push(fm::actions(vec![add_button(0, t)]));
        }
    }
    if f.problem == Some(Problem::Offline) {
        children.push(
            kit::label(
                "note-0",
                "The passkey was not added. Nothing was changed.",
                fm::CAP,
                crate::ui::au::red_text(t),
            )
            .role("alert"),
        );
    }
    (set::pane(children, t), overlay)
}

/// The menu on one passkey (DW-A75): Rename, Remove. Under the row, 12 pt in from its end.
fn menu(passkey: &Passkey, at: usize, can_rename: bool, t: &Theme) -> El {
    let (rename_id, remove_id) = (passkey.id.clone(), passkey.id.clone());
    let name = passkey.name.clone();
    let remove = fm::pop_item(if can_rename { 1 } else { 0 }, "Remove", true, t)
        .on_press(move |s: &mut State| super::ask(s, Dialog::RemovePasskey(remove_id.clone())));
    let mut items = Vec::new();
    // Beebeeb has no route to rename a passkey: a source that says so gets no "Rename" (never a button that cannot work).
    if can_rename {
        items.push(fm::pop_item(0, "Rename", false, t).on_press(move |s: &mut State| {
            s.account.dialog = None;
            let f = &mut s.account.form;
            form::type_in(&mut f.rename, &name);
            f.renaming = Some(rename_id.clone());
            form::focus(f, Some(form::RENAME_KEY));
        }));
    }
    items.push(remove);
    fm::pop(&format!("{}, passkey", passkey.name), items, t)
        .pin(None, Some(40.0 * (at as f32 + 1.0)), Some(12.0), None)
        .modal(true, |s: &mut State| s.account.dialog = None)
}

/// Rename a passkey, here: Beebeeb's name for it only. Enter saves, Esc cancels.
fn rename(id: &str, f: &Form, t: &Theme) -> El {
    let id = id.to_string();
    let (cancel, cancel_b) = button(0, "Cancel", Look::Plain, false, t);
    let (save, save_b) = button(1, "Save", Look::Primary, false, t);
    rune::col()
        .key("rename")
        .w_full()
        .gap(14.0)
        .child(fm::field(
            0,
            &Field {
                label: "Passkey name",
                input_key: form::RENAME_KEY,
                ..Field::default()
            },
            &f.rename,
            |s: &mut State| &mut s.account.form.rename,
            t,
        ))
        .child(fm::actions(vec![
            (
                cancel.on_press(|s: &mut State| s.account.form.renaming = None),
                cancel_b,
            ),
            (
                save.default_button().on_press(move |s: &mut State| {
                    let ui = &mut s.account;
                    let name = ui.form.rename.text.trim().to_string();
                    if let Some(source) = ui.source.as_deref_mut() {
                        if source.rename_passkey(&id, &name) {
                            ui.form.renaming = None;
                        }
                    }
                }),
                save_b,
            ),
        ]))
}

/// Remove a passkey (DW-A76): the last one that can unlock the data gets the warning in bold.
fn remove_confirm(passkey: &Passkey, all: &[Passkey], password: Option<El>, t: &Theme) -> El {
    let title = format!("Remove {}?", passkey.name);
    let last_unlocker = passkey.can_unlock && !all.iter().any(|p| p.id != passkey.id && p.can_unlock);
    let paragraph = if last_unlocker {
        fm::lead_paragraph(
            "It's your only passkey that can unlock your data.",
            "On a new device you'd then need another signed-in device or your 12 words.",
            t,
        )
    } else {
        kit::label(
            "dlg-text-0",
            "You can still sign in with your other passkeys.",
            set::SUB,
            t.text_secondary,
        )
        .wrap()
        .w_full()
    };
    let id = passkey.id.clone();
    let (remove, remove_b) = button(0, "Remove", Look::Danger, false, t);
    let (cancel, cancel_b) = button(1, "Cancel", Look::Plain, false, t);
    set::dialog_with(
        &title,
        440.0,
        &title,
        paragraph,
        password.into_iter().collect(),
        vec![
            (
                remove
                    .label(format!("Remove {}, destructive", passkey.name))
                    .on_press(move |s: &mut State| {
                        let ui = &mut s.account;
                        let Some(source) = ui.source.as_deref_mut() else {
                            ui.dialog = None;
                            return;
                        };
                        if source.passkeys_need_password() {
                            // A password is needed: an empty field keeps the confirm open and asks for it.
                            if ui.form.current.text.is_empty() {
                                ui.form.problem = Some(Problem::WrongPassword);
                                form::focus(&mut ui.form, Some(form::CURRENT_KEY));
                                return;
                            }
                            source.remove_passkey_confirmed(&id, Password::from_field(&ui.form.current.text));
                            ui.form.current = form::secure();
                            ui.form.problem = None;
                        } else {
                            source.remove_passkey(&id);
                        }
                        ui.dialog = None;
                    }),
                remove_b,
            ),
            (
                // Enter is Cancel: the safe choice.
                cancel
                    .default_button()
                    .on_press(|s: &mut State| s.account.dialog = None),
                cancel_b,
            ),
        ],
        t,
    )
    .modal(false, |s: &mut State| s.account.dialog = None)
}

/// This PC has no passkey dialog: said once where "Add a passkey" would be.
fn no_dialog(t: &Theme) -> El {
    kit::label(
        "note-1",
        "Passkeys are not available on this PC.",
        fm::CAP,
        t.text_secondary,
    )
}
