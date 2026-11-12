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
use super::source::{AccountSource, Passkey};
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
            if ui.source.as_deref_mut()?.add_passkey().is_err() {
                ui.form.problem = Some(Problem::Offline);
            }
            None
        })),
        false,
    )
}

pub fn pane(source: &dyn AccountSource, f: &Form, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let passkeys = source.passkeys();
    if passkeys.is_empty() {
        // DW-A74: none yet.
        return (
            set::pane(
                vec![
                    set::heading("Passkeys", None, None, t),
                    fm::empty(&icons::PASSKEY, "No passkeys yet", LINE, t),
                    fm::actions(vec![add_button(0, t)]),
                ],
                t,
            ),
            None,
        );
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
    let mut list = el().key("passkeys").w_full().child(lists::list(0, None, rows, None, t));
    let mut overlay = None;
    match dialog {
        Some(Dialog::PasskeyMenu(id)) => {
            if let Some(at) = passkeys.iter().position(|p| &p.id == id) {
                list = list.child(menu(&passkeys[at], at, t));
            }
        }
        Some(Dialog::RemovePasskey(id)) => {
            overlay = passkeys
                .iter()
                .find(|p| &p.id == id)
                .map(|p| remove_confirm(p, &passkeys, t));
        }
        _ => {}
    }
    let mut children = vec![set::heading("Passkeys", Some(LINE), None, t), list];
    if let Some(id) = &f.renaming {
        children.push(rename(id, f, t));
    } else if dialog.is_none() {
        // Behind its menu and its confirm the board draws the page without the button.
        children.push(fm::actions(vec![add_button(0, t)]));
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
fn menu(passkey: &Passkey, at: usize, t: &Theme) -> El {
    let (rename_id, remove_id) = (passkey.id.clone(), passkey.id.clone());
    let name = passkey.name.clone();
    fm::pop(
        &format!("{}, passkey", passkey.name),
        vec![
            fm::pop_item(0, "Rename", false, t).on_press(move |s: &mut State| {
                s.account.dialog = None;
                let f = &mut s.account.form;
                form::type_in(&mut f.rename, &name);
                f.renaming = Some(rename_id.clone());
                form::focus(f, Some(form::RENAME_KEY));
            }),
            fm::pop_item(1, "Remove", true, t)
                .on_press(move |s: &mut State| super::ask(s, Dialog::RemovePasskey(remove_id.clone()))),
        ],
        t,
    )
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
fn remove_confirm(passkey: &Passkey, all: &[Passkey], t: &Theme) -> El {
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
        Vec::new(),
        vec![
            (
                remove
                    .label(format!("Remove {}, destructive", passkey.name))
                    .on_press(move |s: &mut State| {
                        if let Some(source) = s.account.source.as_deref_mut() {
                            source.remove_passkey(&id);
                        }
                        s.account.dialog = None;
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
