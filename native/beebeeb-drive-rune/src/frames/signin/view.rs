//! The sign-in screens, drawn from [`Ui`] with the account kit (`ui::au`). Every string is the
//! board's; every measurement is the board's computed CSS (see the goldens).

use rune::{col, el, row, Color, Cx, El};

use super::source::{SharedSession, SignInProblem};
use super::{
    act, icons, qr, view_recover, view_recovery, view_steps, HelloFailure, Intent, Screen, Ui, EMAIL_KEY, PASSWORD_KEY,
    REQUEST_SECONDS,
};
use crate::theme::scale::space;
use crate::theme::tokens::{self, type_, Theme};
use crate::ui::au::{self, type_au, ButtonKind, ButtonSize, ButtonState, FieldSpec, FieldState, Keys, Tone};
use crate::ui::kit;
use crate::State;

const PRODUCT: &str = "Drive";
const ONE_ACCOUNT: &str = "One Beebeeb account for Drive, Pass, VPN, Chat and Mail.";
pub(super) const GENERIC_FAILURE: &str = "We couldn't sign you in. Nothing was unlocked.";
/// DW-A63, Drive's own words. Shown only after the password was proven right.
const DELETED: &str = "This account has been deleted. We can't recover it.";
const LOCK_NOTE: &str =
    "Five wrong tries lock an account for 30\u{a0}s to 15\u{a0}min. We email its owner an unlock link.";

pub fn view(ui: &Ui, name: &str, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    // On Windows, Enter presses the focused button or link (and Space does too).
    cx.web_focus();
    let mut k = Keys::new();
    let column = match &ui.screen {
        Screen::Welcome => welcome(&mut k, &t),
        Screen::SignIn => sign_in(ui, name, &mut k, &t),
        Screen::ContinueAs(failure) => continue_as(ui, *failure, &mut k, &t),
        Screen::Device { expired } => device(ui, *expired, &mut k, &t),
        Screen::Approved { by } => approved(by, &mut k, &t),
        Screen::KeysNotReceived => keys_not_received(&mut k, &t),
        Screen::NotSignedIn { other_account } => not_signed_in(*other_account, &mut k, &t),
        Screen::Denied { by } => denied(by, &mut k, &t),
        Screen::TwoFactor { backup } => view_steps::two_factor(ui, *backup, &mut k, &t),
        Screen::Phrase => view_steps::phrase(ui, &mut k, &t),
        Screen::UnlockVault => view_steps::unlock(ui, &mut k, &t),
        Screen::NoPhrase => view_recovery::no_phrase(&mut k, &t),
        Screen::Forgot => view_recovery::forgot(&mut k, ui.in_app_doors(), &t),
        Screen::CannotRecover => view_recovery::cannot_recover(&mut k, &t),
        Screen::Recover => view_recover::recover(ui, &mut k, &t),
        Screen::NewPassword => view_recover::new_password(ui, &mut k, &t),
        Screen::PasswordUpdated => view_recover::password_updated(&mut k, &t),
        Screen::PasswordChanged => view_recover::password_changed(&mut k, &t),
        Screen::Create(step) => super::view_create::view(ui, *step, &mut k, &t),
    };
    au::page(
        name,
        PRODUCT,
        [&icons::LOCK, &icons::PEOPLE, &icons::GLOBE],
        column,
        &t,
        cx,
    )
    .on_key(super::on_key)
}

/// The window with nothing in its column: the state was dropped (signed in) and the shell has
/// not left this group yet.
pub fn closed(name: &str, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    au::page(
        name,
        PRODUCT,
        [&icons::LOCK, &icons::PEOPLE, &icons::GLOBE],
        au::column(),
        &t,
        cx,
    )
}

/// A link out of the screen. While a sign-in runs it takes no press and no focus (DW-A18).
fn way_out(link: El, ui: &Ui, to: fn(&mut Ui)) -> El {
    if ui.busy || ui.passkey_waiting() {
        link.disabled(true)
    } else {
        link.on_press_tx(act(to))
    }
}

/// `.au-actions`: the buttons at the foot of a column, 6 apart, `top` below the column's gap.
pub(super) fn actions(top: f32) -> El {
    col().key("au-actions").w_full().gap(space::S6).mt(top)
}

pub(super) fn back(t: &Theme) -> El {
    back_as("Sign in", t)
}

/// The back link under another name: "Back" while a lock holds the window (it leads to the lock's own screens).
pub(super) fn back_as(label: &str, t: &Theme) -> El {
    au::back_link(label, &icons::BACK, t).on_press_tx(act(Ui::open_sign_in))
}

fn use_password_instead(k: &mut Keys, label: &str, t: &Theme) -> El {
    au::button(
        k,
        label,
        None,
        ButtonKind::Ghost,
        ButtonSize::Md,
        ButtonState::Enabled,
        t,
    )
    .on_press_tx(act(Ui::open_sign_in))
}

// ---------------------------------------------------------------------------
// DW-A01
// ---------------------------------------------------------------------------

fn welcome(k: &mut Keys, t: &Theme) -> El {
    au::column()
        .child(au::heading(
            "Welcome to Beebeeb Drive",
            Some("Sign in to your Beebeeb account, or create one. It works in every Beebeeb app."),
            false,
            t,
        ))
        .child(
            actions(6.0)
                .child(
                    au::button(
                        k,
                        "Create a Beebeeb account",
                        None,
                        ButtonKind::Primary,
                        ButtonSize::Lg,
                        ButtonState::Enabled,
                        t,
                    )
                    .on_press_tx(act(|ui| ui.leave(Intent::CreateAccount))),
                )
                .child(
                    au::button(
                        k,
                        "Sign in",
                        None,
                        ButtonKind::Outline,
                        ButtonSize::Lg,
                        ButtonState::Enabled,
                        t,
                    )
                    .on_press_tx(act(Ui::open_sign_in)),
                ),
        )
}

// ---------------------------------------------------------------------------
// DW-A02, DW-A03 and the form's states DW-A18 to DW-A22
// ---------------------------------------------------------------------------

/// The words under the password field for a problem, if it has any.
fn problem_message(problem: SignInProblem) -> Option<String> {
    match problem {
        // One message for a wrong email and a wrong password: Beebeeb does not say which.
        SignInProblem::Invalid => Some("That email and password do not match an account. Nothing was unlocked.".into()),
        SignInProblem::RateLimited { seconds } => Some(format!(
            "Too many tries from this network. Wait {seconds} {}, then try again. Nothing was unlocked.",
            if seconds == 1 { "second" } else { "seconds" }
        )),
        SignInProblem::LockedHint | SignInProblem::StaleKey | SignInProblem::Other => Some(GENERIC_FAILURE.into()),
        SignInProblem::Offline => None,
    }
}

fn continue_card(session: &SharedSession, ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let (key, card) = au::card(k, (14.0, 14.0), 12.0, t);
    let who = row()
        .gap(space::S10)
        .center_y()
        .child(au::avatar(
            &format!("{key}-avatar"),
            &format!("{key}-initials"),
            &session.initials,
            40.0,
            250.0,
            t.card,
            t,
        ))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label(
                    format!("{key}-title"),
                    format!("Continue as {}", session.display_name),
                    type_::HEAD,
                    t.text,
                ))
                .child(kit::label(
                    format!("{key}-line-0"),
                    session.email.clone(),
                    type_::KBD,
                    t.text_tertiary,
                ))
                .child(kit::label(
                    format!("{key}-line-1"),
                    format!(
                        concat!("Signed in to {} on ", beebeeb_engine::device_word!()),
                        session.held_by
                    ),
                    type_::CAPTION,
                    t.text_tertiary,
                )),
        );
    let state = if ui.busy || ui.passkey_waiting() {
        ButtonState::Busy
    } else {
        ButtonState::Enabled
    };
    let mut hello = au::button(
        k,
        concat!("Continue with ", beebeeb_engine::unlock_name!()),
        Some(&icons::FINGERPRINT),
        ButtonKind::Primary,
        ButtonSize::Lg,
        state,
        t,
    );
    if state == ButtonState::Enabled {
        hello = hello.on_press_tx(act(Ui::continue_as));
    }
    card.role("group")
        .label(format!("{}, {}", session.display_name, session.email))
        .child(who)
        .child(hello)
}

fn other_device_button(ui: &Ui, k: &mut Keys, t: &Theme) -> El {
    let state = if ui.busy || ui.passkey_waiting() {
        ButtonState::Busy
    } else {
        ButtonState::Enabled
    };
    let b = au::button(
        k,
        "Sign in with another device",
        Some(&icons::PHONE),
        ButtonKind::Outline,
        ButtonSize::Md,
        state,
        t,
    );
    if state == ButtonState::Busy {
        b
    } else {
        b.on_press_tx(act(Ui::ask_other_device))
    }
}

fn sign_in(ui: &Ui, name: &str, k: &mut Keys, t: &Theme) -> El {
    let offline = ui.problem == Some(SignInProblem::Offline);
    let message = if ui.deleted() {
        Some(DELETED.to_string())
    } else {
        ui.problem.and_then(problem_message)
    };
    let mut c = au::column()
        .child(au::lockup("col", 28.0, name, PRODUCT, t))
        .child(au::heading("Sign in to Drive", Some(ONE_ACCOUNT), false, t));
    if offline {
        c = c.child(au::banner(
            k,
            Tone::Neutral,
            &icons::OFFLINE,
            Some("You're offline"),
            Some("Connect to sign in. Nothing was sent."),
            true,
            t,
        ));
    }
    if let Some(session) = ui.shared() {
        c = c
            .child(continue_card(session, ui, k, t))
            .children(ui.in_app_doors().then(|| other_device_button(ui, k, t)))
            .child(au::or_divider(k, "or use email and password", t));
    }
    c = c.child(au::field(
        k,
        FieldSpec {
            label: "Email",
            icon: Some(&icons::MAIL),
            state: FieldState::Rest,
            message: None,
            mono: false,
        },
        &ui.email,
        EMAIL_KEY,
        |s: &mut State| &mut s.signin.as_mut().expect("the sign-in window is open").email,
        None,
        t,
    ));
    let eye = au::field_action(
        "field-1",
        if ui.password.is_revealed() {
            "Hide password"
        } else {
            "Show password"
        },
        &icons::EYE,
        t,
    )
    .on_press_tx(act(Ui::toggle_password));
    c = c.child(au::field(
        k,
        FieldSpec {
            label: "Password",
            icon: Some(&icons::LOCK),
            state: if message.is_some() {
                FieldState::Error
            } else {
                FieldState::Rest
            },
            message: message.as_deref(),
            mono: false,
        },
        &ui.password,
        PASSWORD_KEY,
        |s: &mut State| &mut s.signin.as_mut().expect("the sign-in window is open").password,
        Some(eye),
        t,
    ));
    if ui.problem == Some(SignInProblem::LockedHint) {
        c = c.child(au::banner(
            k,
            Tone::Neutral,
            &icons::INFO,
            None,
            // The board keeps "30 s" and "15 min" together (no-break spaces), so the line turns after "to".
            Some(LOCK_NOTE),
            false,
            t,
        ));
        // The gate compares strings with their spaces normalised, as the board's export does.
        kit::note_text(
            "banner-0-text",
            "banner-0-text",
            &LOCK_NOTE.replace('\u{a0}', " "),
            type_au::BANNER_TEXT,
            t.text_secondary,
        );
    }
    // "Use a passkey" is drawn only where the source can sign in with one.
    let mut links = row()
        .key("au-links")
        .w_full()
        .h(15.0)
        .center_y()
        .justify_between()
        .child(way_out(au::link(k, "Forgot password?", None, t), ui, Ui::forgot));
    if ui.passkey_available() {
        links = links.child(way_out(
            au::link(k, "Use a passkey", Some(&icons::PASSKEY), t),
            ui,
            Ui::use_passkey,
        ));
    }
    c = c.child(links);

    let with_card = ui.shared().is_some();
    let (label, state) = if ui.busy {
        ("Unlocking vault\u{2026}", ButtonState::Busy)
    } else if ui.retry_wait().is_some() || ui.passkey_waiting() {
        // Windows' passkey dialog is up (DW-A29): the form waits behind it, as it stands.
        // The per-network wait runs (DW-A21): drawn as the board draws it, and it takes no press
        // until the wait is over. The message above counts the seconds.
        ("Sign in", ButtonState::Busy)
    } else if offline {
        // Dimmed but pressable: a press checks the connection again (DW-A20).
        ("Sign in", ButtonState::Dim)
    } else if ui.can_submit() {
        ("Sign in", ButtonState::Enabled)
    } else {
        ("Sign in", ButtonState::Disabled)
    };
    // With the Continue as card the amber button is the card's; without it, Sign in is the one primary.
    let kind = if with_card {
        ButtonKind::Outline
    } else {
        ButtonKind::Primary
    };
    let mut submit = au::button(k, label, None, kind, ButtonSize::Lg, state, t);
    if matches!(state, ButtonState::Enabled | ButtonState::Dim) {
        submit = submit.on_press_tx(act(Ui::submit)).default_button();
    }
    let mut foot = actions(if with_card { 4.0 } else { 0.0 }).child(submit);
    // Approval from another device is drawn only where the source can do it: never a door that leads elsewhere.
    if !with_card && ui.in_app_doors() {
        foot = foot
            .child(au::or_divider(k, "or", t))
            .child(other_device_button(ui, k, t));
    }
    c.child(
        foot.child(
            au::sentence_link(k, "New to Beebeeb?", "Create an account", t, |link| {
                way_out(link, ui, |ui| ui.leave(Intent::CreateAccount))
            })
            .mt(4.0),
        ),
    )
}

// ---------------------------------------------------------------------------
// DW-A04, DW-A05, DW-A06
// ---------------------------------------------------------------------------

fn continue_as(ui: &Ui, failure: Option<HelloFailure>, k: &mut Keys, t: &Theme) -> El {
    let Some(session) = ui.shared() else {
        return au::column();
    };
    // The ring as the board draws it: seven tenths in ink, and empty in red when not recognized.
    let (fraction, tone) = match failure {
        None => (0.7, Tone::Ink),
        Some(HelloFailure::NotRecognized) => (0.0, Tone::Red),
        Some(HelloFailure::Cancelled) => (0.7, Tone::Ink),
    };
    let mut c = au::column()
        .child(back(t))
        .child(au::hero_ring(&icons::FINGERPRINT, fraction, tone, t))
        .child(au::heading(
            &format!("Continue as {}", session.display_name),
            Some(concat!(
                "Drive joins your Beebeeb account on ",
                beebeeb_engine::device_word!(),
                ". Confirm with ",
                beebeeb_engine::unlock_name!(),
                " first: it never happens silently."
            )),
            true,
            t,
        ))
        .child(
            row()
                .key("who")
                .w_full()
                .h(28.0)
                .gap(space::S10)
                .center()
                .child(au::avatar(
                    "who-avatar",
                    "who-initials",
                    &session.initials,
                    28.0,
                    250.0,
                    t.surface,
                    t,
                ))
                .child(kit::label(
                    "who-email",
                    session.email.clone(),
                    type_::KBD,
                    t.text_tertiary,
                )),
        );
    c = match failure {
        None => c,
        Some(HelloFailure::NotRecognized) => c.child(au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some(concat!(beebeeb_engine::unlock_name!(), " did not recognize you")),
            Some("Nothing was signed in. Try again, or sign in another way."),
            true,
            t,
        )),
        // Cancelling is neutral: a status, not an alert.
        Some(HelloFailure::Cancelled) => c.child(au::banner(
            k,
            Tone::Neutral,
            &icons::INFO,
            Some(concat!("You cancelled ", beebeeb_engine::unlock_name!())),
            Some("Nothing was signed in. You can try again or choose another way."),
            false,
            t,
        )),
    };
    let label = if failure.is_some() {
        concat!("Try ", beebeeb_engine::unlock_name!(), " again")
    } else {
        concat!("Continue with ", beebeeb_engine::unlock_name!())
    };
    c.child(
        actions(6.0)
            .child(
                au::button(
                    k,
                    label,
                    Some(&icons::FINGERPRINT),
                    ButtonKind::Primary,
                    ButtonSize::Lg,
                    ButtonState::Enabled,
                    t,
                )
                .on_press_tx(act(Ui::continue_as)),
            )
            .child(use_password_instead(k, "Use another way to sign in", t)),
    )
}

// ---------------------------------------------------------------------------
// DW-A07, DW-A10
// ---------------------------------------------------------------------------

fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn device(ui: &Ui, expired: bool, k: &mut Keys, t: &Theme) -> El {
    let mut c = au::column().child(back(t)).child(au::heading(
        "Sign in with another device",
        Some(
            if cfg!(target_os = "macos") {
                "On a phone or computer where you're signed in to any Beebeeb app, open Account, then Approve a sign-in. Scan this code with its camera, then type the code below."
            } else {
                "On a phone or PC where you're signed in to any Beebeeb app, open Account, then Approve a sign-in. Scan this code with its camera, then type the code below."
            },
        ),
        false,
        t,
    ));
    if expired {
        // A request that was lost (its connection dropped, or Beebeeb ended it) has its code spent too: DW-A10's
        // layout, with the words of what happened (no board frame: `FRAMES.md`, flagged deviations).
        let title = if ui.lost() {
            "This request was lost"
        } else {
            "This code expired"
        };
        c = c.child(au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some(title),
            Some("Nothing was signed in. Ask for a new code."),
            true,
            t,
        ));
    }
    let mut stack = col().key("au-stack").w_full().gap(space::S8).items_center();
    if let Some(request) = ui.request() {
        let tile = el()
            .key("qr")
            .size(156.0, 156.0)
            .radius(18.0)
            .fill(Color::WHITE)
            .box_shadow(kit::ring(kit::HAIR, t.separator))
            .center()
            .child(qr::code(&request.qr, 132.0));
        stack = stack
            .child(if expired { tile.opacity(0.28) } else { tile })
            // The pairing code is read off this screen and typed on the other device: a real text
            // node, so a screen reader reads it (the board's accessibility note). It is never logged.
            .child(kit::label(
                "code",
                request.code.clone(),
                type_au::CODE,
                if expired { t.text_tertiary } else { t.text },
            ));
    }
    let status = if expired {
        let words = if ui.lost() { "Lost" } else { "Expired" };
        row().child(crate::ui::pill::pill(
            "status-pill",
            words,
            crate::ui::pill::Tone::Neutral,
            false,
            t,
        ))
    } else {
        row()
            .gap(space::S10)
            .center_y()
            .child(au::ring(
                "status-ring",
                26.0,
                ui.seconds_left() as f32 / REQUEST_SECONDS as f32,
                t.text,
                t,
            ))
            .child(
                row()
                    .key("status-countdown")
                    .role("timer")
                    .label(format!("Expires in {}", clock(ui.seconds_left())))
                    .center_y()
                    // The board's text node is "Expires in" and the space before the digits: the gate is
                    // told the words, in the box of the words and their space.
                    .child({
                        kit::note_text(
                            "status-expires",
                            "status-expires-box",
                            "Expires in",
                            type_au::SUB,
                            t.text_tertiary,
                        );
                        // As wide as its words, not a device pixel more: the row is centred, and a
                        // wider text moves the ring beside it off the board's place.
                        rune::text("Expires in ")
                            .key("status-expires-box")
                            .font(type_au::SUB)
                            .color(t.text_tertiary)
                            .w(rune::text::advance("Expires in ", type_au::SUB))
                            .a11y_hidden()
                    })
                    .child(
                        kit::label(
                            "status-time",
                            clock(ui.seconds_left()),
                            type_au::SUB_MONO,
                            t.text_tertiary,
                        )
                        .w(rune::text::advance(&clock(ui.seconds_left()), type_au::SUB_MONO))
                        .a11y_hidden(),
                    ),
            )
            .child({
                // The countdown goes on while the connection is down (DW-A07).
                let words = if ui.reconnecting() {
                    "Reconnecting\u{2026}"
                } else {
                    "Waiting for approval"
                };
                // 8 + the dot's 6 + 5 + the words + 8 (`.kit-pill`), to the fraction of a point.
                crate::ui::pill::pill("status-pill", words, crate::ui::pill::Tone::Neutral, true, t)
                    .w(27.0 + rune::text::advance(words, type_au::PILL))
            })
    };
    c = c.child(stack.child(status.key("status")));
    if let Some(request) = ui.request() {
        let asking = au::item(
            k,
            &icons::LAPTOP,
            "Asking as",
            None,
            Some(&request.asking_as),
            None,
            false,
            t,
        );
        c = c.child(au::list(k, vec![asking], t));
    }
    let mut foot = actions(6.0);
    if expired {
        foot = foot.child(
            au::button(
                k,
                "Get a new code",
                Some(&icons::RETRY),
                ButtonKind::Primary,
                ButtonSize::Lg,
                ButtonState::Enabled,
                t,
            )
            .on_press_tx(act(Ui::ask_other_device)),
        );
    }
    c.child(foot.child(use_password_instead(k, "Use email and password instead", t)))
}

// ---------------------------------------------------------------------------
// DW-A08, DW-A09
// ---------------------------------------------------------------------------

fn approved(by: &str, k: &mut Keys, t: &Theme) -> El {
    au::column()
        .child(au::hero_tile(&icons::CHECK, Tone::Green, 8.0, t))
        .child(au::heading(
            "Approved",
            Some(&format!(
                concat!(
                    "{by} approved ",
                    beebeeb_engine::device_word!(),
                    ". Drive is opening with your account. Nothing was typed, and your password stayed with you."
                ),
                by = by
            )),
            true,
            t,
        ))
        // Amber: this is encryption state.
        .child(au::banner(
            k,
            Tone::Amber,
            &icons::LOCK,
            Some("Keys arrived encrypted for this device only"),
            Some("Beebeeb relayed them without being able to read them."),
            false,
            t,
        ))
        .child(
            actions(6.0).child(
                au::button(
                    k,
                    "Open Drive",
                    None,
                    ButtonKind::Primary,
                    ButtonSize::Lg,
                    ButtonState::Enabled,
                    t,
                )
                .on_press_tx(act(Ui::open_drive)),
            ),
        )
}

fn denied(by: &str, k: &mut Keys, t: &Theme) -> El {
    au::column()
        .child(back(t))
        .child(au::hero_tile(&icons::CROSS, Tone::Neutral, 8.0, t))
        .child(au::heading(
            "Denied",
            Some(&format!(
                concat!(
                    "{by} said no. Nothing was signed in on ",
                    beebeeb_engine::device_word!(),
                    ", and no keys were shared."
                ),
                by = by
            )),
            true,
            t,
        ))
        .child(
            actions(6.0)
                .child(
                    au::button(
                        k,
                        "Ask again",
                        Some(&icons::RETRY),
                        ButtonKind::Primary,
                        ButtonSize::Lg,
                        ButtonState::Enabled,
                        t,
                    )
                    .on_press_tx(act(Ui::ask_other_device)),
                )
                .child(use_password_instead(k, "Use email and password instead", t)),
        )
}

/// The other device approved and this PC did not sign in (no board frame: `FRAMES.md`, flagged deviations). Laid out
/// as DW-A08's error state, with DW-A07 to DW-A10's words for what is true: `other_account`, this PC holds another
/// account (its stored session or its local files); else it moved on while it waited (a sign-in, a lock or a sign-out
/// here), or it could not store the session. No account is named.
fn not_signed_in(other_account: bool, k: &mut Keys, t: &Theme) -> El {
    let (title, words) = if other_account {
        (
            concat!(
                "Approved, but ",
                beebeeb_engine::device_word!(),
                " holds another account"
            ),
            concat!(
                "Nothing was signed in on ",
                beebeeb_engine::device_word!(),
                ", and no keys were kept."
            ),
        )
    } else {
        (
            concat!("Approved, but ", beebeeb_engine::device_word!(), " did not sign in"),
            "Nothing was signed in. Ask again.",
        )
    };
    au::column()
        .child(back(t))
        .child(au::heading("Sign in with another device", None, false, t))
        .child(au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some(title),
            Some(words),
            true,
            t,
        ))
        .child(
            actions(6.0)
                .child(
                    au::button(
                        k,
                        "Ask again",
                        Some(&icons::RETRY),
                        ButtonKind::Primary,
                        ButtonSize::Lg,
                        ButtonState::Enabled,
                        t,
                    )
                    .on_press_tx(act(Ui::ask_other_device)),
                )
                .child(use_password_instead(k, "Use email and password instead", t)),
        )
}

/// DW-A08's error state, which the board gives words and no drawing: "Approved, but the keys
/// did not arrive. Nothing was signed in. Ask again." Laid out as DW-A10 (the other ending of
/// a request with nothing signed in): the group's heading, a red alert, Ask again.
fn keys_not_received(k: &mut Keys, t: &Theme) -> El {
    au::column()
        .child(back(t))
        .child(au::heading("Sign in with another device", None, false, t))
        .child(au::banner(
            k,
            Tone::Red,
            &icons::INFO,
            Some("Approved, but the keys did not arrive"),
            Some("Nothing was signed in. Ask again."),
            true,
            t,
        ))
        .child(
            actions(6.0)
                .child(
                    au::button(
                        k,
                        "Ask again",
                        Some(&icons::RETRY),
                        ButtonKind::Primary,
                        ButtonSize::Lg,
                        ButtonState::Enabled,
                        t,
                    )
                    .on_press_tx(act(Ui::ask_other_device)),
                )
                .child(use_password_instead(k, "Use email and password instead", t)),
        )
}
