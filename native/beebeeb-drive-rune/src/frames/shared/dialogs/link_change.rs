//! The card of Change expiry and Password on a link (DW-0511), and Change expiry on a file request.
//!
//! Expiry: Day, Week, Month or Never, as Files' Share dialog words them; one press sends the change and closes the
//! card (an answer Beebeeb refused is said over the lists, by the source's notice). Password: a secure field (set or
//! change; Remove password takes it off). The field is wiped when the card closes, however it closes, and the window
//! is kept out of screen captures while it shows ([`shows_secret`], checked in `shell::host`).
//!
//! Must not: draw, log or keep a typed password beyond the press that sends it.

use super::kit::{self, Field};
use super::{Dialog, dismiss};
use crate::State;
use crate::frames::files::actions_data::LinkExpiry;
use crate::frames::shared::parts::controls::{ButtonKind, ButtonSize, button_keyed, button_md};
use crate::frames::shared::source::LinkEdit;
use crate::theme::tokens::Theme;
use rune::{El, Tx, col, row};
use zeroize::Zeroizing;

/// The password field's key.
pub const LINK_PASSWORD_INPUT: &str = "link-password-input";
const WIDTH: f32 = 420.0;

/// Is the link password field on screen? The window is then kept out of screen captures (`shell::host` checks the
/// same dialog).
#[allow(dead_code)]
pub fn shows_secret(s: &State) -> bool {
    matches!(s.frames.shared.dialog, Some(Dialog::LinkPassword { .. }))
}

/// The hours a choice stands for (`None`: never).
fn hours(choice: LinkExpiry) -> Option<u32> {
    match choice {
        LinkExpiry::Day => Some(24),
        LinkExpiry::Week => Some(24 * 7),
        LinkExpiry::Month => Some(24 * 30),
        LinkExpiry::Never => None,
    }
}

/// The card of a `LinkExpiry` or `LinkPassword` dialog.
pub(super) fn card(s: &State, dialog: &Dialog, t: &Theme) -> El {
    match dialog {
        Dialog::LinkPassword { has_password, .. } => password_card(s, *has_password, t),
        _ => expiry_card(t),
    }
}

type Choose = fn(&mut State, &mut Tx);

fn expiry_card(t: &Theme) -> El {
    let choices: [(LinkExpiry, Choose); 4] = [
        (LinkExpiry::Day, |s, _| choose(s, LinkExpiry::Day)),
        (LinkExpiry::Week, |s, _| choose(s, LinkExpiry::Week)),
        (LinkExpiry::Month, |s, _| choose(s, LinkExpiry::Month)),
        (LinkExpiry::Never, |s, _| choose(s, LinkExpiry::Never)),
    ];
    let mut list = col().key("expiry-choices").gap(crate::theme::scale::space::S6).w_full();
    for (choice, press) in choices {
        let words = choice.choice();
        list = list.child(
            button_keyed(
                format!("expiry-{}", words.replace(' ', "-").to_lowercase()),
                words,
                ButtonKind::Outline,
                ButtonSize::Medium,
                None,
                t,
            )
            .w_full()
            .on_press_tx(press),
        );
    }
    let foot = row().center_y().justify_end().child(
        button_md("Cancel", ButtonKind::Outline, None, t)
            .autofocus()
            .on_press_tx(dismiss),
    );
    kit::card("link-change", WIDTH, Some(kit::title("Change expiry", t)), vec![list], foot, t)
}

fn password_card(s: &State, has_password: bool, t: &Theme) -> El {
    let title = if has_password { "Change password" } else { "Add a password" };
    let edit = &s.frames.shared.fields.link_password;
    let field = kit::field(
        &Field {
            key: "field-0",
            label: "Password for the link",
            placeholder: "",
            input_key: LINK_PASSWORD_INPUT,
            inner_w: WIDTH - 48.0 - 24.0,
        },
        edit,
        |s: &mut State| &mut s.frames.shared.fields.link_password,
        |input| input.autofocus().on_submit(|s: &mut State, _tx: &mut Tx| save(s)),
        t,
    );
    let note = crate::ui::kit::label(
        "link-password-note",
        "Anyone with the link needs it to open the file.",
        kit::NOTE,
        t.text_secondary,
    )
    .w_full();
    let ready = !edit.text.trim().is_empty();
    let save_btn = if ready {
        button_keyed("btn-save-password".into(), "Save", ButtonKind::Amber, ButtonSize::Medium, None, t)
            .on_press_tx(|s: &mut State, _tx: &mut Tx| save(s))
    } else {
        button_keyed("btn-save-password".into(), "Save", ButtonKind::Outline, ButtonSize::Medium, None, t)
    };
    let remove = button_keyed(
        "btn-remove-password".into(),
        "Remove password",
        ButtonKind::GhostDanger,
        ButtonSize::Medium,
        None,
        t,
    )
    .on_press_tx(|s: &mut State, _tx: &mut Tx| remove(s));
    let foot = row().center_y().justify_end().children(vec![
        remove,
        save_btn,
        button_md("Cancel", ButtonKind::Outline, None, t).on_press_tx(dismiss),
    ]);
    kit::card("link-change", WIDTH, Some(kit::title(title, t)), vec![note, field], foot, t)
}

/// Send what is typed as the link's password; the field is wiped at once and the card closes.
fn save(s: &mut State) {
    let Some(Dialog::LinkPassword { share_id, .. }) = s.frames.shared.dialog.clone() else {
        return;
    };
    let ui = &mut s.frames.shared;
    let typed = Zeroizing::new(ui.fields.link_password.text.clone());
    if typed.trim().is_empty() {
        return;
    }
    ui.source.change_link(&share_id, LinkEdit::Password(Some(typed)));
    ui.close_dialog();
}

/// Take the password off the link.
fn remove(s: &mut State) {
    let Some(Dialog::LinkPassword { share_id, .. }) = s.frames.shared.dialog.clone() else {
        return;
    };
    let ui = &mut s.frames.shared;
    ui.source.change_link(&share_id, LinkEdit::Password(None));
    ui.close_dialog();
}

/// Send the new expiry for the open dialog's row, and close the card.
fn choose(s: &mut State, choice: LinkExpiry) {
    let Some(Dialog::LinkExpiry { share_id }) = s.frames.shared.dialog.clone() else {
        return;
    };
    let ui = &mut s.frames.shared;
    // A link row changes its link; a request row changes its request.
    if ui.source.links().iter().any(|l| l.id == share_id) {
        ui.source.change_link(&share_id, LinkEdit::Expiry(hours(choice)));
    } else {
        ui.source.change_request_expiry(&share_id, hours(choice));
    }
    ui.close_dialog();
}
