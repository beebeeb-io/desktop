//! DW-A08 seen over the gate the approval opened (CP2 row 3.4, the shell half). The engine opens the session before
//! the request's answer reaches the window, so the window keeps its screen until "Open Drive".

use super::source::{DeviceStatus, FixtureSignInSource, Script, SignInEvent};
use super::{Intent, NextAfterSignIn, Screen, Ui};
use crate::shell::app::{AppMachine, GateState, Screen as Shown, Session};

fn waiting() -> Ui {
    let mut ui = Ui::new(Box::new(FixtureSignInSource::new(Script::default())));
    ui.follow_gate(GateState::SignedOut);
    ui.ask_other_device();
    ui
}

/// Mutant: remove the `GateState::Ready if self.holds_open_gate()` arm of `follow_gate` (the open gate resets the screen).
#[test]
fn an_approval_is_seen_over_the_gate_it_opened_and_open_drive_leaves_it() {
    let mut ui = waiting();
    // The gate opens first, while the request still waits; then the answer arrives.
    ui.follow_gate(GateState::Ready);
    assert_eq!(
        ui.screen,
        Screen::Device { expired: false },
        "the waiting request is not dropped"
    );
    ui.apply(SignInEvent::Device(DeviceStatus::Approved {
        by: "another device".into(),
    }));
    ui.follow_gate(GateState::Ready);
    assert!(matches!(ui.screen, Screen::Approved { .. }));
    assert!(ui.holds_open_gate());

    ui.open_drive();
    assert_eq!(ui.screen, Screen::SignIn);
    assert!(!ui.holds_open_gate(), "after Open Drive the gate decides alone");
    assert!(ui.take_intents().contains(&Intent::Next(NextAfterSignIn::Drive)));
}

/// Mutant: `screen` ignores `sign_in_hold`.
#[test]
fn the_shell_keeps_the_sign_in_window_over_an_open_gate_only_while_it_asks() {
    let mut app = AppMachine::for_session(Session::of(GateState::Ready, "user-a"));
    assert!(matches!(app.screen(), Shown::Drive(_)));
    assert!(app.hold_sign_in(true));
    assert_eq!(app.screen(), Shown::SignIn);
    assert!(app.hold_sign_in(false));
    assert!(matches!(app.screen(), Shown::Drive(_)));

    // A gate that is not open is never decided by the hold.
    let mut locked = AppMachine::for_session(Session::of(GateState::Locked, "user-a"));
    locked.hold_sign_in(true);
    assert_eq!(locked.screen(), Shown::Gate(GateState::Locked));
}

/// Mutant: drop `GateState::FirstRun` from the `follow_gate` hold arm (a new PC's approval is reset to the form).
#[test]
fn on_a_new_pc_the_approval_is_seen_before_the_first_run() {
    let mut ui = waiting();
    ui.follow_gate(GateState::FirstRun);
    assert_eq!(
        ui.screen,
        Screen::Device { expired: false },
        "the waiting request is not dropped"
    );
    ui.apply(SignInEvent::Device(DeviceStatus::Approved {
        by: "another device".into(),
    }));
    ui.follow_gate(GateState::FirstRun);
    assert!(matches!(ui.screen, Screen::Approved { .. }));
    assert!(ui.holds_open_gate());
    ui.open_drive();
    assert!(ui.take_intents().contains(&Intent::Next(NextAfterSignIn::Drive)));
}

/// Mutant: drop `GateState::FirstRun` from the `screen` hold arm in `shell/app.rs`.
#[test]
fn the_shell_holds_the_sign_in_window_over_the_first_run_gate_too() {
    let mut app = AppMachine::for_session(Session::of(GateState::FirstRun, "user-a"));
    assert_eq!(app.screen(), Shown::Gate(GateState::FirstRun));
    app.hold_sign_in(true);
    assert_eq!(app.screen(), Shown::SignIn);
    app.hold_sign_in(false);
    assert_eq!(app.screen(), Shown::Gate(GateState::FirstRun));
}
