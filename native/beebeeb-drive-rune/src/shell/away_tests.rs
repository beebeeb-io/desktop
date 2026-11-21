//! The away lock through the main window's poll, on a real engine handle against the engine's fake API: the shell
//! measures (a scripted idle from the fake platform), the engine compares and locks through core. Every name and
//! password is invented; nothing here waits on Windows.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::lock_api::LockState;
use beebeeb_engine::app::lock_password_api::{AppLockProof, LockAfter};
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::config::CONFIG_DIR_OVERRIDE;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::LiveEngine;
use crate::engine_host::open::Opened;
use crate::shell::actions::HostActions;
use crate::shell::app::{Event, GateState};
use crate::shell::host::{self, Shell, Wiring};
use crate::shell::platform::TrayEvent;
use crate::shell::platform::fake::FakePlatform;
use crate::shell::{registry, state};

const EMAIL: &str = "away-lock@example.eu";
const PASSWORD: &str = "an invented password for the away lock";
/// A file in the account's root (on the server and in this PC's local state, recorded as the account's), so a frame
/// that shows the Files window holds its name.
const FILE_NAME: &str = "Harbour plan.txt";
const FILE_ID: &str = "away-lock-file";

type TestEngine = Arc<Engine<MemoryCredentials>>;

struct Live {
    engine: TestEngine,
    platform: Arc<FakePlatform>,
    rt: Runtime<State>,
    /// dc-165: how often an engine thread asked the window to wake (the real app's hook is `shell::engine_woke`).
    wakes: Arc<std::sync::atomic::AtomicUsize>,
    _api: FakeApi,
    _local_state: tempfile::TempDir,
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
    live_on(first_run, Wiring::default())
}

/// [`live_with`], with the shell wired as `wiring` says (a flyout).
fn live_on(first_run: bool, wiring: Wiring) -> Live {
    let (account, phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    account.add_item(FILE_ID, None, FILE_NAME, false, 2_048);
    let owner = Identity::new(Some(&account.user_id), Some(&account.email));
    let api = FakeApi::start(account_handler(Arc::new(account)));
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = wakes.clone();
    let (engine_host, messages) = ChannelHost::new(move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let config = EngineConfig {
        account_id: "away-lock-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, engine_host, MemoryCredentials::default()).expect("opens"));
    engine.restore();
    // This PC's local state, as the account's sync left it: the Files window lists the file from it.
    let local_state = tempfile::tempdir().expect("a folder for the local state");
    let path = local_state.path().join("state.db");
    let db = StateDb::open(&path).expect("the local state opens");
    let entry = FileEntry {
        file_id: FILE_ID.to_string(),
        path: format!("/{FILE_NAME}"),
        status: FileStatus::CloudOnly,
        size_bytes: 2_048,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .expect("the file is recorded");
    db.set_owner(&owner).expect("the local state is the account's");
    drop(db);
    engine.use_local_state_for_test(path);
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
        wiring,
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
    rt.on_key = Some(host::on_key);
    rt.frame(0.0);
    Live {
        engine,
        platform,
        rt,
        wakes,
        _api: api,
        _local_state: local_state,
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

/// Every text and label the window holds, leaving elements too (a row on its way out still shows its name).
fn words(live: &Live) -> String {
    live.rt
        .elements()
        .into_iter()
        .flat_map(|e| [e.text, e.label])
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Frames until the window holds `what` (at most 10 s).
fn until_drawn(live: &mut Live, what: &str) {
    let began = Instant::now();
    loop {
        live.rt.frame(0.016);
        live.rt.settle_all();
        live.rt.frame(0.016);
        if words(live).contains(what) {
            return;
        }
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "{what:?} was never drawn; the window holds:\n{}",
            words(live)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The main window's poll of the away lock alone, as `host::poll` makes it: whether it said the window changed.
fn poll_said(live: &mut Live) -> bool {
    let idle = *live.platform.idle.lock().unwrap();
    let mut said = false;
    live.rt.update_if(|s, tx| {
        let shell = &mut s.shell;
        said = shell
            .away
            .poll(shell.actions.screen_open(), idle, &shell.actions, None, tx);
        said
    });
    said
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

/// The lock the tray says (Windows' session lock, Win+L; the b's menu has no lock item) while the main window is
/// hidden (no window shown: its slot holds no handle): the poll that sees the lock stops playback as the idle lock's
/// does (`media::stop_all`, as a lock), and the OS's media controls are pumped after it in that same poll, so the
/// clear reaches Windows with the window hidden.
#[test]
fn the_trays_os_lock_stops_playback_and_pumps_the_clear_with_the_window_hidden() {
    let mut live = live();
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (stops, pumps) = (seen.clone(), seen.clone());
    live.rt.update(move |s, _| {
        s.shell.set_media_stop_for_test(Arc::new(move |reason| {
            stops.lock().unwrap().push(format!("stop {reason:?}"));
        }));
        s.shell
            .set_media_pump_for_test(Arc::new(move || pumps.lock().unwrap().push("pump".into())));
    });
    assert!(
        live.rt.state.shell.main_window_slot().lock().unwrap().is_none(),
        "no window is shown"
    );
    live.rt.frame(0.016);
    assert_eq!(
        seen.lock().unwrap().iter().filter(|e| e.starts_with("stop")).count(),
        0,
        "open: nothing stops"
    );
    live.rt.update(|s, tx| host::on_tray(s, TrayEvent::SessionLocked, tx));
    until(&mut live, "the tray's lock did not stop playback", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
            && seen.lock().unwrap().iter().any(|e| e.starts_with("stop"))
    });
    let seen = seen.lock().unwrap().clone();
    let stop = seen
        .iter()
        .position(|e| e == "stop Locked")
        .unwrap_or_else(|| panic!("stopped as a lock: {seen:?}"));
    assert!(
        seen[stop + 1..].iter().any(|e| e == "pump"),
        "the clear is pumped after the stop: {seen:?}"
    );
    assert_eq!(
        seen.iter().filter(|e| e.starts_with("stop")).count(),
        1,
        "stopped once: {seen:?}"
    );
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
    // The engine is open: the count runs during the first run, and its first look answers (before the idle below,
    // so that answer does not put the next look a minute off).
    answers(&mut live, 1);
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

/// A thirty minute delay (a longer one: Windows Hello's yes, as the page asks).
fn thirty_minutes(live: &Live) {
    live.engine
        .set_lock_after_for(
            live.engine.session_turn(),
            LockAfter::Minutes30,
            Some(AppLockProof::PlatformVerified),
        )
        .unwrap();
}

/// The poll did not run for `gap` and Windows said nothing of it (review of #452, P2-A: a Modern Standby PC sends the
/// suspend only to a window that registered): the poll's last run moves back on the wall clock (which counts a
/// sleep), the wake's key press leaves the PC's idle near zero, no resume is said, and the next look stays a minute
/// off (as the monotonic clock of a PC that slept may say).
fn slept(live: &mut Live, gap: Duration) {
    *live.platform.idle.lock().unwrap() = Some(Duration::from_secs(1));
    let lock = away(&mut live.rt);
    let seen = lock.seen.as_mut().expect("the poll ran while the engine was open");
    seen.wall -= gap;
    lock.due = Some(Instant::now() + LOOK_AT_MOST_EVERY);
}

/// Fifteen minutes without input, then three hours in which the app did not run: time away past thirty minutes locks
/// at the wake, with no broadcast and a key press already counted as input.
#[test]
fn a_gap_of_hours_after_an_idle_locks_without_a_resume() {
    let mut live = live();
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(15 * 60));
    looked(&mut live);
    assert_eq!(live.engine.lock_state(), LockState::Open, "fifteen minutes of thirty");
    slept(&mut live, Duration::from_secs(3 * 3_600));
    until(&mut live, "the gap did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
}

/// Fifteen minutes without input, then twenty in which the app did not run, with no resume said: the idle seen last and
/// the gap are time away together (thirty-five minutes), so a thirty minute delay locks, as it does after a sleep
/// Windows announced (decision 2539: the gap alone would wait for a sleep past the delay by itself).
#[test]
fn an_idle_then_a_gap_past_the_delay_lock_together_without_a_resume() {
    let mut live = live();
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(15 * 60));
    looked(&mut live);
    assert_eq!(live.engine.lock_state(), LockState::Open, "fifteen minutes of thirty");
    slept(&mut live, Duration::from_secs(20 * 60));
    live.rt.frame(0.016);
    assert!(
        away(&mut live.rt)
            .away
            .is_some_and(|held| held >= Duration::from_secs(35 * 60)),
        "the idle seen last and the gap are held together"
    );
    until(&mut live, "the idle and the gap did not lock together", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
}

/// Frames until [`RESUME_GRACE`] passed since `from`, then until `count` answers were taken in with no idle look out.
/// `from` is read after the poll that could find a gap, so the frames from then on are past its grace: a gap still held
/// is asked in the first of them, and its answer is in when this returns.
fn answers_past_the_grace(live: &mut Live, count: usize, from: Instant) {
    until(live, "the grace did not pass", |_| from.elapsed() >= RESUME_GRACE);
    answers(live, count);
}

/// Five minutes without input, then twenty in which the app did not run: twenty-five minutes of thirty lock nothing.
/// The gap waits for a resume first (none comes), then it is said once, and the engine says when the rest could be
/// reached.
#[test]
fn an_idle_and_a_gap_short_of_the_delay_do_not_lock_and_are_said_once() {
    let mut live = live();
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(5 * 60));
    looked(&mut live);
    let (before, looks) = (away(&mut live.rt).answered, away(&mut live.rt).looks);
    slept(&mut live, Duration::from_secs(20 * 60));
    live.rt.frame(0.016);
    let found = Instant::now();
    let lock = away(&mut live.rt);
    assert!(
        lock.away.is_some_and(|held| held >= Duration::from_secs(25 * 60)),
        "the gap is found and held"
    );
    assert_eq!(lock.looks, looks, "and waits for a resume before it is said");
    answers_past_the_grace(&mut live, before + 1, found);
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "five minutes and twenty, of thirty"
    );
    let lock = away(&mut live.rt);
    assert!(lock.away.is_none() && lock.gap_found_at.is_none(), "said once");
    assert_eq!(lock.looks, looks + 1, "in one look");
    let due = lock.due.expect("a look is due");
    assert!(due <= Instant::now() + LOOK_AT_MOST_EVERY);
    for _ in 0..20 {
        live.rt.frame(0.016);
    }
    assert_eq!(away(&mut live.rt).looks, looks + 1, "and not again before it is due");
}

/// Twenty-nine minutes without input, then three in which the app did not run. The poll finds the gap first and holds
/// it (thirty-two minutes, which would lock); the tray's resume for the same wake comes within the grace and says
/// three minutes, measured better. The gap is dropped: the resume is what counts, and it locks nothing.
#[test]
fn a_resume_for_the_same_wake_drops_the_gap() {
    let mut live = live();
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(3 * 60));
    live.rt.frame(0.016);
    let lock = away(&mut live.rt);
    assert!(
        lock.away.is_some_and(|held| held >= Duration::from_secs(32 * 60)),
        "the gap is found and held"
    );
    let (before, looks) = (lock.answered, lock.looks);
    live.rt.update(|s, tx| {
        host::on_tray(
            s,
            TrayEvent::Resumed {
                away: Duration::from_secs(3 * 60),
            },
            tx,
        )
    });
    let resumed = Instant::now();
    let lock = away(&mut live.rt);
    assert!(
        lock.away.is_none() && lock.gap_found_at.is_none(),
        "the resume drops the gap"
    );
    assert_eq!(lock.looks, looks + 1, "and is said itself");
    answers_past_the_grace(&mut live, before + 1, resumed);
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "three minutes of thirty: the resume counts, not the gap"
    );
    assert!(away(&mut live.rt).away.is_none());
}

/// The same wake, with the tray's resume said before the poll runs again: that poll finds the gap and holds none (the
/// resume already said this wake). Nothing locks.
#[test]
fn a_resume_said_first_means_the_poll_holds_no_gap() {
    let mut live = live();
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    // No frame between the gap and the resume: one would find the gap before the resume is said.
    slept(&mut live, Duration::from_secs(3 * 60));
    let before = away(&mut live.rt).answered;
    live.rt.update(|s, tx| {
        host::on_tray(
            s,
            TrayEvent::Resumed {
                away: Duration::from_secs(3 * 60),
            },
            tx,
        )
    });
    live.rt.frame(0.016);
    let polled = Instant::now();
    let lock = away(&mut live.rt);
    // The poll ran and took the gap in: its last run is now, not three minutes back.
    assert!(
        lock.seen
            .is_some_and(|seen| seen.gap(Instant::now(), SystemTime::now()) < AWAY_GAP),
        "the poll after the resume ran"
    );
    assert!(
        lock.away.is_none() && lock.gap_found_at.is_none(),
        "the poll holds no gap for a wake the resume said"
    );
    assert!(!lock.resumed, "the resume is spent on that poll");
    answers_past_the_grace(&mut live, before + 1, polled);
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "three minutes of thirty: the resume counts, not the gap"
    );
    assert!(away(&mut live.rt).away.is_none());
}

/// Two polls a minute and a half apart are the look interval, not time away: nothing is asked before it is due and
/// nothing locks, also one minute short of the delay.
#[test]
fn polls_a_minute_and_a_half_apart_are_not_time_away() {
    let mut live = live();
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(90));
    let looks = away(&mut live.rt).looks;
    for _ in 0..20 {
        live.rt.frame(0.016);
    }
    assert_eq!(away(&mut live.rt).looks, looks, "no look before it is due");
    assert_eq!(live.engine.lock_state(), LockState::Open);
}

/// D-B2-3 (privacy): after a wake Windows did not announce, the window's content is covered at once, drawn as the lock
/// screen; the grace only decides which look says the wake. Twenty-nine minutes and three, of thirty: no frame from the
/// poll that found the gap to the lock holds the file's name, and after the unlock the files come back.
#[test]
fn a_frame_during_the_grace_holds_no_file_name_and_the_lock_follows() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(3 * 60));
    assert!(poll_said(&mut live), "the poll that covers says the window changed");
    assert!(
        away(&mut live.rt).covers(),
        "the poll that found the gap covers at once"
    );
    live.rt.frame(0.016);
    let drawn = words(&live);
    assert!(
        !drawn.contains(FILE_NAME),
        "the first frame after the wake holds no file name:\n{drawn}"
    );
    assert!(
        drawn.contains("Beebeeb is locked"),
        "it is drawn as the lock screen:\n{drawn}"
    );
    // Every frame through the grace and the gap's look, until the window shows the lock itself.
    let began = Instant::now();
    while live.rt.state.shell.app.gate() != GateState::Locked {
        live.rt.frame(0.016);
        let drawn = words(&live);
        assert!(
            !drawn.contains(FILE_NAME),
            "a frame {:?} after the wake holds the file name:\n{drawn}",
            began.elapsed()
        );
        assert!(began.elapsed() < Duration::from_secs(10), "the gap did not lock");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(live.engine.lock_state(), LockState::ScreenLocked);
    for _ in 0..5 {
        live.rt.frame(0.016);
        assert!(!words(&live).contains(FILE_NAME), "the lock screen holds no file name");
    }
    // Opened again (Windows Hello's face check): the cover was the lock's, and goes with it.
    live.engine
        .unlock_screen_platform_verified(live.engine.session_turn())
        .unwrap();
    until(&mut live, "the cover outlived the unlock", |live| {
        live.rt.state.shell.app.gate() == GateState::Ready && !live.rt.state.shell.away.covers()
    });
    until_drawn(&mut live, FILE_NAME);
}

/// D-B2-3: a wake that locks nothing (five minutes and twenty, of thirty) stays covered until the gap's look answered,
/// then the files come back: the cover never outlives the engine's answer. Another look's answer (leaving the app, which
/// a thirty minute delay does not lock) is not the wake's and lifts nothing.
#[test]
fn a_wake_that_locks_nothing_is_covered_until_its_answer_then_the_files_return() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(5 * 60));
    looked(&mut live);
    let (before, looks) = (away(&mut live.rt).answered, away(&mut live.rt).looks);
    slept(&mut live, Duration::from_secs(20 * 60));
    live.rt.frame(0.016);
    let began = Instant::now();
    assert!(away(&mut live.rt).covers(), "covered at once");
    assert!(
        live.rt.state.shell.flyout_covered.is_some(),
        "the flyout is told: it shows the lock's state"
    );
    let actions = live.rt.state.shell.actions.clone();
    away(&mut live.rt).event(Away::LeftTheApp, &actions, None);
    loop {
        let drawn = words(&live);
        assert!(
            !drawn.contains(FILE_NAME),
            "a frame {:?} after the wake holds the file name:\n{drawn}",
            began.elapsed()
        );
        live.rt.frame(0.016);
        if !away(&mut live.rt).covers() {
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the cover was never lifted");
        std::thread::sleep(Duration::from_millis(5));
    }
    let lock = away(&mut live.rt);
    assert_eq!(
        lock.answered,
        before + 2,
        "lifted by the gap's answer, not by leaving's (which came first)"
    );
    assert_eq!(
        lock.looks,
        looks + 2,
        "leaving's look and the gap's own, after the grace"
    );
    assert_eq!(
        live.rt.state.shell.flyout_covered, None,
        "the flyout is told the cover lifted"
    );
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "twenty-five minutes of thirty"
    );
    until_drawn(&mut live, FILE_NAME);
}

/// D-B2-3: the tray's resume for the same wake, within the grace, says the wake: its answer lifts the cover, and the gap's
/// look is never made.
#[test]
fn a_resume_within_the_grace_lifts_the_cover_at_its_answer() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(3 * 60));
    live.rt.frame(0.016);
    assert!(away(&mut live.rt).covers(), "covered at once");
    assert!(!words(&live).contains(FILE_NAME));
    let (before, looks) = (away(&mut live.rt).answered, away(&mut live.rt).looks);
    live.rt.update(|s, tx| {
        host::on_tray(
            s,
            TrayEvent::Resumed {
                away: Duration::from_secs(3 * 60),
            },
            tx,
        )
    });
    assert!(
        away(&mut live.rt).covers(),
        "still covered until the resume's look answered"
    );
    until(&mut live, "the resume's answer did not lift the cover", |live| {
        !live.rt.state.shell.away.covers()
    });
    let lock = away(&mut live.rt);
    assert_eq!(lock.answered, before + 1, "lifted by the resume's answer");
    assert_eq!(lock.looks, looks + 1, "the resume's look only");
    assert_eq!(live.engine.lock_state(), LockState::Open, "three minutes of thirty");
    until_drawn(&mut live, FILE_NAME);
}

/// App lock off never locks (the owner's ruling of 2026-11-14), so it never shows the lock screen either: a gap covers
/// nothing.
#[test]
fn with_app_lock_off_a_wake_covers_nothing() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    live.engine
        .set_app_lock_for(live.engine.session_turn(), false, Some(AppLockProof::PlatformVerified))
        .unwrap();
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(3 * 3_600));
    live.rt.frame(0.016);
    assert!(
        away(&mut live.rt).away.is_some(),
        "the gap is found (and said to an engine that locks nothing)"
    );
    assert!(!away(&mut live.rt).covers(), "app lock off: nothing is covered");
    assert!(words(&live).contains(FILE_NAME), "the files stay on screen");
}

/// An engine with no automatic lock (the fixture, the frame gate's scenes) asks nothing and locks nothing, so a gap
/// covers nothing: no answer would ever lift it.
#[test]
fn an_engine_without_the_automatic_lock_never_covers() {
    let mut rt = Runtime::new(
        state::init(),
        registry::view,
        (1280.0, 800.0),
        Env {
            system_dark: false,
            scale: 1.0,
        },
    );
    let poll = |rt: &mut Runtime<State>| {
        rt.update(|s, tx| {
            let actions = s.shell.actions.clone();
            s.shell
                .away
                .poll(true, Some(Duration::from_secs(1)), &actions, None, tx);
        })
    };
    poll(&mut rt);
    assert!(rt.state.shell.away.without, "the fixture has no away look");
    rt.state.shell.away.seen.as_mut().expect("the poll ran").wall -= Duration::from_secs(3 * 3_600);
    poll(&mut rt);
    assert!(rt.state.shell.away.away.is_some(), "the gap is found");
    assert!(!rt.state.shell.away.covers(), "and covers nothing");
}

/// The cover stays held while the engine is closed (it lifts at a later open poll), but it is drawn only over the gates
/// of an open engine: a closed engine's own screen (sign-in, the recovery phrase, an unreadable key store) is never
/// hidden behind a lock screen whose Unlock cannot answer.
#[test]
fn the_cover_never_hides_the_screen_of_a_closed_engine() {
    let mut rt = Runtime::new(
        state::init(),
        registry::view,
        (1280.0, 800.0),
        Env {
            system_dark: false,
            scale: 1.0,
        },
    );
    rt.frame(0.0);
    rt.state.shell.away.cover = Cover::Locked;
    let lock_screen = |rt: &Runtime<State>| {
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .flat_map(|e| [e.text, e.label])
            .flatten()
            .any(|words| words == "Beebeeb is locked")
    };
    for gate in [
        GateState::KeyStoreUnavailable,
        GateState::NeedsRecoveryPhrase,
        GateState::SignedOut,
    ] {
        rt.update(|s, _| {
            host::dispatch(s, Event::Gate(gate));
        });
        rt.frame(0.016);
        assert_eq!(rt.state.shell.app.gate(), gate);
        assert!(!lock_screen(&rt), "{gate:?}: its own screen, not the cover");
    }
    rt.update(|s, _| {
        host::dispatch(s, Event::Gate(GateState::Ready));
    });
    rt.frame(0.016);
    assert!(
        lock_screen(&rt),
        "over an open engine's Files window the held cover is drawn"
    );
}

/// Ctrl+A, Enter, Delete (the Files window's keys) and Ctrl+, (the shell's), the full key path.
fn keys_behind_a_cover() -> [KeyEvent; 4] {
    let ctrl = Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    };
    [
        KeyEvent {
            key: Key::Command('a'),
            mods: ctrl,
        },
        KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        },
        KeyEvent {
            key: Key::Delete,
            mods: Mods::default(),
        },
        KeyEvent {
            key: Key::Command(','),
            mods: ctrl,
        },
    ]
}

/// The lead's ruling of 2026-11-15 22:56 (privacy): while the window is covered, keys reach only the lock screen. Ctrl+A,
/// Enter, Delete and Ctrl+, change nothing in the Files window behind the cover (no selection, nothing opened, nothing
/// moved to Trash, no Settings); lifted, the Files window has its keys again.
#[test]
fn keys_while_covered_reach_only_the_lock_screen() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(5 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(20 * 60));
    live.rt.frame(0.016);
    assert!(away(&mut live.rt).covers(), "covered at once");
    for ev in keys_behind_a_cover() {
        live.rt.key(ev);
        live.rt.frame(0.016);
    }
    // A key that reaches the shell's handler is taken there, by the lock screen, and goes no further.
    let mut taken = false;
    live.rt.update(|s, tx| {
        taken = host::on_key(
            s,
            &KeyEvent {
                key: Key::Delete,
                mods: Mods::default(),
            },
            tx,
        );
    });
    assert!(taken, "the lock screen takes the key");
    assert!(
        away(&mut live.rt).covers(),
        "the keys came while the window was covered"
    );
    let s = &live.rt.state;
    assert!(
        s.selected.is_none() && s.actions.selection.is_empty(),
        "Ctrl+A selected nothing behind the cover"
    );
    assert!(s.shell.app.route().item.is_none(), "Enter opened nothing");
    assert!(!s.shell.app.in_settings(), "Ctrl+, opened no Settings");
    assert!(!words(&live).contains(FILE_NAME));
    // Five minutes and twenty, of thirty: lifted. The file is still listed (Delete moved nothing), and Ctrl+A reaches
    // the Files window again.
    until(&mut live, "the cover was never lifted", |live| {
        !live.rt.state.shell.away.covers()
    });
    until_drawn(&mut live, FILE_NAME);
    live.rt.settle_all();
    live.rt.key(KeyEvent {
        key: Key::Command('a'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    live.rt.frame(0.016);
    assert_eq!(
        live.rt.state.actions.selection.len(),
        1,
        "lifted, Ctrl+A reaches the Files window"
    );
}

/// M4, the race the cover's `Locked` state is for: the engine locks after this frame's window read the gate, and the away
/// poll sees the session closed in that same frame. A view drawn then, with the window's gate still `Ready`, shows the
/// lock screen, never the files. (No `host::poll` runs between the lock and the view, so nothing reads the gate.)
#[test]
fn a_lock_the_poll_sees_before_the_window_reads_it_keeps_the_cover() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(3 * 60));
    live.rt.frame(0.016);
    assert!(away(&mut live.rt).covers(), "covered at once");
    live.rt.poll = None;
    live.engine.lock_screen().expect("the screen locks");
    poll_said(&mut live);
    assert_eq!(
        live.rt.state.shell.app.gate(),
        GateState::Ready,
        "the window has not read the lock yet"
    );
    assert!(away(&mut live.rt).covers(), "the cover holds through the lock");
    live.rt.invalidate();
    live.rt.frame(0.016);
    let drawn = words(&live);
    assert!(
        !drawn.contains(FILE_NAME),
        "the frame between the lock and the window's read of it holds the file name:\n{drawn}"
    );
    assert!(drawn.contains("Beebeeb is locked"), "{drawn}");
    // The window reads the lock (its own lock screen), and after the unlock the files come back.
    live.rt.poll = Some(host::poll);
    until(&mut live, "the window did not show the lock", |live| {
        live.rt.state.shell.app.gate() == GateState::Locked
    });
    live.engine
        .unlock_screen_platform_verified(live.engine.session_turn())
        .unwrap();
    until(&mut live, "the cover outlived the unlock", |live| {
        live.rt.state.shell.app.gate() == GateState::Ready && !live.rt.state.shell.away.covers()
    });
    until_drawn(&mut live, FILE_NAME);
}

// Review of #463: the wake cover's follow-up (every wake, a failed look, the flyout) and the lock path's fade.
#[path = "away_wake_tests.rs"]
mod wake;

// fx-wake-inputs' follow-up: the gap before the first poll for what comes from outside the window, and the media keys.
#[path = "away_inputs_tests.rs"]
mod inputs;

/// The window's Close button with "Locks again after: Immediately", end to end (w9-lock-reopen): the close hook of the
/// shell with a b in the notification area (so the app stays) locks the real engine, and the window that opens again
/// reads the lock as its gate. VM run 3's H2 never got here: its click on Close landed while Windows Hello's prompt was
/// up and only dismissed the prompt (the window stayed, unlocked).
#[test]
fn closing_to_the_tray_with_immediately_locks_and_the_reopened_window_shows_it() {
    let mut live = live();
    live.engine
        .set_lock_after_for(live.engine.session_turn(), LockAfter::Immediately, None)
        .unwrap();
    live.rt.state.shell.attach_tray(Arc::new(|_| {}));
    assert!(!live.rt.state.shell.close_quits(), "the premise: closing keeps the app in the tray");
    live.rt.update(host::on_close);
    until(&mut live, "closing did not lock", |live| {
        live.engine.lock_state() == LockState::ScreenLocked
    });
    // The window that opens again: its next frames read the gate.
    until(&mut live, "the window did not take the lock as its gate", |live| {
        live.rt.state.shell.app.gate() == GateState::Locked
    });
}

// dc-165: what the locked window and a progress burst ask to be drawn.
#[path = "redraw_tests.rs"]
mod redraw;
