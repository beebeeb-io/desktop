//! Tests of the sign-in group: every built frame's state, the keyboard, names and roles, and
//! that a secret is in no tree a tool or assistive technology can read.

use rune::{Env, Key, KeyEvent, Mods, Runtime, TextEdit};

use super::source::{DeviceStatus, FixtureSignInSource, NextAfterSignIn, Script, SignInEvent, board};
use super::{EMAIL_KEY, HelloFailure, Intent, PASSWORD_KEY, Screen, Ui, scene};
use crate::{State, init, view};

const BUILT: [(&str, &str); 15] = [
    ("DW-A01", "Welcome to Beebeeb Drive"),
    ("DW-A02", "Sign in to Drive"),
    ("DW-A03", "Sign in to Drive"),
    ("DW-A04", "Continue as Anna Visser"),
    ("DW-A05", "Continue as Anna Visser"),
    ("DW-A06", "Continue as Anna Visser"),
    ("DW-A07", "Sign in with another device"),
    ("DW-A08", "Approved"),
    ("DW-A09", "Denied"),
    ("DW-A10", "Sign in with another device"),
    ("DW-A18", "Sign in to Drive"),
    ("DW-A19", "Sign in to Drive"),
    ("DW-A20", "Sign in to Drive"),
    ("DW-A21", "Sign in to Drive"),
    ("DW-A22", "Sign in to Drive"),
];

fn runtime_of(ui: Ui, dark: bool) -> Runtime<State> {
    let mut state = init();
    state.signin = Some(ui);
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.frame(0.0);
    rt
}

fn runtime(frame: &str) -> Runtime<State> {
    runtime_of(scene(frame).unwrap_or_else(|| panic!("no scene {frame}")), false)
}

fn ui(rt: &mut Runtime<State>) -> &mut Ui {
    rt.state.signin.as_mut().expect("the sign-in window is open")
}

fn info(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(0.0);
}

/// Space presses the focused control.
fn space(rt: &mut Runtime<State>) {
    press(rt, Key::Text(" ".into()));
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = info(rt, key).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements().into_iter().find(|e| e.focused).and_then(|e| e.key)
}

/// Tab until `key` has focus (at most 20 stops).
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..20 {
        if focused(rt).as_deref() == Some(key) {
            return;
        }
        press(rt, Key::Tab);
    }
    panic!("Tab never reached {key}; focus is on {:?}", focused(rt));
}

#[test]
fn every_built_frame_draws_its_heading_first_in_both_themes() {
    for (frame, title) in BUILT {
        for dark in [false, true] {
            let rt = runtime_of(scene(frame).expect("a scene"), dark);
            let heading = info(&rt, "title");
            assert_eq!(heading.text.as_deref(), Some(title), "{frame} (dark: {dark})");
            assert_eq!(heading.role, "heading", "{frame}: the title is a heading");
            assert!(
                has(&rt, "au-side") && has(&rt, "caption-bar"),
                "{frame}: the window's panels"
            );
        }
    }
    assert!(
        scene("DW-0401").is_none(),
        "a frame of another group is not a scene here"
    );
}

#[test]
fn the_first_screen_is_skipped_when_another_beebeeb_app_is_signed_in() {
    assert_eq!(scene("DW-A01").expect("a scene").screen, Screen::Welcome);
    assert_eq!(scene("DW-A02").expect("a scene").screen, Screen::SignIn);
}

#[test]
fn welcome_buttons_work_from_the_keyboard() {
    let mut rt = runtime("DW-A01");
    tab_to(&mut rt, "btn-0");
    space(&mut rt);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::CreateAccount]);
    tab_to(&mut rt, "btn-1");
    space(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(
        !has(&rt, "card-0"),
        "no Continue as card without a shared session (DW-A03)"
    );
}

#[test]
fn tab_order_is_continue_other_device_email_password_sign_in() {
    let mut rt = runtime("DW-A02");
    let mut order = Vec::new();
    // Where no system draws the caption buttons (this headless run), the caption's own three
    // come first; on Windows they are the system's and are not Tab stops.
    while order.len() < 9 {
        press(&mut rt, Key::Tab);
        let key = focused(&rt).unwrap_or_default();
        if !key.starts_with("cap-") {
            order.push(key);
        }
    }
    assert_eq!(
        order,
        [
            "btn-0",
            "btn-1",
            EMAIL_KEY,
            PASSWORD_KEY,
            "field-1-act",
            "link-0-box",
            "link-1-box",
            "btn-2",
            "link-2-box"
        ]
    );
}

#[test]
fn enter_in_a_field_signs_in_and_the_engine_names_the_next_frame() {
    let mut rt = runtime("DW-A02");
    click(&mut rt, PASSWORD_KEY);
    press(&mut rt, Key::Enter);
    assert_eq!(
        ui(&mut rt).take_intents(),
        vec![Intent::Next(NextAfterSignIn::TwoFactor)]
    );
    assert!(!ui(&mut rt).busy);
}

#[test]
fn sign_in_waits_for_an_address_and_a_password() {
    let mut blank = scene("DW-A03").expect("a scene");
    blank.email = TextEdit::new("anna");
    assert!(!blank.can_submit(), "anna is not an address");
    blank.email = TextEdit::new(board::EMAIL);
    blank.password = TextEdit::new("").secure();
    assert!(!blank.can_submit(), "no password");
    let mut rt = runtime_of(blank, false);
    click(&mut rt, "btn-0");
    assert!(
        ui(&mut rt).take_intents().is_empty() && !ui(&mut rt).busy,
        "a disabled button does nothing"
    );
    assert!(!info(&rt, "btn-0").focusable);
}

#[test]
fn a_running_sign_in_says_so_and_takes_no_second_press() {
    let mut rt = runtime("DW-A18");
    assert_eq!(
        info(&rt, "btn-2-label").text.as_deref(),
        Some("Unlocking vault\u{2026}")
    );
    click(&mut rt, "btn-2");
    click(&mut rt, "btn-0");
    click(&mut rt, "btn-1");
    assert_eq!(
        ui(&mut rt).screen,
        Screen::SignIn,
        "the other paths are off while it runs"
    );
    assert!(ui(&mut rt).busy);
}

#[test]
fn each_problem_draws_the_boards_words() {
    let message = |frame: &str| info(&runtime(frame), "field-1-msg").text.unwrap_or_default();
    assert_eq!(
        message("DW-A19"),
        "That email and password do not match an account. Nothing was unlocked."
    );
    assert_eq!(
        message("DW-A21"),
        "Too many tries from this network. Wait 30 seconds, then try again. Nothing was unlocked."
    );
    assert_eq!(message("DW-A22"), "We couldn't sign you in. Nothing was unlocked.");
    assert_eq!(info(&runtime("DW-A19"), "field-1-msg").role, "alert");
    let offline = runtime("DW-A20");
    assert_eq!(info(&offline, "banner-0-title").text.as_deref(), Some("You're offline"));
    assert_eq!(info(&offline, "banner-0").role, "alert");
    assert!(!has(&offline, "field-1-msg"));
    assert!(info(&offline, "btn-2").focusable, "dimmed but pressable (DW-A20)");
    assert_eq!(info(&runtime("DW-A22"), "banner-0").role, "status");
}

#[test]
fn the_fifth_wrong_try_in_a_row_explains_the_lock() {
    let mut rt = runtime("DW-A19");
    for _ in 0..3 {
        click(&mut rt, "btn-2");
        assert!(!has(&rt, "banner-0"));
    }
    click(&mut rt, "btn-2");
    assert_eq!(ui(&mut rt).problem, Some(super::source::SignInProblem::LockedHint));
    assert!(has(&rt, "banner-0"), "the lock's note (DW-A22)");
    assert_eq!(
        ui(&mut rt).password.text.chars().count(),
        board::PASSWORD_LEN,
        "the board keeps the field filled"
    );
}

#[test]
fn windows_hello_is_asked_every_time_and_its_answers_are_frames() {
    let source = FixtureSignInSource::new(Script::default());
    let calls = source.calls.clone();
    let mut rt = runtime_of(Ui::new(Box::new(source)), false);
    tab_to(&mut rt, "btn-0");
    space(&mut rt);
    assert_eq!(*calls.borrow(), ["hello"]);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);

    let mut rt = runtime("DW-A05");
    assert_eq!(
        ui(&mut rt).screen,
        Screen::ContinueAs(Some(HelloFailure::NotRecognized))
    );
    assert_eq!(info(&rt, "banner-0").role, "alert");
    assert_eq!(
        info(&rt, "btn-0-label").text.as_deref(),
        Some("Try Windows Hello again")
    );
    let mut rt6 = runtime("DW-A06");
    assert_eq!(info(&rt6, "banner-0").role, "status", "cancelling is not an error");
    click(&mut rt6, "btn-1");
    assert_eq!(ui(&mut rt6).screen, Screen::SignIn, "Use another way to sign in");
    tab_to(&mut rt, "btn-0");
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn, "Escape goes back");
}

#[test]
fn another_device_asks_waits_and_ends_in_each_answer() {
    let source = FixtureSignInSource::new(Script::default());
    let calls = source.calls.clone();
    let mut rt = runtime_of(Ui::new(Box::new(source)), false);
    click(&mut rt, "btn-1");
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false });
    assert_eq!(
        info(&rt, "code").text.as_deref(),
        Some(board::CODE),
        "the code is a real text node"
    );
    assert_eq!(info(&rt, "status-countdown").label.as_deref(), Some("Expires in 4:12"));
    assert_eq!(
        info(&rt, "qr-code").label.as_deref(),
        Some("QR code that opens this sign-in request, without the code")
    );
    tab_to(&mut rt, "back-link");
    press(&mut rt, Key::Enter);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn, "Return follows a focused link");
    assert_eq!(
        *calls.borrow(),
        ["device_request", "cancel_device_request"],
        "leaving closes the request"
    );
    assert!(ui(&mut rt).request().is_none(), "and forgets the code");

    let mut rt = runtime("DW-A10");
    assert_eq!(info(&rt, "banner-0").role, "alert");
    click(&mut rt, "btn-0");
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false }, "Get a new code");
    assert!(!has(&rt, "banner-0"));

    let mut rt = runtime("DW-A09");
    click(&mut rt, "btn-0");
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false }, "Ask again");

    let mut rt = runtime("DW-A08");
    assert_eq!(
        ui(&mut rt).screen,
        Screen::Approved {
            by: board::OTHER_DEVICE.into()
        }
    );
    tab_to(&mut rt, "btn-0");
    space(&mut rt);
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
}

#[test]
fn an_answer_for_a_screen_already_left_changes_nothing() {
    let mut ui = scene("DW-A07").expect("a scene");
    ui.open_sign_in();
    let mut source = FixtureSignInSource::new(Script::default());
    source.push(SignInEvent::Device(DeviceStatus::Approved { by: "x".into() }));
    let mut late = Ui::new(Box::new(source));
    late.pump();
    assert_eq!(late.screen, Screen::SignIn);
    assert_eq!(ui.screen, Screen::SignIn);
}

/// A seeded secret is in no element text, no label, no accessibility tree and no `Debug`,
/// hidden and revealed. The check is seen to fail: the email, typed in a plain field, IS found.
#[test]
fn a_password_is_in_no_tree_and_no_label() {
    const SECRET: &str = "Zx9-harbor-quartz-41";
    for revealed in [false, true] {
        let mut ui = scene("DW-A02").expect("a scene");
        ui.password = TextEdit::new(SECRET).secure();
        if revealed {
            ui.toggle_password();
        }
        let debug = format!("{:?}", ui.password);
        let rt = runtime_of(ui, false);
        let tree: String = rt
            .elements()
            .iter()
            .map(|e| format!("{:?}|{:?}|{:?}|{}\n", e.key, e.label, e.text, e.path))
            .collect();
        let a11y = rt.accessibility_json().to_string();
        for (name, dump) in [
            ("element tree", &tree),
            ("accessibility tree", &a11y),
            ("Debug", &debug),
        ] {
            assert!(
                !dump.contains(SECRET),
                "the password is in the {name} (revealed: {revealed})"
            );
            assert!(!dump.contains("harbor"), "part of the password is in the {name}");
        }
        assert!(
            tree.contains(board::EMAIL),
            "the same search finds a plain field's text"
        );
        assert!(a11y.contains(board::EMAIL), "and finds it in the accessibility tree");
        let field = info(&rt, PASSWORD_KEY);
        assert_eq!(field.text, None, "a secure field reports no text");
        assert_eq!(
            info(&rt, "field-1-act").label.as_deref(),
            Some(if revealed { "Hide password" } else { "Show password" })
        );
    }
}

#[test]
fn a_typed_password_stays_out_of_the_trees_too() {
    let mut blank = scene("DW-A03").expect("a scene");
    blank.password = TextEdit::new("").secure();
    let mut rt = runtime_of(blank, false);
    click(&mut rt, PASSWORD_KEY);
    press(&mut rt, Key::Text("Qv7-lantern".into()));
    assert_eq!(ui(&mut rt).password.text, "Qv7-lantern", "the field took the keys");
    let tree: String = rt
        .elements()
        .iter()
        .map(|e| format!("{:?}|{:?}\n", e.label, e.text))
        .collect();
    assert!(!tree.contains("lantern") && !rt.accessibility_json().to_string().contains("lantern"));
    tab_to(&mut rt, "field-1-act");
    space(&mut rt);
    assert!(ui(&mut rt).password.is_revealed(), "the eye works from the keyboard");
    assert!(
        !rt.accessibility_json().to_string().contains("lantern"),
        "revealed is still not readable"
    );
}

#[test]
fn the_links_leave_for_their_frames() {
    let mut rt = runtime("DW-A02");
    click(&mut rt, "link-0-box");
    click(&mut rt, "link-1-box");
    tab_to(&mut rt, "link-2-box");
    press(&mut rt, Key::Enter);
    assert_eq!(
        ui(&mut rt).take_intents(),
        vec![Intent::ForgotPassword, Intent::UsePasskey, Intent::CreateAccount]
    );
    let mut rt = runtime("DW-A07");
    click(&mut rt, "back-link");
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
}
