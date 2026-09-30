//! The reducers are each correct alone; these tests run them TOGETHER, because
//! the defects this task exists to fix ("said twice", window sprawl) are
//! disagreements BETWEEN them (spec section 7, section 2).

use super::failure::{
    ErrorSurface, FailureEvent, FailureKind, FailureLedger, FailureReport, Origin, Visible,
    finder_failure_shown_elsewhere, place,
};
use super::phase::{FinderSetup, PopoverPhase, PopoverSnapshot, popover_phase};
use super::registry::{SurfaceKind, SurfaceRegistry};

fn all_visibility() -> Vec<Visible> {
    let mut v = Vec::new();
    for popover in [false, true] {
        for settings in [false, true] {
            for other_window in [false, true] {
                v.push(Visible {
                    popover,
                    settings,
                    other_window,
                });
            }
        }
    }
    v
}

/// How many surfaces show a failed Finder install right now: the popover body
/// (f2) when the phase reducer says so, plus whatever the ledger places.
fn finder_failure_surfaces(origin: Option<Origin>, vis: Visible) -> Vec<&'static str> {
    let mut ledger = FailureLedger::new();
    ledger.apply(FailureEvent::Failed(FailureReport::new(
        "finder-install",
        FailureKind::FinderInstall,
        origin,
    )));
    let snapshot = PopoverSnapshot {
        finder: FinderSetup::Failed,
        finder_failure_elsewhere: finder_failure_shown_elsewhere(origin, vis),
        ..PopoverSnapshot::healthy()
    };
    let mut shown = Vec::new();
    if vis.popover && popover_phase(&snapshot) == PopoverPhase::FinderFailed {
        shown.push("popover f2");
    }
    for r in ledger.rendered(vis) {
        match r.surface {
            // The ledger's PopoverState IS the popover's f2: one surface, and
            // the phase reducer is what renders it. Counted once, above.
            ErrorSurface::PopoverState => {}
            ErrorSurface::SettingsRow => shown.push("settings row"),
            other => shown.push(match other {
                ErrorSurface::Toast => "toast",
                _ => "other",
            }),
        }
    }
    shown
}

#[test]
fn the_popover_and_the_ledger_never_show_a_finder_failure_twice() {
    // Every origin x every visibility, INCLUDING the combinations the registry
    // forbids: the two reducers must agree on their own, not only because the
    // registry keeps the situation from arising.
    let mut combos = 0;
    for origin in [None, Some(Origin::Popover), Some(Origin::Settings)] {
        for vis in all_visibility() {
            let shown = finder_failure_surfaces(origin, vis);
            assert!(shown.len() <= 1, "{origin:?} {vis:?}: {shown:?}");
            if vis.popover || vis.settings {
                assert_eq!(shown.len(), 1, "{origin:?} {vis:?} must show it somewhere: {shown:?}");
            }
            combos += 1;
        }
    }
    assert_eq!(combos, 3 * 8);
}

#[test]
fn a_settings_origin_failure_is_not_repeated_in_the_popover_when_both_are_visible() {
    // The slice-1 review scenario: Repair fails in Settings, the popover is
    // visible as well.
    let vis = Visible {
        popover: true,
        settings: true,
        other_window: false,
    };
    assert_eq!(
        place(FailureKind::FinderInstall, Some(Origin::Settings), vis),
        Some(ErrorSurface::SettingsRow)
    );
    assert_eq!(finder_failure_surfaces(Some(Origin::Settings), vis), ["settings row"]);
    assert_eq!(finder_failure_surfaces(Some(Origin::Popover), vis), ["popover f2"]);
}

#[test]
fn the_popover_is_f2_when_it_is_the_one_surface_showing_the_failure() {
    let popover = Visible {
        popover: true,
        ..Visible::default()
    };
    for origin in [None, Some(Origin::Popover), Some(Origin::Settings)] {
        assert_eq!(finder_failure_surfaces(origin, popover), ["popover f2"], "{origin:?}");
    }
}

#[test]
fn every_reachable_registry_state_shows_a_finder_failure_exactly_once() {
    // Drive the real registry through every sequence of up to 4 opens and
    // feed its visibility into both reducers.
    let mut checked = 0usize;
    let mut stack: Vec<Vec<SurfaceKind>> = vec![vec![]];
    while let Some(seq) = stack.pop() {
        let mut r = SurfaceRegistry::new();
        for &k in &seq {
            r.open(k);
        }
        let vis = r.visible();
        for origin in [Some(Origin::Popover), Some(Origin::Settings)] {
            let shown = finder_failure_surfaces(origin, vis);
            let expected = usize::from(vis.popover || vis.settings);
            assert_eq!(shown.len(), expected, "{seq:?} {origin:?} {vis:?}: {shown:?}");
        }
        checked += 1;
        if seq.len() < 4 {
            for k in SurfaceKind::ALL {
                let mut next = seq.clone();
                next.push(k);
                stack.push(next);
            }
        }
    }
    // 1 + 4 + 16 + 64 + 256.
    assert_eq!(checked, 341);
}
