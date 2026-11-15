//! The away lock through the main window's poll, on a real engine handle against the engine's fake API: the shell
//! measures (a scripted idle from the fake platform), the engine compares and locks through core. Every name and
//! password is invented; nothing here waits on Windows.

use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::app::lock_api::LockState;
use beebeeb_engine::app::lock_password_api::{AppLockProof, LockAfter};
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::config::CONFIG_DIR_OVERRIDE;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use rune::{Env, Runtime};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::LiveEngine;
use crate::engine_host::open::Opened;
use crate::shell::actions::HostActions;
use crate::shell::app::GateState;
use crate::shell::host::{self, Shell, Wiring};
use crate::shell::platform::TrayEvent;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state};

const EMAIL: &str = "away-lock@example.eu";
const PASSWORD: &str = "an invented password for the away lock";

type TestEngine = Arc<Engine<MemoryCredentials>>;

struct Live {
    engine: TestEngine,
    platform: Arc<FakePlatform>,
    rt: Runtime<State>,
    _api: FakeApi,
}

/// A signed-in, open engine behind the real shell, on the fake platform.
fn live() -> Live {
    let mut live = live_with(false);
    // The first poll that sees the session open looks once (no input for no time): its answer first.
    answers(&mut live, 1);
    assert_eq!(live.rt.state.shell.away.looks, 1);
    live
}

/// [`live`], and with `first_run` this PC has not had its first run: the shell shows its steps (the gate is
/// `FirstRun`), with the engine signed in and open. No look is waited for.
fn live_with(first_run: bool) -> Live {
    let (account, phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    let api = FakeApi::start(account_handler(Arc::new(account)));
    let (engine_host, messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "away-lock-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, engine_host, MemoryCredentials::default()).expect("opens"));
    engine.restore();
    engine
        .sign_in_password(EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    assert_eq!(engine.lock_state(), LockState::Open);
    let opened = Opened {
        engine: engine.clone(),
        identity: &TEST,
        gate: engine.gate(),
    };
    let live_engine = Arc::new(LiveEngine::new(&opened, messages));
    if first_run {
        // Fresh settings, read once, as the app's start reads them.
        let settings = tempfile::tempdir().expect("a folder for the settings file");
        CONFIG_DIR_OVERRIDE.sync_scope(settings.path().to_path_buf(), || live_engine.wire_first_run());
    }
    let platform = Arc::new(FakePlatform::default());
    let shell = Shell::new(
        Arc::new(HostActions::new(live_engine, platform.clone())),
        platform.clone(),
        Wiring::default(),
    );
    let mut rt = Runtime::new(
        state::init_on_engine(shell, engine.clone()),
        registry::view,
        (1280.0, 800.0),
        Env {
            system_dark: false,
            scale: 1.0,
        },
    );
    rt.poll = Some(host::poll);
    rt.frame(0.0);
    Live {
        engine,
        platform,
        rt,
        _api: api,
    }
}

fn away(rt: &mut Runtime<State>) -> &mut AwayLock {
    &mut rt.state.shell.away
}

/// The PC had no input for `idle`, and the session opened long before: the next poll looks.
fn idle_for(live: &mut Live, idle: Duration) {
    *live.platform.idle.lock().unwrap() = Some(idle);
    let lock = away(&mut live.rt);
    lock.opened_at = Instant::now().checked_sub(Duration::from_secs(4 * 3_600));
    lock.due = None;
}

/// Frames until `done` holds (at most 10 s): the worker's answer comes in a later poll.
fn until(live: &mut Live, what: &str, done: impl Fn(&Live) -> bool) {
    let began = Instant::now();
    loop {
        live.rt.frame(0.016);
        if done(live) {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Frames until `count` answers in all were taken in, and no idle look is out.
fn answers(live: &mut Live, count: usize) {
    until(live, "a look did not answer", |live| {
        let lock = &live.rt.state.shell.away;
        lock.answered >= count && !lock.looking
    });
}

/// Frames until one more look than now was asked and answered.
fn looked(live: &mut Live) {
    let before = live.rt.state.shell.away.answered;
    answers(live, before + 1);
}

#[test]
fn idle_past_the_delay_locks_with_app_lock_on_and_short_of_it_does_not() {
    let mut live = live();
    idle_for(&mut live, Duration::from_secs(299));
    looked(&mut live);
    assert_eq!(live.engine.lock_state(), LockState::Open, "short of five minutes");
    // The next look is due when five minutes could be reached, and one wake is armed for it.
    let due = away(&mut live.rt).due.expect("a look is due");
    assert!(due <= Instant::now() + Duration::from_secs(1));
    idle_for(&mut live, Duration::from_secs(300));
    looked(&mut live);
    until(&mut live, "the screen did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
    // Locked: the window shows the lock, and no count runs.
    until(&mut live, "the window did not show the lock", |live| {
        live.rt.state.shell.away.opened_at.is_none()
    });
}

#[test]
fn with_app_lock_off_nothing_locks_neither_idle_nor_the_os_lock() {
    let mut live = live();
    live.engine
        .set_app_lock_for(live.engine.session_turn(), false, Some(AppLockProof::PlatformVerified))
        .unwrap();
    idle_for(&mut live, Duration::from_secs(10 * 3_600));
    looked(&mut live);
    assert_eq!(live.engine.lock_state(), LockState::Open);
    // Nothing could lock: the next look waits the longest (a later "on" is followed within it).
    let due = away(&mut live.rt).due.expect("a look is due");
    assert!(due > Instant::now() + LOOK_AT_MOST_EVERY - Duration::from_secs(5));
    let before = away(&mut live.rt).answered;
    live.rt.update(|s, tx| host::on_tray(s, TrayEvent::SessionLocked, tx));
    answers(&mut live, before + 1);
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "app lock off: nothing ever locks"
    );
}

#[test]
fn the_operating_systems_lock_locks_the_screen() {
    let mut live = live();
    live.rt.update(|s, tx| host::on_tray(s, TrayEvent::SessionLocked, tx));
    until(&mut live, "the OS lock did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
}

#[test]
fn an_unlock_without_input_starts_a_new_count() {
    let mut live = live();
    idle_for(&mut live, Duration::from_secs(3_600));
    looked(&mut live);
    until(&mut live, "the screen did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked && live.rt.state.shell.away.opened_at.is_none()
    });
    // Opened by Windows Hello's face check: no input, so the PC's idle stays an hour. The count starts at the opening.
    let before = away(&mut live.rt).answered;
    live.engine
        .unlock_screen_platform_verified(live.engine.session_turn())
        .unwrap();
    until(&mut live, "the window did not open", |live| {
        live.rt.state.shell.away.opened_at.is_some()
    });
    answers(&mut live, before + 1);
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "no lock right after an unlock"
    );
}

#[test]
fn a_look_arms_one_wake_and_waits_at_most_a_minute() {
    let mut live = live();
    live.engine
        .set_lock_after_for(
            live.engine.session_turn(),
            LockAfter::Minutes30,
            Some(AppLockProof::PlatformVerified),
        )
        .unwrap();
    idle_for(&mut live, Duration::from_secs(300));
    looked(&mut live);
    let (due, armed, looks) = (
        away(&mut live.rt).due,
        away(&mut live.rt).armed,
        away(&mut live.rt).looks,
    );
    // Twenty-five minutes are left, but a shorter delay chosen meanwhile is followed within a minute.
    let due = due.expect("a look is due");
    assert!(due <= Instant::now() + LOOK_AT_MOST_EVERY);
    // One wake is armed, no later than the look: a wake armed earlier (the first look's) is kept, and its poll arms
    // the next.
    assert!(
        armed.is_some_and(|armed| armed <= due),
        "a wake is armed for the look or before it"
    );
    for _ in 0..20 {
        live.rt.frame(0.016);
    }
    assert_eq!(away(&mut live.rt).armed, armed, "polls arm no second wake");
    assert_eq!(away(&mut live.rt).looks, looks, "and ask nothing before it is due");
}

#[test]
fn leaving_the_app_locks_only_with_immediately() {
    let mut live = live();
    let actions = live.rt.state.shell.actions.clone();
    let before = away(&mut live.rt).answered;
    away(&mut live.rt).event(Away::LeftTheApp, &actions, None);
    answers(&mut live, before + 1);
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "five minutes: leaving is not away yet"
    );
    live.engine
        .set_lock_after_for(live.engine.session_turn(), LockAfter::Immediately, None)
        .unwrap();
    away(&mut live.rt).event(Away::LeftTheApp, &actions, None);
    until(&mut live, "leaving did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
}

/// A wake counts from the frame's clock, which is read before the poll measures: it can land a little before the time
/// it was armed for. That poll arms again, so no look is left waiting for a poll that never comes (in the tray).
#[test]
fn a_wake_that_lands_just_before_its_time_arms_again() {
    let mut live = live();
    let soon = Instant::now() + Duration::from_millis(200);
    let lock = away(&mut live.rt);
    lock.due = Some(soon);
    lock.armed = Some(soon);
    let (arms, looks) = (lock.arms, lock.looks);
    live.rt.frame(0.016);
    let lock = away(&mut live.rt);
    assert_eq!(lock.looks, looks, "not due yet: no look");
    assert!(lock.arms > arms, "the early wake's poll arms the look again");
}

/// Review of #452 (P1-1, the lead's ruling): no first-run exception. The idle count runs while the ENGINE is open,
/// whatever the shell shows: during the first-run steps the key is in memory, and idle past the delay locks.
#[test]
fn idle_past_the_delay_locks_during_the_first_run_too() {
    let mut live = live_with(true);
    until(&mut live, "the first-run steps did not show", |live| {
        live.rt.state.shell.app.gate() == GateState::FirstRun
    });
    idle_for(&mut live, Duration::from_secs(300));
    until(&mut live, "the first run did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
}

/// Review of #452 (P2-1): a sleep is time away. The PC's time without input before it slept and the sleep itself,
/// past the delay, lock at the wake (before the wake's own key press could count as input); short of it, nothing.
#[test]
fn a_wake_after_a_sleep_past_the_delay_locks_and_a_short_one_does_not() {
    let mut live = live();
    let before = away(&mut live.rt).answered;
    live.rt.update(|s, tx| {
        host::on_tray(
            s,
            TrayEvent::Resumed {
                away: Duration::from_secs(299),
            },
            tx,
        )
    });
    answers(&mut live, before + 1);
    assert_eq!(live.engine.lock_state(), LockState::Open, "away short of five minutes");
    live.rt.update(|s, tx| {
        host::on_tray(
            s,
            TrayEvent::Resumed {
                away: Duration::from_secs(300),
            },
            tx,
        )
    });
    until(&mut live, "the wake did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
}
