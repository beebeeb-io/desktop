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
use crate::ui::menu;
use rune::{El, Tx, el};

use super::form::{self, Form, Problem};
use super::forms::button;
use super::source::{AccountSource, Passkey, Password, Refused};
use super::{AccountUi, Dialog, RowKey, icons};

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
                let started = source.add_passkey(Some(Password::from_field(&f.current.text)));
                // The password has left the field once it is handed over.
                f.current = form::secure();
                started
            } else {
                source.add_passkey(None)
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
            let id = p.id.clone();
            let row = set::item(
                0,
                i,
                &Row::new(Lead::Icon(&icons::PASSKEY), p.name.clone())
                    .sub(format!("{} · {}", p.added, p.does()))
                    .name(format!("{}, added {}, {}", p.name, p.added, p.does()))
                    .selected(about == Some(p.id.as_str())),
                vec![more],
                t,
            );
            // The row's own keys, with the focus on its More button (the row itself is text): Delete is
            // Remove (its confirm), F2 Rename where this source can rename, the Menu key or Shift+F10 the menu.
            super::row_keys(row, move |s: &mut State, k| match k {
                RowKey::Remove => {
                    super::ask(s, Dialog::RemovePasskey(id.clone()));
                    true
                }
                RowKey::Rename => start_rename(s, id.clone()),
                RowKey::Menu => {
                    super::ask(s, Dialog::PasskeyMenu(id.clone()));
                    true
                }
            })
        })
        .collect();
    let mut list = rune::col()
        .key("passkeys")
        .w_full()
        .child(crate::ui::settings_list::list(0, None, rows, None, t).w_full());
    let mut overlay = None;
    match dialog {
        Some(Dialog::PasskeyMenu(id)) => {
            if let Some(at) = passkeys.iter().position(|p| &p.id == id) {
                list = list.child(menu(at, source.can_rename_passkey(), t));
            }
        }
        Some(Dialog::RemovePasskey(id)) => {
            overlay = passkeys
                .iter()
                .enumerate()
                .find(|(_, p)| &p.id == id)
                .map(|(i, p)| remove_confirm(p, i, &passkeys, needs_password.then(|| password_field(1, f, t)), t));
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

/// The menu on one passkey (DW-A75, `ui::menu`): Rename, Remove. Under the row, 12 pt in from its end. A
/// source that cannot rename (no route, or not bound) gets no "Rename" (never a button that cannot work).
fn menu(at: usize, can_rename: bool, t: &Theme) -> El {
    let mut entries = Vec::new();
    if can_rename {
        entries.push(menu::Entry::item("rename", "Rename"));
    }
    entries.push(menu::Entry::Item {
        id: "remove",
        label: "Remove",
        shortcut: None,
        checked: false,
        danger: true,
        off: false,
    });
    menu::menu(
        "pop-0",
        "Passkey",
        180.0,
        &entries,
        None,
        t,
        |s: &mut State, i: usize, _: &mut Tx| {
            let Some(Dialog::PasskeyMenu(id)) = s.account.dialog.clone() else {
                return;
            };
            let can_rename = s.account.source.as_deref().is_some_and(|x| x.can_rename_passkey());
            if can_rename && i == 0 {
                start_rename(s, id);
            } else {
                super::ask(s, Dialog::RemovePasskey(id));
            }
        },
        |_: &mut State, _: usize| {},
        |s: &mut State, _: &mut Tx| s.account.dialog = None,
    )
    .pin(None, Some(40.0 * (at as f32 + 1.0)), Some(12.0), None)
}

/// Start renaming the passkey `id` (the menu's Rename, F2 on its row): the field under the list, filled with its
/// name. `false`: this source cannot rename, nothing happens.
fn start_rename(s: &mut State, id: String) -> bool {
    let Some(source) = s.account.source.as_deref().filter(|x| x.can_rename_passkey()) else {
        return false;
    };
    let name = source.passkeys().into_iter().find(|p| p.id == id).map(|p| p.name).unwrap_or_default();
    s.account.dialog = None;
    let f = &mut s.account.form;
    form::type_in(&mut f.rename, &name);
    f.renaming = Some(id);
    form::focus(f, Some(form::RENAME_KEY));
    true
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
///
/// The opener is the row's More button (`item-0-<at>-more`, the menu's Remove or Delete on the row). Once the
/// passkey is removed the next row has that key: focus goes there, else into the list.
fn remove_confirm(passkey: &Passkey, at: usize, all: &[Passkey], password: Option<El>, t: &Theme) -> El {
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
    let card = set::dialog_with(
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
                            source.remove_passkey(&id, Some(Password::from_field(&ui.form.current.text)));
                            ui.form.current = form::secure();
                            ui.form.problem = None;
                        } else {
                            source.remove_passkey(&id, None);
                        }
                        ui.dialog = None;
                    }),
                remove_b,
            ),
            (
                // Enter is Cancel: the safe choice. Cancel and Esc wipe the password typed into the confirm.
                cancel.default_button().on_press(close_wiping),
                cancel_b,
            ),
        ],
        t,
    );
    super::modal_back(card, &format!("item-0-{at}-more"), "list-0", close_wiping)
}

/// The remove confirm closes with nothing done: what was typed into its password field goes with it.
fn close_wiping(s: &mut State) {
    s.account.dialog = None;
    form::arrive(&mut s.account);
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
