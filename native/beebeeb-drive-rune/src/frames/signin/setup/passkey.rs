//! Step 1, "Add a passkey" (DW-0201), with the account password (owner ruling D-P40-5: a passkey change proves the
//! password). The board draws the step without the password field, without a waiting state and without a refusal
//! line; all three are drawn here with the board's own parts and Settings › Passkeys' words (`FRAMES.md`).

use rune::{el, El};

use super::source::PasskeyRefused;
use super::view::{actions, ghost, primary};
use super::{icons, PasskeyStep, Ui, PASSWORD_KEY};
use crate::theme::tokens::{type_, Theme};
use crate::ui::au::{self, ButtonState, FieldSpec, FieldState, Keys};
use crate::ui::setup;
use crate::State;

/// Why the passkey was not added, in Settings › Passkeys' own words (`engine_host/passkeys.rs`, the toast of the
/// same refusal). A missing password is said by the field, not by a line.
pub(super) fn refused_words(why: PasskeyRefused) -> Option<&'static str> {
    Some(match why {
        PasskeyRefused::NoPassword => return None,
        PasskeyRefused::WrongPassword => "Incorrect password. Nothing was changed.",
        PasskeyRefused::Cancelled => "Adding the passkey was cancelled. Nothing was changed.",
        PasskeyRefused::Offline => "Beebeeb can't be reached. Nothing was changed.",
        PasskeyRefused::TooManyTries => "Too many tries. Wait a moment, then try again. Nothing was changed.",
        PasskeyRefused::NotOnThisPc => concat!(
            "Passkeys are not available on ",
            beebeeb_engine::device_word!(),
            ". Nothing was changed."
        ),
        PasskeyRefused::NotAdded => "The passkey was not added. Nothing was changed.",
    })
}

/// Step 1 with the account password (owner ruling D-P40-5: a passkey change proves the password; the board draws no
/// field). While Windows' dialog is open "Add a passkey" is busy; a refusal is one line, and "Not now" stays.
pub(super) fn step(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    // A missing password is said by the field (in error); a wrong one by the refusal line alone, so the step says it
    // once (focus goes to the field either way).
    let asks = ui.passkey_step == PasskeyStep::Refused(PasskeyRefused::NoPassword);
    let refused = match ui.passkey_step {
        PasskeyStep::Refused(why) => refused_words(why),
        _ => None,
    };
    let adding = if ui.passkey_step == PasskeyStep::Waiting {
        ButtonState::Busy
    } else {
        ButtonState::Enabled
    };
    // The same field and words as Settings › Passkeys' password (the sign-in kit's field: Esc stays this window's).
    let password = au::field(
        k,
        FieldSpec {
            label: "Password",
            icon: Some(&icons::PADLOCK),
            state: if asks { FieldState::Error } else { FieldState::Rest },
            message: Some(if asks {
                "Enter your password to confirm it's you."
            } else {
                "To confirm it's you."
            }),
            mono: false,
        },
        &ui.passkey_password,
        PASSWORD_KEY,
        |s: &mut State| &mut s.setup.as_mut().expect("the first-run window is open").passkey_password,
        None,
        t,
    );
    let rows = vec![
        setup::item(
            k,
            Some(&icons::FACE),
            "Saved by Windows",
            Some(concat!(
                "In Passwords or your password manager, behind ",
                beebeeb_engine::unlock_name!()
            )),
            |_| Vec::new(),
            None,
            false,
            t,
        ),
        setup::item(
            k,
            Some(&icons::PADLOCK),
            "Private",
            Some("Beebeeb can't use it to open your files by itself."),
            |_| Vec::new(),
            None,
            true,
            t,
        ),
    ];
    setup::column()
        .child(au::heading(
            "Add a passkey",
            Some("Sign in and unlock your files on a new device, with no codes. It doesn't replace your 12 words, so keep those safe."),
            false,
            t,
        ))
        .child(setup::list(k, rows, None, t))
        .child(password)
        .children(refused.map(|words| {
            el().key("passkey-refused")
                .role("alert")
                .w_full()
                .child(setup::note(k, words, type_::CAPTION, 0.0, t))
        }))
        .child(
            actions()
                .child(primary(k, "Add a passkey", adding, t, Ui::add_passkey))
                .child(ghost(k, "Not now", ButtonState::Enabled, t, Ui::not_now)),
        )
}
