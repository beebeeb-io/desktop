//! The two states of the main window before Files (the board's group "Before Files: locked, no
//! plan yet"): app lock is on (DW-0009), and the account has no plan (DW-0014).
//!
//! Both are the Files window with nothing of the account's in it: the sidebar and the toolbar
//! stand closed (`ui::sidebar::Standing`, `ui::toolbar::Closed`: drawn, dimmed, out of reach),
//! and where the files would be the window says what is the matter and what to do.
//!
//! Which of the two shows is the session's gate (`shell::app::GateState`), which is the
//! engine's: nothing here opens the gate. A button whose engine action is not built yet says
//! "Not available yet" (a [`GateNotice`], drawn under the buttons, cleared by a gate change).
//! The window moves on when the engine's gate does (Windows Hello recognised the person, a plan
//! arrived).
//!
//! The lock screen's Unlock asks for Windows Hello (`super::lock`: the prompt, its answer as a
//! sentence under the button, and "Use your password" when Windows Hello cannot answer). The
//! lock's password screen (the screen lock's other way in, and a closed vault) is the sign-in
//! group's DW-A30 with the lock's words, drawn here in place of the lock screen.
//!
//! DW-0009 shows no file name anywhere, and it is in the capture-exclusion decision as a whole
//! (`FrameGroup::Locked`, `shell::registry`).

use crate::State;
use crate::frames::GateNotice;
use crate::shell::app::{Event, GateState, Screen};
use crate::shell::host;
use crate::shell::registry::{FeedbackPolicy, Frame, FrameGroup};
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{self, Theme};
use crate::ui::button::{ButtonState, Size, Variant, button as ui_button, keys};
use crate::ui::settings_dialog::DIALOG_TITLE;
use crate::ui::sidebar::{self, Standing};
use crate::ui::status_bar::{self, Dot};
use crate::ui::toolbar::{self, Closed};
use crate::ui::{caption, kit};
use rune::{Align, Cx, El, Icon, TextStyle, col, el, row};

/// The action each button names, for tests and the log.
pub const OPEN_PLANS: &str = "open-plans";
pub const CHECK_PLAN: &str = "check-plan";
pub const SIGN_OUT: &str = "sign-out";
/// The lock screen's second button, shown once Windows Hello could not answer.
pub const USE_PASSWORD_KEY: &str = "gate-btn-1";

/// The cover's sign-in button, shown while the engine's look failed and nothing passed (`shell::away`).
pub const COVER_SIGN_IN_KEY: &str = "cover-sign-in";

/// `.t-sub`: 12px on 15px.
const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// The no-plan column is 380 wide (`max-width: 380px`).
const NO_PLAN_W: f32 = 380.0;

/// The registry's lines (`shell::registry::FRAMES`).
pub const LOCKED: Frame = Frame {
    group: FrameGroup::Locked,
    view,
    on_key: Some(on_key),
    feedback: FeedbackPolicy::Denied("the window is locked"),
};
pub const NO_PLAN: Frame = Frame {
    group: FrameGroup::NoPlan,
    view,
    on_key: None,
    feedback: FeedbackPolicy::Denied("its lane has not opted in yet"),
};

/// The credential store did not answer (no board frame): `keystore.rs`.
pub const KEYSTORE: Frame = Frame {
    group: FrameGroup::KeyStoreUnavailable,
    view,
    on_key: None,
    feedback: FeedbackPolicy::Denied("its lane has not opted in yet"),
};

/// The board's frames of this file: the id, and what shows it on a fresh `State`.
pub const FRAMES: &[(&str, fn(&mut State))] = &[
    ("DW-0009", |s| {
        host::dispatch(s, Event::Gate(GateState::Locked));
    }),
    ("DW-0014", |s| {
        host::dispatch(s, Event::Gate(GateState::NoPlan));
    }),
];

/// What a gate button says when the engine cannot be asked (no sign-in source in this window).
const NOT_AVAILABLE: &str = "Not available yet";

#[cfg(test)]
#[path = "gates_tests.rs"]
mod gates_tests;
#[path = "keystore.rs"]
pub mod keystore;

/// Asks the window's sign-in source (the engine's) for one of the gate buttons; says [`NOT_AVAILABLE`] under the
/// buttons when this window has none. The engine's gate then changes by itself and the window follows it.
fn ask_source(s: &mut State, what: &'static str, ask: impl FnOnce(&mut dyn super::source::SignInSource)) {
    match s.signin.as_mut() {
        Some(ui) => ask(ui.source.as_mut()),
        None => {
            let gate = s.shell.app.gate();
            s.frames.gate_notice = Some(GateNotice {
                message: NOT_AVAILABLE,
                gate,
            });
            eprintln!("beebeeb gate: NotAvailable({what})");
        }
    }
}

/// "Check again" / "Try again": the engine reads its gate once more.
fn recheck(s: &mut State) {
    s.frames.gate_notice = None;
    ask_source(s, CHECK_PLAN, |source| source.recheck_gate());
}

/// "Sign out" on the no-plan screen: the existing sign-out of this app.
fn sign_out(s: &mut State) {
    s.frames.gate_notice = None;
    // An open editor with unsaved changes asks first (Save, Discard and sign out, Cancel).
    if crate::frames::viewers::unsaved_guard::ask(s, crate::frames::viewers::unsaved_guard::Then::GateSignOut) {
        return;
    }
    sign_out_now(s, false);
}

/// The gate's sign-out itself, once nothing unsaved is in the way (or the person chose to discard it).
pub(crate) fn sign_out_now(s: &mut State, discard: bool) {
    if discard {
        ask_source(s, SIGN_OUT, |source| source.sign_out_from_gate_discarding_unsaved());
    } else {
        ask_source(s, SIGN_OUT, |source| source.sign_out_from_gate());
    }
}

/// Opens one of the website's pages by its name in `shell::web_pages`; says so when it did not open.
fn open_page(s: &mut State, name: &'static str) {
    s.frames.gate_notice = None;
    let opened = crate::shell::web_pages::url(name)
        .is_some_and(|address| crate::engine_host::signin::open_web_page(address).is_ok());
    if !opened {
        let gate = s.shell.app.gate();
        s.frames.gate_notice = Some(GateNotice {
            message: "Could not open that page",
            gate,
        });
    }
}

/// A button of the block (`ui::button::button`), as wide as its words unless the caller sets a width.
/// Keys: `<key>`, `<key>-icon`, `<key>-label`.
fn button(key: &str, label: &str, icon: Option<&'static Icon>, variant: Variant, size: Size, t: &Theme) -> El {
    ui_button(keys(key), label, icon, variant, size, ButtonState::Enabled, t)
}

/// The head of the block: a neutral tile, the heading, one sentence, centred.
fn head(icon: &'static Icon, title: &str, words: &str, fill: bool, t: &Theme) -> Vec<El> {
    let sentence = kit::label("gate-text", words, SUB, t.text_tertiary)
        .wrap()
        .align(Align::Center);
    vec![
        el().key("gate-tile")
            .a11y_hidden()
            .size(56.0, 56.0)
            .radius(16.0)
            .fill(t.text.alpha(0.07))
            .center()
            .child(kit::glyph("gate-icon", icon, 28.0, t.text_secondary)),
        kit::label("gate-title", title, DIALOG_TITLE, t.text)
            .role("heading")
            .level(1)
            .mt(8.0),
        // A sentence that wraps takes the block's width (DW-0014); a short one is as wide as its words.
        if fill { sentence.w_full() } else { sentence },
    ]
}

/// The lock's password screen is in front (the screen lock's other way in, or a closed vault), or the twelve words a
/// strong unlock ended at while the shell's gate has not moved yet (they stay in front, B-R2).
fn password_in_front(s: &State) -> bool {
    s.shell.app.screen() == Screen::Gate(GateState::Locked)
        && s.signin.as_ref().is_some_and(|ui| {
            ui.lock_shows_password() || matches!(ui.screen, super::Screen::Phrase | super::Screen::NoPhrase)
        })
}

/// The lock's password screen takes the sign-in group's keys (Escape goes back); the lock screen has none.
fn on_key(s: &mut State, ev: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    password_in_front(s) && super::on_key(s, ev, tx)
}

/// DW-0009: "Beebeeb is locked", and Unlock, which starts Windows Hello again. `said`: Windows Hello's last
/// answer, when it was not a yes; "Use your password" shows with it, never before (the board's frame has one
/// button).
fn locked(said: Option<super::lock::Said>, t: &Theme) -> El {
    let mut block = col().key("gate").items_center().gap(8.0);
    for child in head(&ic::PADLOCK, "Beebeeb is locked", "Unlock to see your files.", false, t) {
        block = block.child(child);
    }
    let mut actions = row().key("gate-actions").mt(8.0).gap(8.0).child(
        button(
            super::lock::UNLOCK_KEY,
            "Unlock",
            Some(&ic::APP_LOCK),
            Variant::Primary,
            Size::Md,
            t,
        )
        .default_button()
        .autofocus()
        .on_press_tx(super::act(super::Ui::unlock_screen)),
    );
    if said.is_some() {
        actions = actions.child(
            button(
                USE_PASSWORD_KEY,
                "Use your password",
                None,
                Variant::Secondary,
                Size::Md,
                t,
            )
            .on_press_tx(super::act(super::Ui::lock_use_password)),
        );
    }
    block.child(actions)
}

/// The cover whose look failed: the lock screen's look with the sign-in path in place of an Unlock that would ask
/// nothing (the engine's lock is open). The press closes the engine's lock; its own password screen takes the window.
fn cover_sign_in(t: &Theme) -> El {
    let mut block = col().key("gate").items_center().gap(8.0);
    for child in head(
        &ic::PADLOCK,
        "Beebeeb is locked",
        "Sign in to see your files.",
        false,
        t,
    ) {
        block = block.child(child);
    }
    block.child(
        row().key("gate-actions").mt(8.0).gap(8.0).child(
            button(
                COVER_SIGN_IN_KEY,
                "Sign in",
                Some(&ic::APP_LOCK),
                Variant::Primary,
                Size::Md,
                t,
            )
            .default_button()
            .autofocus()
            .on_press_tx(|s: &mut State, _: &mut rune::Tx| s.shell.actions.lock_vault_now()),
        ),
    )
}

/// DW-0014: where to get a plan (only on the website), Check again, Sign out.
fn no_plan(t: &Theme) -> El {
    let mut block = col().key("gate").w(NO_PLAN_W).items_center().gap(8.0);
    for child in head(
        &ic::DATABASE,
        "Choose a plan",
        "Your account is ready. Choose a plan on beebeeb.io, including the free trial. Beebeeb continues here by itself.",
        true,
        t,
    ) {
        block = block.child(child);
    }
    let wide = |key: &str, label: &str, icon, look| button(key, label, icon, look, Size::Lg, t).w_full();
    block.child(
        col()
            .key("gate-actions")
            .w_full()
            .gap(8.0)
            .mt(12.0)
            .child(
                wide(
                    "gate-btn-0",
                    "Choose a plan on beebeeb.io",
                    Some(&ic::EXTERNAL),
                    Variant::Primary,
                )
                // It opens the browser: said to assistive technology, as the glyph says it to the eye.
                .label("Choose a plan on beebeeb.io, opens in your browser")
                .default_button()
                .autofocus()
                .on_press(|s: &mut State| open_page(s, "plan")),
            )
            .child(wide("gate-btn-1", "Check again", None, Variant::Secondary).on_press(recheck))
            .child(wide("gate-btn-2", "Sign out", None, Variant::Ghost).on_press(sign_out)),
    )
}

/// The lock screen's sync line: the tray's words (`frames/tray/source.rs`), true to what the engine does.
pub(super) fn locked_line(running: bool) -> &'static str {
    if running {
        "Locked \u{00B7} sync keeps running"
    } else {
        "Locked \u{00B7} nothing syncs"
    }
}

/// The main window while the gate is `Locked` or `NoPlan`.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    if password_in_front(s) {
        return super::frame(s, cx);
    }
    let is_locked = s.shell.app.screen() == Screen::Gate(GateState::Locked);
    drawn(s, cx, is_locked)
}

/// DW-0009 over a window whose lock is still open: the away lock's cover after a wake Windows may not have announced
/// (`shell::away`, the owner's ruling D-B2-3), until the engine said whether the wake locks. Nothing of the account's
/// is in it, as on the lock screen. Unlock asks nothing while the lock is open (`Ui::unlock_screen` asks only at a
/// screen lock); when the engine locks, the lock screen itself takes its place, the same to the eye.
pub fn cover(s: &State, cx: &mut Cx<State>) -> El {
    drawn(s, cx, true)
}

/// The window before Files: DW-0009 when `is_locked`, else DW-0014.
fn drawn(s: &State, cx: &mut Cx<State>, is_locked: bool) -> El {
    let t = tokens::theme(cx.system_dark());
    cx.web_focus();
    let current_gate = s.shell.app.gate();
    // Windows Hello's last answer on the lock screen, when it was not a yes (`super::lock`); never on the cover.
    let lock_said = s
        .signin
        .as_ref()
        .and_then(super::Ui::lock_said)
        .filter(|_| current_gate == GateState::Locked);
    let (standing, closed, block, words, dot) = if current_gate == GateState::KeyStoreUnavailable {
        (
            Standing::Locked,
            Closed {
                title: "Files",
                sub: Some("Can't read your keys"),
            },
            keystore::block(&t),
            "Not syncing \u{00B7} can't read your keys",
            Dot::Idle,
        )
    } else if is_locked {
        (
            Standing::Locked,
            Closed {
                title: "Beebeeb is locked",
                sub: None,
            },
            if s.shell.away.sign_in_offered() && current_gate != GateState::Locked {
                cover_sign_in(&t)
            } else {
                locked(lock_said, &t)
            },
            locked_line(s.shell.sync_runs()),
            Dot::Busy,
        )
    } else {
        (
            Standing::NoPlan,
            Closed {
                title: "Files",
                sub: Some("No plan yet"),
            },
            no_plan(&t),
            "Not syncing \u{00B7} no plan",
            Dot::Idle,
        )
    };
    // The notice a button raised: drawn under the buttons, shown only while the gate that
    // raised it still holds (a gate change is the engine's answer to the person's action).
    let notice_text = lock_said.map(super::lock::Said::words).or_else(|| {
        s.frames
            .gate_notice
            .as_ref()
            .filter(|n| n.gate == current_gate)
            .map(|n| n.message)
    });
    let mut gate_block = col()
        .key("gate-area")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .center()
        .child(block);
    if let Some(notice) = notice_text {
        gate_block = gate_block.child(
            kit::label("gate-notice", notice, SUB, t.text_tertiary)
                .mt(12.0)
                .role("status"),
        );
    }
    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar::toolbar_in(s, &t, 0.0, Some(&closed)))
        .child(gate_block)
        .child(status_bar::bar(Some(dot), Some(words), "", &t));
    col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(
            row()
                .key("body")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(sidebar::sidebar_in(s, &t, standing))
                .child(main),
        )
}
