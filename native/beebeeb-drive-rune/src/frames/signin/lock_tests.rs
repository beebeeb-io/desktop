//! Tests of the two lock levels in the window (`lock.rs`, and the lock screen in `before_files.rs`): what each
//! press asks of the source, what each answer of Windows Hello leaves on the screen, and that nothing typed or
//! said under one lock is left when it goes. The window runs on the shell's fixture engine; the sign-in source
//! is a script whose calls and answers the test holds.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use super::source::{
    CodeKind, GateState as SourceGate, HelloOutcome, NextAfterSignIn, SharedSession, SignInEvent, SignInProblem,
    SignInSource, TwoFactorProblem,
};
use super::{PASSWORD_KEY, Ui, lock};
use crate::State;
use crate::shell::actions::FixtureEngine;
use crate::shell::app::{GateState, Screen};
use crate::shell::host::{self, Shell};
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state};
use rune::{Env, Key, KeyEvent, Mods, Runtime};

#[derive(Clone, Default)]
struct Script {
    /// The level the source names; the shell's gate says only "locked".
    vault: Rc<Cell<bool>>,
    /// A gate the source names instead of the two above (the engine's own word, before the shell follows).
    gate_override: Rc<Cell<Option<SourceGate>>>,
    calls: Rc<RefCell<Vec<&'static str>>>,
    answers: Rc<RefCell<VecDeque<SignInEvent>>>,
}

impl Script {
    fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
    fn count(&self, name: &str) -> usize {
        self.calls.borrow().iter().filter(|call| **call == name).count()
    }
}

impl SignInSource for Script {
    fn gate(&self) -> SourceGate {
        if let Some(gate) = self.gate_override.get() {
            gate
        } else if self.vault.get() {
            SourceGate::VaultClosed
        } else {
            SourceGate::Locked
        }
    }
    fn shared_session(&self) -> Option<SharedSession> {
        None
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
    fn begin_device_request(&mut self) {}
    fn cancel_device_request(&mut self) {}
    fn begin_two_factor(&mut self, _: &str, _: CodeKind) {
        self.calls.borrow_mut().push("two_factor");
    }
    fn begin_restore(&mut self, _: &[&str]) {}
    fn begin_passkey(&mut self) {}
    fn begin_unlock(&mut self, _: &str) {
        self.calls.borrow_mut().push("unlock");
    }
    fn begin_recovery_verify(&mut self, _: &str, _: &[&str]) {}
    fn begin_set_password(&mut self, _: &str) {}
    fn poll(&mut self) -> Option<SignInEvent> {
        self.answers.borrow_mut().pop_front()
    }
    fn discard_partial_sign_in(&mut self) {
        self.calls.borrow_mut().push("discard");
    }
}

/// The main window behind a lock; `vault`: the vault is closed, else the screen is locked.
fn window(vault: bool) -> (Arc<FixtureEngine>, Script, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::ready());
    engine.set_gate(GateState::Locked);
    let script = Script::default();
    script.vault.set(vault);
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

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| e.text)
        .collect()
}

fn shows(rt: &Runtime<State>, words: &str) -> bool {
    texts(rt).iter().any(|text| text.contains(words))
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

fn press(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

/// The source answered between presses.
fn answer(rt: &mut Runtime<State>, script: &Script, event: SignInEvent) {
    script.answers.borrow_mut().push_back(event);
    rt.frame(1.0);
    settle(rt);
}

fn type_password(rt: &mut Runtime<State>, text: &str) {
    rt.update(|s, _| {
        let ui = s.signin.as_mut().unwrap();
        ui.password.text = text.into();
    });
    settle(rt);
}

fn ui(rt: &Runtime<State>) -> &Ui {
    rt.state.signin.as_ref().unwrap()
}

#[test]
fn unlock_asks_windows_hello_once_and_each_answer_is_a_sentence_or_nothing() {
    let (_, script, mut rt) = window(false);
    assert_eq!(ui(&rt).lock_level(), Some(lock::Level::Screen));
    assert!(find(&rt, "gate-title").is_some(), "the lock screen");
    assert!(
        find(&rt, "gate-btn-1").is_none(),
        "one button until Windows Hello could not answer"
    );
    assert!(find(&rt, "gate-notice").is_none());

    // One prompt at a time: a second press while it is up asks nothing.
    click(&mut rt, lock::UNLOCK_KEY);
    click(&mut rt, lock::UNLOCK_KEY);
    press(&mut rt, Key::Enter);
    assert_eq!(script.calls(), ["hello"]);

    // The person closed the prompt: nothing is said, and Unlock asks again.
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::Cancelled));
    assert!(find(&rt, "gate-notice").is_none());
    assert!(find(&rt, "gate-btn-1").is_none());
    click(&mut rt, lock::UNLOCK_KEY);
    assert_eq!(script.count("hello"), 2);

    // Not recognised: said under the button, and the password is offered.
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::NotRecognized));
    assert!(shows(&rt, "Windows Hello didn't recognise you."));
    assert!(find(&rt, "gate-btn-1").is_some());
    // The next prompt takes the sentence away while it is up.
    click(&mut rt, lock::UNLOCK_KEY);
    assert_eq!(script.count("hello"), 3);
    assert!(find(&rt, "gate-notice").is_none());

    // No prompt on this PC: said, and the password is offered.
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::Unavailable));
    assert!(shows(&rt, "Windows Hello isn't available on this PC."));
    assert!(find(&rt, "gate-btn-1").is_some());

    // A yes moves nothing in the window: the engine's gate does.
    click(&mut rt, lock::UNLOCK_KEY);
    answer(
        &mut rt,
        &script,
        SignInEvent::Hello(HelloOutcome::Succeeded(NextAfterSignIn::Drive)),
    );
    assert_eq!(rt.state.shell.app.screen(), Screen::Gate(GateState::Locked));
    assert!(find(&rt, "gate-notice").is_none());
    assert_eq!(script.count("sign_in") + script.count("unlock"), 0);
}

#[test]
fn an_answer_of_windows_hello_nobody_waits_for_changes_nothing() {
    let (engine, script, mut rt) = window(false);
    // Never asked: a stray answer leaves no sentence.
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::NotRecognized));
    assert!(find(&rt, "gate-notice").is_none());

    // Asked, then the lock went (the engine opened it): the prompt is taken away, and its late "no" says nothing,
    // not now and not when the screen locks again.
    click(&mut rt, lock::UNLOCK_KEY);
    engine.set_gate(GateState::Ready);
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(script.calls(), ["hello", "cancel_hello"]);
    assert_eq!(ui(&rt).lock_level(), None);
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::NotRecognized));
    engine.set_gate(GateState::Locked);
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(ui(&rt).lock_level(), Some(lock::Level::Screen));
    assert!(
        find(&rt, "gate-notice").is_none(),
        "nothing of the earlier lock is left"
    );
    assert!(find(&rt, "gate-btn-1").is_none());
    assert!(!ui(&rt).lock_hello_waiting());
}

#[test]
fn a_sentence_and_a_typed_password_do_not_outlive_their_lock() {
    let (engine, script, mut rt) = window(false);
    click(&mut rt, lock::UNLOCK_KEY);
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::Unavailable));
    click(&mut rt, "gate-btn-1");
    assert!(find(&rt, PASSWORD_KEY).is_some(), "the password screen is in front");
    type_password(&mut rt, "typed under the first lock");

    // Unlocked, then locked again: the lock screen, one button, no sentence, no password.
    engine.set_gate(GateState::Ready);
    rt.frame(1.0);
    settle(&mut rt);
    engine.set_gate(GateState::Locked);
    rt.frame(1.0);
    settle(&mut rt);
    assert!(find(&rt, "gate-title").is_some());
    assert!(find(&rt, PASSWORD_KEY).is_none());
    assert!(find(&rt, "gate-notice").is_none());
    assert!(find(&rt, "gate-btn-1").is_none());
    assert!(ui(&rt).password.text.is_empty(), "what was typed is gone");
    assert!(!ui(&rt).lock_shows_password());
}

#[test]
fn the_screen_locks_password_is_in_front_of_the_lock_screen_and_back_returns_to_it() {
    let (_, script, mut rt) = window(false);
    click(&mut rt, lock::UNLOCK_KEY);
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::NotRecognized));
    click(&mut rt, "gate-btn-1");
    assert!(find(&rt, "gate-title").is_none(), "the lock screen is behind");
    assert!(shows(&rt, "Unlock with your password"));
    assert!(shows(&rt, "Beebeeb checks it with the server"));
    assert!(!shows(&rt, "Passkey"), "none of the passkey step's words");
    assert!(!shows(&rt, "decrypt"), "the lock decrypts nothing");
    assert_eq!(rt.state.shell.app.screen(), Screen::Gate(GateState::Locked));

    // Each refusal has its own words; nothing opens.
    for (problem, words) in [
        (SignInProblem::Invalid, "That password isn't right."),
        (
            SignInProblem::Offline,
            "You're offline. Beebeeb can't unlock until it can reach the server.",
        ),
        (SignInProblem::RateLimited { seconds: 30 }, "Too many tries."),
    ] {
        type_password(&mut rt, "a password");
        rt.update(|s, _| s.signin.as_mut().unwrap().unlock());
        answer(&mut rt, &script, SignInEvent::VaultUnlocked(Err(problem)));
        assert!(shows(&rt, words), "{problem:?}");
    }
    assert_eq!(script.count("unlock"), 3);

    // Escape goes back to the lock screen, never to the sign-in form; what was typed goes.
    press(&mut rt, Key::Escape);
    assert!(find(&rt, "gate-title").is_some());
    assert!(find(&rt, PASSWORD_KEY).is_none());
    assert!(ui(&rt).password.text.is_empty());
    assert_eq!(script.count("sign_in"), 0);
    assert!(
        script.count("discard") >= 1,
        "a proven password that waited for its code is forgotten"
    );
    assert!(
        find(&rt, "gate-btn-1").is_some(),
        "the way to the password is still offered"
    );
}

#[test]
fn a_closed_vault_shows_the_password_with_the_locks_words_then_the_code_and_never_the_form() {
    let (_, script, mut rt) = window(true);
    assert_eq!(ui(&rt).lock_level(), Some(lock::Level::Vault));
    assert!(
        find(&rt, "gate-title").is_none(),
        "no lock screen: Windows Hello opens no closed vault"
    );
    assert!(find(&rt, PASSWORD_KEY).is_some());
    assert!(shows(&rt, "Beebeeb is locked"));
    assert!(shows(&rt, "Sync is paused until you unlock."));
    assert!(!shows(&rt, "sync keeps running"));
    assert!(!shows(&rt, "Passkey"));
    assert_eq!(script.count("hello"), 0);

    // Nothing behind it: Escape leaves the screen where it is and asks nothing.
    press(&mut rt, Key::Escape);
    assert!(find(&rt, PASSWORD_KEY).is_some());
    assert_eq!(ui(&rt).screen, super::Screen::UnlockVault);

    // The password was accepted and the account has an authenticator: the code step.
    type_password(&mut rt, "the password");
    rt.update(|s, _| s.signin.as_mut().unwrap().unlock());
    assert_eq!(script.count("unlock"), 1);
    answer(
        &mut rt,
        &script,
        SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::TwoFactor)),
    );
    assert_eq!(ui(&rt).screen, super::Screen::TwoFactor { backup: false });
    assert!(ui(&rt).password.text.is_empty(), "the password did its work");

    // Back from the code: the password again (the proven password is forgotten), never the sign-in form.
    press(&mut rt, Key::Escape);
    assert_eq!(ui(&rt).screen, super::Screen::UnlockVault);
    assert_eq!(script.count("discard"), 1);
    assert_eq!(script.count("sign_in"), 0);
    assert!(find(&rt, PASSWORD_KEY).is_some());
}

#[test]
fn a_password_on_its_way_keeps_its_screen_when_the_engine_closes_the_vault_under_it() {
    let (_, script, mut rt) = window(false);
    click(&mut rt, lock::UNLOCK_KEY);
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::Unavailable));
    click(&mut rt, "gate-btn-1");
    type_password(&mut rt, "the password");
    rt.update(|s, _| s.signin.as_mut().unwrap().unlock());
    assert!(ui(&rt).unlocking());

    // The strong unlock stops the work and drops the key before it stores the new session: the level moves
    // to the vault while the answer is still on its way.
    script.vault.set(true);
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(ui(&rt).lock_level(), Some(lock::Level::Vault));
    assert!(ui(&rt).unlocking(), "the step is still waited for");
    assert_eq!(ui(&rt).password.text, "the password");
    answer(
        &mut rt,
        &script,
        SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::TwoFactor)),
    );
    assert_eq!(ui(&rt).screen, super::Screen::TwoFactor { backup: false });
}

#[test]
fn windows_hello_starts_by_itself_once_per_lock_when_the_window_comes_forward() {
    let (engine, script, mut rt) = window(false);
    assert_eq!(script.count("hello"), 0, "not before the window is in front");
    rt.update(|s, _| super::on_window_focus(s, true));
    assert_eq!(script.count("hello"), 1);
    answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::Cancelled));
    // The prompt went and the window is in front again: it does not come back by itself.
    rt.update(|s, _| super::on_window_focus(s, true));
    assert_eq!(script.count("hello"), 1);
    // A new lock asks again.
    engine.set_gate(GateState::Ready);
    rt.frame(1.0);
    settle(&mut rt);
    engine.set_gate(GateState::Locked);
    rt.frame(1.0);
    settle(&mut rt);
    rt.update(|s, _| super::on_window_focus(s, true));
    assert_eq!(script.count("hello"), 2);

    // A closed vault never asks Windows Hello.
    let (_, script, mut rt) = window(true);
    rt.update(|s, _| super::on_window_focus(s, true));
    assert_eq!(script.count("hello"), 0);
}

/// Sync is the app's work: the shell asks for it when a lock closes the gate too (the engine starts it behind a
/// locked screen and refuses it for a closed vault).
#[test]
fn the_shell_asks_for_sync_when_the_gate_closes_by_a_lock() {
    use crate::shell::actions::FixtureCall;
    let engine = Arc::new(FixtureEngine::ready());
    engine.set_gate(GateState::SignedOut);
    let state = state::init_with(Shell::fixture_on(engine.clone(), Arc::new(FakePlatform::default())));
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, registry::view, (1278.0, 798.0), env);
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    settle(&mut rt);
    let starts = |engine: &FixtureEngine| engine.calls().iter().filter(|c| **c == FixtureCall::StartSync).count();
    let before = starts(&engine);
    // The app starts with a stored session: the first gate the window sees after "signed out" is the lock.
    engine.set_gate(GateState::Locked);
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(starts(&engine), before + 1, "asked once, behind the lock");
    // A gate that closes for another reason asks for nothing.
    engine.set_gate(GateState::NoPlan);
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(starts(&engine), before + 1);
}

/// Both levels: the code step's partial sign-in ran out.
fn an_expired_code_returns_to_the_locks_password(vault: bool) {
    let (_, script, mut rt) = window(vault);
    if !vault {
        click(&mut rt, lock::UNLOCK_KEY);
        answer(&mut rt, &script, SignInEvent::Hello(HelloOutcome::Unavailable));
        click(&mut rt, "gate-btn-1");
    }
    type_password(&mut rt, "the password");
    rt.update(|s, _| s.signin.as_mut().unwrap().unlock());
    answer(
        &mut rt,
        &script,
        SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::TwoFactor)),
    );
    assert_eq!(ui(&rt).screen, super::Screen::TwoFactor { backup: false });
    rt.update(|s, _| {
        let ui = s.signin.as_mut().unwrap();
        ui.code.text = "123456".into();
        ui.verify_code();
    });
    answer(&mut rt, &script, SignInEvent::TwoFactor(Err(TwoFactorProblem::Expired)));
    assert_eq!(ui(&rt).screen, super::Screen::UnlockVault);
    assert!(find(&rt, PASSWORD_KEY).is_some(), "the lock's password screen");
    assert!(find(&rt, super::EMAIL_KEY).is_none(), "never the sign-in form");
    assert!(ui(&rt).unlock_problem().is_some(), "a problem line says it ran out");
    assert!(ui(&rt).password.text.is_empty());
    if !vault {
        press(&mut rt, Key::Escape);
        assert!(find(&rt, lock::UNLOCK_KEY).is_some(), "Escape shows the lock screen");
        assert!(find(&rt, super::EMAIL_KEY).is_none());
    }
}

#[test]
fn an_expired_code_under_a_closed_vault_returns_to_the_locks_password() {
    an_expired_code_returns_to_the_locks_password(true);
}

#[test]
fn an_expired_code_under_a_locked_screen_returns_to_the_locks_password() {
    an_expired_code_returns_to_the_locks_password(false);
}

/// A strong unlock that ends keyless: the engine's gate is `NeedsRecoveryPhrase` (core's `VaultClosed`) and the
/// twelve words stay in front whichever of the shell's gate and the source's gate moves first.
#[test]
fn a_keyless_strong_unlock_keeps_the_twelve_words_in_front() {
    for shell_first in [false, true] {
        let (engine, script, mut rt) = window(true);
        type_password(&mut rt, "the password");
        rt.update(|s, _| s.signin.as_mut().unwrap().unlock());
        if shell_first {
            engine.set_gate(GateState::NeedsRecoveryPhrase);
        }
        script.gate_override.set(Some(SourceGate::NeedsRecoveryPhrase));
        answer(
            &mut rt,
            &script,
            SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::RecoveryPhrase)),
        );
        let words_in_front = |rt: &Runtime<State>| {
            find(rt, super::steps::WORD_KEYS[0]).is_some() && find(rt, super::EMAIL_KEY).is_none()
        };
        assert!(words_in_front(&rt), "after the answer (shell_first {shell_first})");
        if !shell_first {
            engine.set_gate(GateState::NeedsRecoveryPhrase);
        }
        for frame in 0..2 {
            rt.frame(1.0);
            settle(&mut rt);
            assert!(words_in_front(&rt), "frame {frame} (shell_first {shell_first})");
        }
    }
}

#[test]
fn windows_hello_starts_by_itself_when_the_focus_came_before_the_first_poll() {
    // The window is built WITHOUT the initial frames: the focus comes first.
    let engine = Arc::new(FixtureEngine::ready());
    engine.set_gate(GateState::Locked);
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
    rt.update(|s, _| super::on_window_focus(s, true));
    assert_eq!(script.count("hello"), 0, "nothing follows the lock yet");
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(script.count("hello"), 1, "the first poll starts the prompt");
    rt.frame(1.0);
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(script.count("hello"), 1, "once per lock");

    // A window that polled unlocked and has the focus: a lock that begins later starts nothing by itself.
    let (engine, script, mut rt) = window(false);
    engine.set_gate(GateState::Ready);
    rt.frame(1.0);
    settle(&mut rt);
    // The lock and the focus arrive before the next poll: the focus is not "before the first poll".
    rt.update(|s, _| {
        engine.set_gate(GateState::Locked);
        super::on_window_focus(s, true);
    });
    rt.frame(1.0);
    settle(&mut rt);
    assert_eq!(ui(&rt).lock_level(), Some(lock::Level::Screen));
    assert_eq!(script.count("hello"), 0);
}
