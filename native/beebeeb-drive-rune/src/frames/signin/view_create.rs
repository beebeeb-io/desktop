//! Create an account, drawn: the email (DW-A31), the code (DW-A32 to DW-A34), the terms
//! (DW-A35) and the password (DW-A36 to DW-A38). `view_create_phrase.rs` draws the steps after
//! them. Every string is the board's; a state the board gives a rule and no drawing for uses
//! the rule's own words.

use rune::{El, col, el};

use super::create::{CODE_KEY, EmailProblem, Step};
use super::create_kit::{self as ck, Field, cact, changed};
use super::create_source::{CodeProblem, PasswordProblem, SendProblem};
use super::steps::CONFIRM_KEY;
use super::view::actions;
use super::view_steps::{escape_first, open};
use super::{EMAIL_KEY, Intent, PASSWORD_KEY, Ui, icons, view_create_phrase as later};
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::au::{self, ButtonKind, ButtonSize, ButtonState, Keys};
use crate::ui::kit;

const NOT_SENT: &str = "We could not send the email. Check your connection and try again.";
const BREACH_NOTE: &str = "Checked against known breaches on Beebeeb's own servers. Only a short hash prefix leaves this PC, never the password.";
const BREACH_UNREACHABLE: &str = "We could not check this password against known breaches, and we do not let you continue without that check. Try again.";

pub fn view(ui: &Ui, step: Step, k: &mut Keys, t: &Theme) -> El {
    match step {
        Step::Email => email(ui, k, t),
        Step::Verify => verify(ui, k, t),
        Step::Terms => terms(ui, k, t),
        Step::Password => password(ui, k, t),
        Step::Phrase => later::phrase(ui, k, t),
        Step::Confirm => later::confirm(ui, k, t),
        Step::Device => later::device(ui, k, t),
        Step::Creating => later::creating(ui, k, t),
        Step::Ready => later::ready(ui, k, t),
    }
}

/// The back link and the progress line every step starts with.
pub(super) fn head(back: &str, step: usize, name: &str, t: &Theme) -> El {
    au::column()
        .child(au::back_link(back, &icons::BACK, t).on_press_tx(cact(Ui::create_back)))
        .child(ck::steps_line(step, name, t))
}

/// The amber button at the foot of a step. Enter presses it while it is on.
pub(super) fn primary(k: &mut Keys, label: &str, state: ButtonState, to: fn(&mut Ui), t: &Theme) -> El {
    let b = au::button(k, label, None, ButtonKind::Primary, ButtonSize::Lg, state, t);
    if state == ButtonState::Enabled {
        b.on_press_tx(cact(to)).default_button()
    } else {
        b
    }
}

pub(super) fn state(busy: bool, can: bool) -> ButtonState {
    if busy {
        ButtonState::Busy
    } else if can {
        ButtonState::Enabled
    } else {
        ButtonState::Disabled
    }
}

fn send_words(problem: SendProblem) -> &'static str {
    match problem {
        SendProblem::RateLimited => "Too many requests for this address. Try again later.",
        SendProblem::WebOnly => "Create your account on the web for now.",
        SendProblem::Offline | SendProblem::Other => NOT_SENT,
    }
}

// ---------------------------------------------------------------------------
// DW-A31
// ---------------------------------------------------------------------------

fn email(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let c = ui.create_state();
    let problem = ui.email_problem().map(|problem| match problem {
        EmailProblem::NotAnAddress => "Enter a valid email address.".to_string(),
        // The page the server names is written out; this window has no way to open a page in the browser yet.
        EmailProblem::Send(SendProblem::WebOnly) => match ui.web_signup_address() {
            Some(address) => format!("{} Open {address} in your browser.", send_words(SendProblem::WebOnly)),
            None => send_words(SendProblem::WebOnly).to_string(),
        },
        EmailProblem::Send(problem) => send_words(problem).to_string(),
    });
    let back = if ui.back_goes_to_sign_in() {
        "Sign in"
    } else {
        "Welcome"
    };
    head(back, 1, "Email", t)
        .child(au::heading(
            "Create your Beebeeb account",
            Some("Start with your email. Everything you store is encrypted on this PC before it leaves it, so we can't read any of it. The same account works in Drive, Pass, VPN, Chat and Mail."),
            false,
            t,
        ))
        .child(ck::field(
            k,
            Field {
                label: "Email",
                name: None,
                icon: Some(&icons::MAIL),
                message: Some(problem.as_deref().unwrap_or("We'll send an 8-digit code to this address.")),
                error: problem.is_some(),
                mono: false,
                pictured: c.pictured == Some(super::create::Pictured::Email),
            },
            &ui.email,
            EMAIL_KEY,
            |s: &mut State| &mut open(s).email,
            changed(Ui::sign_up_email_changed),
            escape_first,
            None,
            None,
            t,
        ))
        .child(actions(6.0).child(primary(k, "Send the code", state(ui.sending(), true), Ui::send_code, t)))
}

// ---------------------------------------------------------------------------
// DW-A32, DW-A33, DW-A34
// ---------------------------------------------------------------------------

fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn verify(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let c = ui.create_state();
    let problem = match ui.email_code_problem() {
        Some(CodeProblem::WrongOrExpired) => Some("That code is not right, or it has expired."),
        Some(CodeProblem::TooManyWrong) => Some("Too many wrong codes. Ask for a new one."),
        Some(CodeProblem::Offline | CodeProblem::Other) => {
            Some("We could not check the code. Check your connection and try again.")
        }
        None if ui.code_expired() => Some(
            "Your email code expired before the account was created. Request a new one. Your password and recovery phrase are kept.",
        ),
        None => None,
    };
    let label = format!("{}-digit code", ui.code_digits());
    let mut column = head("Back", 2, "Verify", t)
        .child(au::heading(
            "Check your email",
            Some("If this address can be used, we sent an email. Open it to continue. If you already have an account, the email says so and links to sign-in."),
            false,
            t,
        ))
        .child(ck::field(
            k,
            Field {
                label: &label,
                name: None,
                icon: None,
                message: problem,
                error: problem.is_some(),
                mono: true,
                pictured: c.pictured == Some(super::create::Pictured::Code),
            },
            &c.code,
            CODE_KEY,
            |s: &mut State| &mut open(s).create.code,
            changed(Ui::sign_up_code_changed),
            escape_first,
            None,
            None,
            t,
        ));
    // The same words whether or not an account exists (DW-A34): a status, not an alert.
    if let Some(problem) = ui.resend_problem() {
        column = column.child(
            kit::label(k.next("note"), send_words(problem), type_::CAPTION, au::red_text(t))
                .role("alert")
                .wrap()
                .w_full(),
        );
    } else if ui.resent() {
        column = column.child(
            kit::label(
                k.next("note"),
                "If this address can be used, a new email is on its way.",
                type_::CAPTION,
                t.text_tertiary,
            )
            .role("status")
            .wrap()
            .w_full(),
        );
    }
    let centred = |link: El| el().w_full().h(15.0).center().child(link);
    // The countdown, and "Send a new code" in its place once it is over.
    let wait = if ui.can_resend() {
        centred(au::link(k, "Send a new code", None, t).on_press_tx(cact(Ui::resend_code)))
    } else {
        ck::sentence_with_time("resend", "You can ask for a new code in", &clock(ui.resend_left()), t)
    };
    column
        .child(col().w_full().gap(8.0).child(wait).child(centred(
            au::link(k, "Use a different email", None, t).on_press_tx(cact(Ui::different_email)),
        )))
        .child(actions(6.0).child(primary(
            k,
            "Verify",
            state(ui.email_verifying(), ui.can_verify_email()),
            Ui::verify_email,
            t,
        )))
}

// ---------------------------------------------------------------------------
// DW-A35
// ---------------------------------------------------------------------------

fn terms(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let (accepted, understood) = ui.terms_accepted();
    let page = |item: El, name: &str, to: fn(&mut Ui)| {
        item.role("link")
            .label(format!("{name}, opens in your browser"))
            .focusable(true)
            .focus_ring(2.0, -2.0, t.accent_deep)
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(cact(to))
    };
    let rows = vec![
        page(
            ck::page_item(
                k,
                &crate::theme::icons::DOC_GENERIC,
                "Terms of Service",
                "Opens beebeeb.io/terms",
                &icons::CHEVRON,
                t,
            ),
            "Terms of Service",
            |ui| ui.open_page(Intent::OpenTerms),
        ),
        page(
            ck::page_item(
                k,
                &icons::SHIELD,
                "Privacy Policy",
                "Opens beebeeb.io/privacy",
                &icons::CHEVRON,
                t,
            ),
            "Privacy Policy",
            |ui| ui.open_page(Intent::OpenPrivacy),
        ),
    ];
    let (_, card) = au::card(k, (14.0, 14.0), 12.0, t);
    head("Back", 3, "Terms", t)
        .child(au::heading(
            "Terms and privacy",
            Some("Two things to read, and one to understand."),
            false,
            t,
        ))
        .child(au::list(k, rows, t))
        .child(
            card.child(
                ck::checkbox(
                    k,
                    accepted,
                    "I accept the Terms of Service and the Privacy Policy.",
                    true,
                    t,
                )
                .on_press_tx(cact(Ui::toggle_accepted)),
            )
            .child(
                ck::checkbox(
                    k,
                    understood,
                    "I understand that Beebeeb cannot recover my account if I lose both my password and my recovery phrase.",
                    true,
                    t,
                )
                .on_press_tx(cact(Ui::toggle_understood)),
            ),
        )
        .child(actions(6.0).child(primary(
            k,
            "Continue",
            state(false, ui.can_accept_terms()),
            Ui::accept_terms,
            t,
        )))
}

// ---------------------------------------------------------------------------
// DW-A36, DW-A37, DW-A38
// ---------------------------------------------------------------------------

fn password(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let c = ui.create_state();
    let verdict = ui.password_verdict();
    let problem = ui.password_problem().map(|problem| match problem {
        PasswordProblem::Breached => "This password appears in known data breaches. Choose a different one.",
        PasswordProblem::CheckUnreachable | PasswordProblem::Other => BREACH_UNREACHABLE,
    });
    // A long password that is known: three bars and the board's words for it (DW-A38).
    let meter = match (&verdict, ui.password_problem()) {
        (Some(_), Some(PasswordProblem::Breached)) => {
            Some(ck::meter("field-0", 3, "Strong in length, but it's known.", t))
        }
        (Some(verdict), _) => Some(ck::meter("field-0", verdict.bars, &verdict.words, t)),
        (None, _) => None,
    };
    let eye = au::field_action(
        "field-0",
        if ui.password.is_revealed() {
            "Hide password"
        } else {
            "Show password"
        },
        &icons::EYE,
        t,
    )
    .on_press_tx(cact(Ui::toggle_password));
    let new = ck::field(
        k,
        Field {
            label: "Password",
            name: None,
            icon: Some(&icons::LOCK),
            message: problem,
            error: problem.is_some(),
            mono: false,
            pictured: c.pictured == Some(super::create::Pictured::Password),
        },
        &ui.password,
        PASSWORD_KEY,
        |s: &mut State| &mut open(s).password,
        changed(Ui::sign_up_password_changed),
        escape_first,
        Some(eye),
        meter,
        t,
    );
    // The kit's own field, so Escape in it is the screen's Back as in the field above.
    let differ = ui.passwords_differ().then_some("Does not match yet.");
    let again = ck::field(
        k,
        Field {
            label: "Confirm password",
            name: None,
            icon: Some(&icons::LOCK),
            message: differ,
            error: false,
            mono: false,
            pictured: false,
        },
        &ui.confirm,
        CONFIRM_KEY,
        |s: &mut State| &mut open(s).confirm,
        changed(|ui| ui.create.pictured = None),
        escape_first,
        None,
        None,
        t,
    );
    let mut column = head("Back", 4, "Password", t)
        .child(au::heading(
            "Set a password",
            Some("It unlocks your vault on this PC. Your recovery phrase stays the ultimate backup."),
            false,
            t,
        ))
        .child(new)
        .child(again);
    // One grey line under the fields: the guidance while there is some, else what the check is.
    let note = match (&verdict, problem) {
        (_, Some(_)) => None,
        (Some(verdict), None) if verdict.hint.is_some() => verdict.hint.clone(),
        _ => Some(BREACH_NOTE.to_string()),
    };
    if let Some(note) = note {
        column = column.child(
            kit::label(k.next("note"), note, type_::CAPTION, t.text_tertiary)
                .wrap()
                .w_full(),
        );
    }
    column.child(actions(6.0).child(primary(
        k,
        "Continue",
        state(ui.password_checking(), ui.can_set_new_password()),
        Ui::check_password,
        t,
    )))
}
