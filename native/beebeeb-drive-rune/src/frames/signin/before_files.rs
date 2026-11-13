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
//! DW-0009 shows no file name anywhere, and it is in the capture-exclusion decision as a whole
//! (`FrameGroup::Locked`, `shell::registry`).

use crate::State;
use crate::frames::GateNotice;
use crate::shell::app::{Event, GateState, Screen};
use crate::shell::host;
use crate::shell::registry::{FeedbackPolicy, Frame, FrameGroup};
use crate::theme::icons_settings as ic;
use crate::theme::tokens::{self, Theme, radius, type_};
use crate::ui::settings_dialog::DIALOG_TITLE;
use crate::ui::sidebar::{self, Standing};
use crate::ui::toolbar::{self, Closed};
use crate::ui::{caption, kit};
use rune::{Align, Color, Cx, El, Icon, TextStyle, col, el, row};

/// The action each button names, for tests and the log. None of these is wired to an engine
/// action yet: each button shows [`NOT_AVAILABLE`].
pub const UNLOCK: &str = "unlock";
pub const OPEN_PLANS: &str = "open-plans";
pub const CHECK_PLAN: &str = "check-plan";
pub const SIGN_OUT: &str = "sign-out";

/// `.kit-btn-lg`'s words: 13px/500 on 19.5px.
const BUTTON_LG: TextStyle = TextStyle::new(13.0, 500.0).lh(19.5).tracking(-0.065);
/// `.t-sub`: 12px on 15px.
const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// The no-plan column is 380 wide (`max-width: 380px`).
const NO_PLAN_W: f32 = 380.0;

/// The registry's lines (`shell::registry::FRAMES`).
pub const LOCKED: Frame = Frame {
    group: FrameGroup::Locked,
    view,
    on_key: None,
    feedback: FeedbackPolicy::Denied("the window is locked"),
};
pub const NO_PLAN: Frame = Frame {
    group: FrameGroup::NoPlan,
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

/// The message every gate button shows today: none of the four engine actions exists yet.
const NOT_AVAILABLE: &str = "Not available yet";

/// Set the gate notice and log the attempt. The engine actions these buttons need (unlock via
/// Windows Hello, opening the plans page in the browser, re-checking the plan with the server,
/// and sign-out while the gate is closed) are phase 2: this file does not edit `engine_host.rs`.
fn not_built(what: &'static str) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        let gate = s.shell.app.gate();
        s.frames.gate_notice = Some(GateNotice {
            message: NOT_AVAILABLE,
            gate,
        });
        eprintln!("beebeeb gate: NotBuilt({what})");
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Primary,
    Plain,
    Quiet,
}

/// A button of the block: `h` tall, as wide as its words unless the caller sets a width.
/// Keys: `<key>`, `<key>-icon`, `<key>-label`.
fn button(key: &str, label: &str, icon: Option<&'static Icon>, look: Look, h: f32, style: TextStyle, t: &Theme) -> El {
    let (fill, ink) = match look {
        Look::Primary => (t.accent, t.accent_text_on),
        Look::Plain => (t.paper, t.text),
        Look::Quiet => (Color::TRANSPARENT, t.text_secondary),
    };
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .h(h)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(radius::MD)
        .fill(fill)
        .focusable(true);
    b = match look {
        Look::Primary => b
            .hover(fill.mix(Color::BLACK, 0.05))
            .press(fill.mix(Color::BLACK, 0.10)),
        _ => b.hover(t.paper2).press(t.paper3),
    };
    if look == Look::Plain {
        b = b.border(1.0, t.card_border).border_layout();
    }
    if let Some(icon) = icon {
        b = b.child(kit::glyph(format!("{key}-icon"), icon, 16.0, ink));
    }
    b.child(kit::label(format!("{key}-label"), label, style, ink))
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

/// DW-0009: "Beebeeb is locked", and Unlock, which starts Windows Hello again.
fn locked(t: &Theme) -> El {
    let mut block = col().key("gate").items_center().gap(8.0);
    for child in head(&ic::PADLOCK, "Beebeeb is locked", "Unlock to see your files.", false, t) {
        block = block.child(child);
    }
    block.child(
        el().key("gate-actions").mt(8.0).child(
            button(
                "gate-btn-0",
                "Unlock",
                Some(&ic::APP_LOCK),
                Look::Primary,
                28.0,
                crate::ui::settings_dialog::BUTTON_MD,
                t,
            )
            .default_button()
            .autofocus()
            .on_press(not_built(UNLOCK)),
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
    let wide = |key: &str, label: &str, icon, look| button(key, label, icon, look, 32.0, BUTTON_LG, t).w_full();
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
                    Look::Primary,
                )
                // It opens the browser: said to assistive technology, as the glyph says it to the eye.
                .label("Choose a plan on beebeeb.io, opens in your browser")
                .default_button()
                .autofocus()
                .on_press(not_built(OPEN_PLANS)),
            )
            .child(wide("gate-btn-1", "Check again", None, Look::Plain).on_press(not_built(CHECK_PLAN)))
            .child(wide("gate-btn-2", "Sign out", None, Look::Quiet).on_press(not_built(SIGN_OUT))),
    )
}

/// The status line (`.mk-foot`): a dot and what sync is doing. The dot is never green here.
fn status(words: &str, dot: Color, t: &Theme) -> El {
    row()
        .key("status-bar")
        .role("status")
        .h(28.0)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .child(
            row()
                .key("status-left")
                .center_y()
                .gap(7.0)
                .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(dot))
                .child(kit::label("status-text", words, type_::STATUS_STRONG, t.text_secondary)),
        )
        .child(el().key("status-spacer").grow(1.0))
        // Nothing to count: the board keeps the place, empty.
        .child(kit::label("status-details", "", type_::STATUS, t.text_tertiary).w(0.0))
}

/// The main window while the gate is `Locked` or `NoPlan`.
pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    cx.web_focus();
    let is_locked = s.shell.app.screen() == Screen::Gate(GateState::Locked);
    let current_gate = s.shell.app.gate();
    let (standing, closed, block, words, dot) = if is_locked {
        (
            Standing::Locked,
            Closed {
                title: "Beebeeb is locked",
                sub: None,
            },
            locked(&t),
            "Locked \u{00B7} sync keeps running",
            t.text_tertiary,
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
            t.text_quaternary,
        )
    };
    // The notice a button raised: drawn under the buttons, shown only while the gate that
    // raised it still holds (a gate change is the engine's answer to the person's action).
    let notice_text = s
        .frames
        .gate_notice
        .as_ref()
        .filter(|n| n.gate == current_gate)
        .map(|n| n.message);
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
        .child(status(words, dot, &t));
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
