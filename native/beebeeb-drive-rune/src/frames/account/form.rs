//! What the account pages with a field, a code or a secret hold while they are open: the typed
//! text, what went wrong, the step-up's confirmation between two steps, and the secrets a page
//! shows once.
//!
//! Every password field is a secure `TextEdit` (Rune draws bullets, keeps the characters out
//! of the element and accessibility trees, and wipes the buffer when it is dropped). A page's
//! secrets go when the page goes: [`arrive`] drops them on every move, and `super::sync` drops
//! the whole form when the route leaves the section.

use rune::{SecretCopy, TextEdit};

use super::source::{Secret, Setup, StepUp};
use super::{AccountUi, CodeFor, Dialog, Pane};

/// The text inputs' keys (tests, `Tx::focus`).
pub const NAME_KEY: &str = "acct-name";
pub const EMAIL_KEY: &str = "acct-email";
pub const CURRENT_KEY: &str = "acct-password";
pub const NEW_KEY: &str = "acct-new-password";
pub const AGAIN_KEY: &str = "acct-new-password-again";
pub const CODE_KEY: &str = "acct-code";
pub const RENAME_KEY: &str = "acct-passkey-name";

/// What went wrong with the last press. Nothing changed in any of these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    EmptyName,
    EmailInUse,
    NotAnAddress,
    WrongPassword,
    /// The vault is locked: "Unlock first, then change your password."
    Locked,
    Offline,
    Mismatch,
    Weak,
    WrongCode,
    /// "I've opened the link", and the link has not been used.
    LinkNotUsed,
}

pub struct Form {
    pub name: TextEdit,
    pub email: TextEdit,
    /// The current password (secure): change email, change password, delete the account.
    pub current: TextEdit,
    pub new: TextEdit,
    pub again: TextEdit,
    /// The digits of a code (secure, shown): the email's 8, the authenticator's 6, a backup code's 8.
    pub code: TextEdit,
    /// A passkey's new name, while it is renamed.
    pub rename: TextEdit,
    pub problem: Option<Problem>,
    /// The proof of the password between step 1 and step 2 of deleting the account. Used once.
    pub confirmation: Option<StepUp>,
    /// The two-factor set-up that waits for its first code (DW-A69).
    pub setup: Option<Setup>,
    /// The backup codes, shown once (DW-A70).
    pub codes: Option<Vec<Secret>>,
    /// Step 2 of deleting takes an 8-digit backup code instead of the authenticator's 6.
    pub backup_code: bool,
    /// The passkey that is being renamed (the server's id).
    pub renaming: Option<String>,
    /// The secret Rune copied last from these pages (it clears the clipboard after 60 s).
    pub copied: Option<SecretCopy>,
    /// The file the backup codes were saved to, or why not: said under the buttons.
    pub saved: Option<Result<String, String>>,
}

fn secure() -> TextEdit {
    TextEdit::new("").secure()
}

fn code_edit() -> TextEdit {
    TextEdit::new("").secure().revealed(true)
}

impl Default for Form {
    fn default() -> Self {
        Self {
            name: TextEdit::new(""),
            email: TextEdit::new(""),
            current: secure(),
            new: secure(),
            again: secure(),
            code: code_edit(),
            rename: TextEdit::new(""),
            problem: None,
            confirmation: None,
            setup: None,
            codes: None,
            backup_code: false,
            renaming: None,
            copied: None,
            saved: None,
        }
    }
}

/// Put `text` in a field with the caret at its end (a page that opens filled, and scenes).
pub fn type_in(edit: &mut TextEdit, text: &str) {
    edit.text.clear();
    edit.text.push_str(text);
    edit.anchor = edit.text.len();
    edit.focus = edit.text.len();
}

/// DW-A67 and DW-A68 as the board draws them: the three fields filled, none in focus.
pub fn fill_passwords(form: &mut Form) {
    type_in(&mut form.current, "............");
    type_in(&mut form.new, "..............");
    type_in(&mut form.again, "..............");
    focus(form, None);
}

/// Give the keyboard to the field keyed `key` (Rune reads `TextEdit::focused`), and to no other.
pub fn focus(form: &mut Form, key: Option<&str>) {
    for (edit, its) in [
        (&mut form.name, NAME_KEY),
        (&mut form.email, EMAIL_KEY),
        (&mut form.current, CURRENT_KEY),
        (&mut form.new, NEW_KEY),
        (&mut form.again, AGAIN_KEY),
        (&mut form.code, CODE_KEY),
        (&mut form.rename, RENAME_KEY),
    ] {
        edit.focused = key == Some(its);
    }
}

/// The digits typed so far, at most `count`.
pub fn code(form: &Form, count: usize) -> String {
    form.code
        .text
        .chars()
        .filter(char::is_ascii_digit)
        .take(count)
        .collect()
}

/// Makes the form ready for what `ui` shows now (a page that was just opened, a confirm that
/// was just opened or closed): the last problem goes, the secrets of other pages go, the
/// field a person types in first gets the keyboard.
pub fn arrive(ui: &mut AccountUi) {
    let f = &mut ui.form;
    f.problem = None;
    f.renaming = None;
    f.saved = None;
    focus(f, None);
    if ui.pane != Pane::TwoFactorSetup {
        f.setup = None;
    }
    if ui.pane != Pane::BackupCodes {
        f.codes = None;
    }
    if !matches!(ui.pane, Pane::Delete | Pane::DeleteFactor) {
        f.confirmation = None;
    }
    if !matches!(ui.pane, Pane::Email | Pane::EmailCode) {
        f.email = TextEdit::new("");
    }
    if ui.pane != Pane::Password {
        (f.new, f.again) = (secure(), secure());
    }
    f.code = code_edit();
    if let Some(dialog) = &ui.dialog {
        if matches!(dialog, Dialog::Code(_)) {
            focus(f, Some(CODE_KEY));
        }
        return;
    }
    match ui.pane {
        Pane::Name => {
            let name = ui.source.as_deref().map(|s| s.identity().name).unwrap_or_default();
            type_in(&mut f.name, &name);
            focus(f, Some(NAME_KEY));
        }
        Pane::Email => {
            f.current = secure();
            f.email.focus = f.email.text.len();
            f.email.anchor = f.email.focus;
            focus(f, Some(EMAIL_KEY));
        }
        Pane::Password | Pane::Delete => {
            f.current = secure();
            f.confirmation = None;
            focus(f, Some(CURRENT_KEY));
        }
        Pane::EmailCode | Pane::DeleteFactor => {
            f.current = secure();
            f.backup_code = false;
            focus(f, Some(CODE_KEY));
        }
        Pane::TwoFactorSetup => {
            f.current = secure();
            if f.setup.is_none() {
                f.setup = ui.source.as_deref_mut().and_then(|s| s.begin_two_factor(None).ok());
            }
            focus(f, Some(CODE_KEY));
        }
        _ => f.current = secure(),
    }
}

/// What a code's confirm does when the code is right, in words for its button.
pub fn code_action(code_for: CodeFor) -> &'static str {
    match code_for {
        CodeFor::TurnOffTwoFactor | CodeFor::StopActivity => "Turn off",
        CodeFor::NewBackupCodes => "Make new codes",
        CodeFor::MoveAuthenticator => "Continue",
    }
}
