//! Two-factor in the running app's window, through the window's own poll (`shell::host::poll`, which runs
//! `frames::account::poll` and the shell's gate change) on the real engine against the parent module's fake: backup
//! codes Beebeeb made while the screen locked are shown after the unlock, once, and never after a sign-out or a
//! closed vault. Every name and code is invented.

use rune::{Env, Runtime, Tx};

use super::*;
use crate::State;
use crate::frames::account::{CodeFor, Dialog};
use crate::shell::app::{Event as ShellEvent, GateState, SettingsPane};
use crate::shell::host;

const CODES_ROUTE: &str = "POST /api/v1/auth/2fa/backup-codes";

fn window_on(engine: TestEngine) -> Runtime<State> {
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 638.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(|s: &mut State, seconds: f64, tx: &mut Tx| host::poll(s, seconds, tx));
    rt.frame(0.0);
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::OpenSettings(SettingsPane::Security));
    });
    rt.frame(0.016);
    rt
}

/// Frames for `millis`, the window polling as it runs.
fn run_for(rt: &mut Runtime<State>, millis: u64) {
    let began = Instant::now();
    while began.elapsed() < Duration::from_millis(millis) {
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.05);
    }
}

fn wait_for(rt: &mut Runtime<State>, what: &str, done: impl Fn(&Runtime<State>) -> bool) {
    let began = Instant::now();
    while !done(rt) {
        assert!(began.elapsed() < Duration::from_secs(10), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.05);
    }
}

/// Presses the control that says `words` (its text or its label), the one drawn last (a confirm is over its page).
fn click_text(rt: &mut Runtime<State>, words: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .rfind(|e| {
            !e.exiting
                && (e.text.as_deref() == Some(words) || e.label.as_deref().is_some_and(|label| label.contains(words)))
        })
        .unwrap_or_else(|| {
            let drawn: Vec<String> = rt
                .elements()
                .iter()
                .filter(|e| !e.exiting)
                .filter_map(|e| e.text.clone().or_else(|| e.label.clone()))
                .collect();
            panic!("nothing says {words}; drawn: {drawn:?}")
        })
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.016);
}

/// The codes the Backup codes page holds to show: how many, when it shows.
fn codes_shown(rt: &Runtime<State>) -> Option<usize> {
    let ui = &rt.state.account;
    (ui.pane == Pane::BackupCodes)
        .then(|| ui.form.codes.as_ref().map(Vec::len))
        .flatten()
}

/// The Two-factor page, "Make new backup codes", a current code, the press: the request is out.
fn ask_for_new_codes(fake: &Fake, rt: &mut Runtime<State>) {
    fake.disable_delay_ms.store(600, Ordering::SeqCst);
    let read = |rt: &Runtime<State>| {
        !rt.elements().iter().any(|e| {
            !e.exiting
                && e.text
                    .as_deref()
                    .is_some_and(|text| text.contains("Asking Beebeeb for your account"))
        })
    };
    // The Security page no longer waits for the account read (it is drawn at once, so Sign out works offline):
    // the Two-factor page, which does wait for it, is drawn first, then its "Asking" waited out.
    rt.update(|s, _| crate::frames::account::go(s, "DW-A71"));
    rt.frame(0.016);
    wait_for(rt, "the Two-factor page is read", |rt| {
        read(rt) && rt.state.account.source.as_deref().is_some_and(|s| s.pane_state(Pane::TwoFactor) == PaneState::Ready)
    });
    rt.update(|s, _| {
        s.account.dialog = Some(Dialog::Code(CodeFor::NewBackupCodes));
        s.account.form.code.text = "123456".into();
    });
    rt.frame(0.016);
    click_text(rt, "Make new codes");
    assert_eq!(rt.state.account.form.waiting, Some(CallKind::NewBackupCodes));
    wait_for(rt, "the request left", |_| {
        fake.asked.lock().unwrap().iter().any(|route| route == CODES_ROUTE)
    });
}

/// The engine locks the screen, and the window's gate follows.
fn lock_the_screen(engine: &TestEngine, rt: &mut Runtime<State>) {
    engine.lock_screen().expect("an open lock locks the screen");
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::Gate(GateState::Locked));
    });
    rt.frame(0.016);
}

/// Review of desktop#424 (delta 2 P1): the screen locks while "New backup codes" is out. Beebeeb makes the codes (the
/// old ones stop working); after the unlock the Backup codes page shows them, once. They are never drawn while the
/// screen is locked.
#[test]
fn backup_codes_made_while_the_screen_locks_are_shown_after_the_unlock() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut rt = window_on(engine.clone());
    ask_for_new_codes(&fake, &mut rt);
    lock_the_screen(&engine, &mut rt);
    // The answer lands under the lock screen.
    run_for(&mut rt, 1_200);
    assert_eq!(codes_shown(&rt), None, "nothing is drawn while the screen is locked");

    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::Gate(GateState::Ready));
    });
    wait_for(&mut rt, "the codes after the unlock", |rt| codes_shown(rt) == Some(2));

    // Once: left with "I've saved them", they are not shown again.
    rt.update(|s, _| crate::frames::account::go(s, "DW-A71"));
    run_for(&mut rt, 300);
    assert_eq!(codes_shown(&rt), None);
}

/// The other half: codes held under a lock are dropped (wiped) by a sign-out and by a closed vault; a later sign-in
/// of the same account shows nothing of them.
#[test]
fn backup_codes_held_under_a_lock_are_dropped_by_a_sign_out_or_a_closed_vault() {
    // A sign-out from the lock screen.
    let fake = fake();
    let engine = signed_in(&fake);
    let mut rt = window_on(engine.clone());
    ask_for_new_codes(&fake, &mut rt);
    lock_the_screen(&engine, &mut rt);
    run_for(&mut rt, 1_200);
    engine.sign_out_keeping_local_data().expect("the sign-out goes through");
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::Gate(GateState::SignedOut));
    });
    run_for(&mut rt, 200);
    engine
        .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::Gate(GateState::Ready));
        host::dispatch(s, ShellEvent::OpenSettings(SettingsPane::Security));
    });
    run_for(&mut rt, 400);
    assert_eq!(codes_shown(&rt), None, "a sign-out drops them");

    // A closed vault (the session leaves memory) under the lock.
    let fake = super::fake();
    let engine = signed_in(&fake);
    let mut rt = window_on(engine.clone());
    ask_for_new_codes(&fake, &mut rt);
    lock_the_screen(&engine, &mut rt);
    run_for(&mut rt, 1_200);
    engine.lock();
    run_for(&mut rt, 200);
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::Gate(GateState::Ready));
    });
    run_for(&mut rt, 400);
    assert_eq!(codes_shown(&rt), None, "a closed vault drops them");
}

/// The other order of the review: the codes already reached the Backup codes page when the screen locks. The lock
/// ends the page, the codes go back to the source (never drawn while locked), and the page shows them again after the
/// unlock.
#[test]
fn backup_codes_on_the_page_when_the_screen_locks_are_shown_again_after_the_unlock() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut rt = window_on(engine.clone());
    ask_for_new_codes(&fake, &mut rt);
    wait_for(&mut rt, "the codes on their page", |rt| codes_shown(rt) == Some(2));
    lock_the_screen(&engine, &mut rt);
    run_for(&mut rt, 300);
    assert_eq!(codes_shown(&rt), None, "nothing is drawn while the screen is locked");
    assert!(
        rt.state.account.form.codes.is_none(),
        "the page holds none under the lock"
    );

    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    rt.update(|s, _| {
        host::dispatch(s, ShellEvent::Gate(GateState::Ready));
    });
    wait_for(&mut rt, "the codes again after the unlock", |rt| {
        codes_shown(rt) == Some(2)
    });
}
