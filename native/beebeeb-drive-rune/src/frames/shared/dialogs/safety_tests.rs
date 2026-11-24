//! People's safety number and "Share anyway" with a source that works numbers out (the test double of
//! `source.rs`): what each press asks, what the dialogs show, and that nothing shares but "Share anyway".

use super::Dialog;
use crate::frames::Intent;
use crate::frames::shared::scenes;
use crate::frames::shared::source::{BoardShared, SafetyAsk, SafetyState};
use crate::frames::shared::tests::{click, info, runtime};
use crate::{State, init};

const LISBON: &str = "p-lisbon";
const JONAS: &str = "m-jonas";

/// The scene's sample lists, with a source that works safety numbers out in their place.
fn wired(s: &mut State) {
    let mut board = BoardShared::new();
    board.safety = Some(Default::default());
    s.frames.shared.source = Box::new(board);
}

fn person(s: &mut State) {
    scenes::dw_0512(s);
    wired(s);
}

fn changed_key(s: &mut State) {
    scenes::dw_0515(s);
    wired(s);
}

#[test]
fn a_person_s_safety_row_opens_the_engine_s_number_and_asks_no_intent() {
    let mut rt = runtime(person);
    click(&mut rt, "item-4-0");
    assert!(
        rt.state.frames.intents.is_empty(),
        "the press is the source's, not a recorded intent"
    );
    assert!(matches!(
        rt.state.frames.shared.dialog,
        Some(Dialog::SafetyNumber { .. })
    ));
    let groups = match rt.state.frames.shared.source.safety_state(LISBON, JONAS) {
        SafetyState::Number(g) => g,
        other => panic!("{other:?}"),
    };
    assert_eq!(groups.len(), 12);
    assert_eq!(info(&rt, "safety-0").text.as_deref(), Some(groups[0].as_str()));
}

#[test]
fn they_match_marks_compared_and_they_dont_match_stops_sharing() {
    let mut rt = runtime(person);
    click(&mut rt, "item-4-0");
    click(&mut rt, "btn-they-match");
    assert_eq!(
        rt.state.frames.shared.member(LISBON, JONAS).map(|m| m.compared),
        Some(true)
    );
    assert!(matches!(
        rt.state.frames.shared.source.safety_state(LISBON, JONAS),
        SafetyState::Marked { matched: true, .. }
    ));
    click(&mut rt, "btn-done");
    assert!(matches!(rt.state.frames.shared.dialog, Some(Dialog::Person { .. })));
    assert_eq!(
        info(&rt, "item-4-0")
            .label
            .as_deref()
            .map(|l| l.contains("Not compared")),
        Some(false)
    );

    let mut rt = runtime(person);
    click(&mut rt, "item-4-0");
    click(&mut rt, "btn-they-don-t-match");
    // Stopped, over the same number.
    assert_eq!(info(&rt, "dlg").label.as_deref(), Some("Sharing with Jonas is stopped"));
    click(&mut rt, "btn-compare-again");
    assert!(matches!(
        rt.state.frames.shared.dialog,
        Some(Dialog::SafetyNumber { .. })
    ));
    assert!(matches!(
        rt.state.frames.shared.source.safety_state(LISBON, JONAS),
        SafetyState::Number(_)
    ));
    assert!(rt.state.frames.intents.is_empty());
}

#[test]
fn only_the_share_anyway_press_shares_and_it_leaves_the_warning() {
    let mut rt = runtime(changed_key);
    assert_eq!(
        rt.state.frames.shared.source.safety_state(LISBON, JONAS),
        SafetyState::Idle
    );
    assert!(rt.find("alert").is_some());
    click(&mut rt, "btn-share-anyway");
    assert_eq!(
        rt.state.frames.shared.source.safety_state(LISBON, JONAS),
        SafetyState::Shared
    );
    assert!(rt.find("alert").is_none(), "nothing left to warn about");
    assert!(rt.state.frames.intents.is_empty());

    // Compare safety numbers on the warning asks for the number only; it never shares.
    let mut rt = runtime(changed_key);
    click(&mut rt, "btn-compare-safety-numbers");
    assert!(matches!(
        rt.state.frames.shared.dialog,
        Some(Dialog::SafetyNumber { .. })
    ));
    assert!(matches!(
        rt.state.frames.shared.source.safety_state(LISBON, JONAS),
        SafetyState::Number(_)
    ));
}

#[test]
fn an_unwired_source_still_only_records_the_press() {
    let mut rt = runtime(scenes::dw_0515);
    click(&mut rt, "btn-share-anyway");
    assert_eq!(rt.state.frames.intents, vec![Intent::Do("share-anyway", JONAS.into())]);
    let mut source = init().frames.shared.source;
    assert!(!source.safety_wired());
    assert!(!source.safety_start(LISBON, JONAS, SafetyAsk::Anyway));
}
