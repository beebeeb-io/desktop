//! Tests of the sign-in group: every built frame's state, the keyboard, names and roles, and
//! that a secret is in no tree a tool or assistive technology can read.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use rune::{Env, Key, KeyEvent, Mods, Runtime, TextEdit};

use super::source::{
    CodeKind, DeviceStatus, FixtureSignInSource, GateState, HelloOutcome, NextAfterSignIn, Script, SharedSession,
    SignInEvent, SignInProblem, SignInSource, board,
};
use super::{EMAIL_KEY, FIRST_BUTTON_KEY, HelloFailure, Intent, PASSWORD_KEY, Screen, TITLE_KEY, Ui, scene};
use crate::shell::app::Event;
use crate::ui::{au, kit};
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
    // Signed out: the registry draws this group (`shell/registry.rs`).
    crate::shell::host::dispatch(&mut state, Event::Gate(crate::shell::app::GateState::SignedOut));
    state.signin = Some(ui);
    let env = Env {
        system_dark: dark,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    // As the app runs: the shell's keys first, then this group's through the registry.
    rt.on_key = Some(crate::shell::host::on_key);
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
    // A screen that just replaced another is still moving in: let it land before aiming.
    rt.settle_all();
    rt.frame(0.0);
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
    assert!(
        ui(&mut rt).password.text.is_empty(),
        "the password is gone once it did its work"
    );
}

#[test]
fn sign_in_waits_for_an_address_and_a_password() {
    let mut blank = scene("DW-A03").expect("a scene");
    blank.email = TextEdit::new("anna");
    assert!(!blank.can_submit(), "anna is not an address");
    blank.email = TextEdit::new(board::EMAIL);
    blank.password.clear();
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

/// What a tool or assistive technology can read of the window: every element's key, label,
/// text and path, and the accessibility tree.
fn readable(rt: &Runtime<State>) -> [(&'static str, String); 2] {
    let tree: String = rt
        .elements()
        .iter()
        .map(|e| format!("{:?}|{:?}|{:?}|{}\n", e.key, e.label, e.text, e.path))
        .collect();
    [
        ("element tree", tree),
        ("accessibility tree", rt.accessibility_json().to_string()),
    ]
}

/// The PRODUCT's password field (`Ui::new`, which every scene and the running app start from)
/// is secure: a password typed into it is in no element text, no label, no accessibility tree
/// and no `Debug`, hidden and shown. No test here makes a field of its own, so a field that
/// loses `.secure()` in `Ui::new` fails this. The same search finds the email, typed the same
/// way into the plain field beside it.
#[test]
fn a_password_typed_into_the_products_field_is_in_no_tree_and_no_label() {
    const SECRET: &str = "Zx9-harbor-quartz-41";
    for revealed in [false, true] {
        let mut rt = runtime_of(Ui::new(Box::new(FixtureSignInSource::new(Script::default()))), false);
        assert!(
            ui(&mut rt).password.is_secure(),
            "the product's password field is a secure field"
        );
        click(&mut rt, EMAIL_KEY);
        press(&mut rt, Key::Text(board::EMAIL.into()));
        click(&mut rt, PASSWORD_KEY);
        press(&mut rt, Key::Text(SECRET.into()));
        assert_eq!(ui(&mut rt).password.text, SECRET, "the field took the keys");
        if revealed {
            tab_to(&mut rt, "field-1-act");
            space(&mut rt);
            assert!(ui(&mut rt).password.is_revealed(), "the eye works from the keyboard");
        }
        let debug = format!("{:?}", ui(&mut rt).password);
        let mut dumps = readable(&rt).to_vec();
        dumps.push(("Debug", debug));
        for (name, dump) in &dumps {
            assert!(
                !dump.contains(SECRET),
                "the password is in the {name} (revealed: {revealed})"
            );
            assert!(!dump.contains("harbor"), "part of the password is in the {name}");
        }
        assert!(
            dumps[0].1.contains(board::EMAIL),
            "the same search finds a plain field's text"
        );
        assert!(
            dumps[1].1.contains(board::EMAIL),
            "and finds it in the accessibility tree"
        );
        assert_eq!(info(&rt, PASSWORD_KEY).text, None, "a secure field reports no text");
        assert_eq!(
            info(&rt, "field-1-act").label.as_deref(),
            Some(if revealed { "Hide password" } else { "Show password" })
        );
    }
}

#[test]
fn every_scene_keeps_the_products_secure_field() {
    for (frame, _) in BUILT {
        let ui = scene(frame).expect("a scene");
        assert!(ui.password.is_secure(), "{frame}: the password field is secure");
        assert!(!ui.password.is_revealed(), "{frame}: and hidden");
    }
    assert!(
        super::start().expect("a state").password.is_secure(),
        "the running app's field"
    );
}

/// A shown password is told to nobody but Rune's secure input: the frame gate gets no note of
/// it and this kit does not measure or copy it. Hidden, the gate gets bullets: the count only.
#[test]
fn a_shown_password_is_not_noted_measured_or_copied() {
    const SECRET: &str = "Qv7-lantern-88";
    for revealed in [false, true] {
        let mut ui = Ui::new(Box::new(FixtureSignInSource::new(Script::default())));
        ui.email.replace_text(board::EMAIL);
        ui.password.replace_text(SECRET);
        if revealed {
            ui.toggle_password();
        }
        match (revealed, au::gate_text(&ui.password)) {
            (true, told) => assert_eq!(told, None, "a shown secret is not handed out"),
            (false, told) => assert_eq!(
                told.as_deref(),
                Some("\u{2022}".repeat(SECRET.chars().count()).as_str())
            ),
        }
        assert_eq!(
            au::gate_text(&ui.email).as_deref(),
            Some(board::EMAIL),
            "a plain field's text is"
        );
        // As the gate export runs: notes are kept while the view is built.
        kit::record();
        let rt = runtime_of(ui, false);
        let notes = kit::take();
        assert!(has(&rt, PASSWORD_KEY));
        assert_eq!(
            notes.get("field-0-value").and_then(|n| n.text.as_deref()),
            Some(board::EMAIL),
            "the gate is told the email"
        );
        for (key, note) in &notes {
            let text = note.text.as_deref().unwrap_or_default();
            assert!(
                !text.contains("lantern"),
                "the gate's note {key} holds the password (revealed: {revealed})"
            );
        }
        assert_eq!(
            notes.contains_key("field-1-value"),
            !revealed,
            "bullets are noted, a shown password is not"
        );
    }
}

#[test]
fn leaving_the_window_hides_a_shown_password() {
    let mut rt = runtime("DW-A02");
    ui(&mut rt).toggle_password();
    assert!(ui(&mut rt).password.is_revealed());
    super::on_window_focus(&mut rt.state, true);
    assert!(ui(&mut rt).password.is_revealed(), "coming back changes nothing");
    super::on_window_focus(&mut rt.state, false);
    assert!(
        !ui(&mut rt).password.is_revealed(),
        "the app switcher's picture holds bullets"
    );
}

// ---------------------------------------------------------------------------
// An engine that answers late, and whenever the test says
// ---------------------------------------------------------------------------

type Queue = Rc<RefCell<VecDeque<SignInEvent>>>;
type Calls = Rc<RefCell<Vec<&'static str>>>;

/// Answers only what the test pushes, and goes on answering after a cancel: the worst engine.
struct Late {
    queue: Queue,
    calls: Calls,
}

fn late() -> (Ui, Queue, Calls) {
    let (queue, calls) = (Queue::default(), Calls::default());
    let mut ui = Ui::new(Box::new(Late {
        queue: queue.clone(),
        calls: calls.clone(),
    }));
    ui.email.replace_text(board::EMAIL);
    ui.password.replace_text("x".repeat(board::PASSWORD_LEN));
    (ui, queue, calls)
}

impl SignInSource for Late {
    fn gate(&self) -> GateState {
        GateState::SignedOut
    }
    fn shared_session(&self) -> Option<SharedSession> {
        FixtureSignInSource::new(Script::default()).shared_session()
    }
    fn begin_sign_in(&mut self, _: &str, _: &str) {
        self.calls.borrow_mut().push("sign_in");
    }
    fn begin_hello(&mut self) {
        self.calls.borrow_mut().push("hello");
    }
    fn cancel_hello(&mut self) {
        self.calls.borrow_mut().push("cancel_hello");
    }
    fn begin_device_request(&mut self) {
        self.calls.borrow_mut().push("device_request");
    }
    fn cancel_device_request(&mut self) {
        self.calls.borrow_mut().push("cancel_device_request");
    }
    fn begin_two_factor(&mut self, _: &str, _: CodeKind) {}
    fn begin_restore(&mut self, _: &[&str]) {}
    fn begin_passkey(&mut self) {}
    fn begin_unlock(&mut self, _: &str) {}
    fn begin_recovery_verify(&mut self, _: &str, _: &[&str]) {}
    fn begin_set_password(&mut self, _: &str) {}
    fn poll(&mut self) -> Option<SignInEvent> {
        self.queue.borrow_mut().pop_front()
    }
}

/// The engine answered between presses: what `shell::host::poll` will run every frame.
fn deliver(rt: &mut Runtime<State>, queue: &Queue, event: SignInEvent) -> bool {
    queue.borrow_mut().push_back(event);
    let mut changed = false;
    rt.update(|s, tx| changed = super::poll(s, tx));
    rt.frame(0.0);
    changed
}

fn ctrl_c(rt: &mut Runtime<State>) {
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(0.0);
}

/// Review P2-4. Continue, back to Sign in, and THEN Windows Hello answers.
#[test]
fn a_late_windows_hello_answer_changes_nothing() {
    let (ui_, queue, calls) = late();
    let mut rt = runtime_of(ui_, false);
    click(&mut rt, "btn-0");
    assert_eq!(ui(&mut rt).screen, Screen::ContinueAs(None));
    click(&mut rt, "btn-0");
    assert_eq!(*calls.borrow(), ["hello"], "one prompt: a second press asks nothing");
    click(&mut rt, "btn-1");
    assert_eq!(ui(&mut rt).screen, Screen::SignIn, "Use another way to sign in");
    assert_eq!(
        *calls.borrow(),
        ["hello", "cancel_hello"],
        "leaving dismisses the prompt"
    );

    deliver(&mut rt, &queue, SignInEvent::Hello(HelloOutcome::NotRecognized));
    assert_eq!(
        ui(&mut rt).screen,
        Screen::SignIn,
        "a late 'not recognized' moves nothing"
    );
    deliver(
        &mut rt,
        &queue,
        SignInEvent::Hello(HelloOutcome::Succeeded(NextAfterSignIn::Drive)),
    );
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert!(
        ui(&mut rt).take_intents().is_empty(),
        "a late success signs nobody in from the form"
    );

    // Asked for again, the answer counts again.
    click(&mut rt, "btn-0");
    deliver(
        &mut rt,
        &queue,
        SignInEvent::Hello(HelloOutcome::Succeeded(NextAfterSignIn::Drive)),
    );
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
}

/// Review P2-3. Board DW-A18: "Back and the other paths are disabled while it runs".
#[test]
fn while_a_sign_in_runs_no_other_path_leaves() {
    let (ui_, queue, calls) = late();
    let mut rt = runtime_of(ui_, false);
    for link in ["link-0-box", "link-1-box", "link-2-box"] {
        assert!(info(&rt, link).focusable, "{link} is a Tab stop before");
    }
    click(&mut rt, "btn-2");
    assert!(ui(&mut rt).busy);
    for link in ["link-0-box", "link-1-box", "link-2-box"] {
        click(&mut rt, link);
        assert!(!info(&rt, link).focusable, "{link} takes no focus while it runs");
    }
    click(&mut rt, "btn-0");
    click(&mut rt, "btn-1");
    click(&mut rt, "btn-2");
    assert!(
        ui(&mut rt).take_intents().is_empty(),
        "Forgot password?, Use a passkey and Create an account lead nowhere while it runs"
    );
    ui(&mut rt).leave(Intent::ForgotPassword);
    assert!(ui(&mut rt).take_intents().is_empty(), "not from the keyboard either");
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
    assert_eq!(*calls.borrow(), ["sign_in"], "and nothing else was asked of the engine");

    // The engine's one answer is the one destination.
    deliver(&mut rt, &queue, SignInEvent::SignedIn(Ok(NextAfterSignIn::Drive)));
    assert_eq!(ui(&mut rt).take_intents(), vec![Intent::Next(NextAfterSignIn::Drive)]);
    // An answer nobody waits for (a second one) is no second destination.
    deliver(&mut rt, &queue, SignInEvent::SignedIn(Ok(NextAfterSignIn::Drive)));
    deliver(&mut rt, &queue, SignInEvent::AccountDeleted);
    assert!(ui(&mut rt).take_intents().is_empty());
    click(&mut rt, "link-0-box");
    assert_eq!(
        ui(&mut rt).take_intents(),
        vec![Intent::ForgotPassword],
        "the paths work again"
    );
}

/// Review P2-6, focus. DW-A05: "focus returns to Try again".
#[test]
fn a_failed_windows_hello_puts_focus_on_try_again() {
    for outcome in [HelloOutcome::NotRecognized, HelloOutcome::Cancelled] {
        let (ui_, queue, _) = late();
        let mut rt = runtime_of(ui_, false);
        click(&mut rt, "btn-0");
        tab_to(&mut rt, "btn-1");
        assert!(deliver(&mut rt, &queue, SignInEvent::Hello(outcome)));
        assert_eq!(
            info(&rt, "btn-0-label").text.as_deref(),
            Some("Try Windows Hello again")
        );
        assert_eq!(focused(&rt).as_deref(), Some(FIRST_BUTTON_KEY), "{outcome:?}");
    }
}

/// Review P2-6, focus. DW-A19 to DW-A22: "the field with the problem keeps focus".
#[test]
fn a_refused_sign_in_puts_focus_in_the_password_field() {
    for problem in [
        SignInProblem::Invalid,
        SignInProblem::RateLimited { seconds: 30 },
        SignInProblem::Other,
    ] {
        let (ui_, queue, _) = late();
        let mut rt = runtime_of(ui_, false);
        tab_to(&mut rt, "btn-2");
        space(&mut rt);
        assert!(ui(&mut rt).busy);
        deliver(&mut rt, &queue, SignInEvent::SignedIn(Err(problem)));
        assert_eq!(focused(&rt).as_deref(), Some(PASSWORD_KEY), "{problem:?}");
        assert_eq!(info(&rt, "field-1-msg").role, "alert");
    }
    // Offline: the same field, and Enter there checks the connection again (DW-A20).
    let (ui_, queue, calls) = late();
    let mut rt = runtime_of(ui_, false);
    tab_to(&mut rt, "btn-2");
    space(&mut rt);
    assert_eq!(focused(&rt), None, "the running button holds no focus");
    deliver(&mut rt, &queue, SignInEvent::SignedIn(Err(SignInProblem::Offline)));
    assert_eq!(focused(&rt).as_deref(), Some(PASSWORD_KEY));
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["sign_in", "sign_in"]);
}

/// Review P2-6. DW-A21: "the button counts down and re-enables itself".
#[test]
fn the_network_wait_counts_down_and_sign_in_comes_back() {
    let (ui_, queue, calls) = late();
    let mut rt = runtime_of(ui_, false);
    click(&mut rt, "btn-2");
    deliver(
        &mut rt,
        &queue,
        SignInEvent::SignedIn(Err(SignInProblem::RateLimited { seconds: 30 })),
    );
    let message = |rt: &Runtime<State>| info(rt, "field-1-msg").text.unwrap_or_default();
    assert!(message(&rt).contains("Wait 30 seconds"));
    assert!(!info(&rt, "btn-2").focusable, "no press during the wait");
    rt.settle_all();
    rt.frame(0.0);
    assert_eq!(
        info(&rt, "btn-2").opacity,
        1.0,
        "drawn at full strength, as the board draws DW-A21"
    );
    click(&mut rt, "btn-2");
    click(&mut rt, PASSWORD_KEY);
    press(&mut rt, Key::Enter);
    assert_eq!(*calls.borrow(), ["sign_in"], "a press does not earn another 429");

    deliver(&mut rt, &queue, SignInEvent::RetryAfter { seconds: 12 });
    assert!(message(&rt).contains("Wait 12 seconds"));
    deliver(&mut rt, &queue, SignInEvent::RetryAfter { seconds: 1 });
    assert!(message(&rt).contains("Wait 1 second,"));
    assert_eq!(ui(&mut rt).retry_wait(), Some(1));
    deliver(&mut rt, &queue, SignInEvent::RetryAfter { seconds: 0 });
    assert!(!has(&rt, "field-1-msg"), "the message goes with the wait");
    assert!(info(&rt, "btn-2").focusable, "Sign in is back by itself");
    click(&mut rt, "btn-2");
    assert_eq!(*calls.borrow(), ["sign_in", "sign_in"]);
}

/// Review P2-6. DW-A07: the pill reads "Reconnecting…", the countdown goes on; DW-A08:
/// focus moves to the heading; and Ctrl+C copies the code.
#[test]
fn a_waiting_request_reconnects_counts_on_and_ends_on_the_heading() {
    let (ui_, queue, calls) = late();
    let mut rt = runtime_of(ui_, false);
    click(&mut rt, "btn-1");
    ctrl_c(&mut rt);
    assert!(rt.copy_request.is_none(), "no code yet: nothing to copy");
    assert!(deliver(
        &mut rt,
        &queue,
        SignInEvent::DeviceRequested(Ok(FixtureSignInSource::request()))
    ));
    assert_eq!(
        info(&rt, "status-pill-text").text.as_deref(),
        Some("Waiting for approval")
    );

    deliver(&mut rt, &queue, SignInEvent::DeviceConnection { connected: false });
    assert_eq!(
        info(&rt, "status-pill-text").text.as_deref(),
        Some("Reconnecting\u{2026}")
    );
    assert_eq!(info(&rt, "status-pill").role, "status");
    deliver(&mut rt, &queue, SignInEvent::DeviceTick { seconds_left: 200 });
    assert_eq!(
        info(&rt, "status-countdown").label.as_deref(),
        Some("Expires in 3:20"),
        "the countdown goes on"
    );
    assert_eq!(
        info(&rt, "code").text.as_deref(),
        Some(board::CODE),
        "the request is not lost"
    );
    deliver(&mut rt, &queue, SignInEvent::DeviceConnection { connected: true });
    assert_eq!(
        info(&rt, "status-pill-text").text.as_deref(),
        Some("Waiting for approval")
    );

    ctrl_c(&mut rt);
    let copied = rt.copy_request.take().expect("Ctrl+C copies the code");
    assert_eq!(copied.content().text(), board::CODE);

    tab_to(&mut rt, "btn-0");
    deliver(
        &mut rt,
        &queue,
        SignInEvent::Device(DeviceStatus::Approved {
            by: board::OTHER_DEVICE.into(),
        }),
    );
    assert_eq!(info(&rt, TITLE_KEY).text.as_deref(), Some("Approved"));
    assert_eq!(focused(&rt).as_deref(), Some(TITLE_KEY), "focus moves to the heading");
    ctrl_c(&mut rt);
    assert!(rt.copy_request.is_none(), "the code is spent: nothing to copy");
    assert_eq!(*calls.borrow(), ["device_request"]);
}

/// Review P2-6. DW-A08's rule: "Approved, but the keys did not arrive. Nothing was signed in.
/// Ask again."
#[test]
fn approved_without_keys_signs_nobody_in_and_offers_to_ask_again() {
    let (ui_, queue, calls) = late();
    let mut rt = runtime_of(ui_, false);
    click(&mut rt, "btn-1");
    deliver(
        &mut rt,
        &queue,
        SignInEvent::DeviceRequested(Ok(FixtureSignInSource::request())),
    );
    deliver(
        &mut rt,
        &queue,
        SignInEvent::Device(DeviceStatus::KeysNotReceived {
            by: board::OTHER_DEVICE.into(),
        }),
    );
    assert_eq!(ui(&mut rt).screen, Screen::KeysNotReceived);
    assert_eq!(info(&rt, "banner-0").role, "alert");
    assert_eq!(
        info(&rt, "banner-0-title").text.as_deref(),
        Some("Approved, but the keys did not arrive")
    );
    assert_eq!(
        info(&rt, "banner-0-text").text.as_deref(),
        Some("Nothing was signed in. Ask again.")
    );
    assert!(!has(&rt, "code"), "the spent code is not shown");
    assert!(ui(&mut rt).take_intents().is_empty(), "nothing was signed in");
    assert_eq!(info(&rt, "btn-0-label").text.as_deref(), Some("Ask again"));
    tab_to(&mut rt, "btn-0");
    space(&mut rt);
    assert_eq!(ui(&mut rt).screen, Screen::Device { expired: false });
    assert_eq!(*calls.borrow(), ["device_request", "device_request"]);
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&mut rt).screen, Screen::SignIn);
}

#[test]
fn answers_of_a_request_already_left_change_nothing() {
    let (ui_, queue, calls) = late();
    let mut rt = runtime_of(ui_, false);
    click(&mut rt, "btn-1");
    click(&mut rt, "back-link");
    assert_eq!(*calls.borrow(), ["device_request", "cancel_device_request"]);
    for event in [
        SignInEvent::DeviceRequested(Ok(FixtureSignInSource::request())),
        SignInEvent::DeviceTick { seconds_left: 9 },
        SignInEvent::DeviceConnection { connected: false },
        SignInEvent::Device(DeviceStatus::Approved { by: "x".into() }),
        SignInEvent::DeviceRequested(Err(SignInProblem::Offline)),
    ] {
        deliver(&mut rt, &queue, event);
    }
    let ui = ui(&mut rt);
    assert_eq!(ui.screen, Screen::SignIn);
    assert!(ui.request().is_none() && ui.seconds_left() == 0 && !ui.reconnecting());
    assert_eq!(ui.problem, None);
    let mut changed = true;
    rt.update(|s, tx| changed = super::poll(s, tx));
    assert!(!changed, "nothing waiting: no change");
}

#[test]
fn ctrl_c_copies_nothing_where_no_live_code_is() {
    for frame in ["DW-A02", "DW-A10", "DW-A09"] {
        let mut rt = runtime(frame);
        ctrl_c(&mut rt);
        assert!(rt.copy_request.is_none(), "{frame}");
    }
    let mut rt = runtime("DW-A07");
    ctrl_c(&mut rt);
    assert_eq!(rt.copy_request.take().expect("the code").content().text(), board::CODE);
}

/// Signed out, the shell's registry draws this group, and the state the app starts from
/// names nobody: no Continue as card until the engine reports a real shared session.
#[test]
fn signed_out_the_registry_draws_sign_in_without_an_invented_person() {
    let mut state = init();
    crate::shell::host::dispatch(&mut state, Event::Gate(crate::shell::app::GateState::SignedOut));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, view, (958.0, 638.0), env);
    rt.frame(0.0);
    assert_eq!(info(&rt, TITLE_KEY).text.as_deref(), Some("Welcome to Beebeeb Drive"));
    click(&mut rt, "btn-1");
    assert_eq!(info(&rt, TITLE_KEY).text.as_deref(), Some("Sign in to Drive"));
    assert!(!has(&rt, "card-0"));
    let dump = readable(&rt)[0].1.clone();
    assert!(!dump.contains(board::NAME) && !dump.contains(board::EMAIL));
    // Signed in again, the registry no longer draws this group.
    crate::shell::host::dispatch(&mut rt.state, Event::Gate(crate::shell::app::GateState::Ready));
    rt.invalidate();
    rt.frame(0.0);
    assert!(!has(&rt, "au-side"));
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
