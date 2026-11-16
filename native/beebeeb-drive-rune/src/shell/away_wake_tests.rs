//! Review of #463 (P3-1, P3-2, P3-15, the lock path's fade), on the away lock's live window (`super`): a wake Windows
//! announced covers too, a look of the wake that failed keeps the cover until the engine decided, a click on the b
//! before the first poll after a wake opens a covered flyout, and the lock screen's first frame holds nothing of the
//! content. Every name and password is invented.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::frames::global::sync_off::SyncOff;
use crate::shell::actions::{ActionError, AwayJob};
use crate::shell::app::Session;
use crate::shell::flyout_window;
use crate::shell::surfaces::WindowKind;
use crate::shell::tray::{LeftClick, TrayInputs};

/// The live window's actions with the away look changed: each look's job is the engine's own, held until the test
/// releases `hold` (at most 10 s), or one that panics (`fails`). Everything else is the live window's.
struct Scripted {
    inner: Arc<dyn ShellActions>,
    hold: Option<Arc<AtomicBool>>,
    fails: bool,
}

impl Scripted {
    fn over(live: &Live, hold: Option<Arc<AtomicBool>>, fails: bool) -> Arc<dyn ShellActions> {
        Arc::new(Scripted {
            inner: live.rt.state.shell.actions.clone(),
            hold,
            fails,
        })
    }
}

impl ShellActions for Scripted {
    fn session(&self) -> Session {
        self.inner.session()
    }
    fn start_sync(&self) -> Result<(), ActionError> {
        self.inner.start_sync()
    }
    fn stop_for_quit(&self) {
        self.inner.stop_for_quit()
    }
    fn resume_after_cancelled_quit(&self) {
        self.inner.resume_after_cancelled_quit()
    }
    fn tray_inputs(&self) -> TrayInputs {
        self.inner.tray_inputs()
    }
    fn pause_sync(&self) -> Result<(), ActionError> {
        self.inner.pause_sync()
    }
    fn resume_sync(&self) -> Result<(), ActionError> {
        self.inner.resume_sync()
    }
    fn open_in_file_explorer(&self) -> Result<(), ActionError> {
        self.inner.open_in_file_explorer()
    }
    fn autostart_enabled(&self) -> bool {
        self.inner.autostart_enabled()
    }
    fn set_autostart(&self, enabled: bool) -> Result<(), ActionError> {
        self.inner.set_autostart(enabled)
    }
    fn set_gate_for_scene(&self, gate: GateState) {
        self.inner.set_gate_for_scene(gate)
    }
    fn sync_refusal(&self) -> Option<SyncOff> {
        self.inner.sync_refusal()
    }
    fn away_job(&self, seen: Away) -> Option<AwayJob> {
        let job = self.inner.away_job(seen)?;
        let (hold, fails) = (self.hold.clone(), self.fails);
        Some(Box::new(move || {
            if let Some(hold) = hold {
                let began = Instant::now();
                while !hold.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(10) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            assert!(!fails, "a look that fails (the test's)");
            job()
        }))
    }
    fn screen_open(&self) -> bool {
        self.inner.screen_open()
    }
    fn lock_vault_now(&self) {
        self.inner.lock_vault_now()
    }
}

/// The tray's resume of a wake, said as the window's `on_tray` says it.
fn resumed(live: &mut Live, away: Duration) {
    live.rt
        .update(|s, tx| host::on_tray(s, TrayEvent::Resumed { away }, tx));
}

/// Review of #463, P3-1 (D-B2-3): a wake Windows announced, its resume said before the window's first poll after it,
/// covers the content at once, as an unannounced one does. That poll holds no gap (the resume said this wake), and the
/// resume's answer (three minutes of thirty: nothing locks) lifts the cover; the files come back.
/// Mutant: in `AwayLock::event`, cover only a wake already covered (back to `matches!(self.cover, Cover::Held |
/// Cover::Asked(_))`) -> red at "the resume covers at once".
#[test]
fn a_wake_windows_announced_before_the_first_poll_is_covered_at_once() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(3 * 60));
    let (before, looks) = (away(&mut live.rt).answered, away(&mut live.rt).looks);
    resumed(&mut live, Duration::from_secs(3 * 60));
    assert!(away(&mut live.rt).covers(), "the resume covers at once");
    assert_eq!(away(&mut live.rt).looks, looks + 1, "and is said");
    live.rt.frame(0.016);
    let lock = away(&mut live.rt);
    assert!(
        lock.away.is_none() && lock.gap_found_at.is_none(),
        "the poll holds no gap for the wake the resume said"
    );
    let began = Instant::now();
    loop {
        let drawn = words(&live);
        if !away(&mut live.rt).covers() {
            break;
        }
        assert!(
            !drawn.contains(FILE_NAME),
            "a covered frame {:?} after the resume holds the file name:\n{drawn}",
            began.elapsed()
        );
        assert!(began.elapsed() < Duration::from_secs(10), "the cover was never lifted");
        live.rt.frame(0.016);
        std::thread::sleep(Duration::from_millis(5));
    }
    let lock = away(&mut live.rt);
    assert_eq!(lock.answered, before + 1, "lifted by the resume's answer");
    assert_eq!(lock.looks, looks + 1, "the resume's look only");
    assert_eq!(live.engine.lock_state(), LockState::Open, "three minutes of thirty");
    until_drawn(&mut live, FILE_NAME);
}

/// P3-1: a cover an event turned on between two polls is new to the next frame too: that frame's commit takes the files
/// away at once, with no element of them left fading out under the cover. (The resume's look is held until the test
/// releases it, so the cover is still on in that frame.)
/// Mutant: in `AwayLock::poll`, compare with the cover at this poll's start (`let covered = self.covers();` before
/// `measure`) -> red at "the first frame after the resume holds no file name" (the Files window, leaving).
#[test]
fn a_cover_turned_on_between_two_polls_takes_the_files_away_in_one_frame() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    live.rt.settle_all();
    live.rt.frame(0.016);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    let hold = Arc::new(AtomicBool::new(false));
    let held = Scripted::over(&live, Some(hold.clone()), false);
    live.rt
        .update(|s, _| s.shell.away.event(Away::Idle(Duration::from_secs(3 * 60)), &held, None));
    assert!(away(&mut live.rt).covers(), "the resume covers at once");
    live.rt.frame(0.016);
    assert!(away(&mut live.rt).covers(), "its held look has not answered");
    let drawn = words(&live);
    assert!(
        !drawn.contains(FILE_NAME),
        "the first frame after the resume holds no file name:\n{drawn}"
    );
    assert!(drawn.contains("Beebeeb is locked"), "{drawn}");
    hold.store(true, Ordering::SeqCst);
    until(&mut live, "the held look's answer did not lift the cover", |live| {
        !live.rt.state.shell.away.covers()
    });
    until_drawn(&mut live, FILE_NAME);
}

/// Review of #463, P3-2 (fail closed, as row 7.2): the look that says a wake panics, so the engine decided nothing. The
/// cover holds through that answer, and after the grace the wake is said again as the longest time away: the engine
/// locks (thirty minutes, app lock on), although the resume itself said three minutes. No frame from the wake to the
/// lock holds the file's name; after the unlock the files come back.
/// Mutant (the reviewer's): take a panicked answer as one that locked nothing (delete the `looked.answer.is_none()`
/// branch in `measure`) -> red at "a covered frame holds the file name" / "the cover holds through the failed look".
#[test]
fn a_wake_whose_look_failed_stays_covered_and_the_engine_locks_it() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    let before = away(&mut live.rt).answered;
    let failing = Scripted::over(&live, None, true);
    live.rt.update(|s, _| {
        s.shell
            .away
            .event(Away::Idle(Duration::from_secs(3 * 60)), &failing, None)
    });
    assert!(away(&mut live.rt).covers(), "the resume covers at once");
    let began = Instant::now();
    while live.rt.state.shell.app.gate() != GateState::Locked {
        live.rt.frame(0.016);
        let drawn = words(&live);
        assert!(
            !drawn.contains(FILE_NAME),
            "a covered frame {:?} after the wake holds the file name:\n{drawn}",
            began.elapsed()
        );
        if away(&mut live.rt).answered > before && live.engine.lock_state() == LockState::Open {
            assert!(away(&mut live.rt).covers(), "the cover holds through the failed look");
        }
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "the failed wake never locked"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        live.engine.lock_state(),
        LockState::ScreenLocked,
        "the engine locked it"
    );
    live.engine
        .unlock_screen_platform_verified(live.engine.session_turn())
        .unwrap();
    until(&mut live, "the cover outlived the unlock", |live| {
        live.rt.state.shell.app.gate() == GateState::Ready && !live.rt.state.shell.away.covers()
    });
    until_drawn(&mut live, FILE_NAME);
}

/// P3-2: the wake said again after a failed look is the engine's to decide. App lock turned off since the engine's last
/// idle answer: the retry locks nothing, and that answer lifts the cover (never a cover left over a decision made).
#[test]
fn a_failed_wake_said_again_to_an_engine_that_locks_nothing_lifts_the_cover() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    idle_for(&mut live, Duration::from_secs(60));
    looked(&mut live);
    live.engine
        .set_app_lock_for(live.engine.session_turn(), false, Some(AppLockProof::PlatformVerified))
        .unwrap();
    let (before, looks) = (away(&mut live.rt).answered, away(&mut live.rt).looks);
    let failing = Scripted::over(&live, None, true);
    live.rt.update(|s, _| {
        s.shell
            .away
            .event(Away::Idle(Duration::from_secs(3 * 60)), &failing, None)
    });
    assert!(
        away(&mut live.rt).covers(),
        "the resume covers (the last idle answer could lock)"
    );
    answers(&mut live, before + 1);
    assert!(away(&mut live.rt).covers(), "the failed look lifts nothing");
    assert_eq!(
        away(&mut live.rt).away,
        Some(Duration::MAX),
        "the wake is held as the longest time away"
    );
    until(&mut live, "the retry's answer did not lift the cover", |live| {
        !live.rt.state.shell.away.covers()
    });
    assert_eq!(away(&mut live.rt).looks, looks + 2, "the failed look and the retry");
    assert_eq!(live.engine.lock_state(), LockState::Open, "app lock off locks nothing");
    until_drawn(&mut live, FILE_NAME);
}

/// Review of #463, P3-15: a click on the b after a wake Windows did not announce, before the main window's first poll:
/// the click covers the wake itself and tells the flyout before it reads and shows it, so the flyout opens on the
/// lock's state (`frames::tray::Ui::cover`), never on file names or activity. The poll that follows finds the gap and
/// says it, as without the click, and its answer (five minutes and twenty, of thirty) lifts both.
/// Mutants: in `Shell::toggle_flyout`, drop `self.away.cover_a_gap_not_yet_found()` (or the `tell_flyout_covered`
/// call before `read_flyout`) -> red at "the flyout is told before it shows"; in `cover_a_gap_not_yet_found`, drop
/// `self.wake_covers()` (M5/M6 at their new place: app lock off, an engine without the lock) -> red in
/// `a_click_on_the_b_after_a_wake_with_app_lock_off_covers_nothing`.
#[test]
fn a_click_on_the_b_before_the_first_poll_after_a_wake_opens_a_covered_flyout() {
    let mut live = live_flyout();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(5 * 60));
    looked(&mut live);
    assert_eq!(
        live.rt.state.shell.tray_model().left_click,
        LeftClick::ToggleFlyout,
        "signed in: a click on the b opens the flyout"
    );
    let (before, looks) = (away(&mut live.rt).answered, away(&mut live.rt).looks);
    slept(&mut live, Duration::from_secs(20 * 60));
    live.rt
        .update(|s, tx| host::on_tray(s, TrayEvent::Activate { anchor: None }, tx));
    assert!(away(&mut live.rt).covers(), "the click covers the wake");
    assert_eq!(
        live.rt.state.shell.flyout_covered,
        Some(live.rt.state.shell.sync_runs()),
        "the flyout is told before it shows"
    );
    assert_eq!(away(&mut live.rt).looks, looks, "the click says nothing to the engine");
    live.rt.frame(0.016);
    let drawn = words(&live);
    assert!(
        !drawn.contains(FILE_NAME),
        "the window's first frame after the wake:\n{drawn}"
    );
    let lock = away(&mut live.rt);
    assert!(
        lock.away.is_some_and(|held| held >= Duration::from_secs(20 * 60)),
        "the poll after the click found the gap and holds it"
    );
    until(&mut live, "the cover was never lifted", |live| {
        !live.rt.state.shell.away.covers()
    });
    let lock = away(&mut live.rt);
    assert_eq!(lock.answered, before + 1, "lifted by the gap's own look");
    assert_eq!(lock.looks, looks + 1);
    assert_eq!(
        live.rt.state.shell.flyout_covered, None,
        "the flyout is told the cover lifted"
    );
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "twenty-five minutes of thirty"
    );
}

/// P3-15 with app lock off: a click on the b after a gap covers nothing (app lock off never shows the lock screen).
#[test]
fn a_click_on_the_b_after_a_wake_with_app_lock_off_covers_nothing() {
    let mut live = live_flyout();
    until_drawn(&mut live, FILE_NAME);
    live.engine
        .set_app_lock_for(live.engine.session_turn(), false, Some(AppLockProof::PlatformVerified))
        .unwrap();
    idle_for(&mut live, Duration::from_secs(29 * 60));
    looked(&mut live);
    slept(&mut live, Duration::from_secs(3 * 3_600));
    live.rt
        .update(|s, tx| host::on_tray(s, TrayEvent::Activate { anchor: None }, tx));
    assert!(!away(&mut live.rt).covers(), "app lock off: nothing is covered");
    assert_eq!(
        live.rt.state.shell.flyout_covered, None,
        "the flyout shows what it shows"
    );
}

/// [`live`], with a flyout wired to the shell (a rune handle that is never run: its queued updates are not applied).
fn live_flyout() -> Live {
    let flyout = rune::App::new(rune::AppSpec::new(
        "flyout",
        WindowKind::Flyout.default_size(),
        flyout_window::fixture_state,
        flyout_window::view,
    ))
    .handle();
    let mut live = live_on(
        false,
        Wiring {
            flyout: Some(flyout),
            ..Wiring::default()
        },
    );
    answers(&mut live, 1);
    live
}

/// The lock path's fade (review of #463, the lead's 'lock fade' row; privacy): when the engine locks, the frame that
/// first shows the lock screen holds nothing of the Files window, not even an element leaving. `between_polls`: the
/// window reads the lock in a refresh between two polls (the engine lane's status events, the b's menu), not in the
/// poll's own read of the gate.
fn the_first_locked_frame_holds_no_file_name(between_polls: bool) {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    live.rt.settle_all();
    live.rt.frame(0.016);
    live.engine.lock_screen().expect("the screen locks");
    if between_polls {
        live.rt.update(|s, _| {
            s.shell.refresh();
        });
        assert_eq!(
            live.rt.state.shell.app.gate(),
            GateState::Locked,
            "read between two polls"
        );
    }
    // The frame that first draws the window with its gate Locked.
    let began = Instant::now();
    loop {
        live.rt.frame(0.016);
        if live.rt.state.shell.app.gate() == GateState::Locked {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "the window never read the lock"
        );
    }
    let drawn = words(&live);
    assert!(
        !drawn.contains(FILE_NAME),
        "the first locked frame holds the file name:\n{drawn}"
    );
    assert!(drawn.contains("Beebeeb is locked"), "{drawn}");
}

/// The lock the poll's own read of the gate finds.
/// Mutant: drop the `tx.instant()` where the poll sees the session leave an open gate (`host::poll_screen`, the
/// `session_seen` block) -> red (the file's row is still listed, leaving, in the first locked frame).
#[test]
fn the_lock_the_poll_reads_takes_the_files_away_in_the_frame_that_shows_it() {
    the_first_locked_frame_holds_no_file_name(false);
}

/// The lock a refresh read between two polls.
/// Mutants: drop that `tx.instant()` -> red; key it on this poll's own refresh (`read_by_the_gate`) instead of the
/// last poll's session -> red here only.
#[test]
fn a_lock_read_between_two_polls_takes_the_files_away_in_the_frame_that_shows_it() {
    the_first_locked_frame_holds_no_file_name(true);
}

/// The engine's look that fails EVERY time (an engine that panics on each look): the wake cover fails CLOSED (Guus's
/// ruling). The first failure says the wake again; the retry's failure holds the cover too, and nothing lifts it
/// without a passing check.
/// Mutant: back to "a second failure sets `Cover::Off`" -> red at "an always-failing look never lifts the cover".
fn always_failing(live: &mut Live) -> Arc<dyn ShellActions> {
    idle_for(live, Duration::from_secs(60));
    looked(live);
    let failing = Scripted::over(live, None, true);
    live.rt.update(|s, _| {
        s.shell.actions = failing.clone();
        s.shell
            .away
            .event(Away::Idle(Duration::from_secs(3 * 60)), &failing, None)
    });
    assert!(away(&mut live.rt).covers(), "the resume covers at once");
    let before = away(&mut live.rt).answered;
    // The failed look and the failed retry (after the grace).
    answers(live, before + 2);
    failing
}

#[test]
fn an_always_failing_look_never_lifts_the_cover() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    always_failing(&mut live);
    let began = Instant::now();
    while began.elapsed() < Duration::from_millis(600) {
        live.rt.frame(0.016);
        assert!(away(&mut live.rt).covers(), "the cover lifted with no passing check");
        assert!(
            !words(&live).contains(FILE_NAME),
            "the files showed under a cover that no look cleared"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(live.engine.lock_state(), LockState::Open, "nothing locked it either");
}

/// The cover that stays up offers the normal sign-in path, never an Unlock that does nothing (the engine is open: the
/// lock screen's Unlock asks nothing). Its press closes the engine's lock through the engine, so the window shows the
/// engine's own password screen, which is the path that checks.
/// Mutant: leave the cover's Unlock in place -> red at "no Unlock on the cover".
#[test]
fn the_sign_in_path_is_offered() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    always_failing(&mut live);
    until_drawn(&mut live, "Sign in");
    let drawn = words(&live);
    assert!(!drawn.contains("Unlock"), "no Unlock on the cover:\n{drawn}");
    assert!(!drawn.contains(FILE_NAME), "no file on the cover");
    let r = live
        .rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(crate::frames::signin::before_files::COVER_SIGN_IN_KEY))
        .expect("a sign-in button on the cover")
        .rect;
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    live.rt.pointer_down(x, y);
    live.rt.pointer_up(x, y);
    until(&mut live, "the press closed nothing", |live| {
        live.engine.lock_state() != LockState::Open
    });
    until(&mut live, "the cover held the engine's own screen back", |live| {
        !registry::covered(&live.rt.state)
    });
    assert!(!words(&live).contains(FILE_NAME), "the files stay away");
}

/// `Shell::covered` is `registry::covered`'s predicate, not a copy.
#[test]
fn the_shells_cover_is_the_registrys() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    assert_eq!(live.rt.state.shell.covered(), registry::covered(&live.rt.state));
    live.rt.update(|s, _| s.shell.away.cover_a_gap_not_yet_found());
    assert_eq!(live.rt.state.shell.covered(), registry::covered(&live.rt.state));
}
