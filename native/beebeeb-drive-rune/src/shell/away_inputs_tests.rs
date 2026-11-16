//! The follow-up of fx-wake-inputs (review of #464, P3-1), on the away lock's live window (`super`): what comes from
//! outside the window between a wake and the main window's first poll after it. An ask to show a place waits for the
//! cover (`host::under_the_wake_cover` covers the gap first), and the away lock tells the media keys of the cover
//! ([`WakeCover`]): at once at a resume, before any poll after a wake Windows did not announce, and when it is gone.
//! Every name is invented.

use super::*;
use crate::shell::app::{Intent, Outcome, Route, Screen, Section};

/// The window shows `section`.
fn shows(live: &Live, section: Section) -> bool {
    matches!(live.rt.state.shell.app.screen(), Screen::Drive(route) if route.section == section)
}

/// An ask to show a place (`host::request`: a `beebeeb://` link, the b's Settings…, the flyout) that comes after a wake
/// Windows did not announce, before the main window's first poll, waits: the gap that poll will find is covered first.
/// The poll finds the gap and says it; its answer (five minutes and twenty, of thirty) lifts the cover, and the ask
/// goes on.
/// Mutant (unkilled on fx-wake-inputs): in `host::under_the_wake_cover`, drop `s.shell.away.cover_a_gap_not_yet_found()`
/// -> red at "nothing is navigated before the first poll" (the ask is carried out: `Some(Changed)`).
#[test]
fn wake_hold_an_ask_before_the_first_poll_after_a_wake_waits() {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(5 * 60));
    looked(&mut live);
    assert!(!shows(&live, Section::Photos));
    slept(&mut live, Duration::from_secs(20 * 60));
    // No frame between the wake and the ask: the poll has not found the gap.
    let mut outcome = None;
    live.rt.update(|s, _| {
        outcome = Some(host::request(s, Intent::Go(Route::section(Section::Photos))));
    });
    assert_eq!(
        outcome,
        Some(Outcome::Deferred),
        "nothing is navigated before the first poll"
    );
    assert!(
        !shows(&live, Section::Photos),
        "nothing is navigated before the first poll"
    );
    assert!(registry::covered(&live.rt.state), "the ask covers the gap");
    until(&mut live, "the ask never went on", |live| shows(live, Section::Photos));
    assert!(!live.rt.state.shell.away.covers(), "it went on once the cover was gone");
    assert_eq!(
        live.engine.lock_state(),
        LockState::Open,
        "twenty-five minutes of thirty"
    );
}

/// The away lock tells the media keys' [`WakeCover`] of its cover. A resume covers at once, before any poll, and the
/// wake's answer lifts it. After a wake Windows did not announce, the gap the poll has not found yet holds the keys
/// before any poll runs, and the gap's answer lets them go. The shell's away lock tells the process's one, which the
/// shell's controls ask (`media::os_controls`' test checks that half).
/// Mutants: in `AwayLock::event`, drop `self.tell()` -> red at "the resume holds the keys at once"; in `AwayLock::poll`,
/// drop `self.tell()` -> red at "the answer lets the keys go"; in `WakeCover::covers_now`, drop the
/// `gap_not_yet_found` term -> red at "a gap the poll has not found yet holds the keys"; `AwayLock::default` telling
/// `WakeCover::default()` -> red at "the shell's away lock tells the shell's cover".
#[test]
fn wake_hold_the_away_lock_tells_the_media_keys() {
    assert!(
        AwayLock::default().tells.same_as(&WakeCover::shell()),
        "the shell's away lock tells the shell's cover"
    );
    let mut live = live();
    // A cover of its own: every test's away lock tells the process's one.
    let cover = WakeCover::default();
    away(&mut live.rt).tells = cover.clone();
    until_drawn(&mut live, FILE_NAME);
    thirty_minutes(&live);
    idle_for(&mut live, Duration::from_secs(5 * 60));
    looked(&mut live);
    assert!(!cover.covers_now(), "open, no wake: the keys act");

    // A wake Windows announced: the tray's resume, before any poll.
    live.rt.update(|s, tx| {
        host::on_tray(
            s,
            TrayEvent::Resumed {
                away: Duration::from_secs(3 * 60),
            },
            tx,
        )
    });
    assert!(cover.covers_now(), "the resume holds the keys at once");
    until(&mut live, "the resume's cover was never lifted", |live| {
        !live.rt.state.shell.away.covers()
    });
    assert!(!cover.covers_now(), "the answer lets the keys go");

    // A wake Windows did not announce: no resume, no poll yet. The clock moved on since the last poll told the cover.
    let gap = Duration::from_secs(20 * 60);
    slept(&mut live, gap);
    cover.lock().seen.as_mut().expect("the last poll told it").wall -= gap;
    assert!(cover.covers_now(), "a gap the poll has not found yet holds the keys");
    until(&mut live, "the gap's cover was never lifted", |live| {
        !live.rt.state.shell.away.covers()
    });
    assert!(!cover.covers_now(), "the gap's answer lets the keys go");
    assert_eq!(live.engine.lock_state(), LockState::Open);
}
