//! dc-165, on the away lock's live window (`super`): what the locked window asks to be drawn while the engine rings
//! with nothing new. The real engine runs behind it (its sync, its reports); every name and password is invented.

use super::*;
use crate::shell::redraw;

/// Locked by the away lock, drawn and at rest.
fn locked() -> Live {
    let mut live = live();
    until_drawn(&mut live, FILE_NAME);
    idle_for(&mut live, Duration::from_secs(300));
    looked(&mut live);
    until(&mut live, "the window did not show the lock", |live| {
        live.rt.state.shell.app.gate() == GateState::Locked
    });
    until_drawn(&mut live, "Beebeeb is locked");
    for _ in 0..30 {
        live.rt.frame(0.016);
        std::thread::sleep(Duration::from_millis(5));
    }
    live
}

/// The red half: what the engine's hook did before (`main.update(|_, _| {})`). An update that changes nothing still
/// runs the view and draws the window, once per ring.
#[test]
fn an_empty_update_of_the_locked_view_is_a_redraw_each_time() {
    let mut live = locked();
    let before = live.rt.stats.commits;
    for _ in 0..10 {
        live.rt.update(|_, _| {});
        live.rt.frame(0.016);
    }
    assert_eq!(live.rt.stats.commits - before, 10, "one commit per empty update");
}

/// Two seconds of the locked window with the engine ringing all the time (200 rings) and the real engine running
/// behind it: no frame is asked for unless what the window shows moved, nothing animates, and no timer is armed
/// that would wake it within the two seconds.
#[test]
fn the_locked_view_asks_for_no_redraw_without_a_state_change() {
    let mut live = locked();
    let commits = live.rt.stats.commits;
    let began = Instant::now();
    let (mut rings, mut redraws) = (0, 0);
    while began.elapsed() < Duration::from_secs(2) {
        rings += 1;
        let drawn = words(&live);
        // The ring as the window takes it (`AppHandle::update_if`): `false` asks for no frame.
        if live.rt.update_if(redraw::engine_woke) {
            live.rt.frame(0.016);
            redraws += 1;
            assert_ne!(
                words(&live),
                drawn,
                "ring {rings}: the window was drawn again and shows the same"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(rings >= 100, "{rings} rings in two seconds");
    assert_eq!(
        live.rt.stats.commits - commits,
        redraws,
        "the view ran only for a ring that moved something"
    );
    assert!(redraws <= 2, "{redraws} redraws of a locked window in two seconds");
    // What Rune's own loop would draw for: an animation, or a timer the poll armed.
    assert!(!live.rt.frame(0.016), "the locked window animates");
    assert!(words(&live).contains("Beebeeb is locked"));
}
