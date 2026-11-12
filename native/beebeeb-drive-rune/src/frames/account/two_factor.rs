//! Two-factor in settings: its status (DW-A71), setting it up (DW-A69), the backup codes shown
//! once (DW-A70), and the confirm that takes a current code (DW-A72; DW-A78 uses it too).
//!
//! The set-up key and the backup codes are secrets: `ui::account_form` draws each group as one
//! secure element; copying goes through Rune's secret clipboard, which clears after 60 seconds.

use std::time::Duration;

use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::account_form as fm;
use crate::ui::account_set::{self as set, Lead, Look, Row, Size, Tail};
use crate::ui::au::Tone;
use crate::ui::kit;
use crate::ui::settings_list as lists;
use rune::{El, Tx, row};

use super::form::{self, Form, Problem};
use super::forms::{button, code_error};
use super::source::{AccountSource, CodeRefused};
use super::{AccountUi, CodeFor, Dialog, Pane, icons};

/// The clipboard is cleared 60 seconds after a secret was copied (the board's rule).
const CLIPBOARD_CLEARS: Duration = Duration::from_secs(60);

pub fn pane(pane: Pane, source: &dyn AccountSource, f: &Form, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    match (pane, &f.setup, &f.codes) {
        (Pane::TwoFactorSetup, Some(_), _) => (setup(f, t), None),
        (Pane::BackupCodes, _, Some(_)) => (
            backup_codes(f, t),
            (dialog == Some(&Dialog::LeaveCodes)).then(|| leave_codes(t)),
        ),
        _ => status(source, f, dialog, t),
    }
}

/// Two-factor authentication (DW-A69): the QR code, the key, the first code.
fn setup(f: &Form, t: &Theme) -> El {
    let Some(setup) = &f.setup else {
        return set::pane(Vec::new(), t);
    };
    let complete = form::code(f, 6).len() == 6;
    let copy = fm::link(0, "Copy key", fm::CAP, false, t)
        .w_full()
        .on_press_tx(|s: &mut State, tx: &mut Tx| {
            let f = &mut s.account.form;
            if let Some(setup) = &f.setup {
                f.copied = Some(tx.copy_secret(setup.key.expose(), CLIPBOARD_CLEARS));
            }
        });
    let (cancel, cancel_b) = button(0, "Cancel", Look::Plain, false, t);
    let (turn_on_el, turn_on_b) = button(1, "Turn on", Look::Primary, !complete, t);
    let turn_on_el = if complete {
        turn_on_el.default_button().on_press(super::act(turn_on))
    } else {
        turn_on_el
    };
    let mut children = vec![
        set::heading(
            "Two-factor authentication",
            Some("Scan the QR code with your authenticator app, then enter the 6-digit code."),
            None,
            t,
        ),
        fm::setup(&setup.modules, setup.key.expose(), copy, t),
        fm::digits(
            0,
            6,
            &f.code,
            form::CODE_KEY,
            "6-digit code",
            |s: &mut State| &mut s.account.form.code,
            t,
        ),
    ];
    if f.problem.is_some() {
        children.push(code_error("Invalid code. Try again.", t));
    }
    children.push(fm::actions(vec![
        (cancel.on_press(|s: &mut State| super::go(s, "DW-A52")), cancel_b),
        (turn_on_el, turn_on_b),
    ]));
    set::pane(children, t)
}

/// Checks the first code; on success two-factor is on and the backup codes are shown once.
pub(super) fn turn_on(ui: &mut AccountUi) -> Option<&'static str> {
    let code = form::code(&ui.form, 6);
    match ui.source.as_deref_mut()?.turn_on_two_factor(&code) {
        Ok(codes) => {
            ui.form.codes = Some(codes);
            ui.pane = Pane::BackupCodes;
            Some("DW-A70")
        }
        Err(_) => {
            ui.form.problem = Some(Problem::WrongCode);
            None
        }
    }
}

/// The codes as they are copied and saved: numbered, under a heading.
fn codes_text(f: &Form) -> String {
    let mut text = String::from("Beebeeb backup codes\n");
    for (i, code) in f.codes.iter().flatten().enumerate() {
        text.push_str(&format!("{}. {}\n", i + 1, code.expose()));
    }
    text
}

/// Save your backup codes (DW-A70): shown on this page only.
fn backup_codes(f: &Form, t: &Theme) -> El {
    let shown: Vec<&str> = f.codes.iter().flatten().map(|c| c.expose()).collect();
    let copy = set::button(
        "btn-0",
        "Copy all codes",
        Some(&icons::COPY),
        Look::Plain,
        Size::Small,
        false,
        t,
    )
    .on_press_tx(|s: &mut State, tx: &mut Tx| {
        let f = &mut s.account.form;
        let text = codes_text(f);
        f.copied = Some(tx.copy_secret(text, CLIPBOARD_CLEARS));
    });
    let save = set::button("btn-1", "Save as text…", None, Look::Plain, Size::Small, false, t).on_press_tx(
        |_: &mut State, tx: &mut Tx| {
            let options = rune::FileDialogOptions {
                title: "Save your backup codes".into(),
                file_name: Some("beebeeb-backup-codes.txt".into()),
                ..Default::default()
            }
            .filter("Text", &["txt"]);
            tx.save_file(options, |s: &mut State, answer, _: &mut Tx| {
                let f = &mut s.account.form;
                if let Ok(Some(path)) = answer {
                    f.saved = Some(match std::fs::write(&path, codes_text(f)) {
                        Ok(()) => Ok(path.display().to_string()),
                        Err(e) => Err(e.to_string()),
                    });
                }
            });
        },
    );
    let (done, done_b) = button(2, "I've saved them", Look::Primary, false, t);
    let mut children = vec![
        set::heading(
            "Save your backup codes",
            Some("They won't be shown again. Each one works once if you lose your authenticator."),
            None,
            t,
        ),
        fm::codes(&shown, t),
        row().key("codes-actions").w_full().gap(8.0).child(copy).child(save),
    ];
    if let Some(saved) = &f.saved {
        let (words, ink) = match saved {
            Ok(path) => (format!("Saved to {path}."), t.text_tertiary),
            Err(why) => (format!("Not saved: {why}"), crate::ui::au::red_text(t)),
        };
        children.push(kit::label("saved", words, fm::CAP, ink).role("status").wrap().w_full());
    }
    children.push(
        kit::label(
            "note-0",
            "Keep them apart from this PC: in a password manager or printed.",
            fm::CAP,
            t.text_tertiary,
        )
        .w_full(),
    );
    children.push(fm::actions(vec![(
        done.default_button().on_press(|s: &mut State| super::go(s, "DW-A71")),
        done_b,
    )]));
    set::pane(children, t)
}

/// Leaving the codes without "I've saved them": the board's own question.
fn leave_codes(t: &Theme) -> El {
    let (leave, leave_b) = button(3, "I've saved them", Look::Plain, false, t);
    let (stay, stay_b) = button(4, "Go back", Look::Plain, false, t);
    set::dialog(
        "Did you save your backup codes?",
        440.0,
        "Did you save your backup codes?",
        "They won't be shown again.",
        Vec::new(),
        vec![
            (leave.on_press(|s: &mut State| super::go(s, "DW-A71")), leave_b),
            (
                stay.default_button().on_press(|s: &mut State| s.account.dialog = None),
                stay_b,
            ),
        ],
        t,
    )
    .modal(false, |s: &mut State| s.account.dialog = None)
}

/// Two-factor authentication (DW-A71): on since when, the codes left, the ways to change it.
fn status(source: &dyn AccountSource, f: &Form, dialog: Option<&Dialog>, t: &Theme) -> (El, Option<El>) {
    let two = source.two_factor();
    let confirm = match dialog {
        Some(Dialog::Code(code_for)) => Some(code_dialog(*code_for, f, t)),
        _ => None,
    };
    let mut children = Vec::new();
    // Behind the confirm the board draws the page without its heading (DW-A72).
    if confirm.is_none() {
        children.push(set::heading("Two-factor authentication", None, None, t));
    }
    if !two.on {
        // Off: the one thing to do here is to turn it on (the set-up page, DW-A69).
        let turn_on = set::button("btn-0", "Turn on", None, Look::Plain, Size::Small, false, t)
            .on_press(|s: &mut State| super::go(s, "DW-A69"));
        children.push(lists::list(
            0,
            None,
            vec![set::item(
                0,
                0,
                &Row::new(Lead::Icon(&icons::SHIELD), "Two-factor authentication")
                    .tail(Tail::Pill {
                        label: "Off".into(),
                        tone: Tone::Neutral,
                        dot: false,
                    })
                    .name("Two-factor off"),
                vec![turn_on],
                t,
            )],
            Some("A new sign-in needs only your password. A second factor keeps out someone who has it."),
            t,
        ));
        return (set::pane(children, t), None);
    }
    let fact = |index: usize, title: &str, value: &str| {
        set::item(
            0,
            index,
            &Row::new(Lead::None, title).tail(Tail::Value(value.into())),
            Vec::new(),
            t,
        )
    };
    children.push(lists::list(
        0,
        None,
        vec![
            set::item(
                0,
                0,
                &Row::new(Lead::Icon(&icons::SHIELD), "Two-factor authentication")
                    .tail(Tail::Pill {
                        label: "On".into(),
                        tone: Tone::Green,
                        dot: true,
                    })
                    .name("Two-factor on"),
                Vec::new(),
                t,
            ),
            fact(1, "Added", &two.added),
            fact(2, "Last used", &two.last_used),
        ],
        None,
        t,
    ));
    // At 2 codes or fewer the row is red and says what to do.
    let low = two.codes_left <= 2;
    let mut left = Row::new(Lead::Icon(&icons::KEY), "Codes left")
        .tail(Tail::Value(format!("{} of {}", two.codes_left, two.codes_total)));
    if low {
        left = left.sub("Make new codes before you run out.").danger();
    }
    let asks = |list: usize, index: usize, it: Row, code_for: CodeFor| {
        set::item(list, index, &it, Vec::new(), t).on_press(move |s: &mut State| super::ask(s, Dialog::Code(code_for)))
    };
    children.push(lists::list(
        1,
        Some("Backup codes"),
        vec![
            set::item(1, 0, &left, Vec::new(), t),
            asks(
                1,
                1,
                Row::new(Lead::Icon(&icons::REFRESH), "Make new backup codes").link(true),
                CodeFor::NewBackupCodes,
            ),
        ],
        Some("Each backup code works once. Making new codes stops the old ones working."),
        t,
    ));
    children.push(lists::list(
        2,
        None,
        vec![
            asks(
                2,
                0,
                Row::new(Lead::Icon(&icons::REFRESH), "Move to a new authenticator").link(true),
                CodeFor::MoveAuthenticator,
            ),
            asks(
                2,
                1,
                Row::new(Lead::Icon(&icons::TRASH), "Turn off two-factor")
                    .danger()
                    .link(false),
                CodeFor::TurnOffTwoFactor,
            ),
        ],
        Some("Lost your authenticator? Sign in with a backup code, another signed-in device or your recovery phrase."),
        t,
    ));
    (set::pane(children, t), confirm)
}

/// The confirm that takes a current code (DW-A72, DW-A78, and the two the board names without
/// drawing them: new backup codes, a new authenticator). Its action is the default button once
/// six digits are typed; Esc cancels.
pub(super) fn code_dialog(code_for: CodeFor, f: &Form, t: &Theme) -> El {
    let complete = form::code(f, 6).len() == 6;
    let destructive = matches!(code_for, CodeFor::TurnOffTwoFactor | CodeFor::StopActivity);
    let action = form::code_action(code_for);
    let (act_el, act_b) = button(
        0,
        action,
        if destructive { Look::Danger } else { Look::Primary },
        false,
        t,
    );
    let mut act_el = act_el.on_press(super::act(confirm_code));
    if destructive {
        act_el = act_el.label(format!("{action}, destructive"));
    }
    if complete {
        act_el = act_el.default_button();
    }
    let (cancel, cancel_b) = button(1, "Cancel", Look::Plain, false, t);
    let mut extra = vec![fm::digits(
        0,
        6,
        &f.code,
        form::CODE_KEY,
        "6-digit code",
        |s: &mut State| &mut s.account.form.code,
        t,
    )];
    if f.problem.is_some() {
        extra.push(code_error("Incorrect code. Try again.", t));
    }
    let foot = vec![
        (act_el, act_b),
        (
            cancel.on_press(|s: &mut State| {
                s.account.dialog = None;
                form::arrive(&mut s.account);
            }),
            cancel_b,
        ),
    ];
    let (title, paragraph) = match code_for {
        CodeFor::TurnOffTwoFactor => (
            "Turn off two-factor?",
            plain(
                "A new sign-in will then need only your password, which is less safe. Your backup codes stop working too. Enter a code from your authenticator to confirm.",
                t,
            ),
        ),
        CodeFor::NewBackupCodes => (
            "Make new backup codes?",
            plain(
                "The old codes stop working. Enter a code from your authenticator to confirm.",
                t,
            ),
        ),
        CodeFor::MoveAuthenticator => (
            "Move to a new authenticator?",
            plain(
                "Enter a code from your current authenticator. Then you scan a new QR code, and the old authenticator and backup codes stop working.",
                t,
            ),
        ),
        CodeFor::StopActivity => (
            "Turn off security logging?",
            fm::lead_paragraph(
                "You may not notice if someone else gets into your account.",
                "Nothing new is recorded, and what was recorded is deleted within 30 days. This can't be undone.",
                t,
            ),
        ),
    };
    set::dialog_with(title, 440.0, title, paragraph, extra, foot, t).modal(false, |s: &mut State| {
        s.account.dialog = None;
        form::arrive(&mut s.account);
    })
}

fn plain(words: &str, t: &Theme) -> El {
    kit::label("dlg-text-0", words, set::SUB, t.text_secondary)
        .wrap()
        .w_full()
}

/// The press of a code confirm's action: nothing changes unless the server takes the code.
fn confirm_code(ui: &mut AccountUi) -> Option<&'static str> {
    let Some(Dialog::Code(code_for)) = ui.dialog.clone() else {
        return None;
    };
    let code = form::code(&ui.form, 6);
    let source = ui.source.as_deref_mut()?;
    let outcome: Result<&'static str, CodeRefused> = if code.len() < 6 {
        Err(CodeRefused::Wrong)
    } else {
        match code_for {
            CodeFor::TurnOffTwoFactor => source.turn_off_two_factor(&code).map(|()| "DW-A52"),
            CodeFor::MoveAuthenticator => source.check_code(&code).map(|()| "DW-A69"),
            CodeFor::StopActivity => source.stop_activity(&code).map(|()| "DW-A79"),
            CodeFor::NewBackupCodes => source.new_backup_codes(&code).map(|codes| {
                ui.form.codes = Some(codes);
                ui.pane = Pane::BackupCodes;
                "DW-A70"
            }),
        }
    };
    match outcome {
        Ok(frame) => Some(frame),
        Err(_) => {
            ui.form.problem = Some(Problem::WrongCode);
            form::focus(&mut ui.form, Some(form::CODE_KEY));
            None
        }
    }
}
