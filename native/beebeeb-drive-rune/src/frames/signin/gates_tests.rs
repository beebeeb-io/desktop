//! w16-gates: the No plan buttons and the credential-store-unavailable card do what they say. The window runs on
//! the shell's fixture engine with a sign-in source whose calls the test holds; the gate is forced.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use super::super::Ui;
use super::super::source::{CodeKind, GateState as SourceGate, SharedSession, SignInEvent, SignInSource};
use crate::State;
use crate::engine_host::signin::OPENED;
use crate::shell::actions::FixtureEngine;
use crate::shell::app::{GateState, Screen};
use crate::shell::host::{self, Shell};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state, web_pages};
use rune::{Env, Runtime};

#[derive(Clone, Default)]
struct Script {
    calls: Rc<RefCell<Vec<&'static str>>>,
}

impl SignInSource for Script {
    fn gate(&self) -> SourceGate {
        SourceGate::SignedOut
    }
    fn shared_session(&self) -> Option<SharedSession> {
        None
    }
    fn begin_sign_in(&mut self, _: &str, _: &str) {}
    fn begin_hello(&mut self) {}
    fn cancel_hello(&mut self) {}
    fn begin_device_request(&mut self) {}
    fn cancel_device_request(&mut self) {}
    fn begin_two_factor(&mut self, _: &str, _: CodeKind) {}
    fn begin_restore(&mut self, _: &[&str]) {}
    fn begin_passkey(&mut self) {}
    fn begin_unlock(&mut self, _: &str) {}
    fn begin_screen_password(&mut self, _: &str) {}
    fn begin_recovery_verify(&mut self, _: &str, _: &[&str]) {}
    fn begin_set_password(&mut self, _: &str) {}
    fn poll(&mut self) -> Option<SignInEvent> {
        None
    }
    fn discard_partial_sign_in(&mut self) {}
    fn recheck_gate(&mut self) {
        self.calls.borrow_mut().push("recheck_gate");
    }
    fn sign_out_from_gate(&mut self) {
        self.calls.borrow_mut().push("sign_out");
    }
}

fn window(gate: GateState) -> (Arc<FixtureEngine>, Script, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::ready());
    engine.set_gate(gate);
    let script = Script::default();
    let mut state = state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    state.signin = Some(Ui::new(Box::new(script.clone())));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, registry::view, (1278.0, 798.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    (engine, script, rt)
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn settle(rt: &mut Runtime<State>) {
    rt.frame(0.016);
    rt.settle_all();
    rt.frame(0.016);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let r = find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}")).rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn says(rt: &Runtime<State>, key: &str) -> String {
    find(rt, key)
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .text
        .unwrap_or_default()
}

fn opened() -> Vec<String> {
    OPENED.with(|o| o.borrow().clone())
}

#[test]
fn choose_a_plan_opens_the_plan_page() {
    let (_, script, mut rt) = window(GateState::NoPlan);
    OPENED.with(|o| o.borrow_mut().clear());
    click(&mut rt, "gate-btn-0");
    assert_eq!(opened(), vec![web_pages::url("plan").unwrap().to_string()]);
    assert!(rt.state.frames.gate_notice.is_none(), "no 'Not available yet'");
    assert!(script.calls.borrow().is_empty());
}

#[test]
fn check_again_asks_the_engine_to_recheck_the_gate() {
    let (_, script, mut rt) = window(GateState::NoPlan);
    click(&mut rt, "gate-btn-1");
    assert_eq!(*script.calls.borrow(), vec!["recheck_gate"]);
    assert!(rt.state.frames.gate_notice.is_none());
    assert_eq!(
        rt.state.shell.app.screen(),
        Screen::Gate(GateState::NoPlan),
        "the engine moves the gate, not the press"
    );
}

#[test]
fn sign_out_takes_the_existing_sign_out_path() {
    let (_, script, mut rt) = window(GateState::NoPlan);
    click(&mut rt, "gate-btn-2");
    assert_eq!(*script.calls.borrow(), vec!["sign_out"]);
    assert!(rt.state.frames.gate_notice.is_none());
}

#[test]
fn the_keystore_card_says_what_is_wrong_and_has_no_sign_out() {
    let (_, _, rt) = window(GateState::KeyStoreUnavailable);
    assert!(find(&rt, "not-built").is_none(), "a frame, not the not-built card");
    assert_eq!(says(&rt, "gate-title"), "Can't read your keys");
    assert_eq!(
        says(&rt, "gate-text"),
        "Windows didn't answer. Nothing syncs until it does."
    );
    assert_eq!(says(&rt, "gate-btn-0-label"), "Try again");
    assert_eq!(says(&rt, "gate-btn-1-label"), "Get help");
    assert_eq!(says(&rt, "gate-btn-2-label"), "Quit");
    assert!(find(&rt, "gate-btn-3").is_none());
}

#[test]
fn try_again_rechecks_and_the_card_leaves_when_the_store_answers() {
    let (engine, script, mut rt) = window(GateState::KeyStoreUnavailable);
    click(&mut rt, "gate-btn-0");
    assert_eq!(*script.calls.borrow(), vec!["recheck_gate"]);
    assert!(
        find(&rt, "gate-title").is_some(),
        "the store has not answered: the card stays"
    );
    engine.set_gate(GateState::Ready);
    rt.frame(1.0);
    settle(&mut rt);
    assert!(find(&rt, "gate-title").is_none(), "the card is gone");
}

#[test]
fn get_help_opens_the_support_page() {
    let (_, _, mut rt) = window(GateState::KeyStoreUnavailable);
    OPENED.with(|o| o.borrow_mut().clear());
    click(&mut rt, "gate-btn-1");
    assert_eq!(opened(), vec![web_pages::url("support").unwrap().to_string()]);
}
