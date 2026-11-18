//! People's safety calls (w18-people-safety-number) against the fake accounts of `shared_tests.rs` (a child of it).
//! The seal-side behaviour (the number from a sealed key, the marks' effect on the next seal) is the engine's own
//! tests'; here: what the source lets through, what a refusal shows, and that only "Share anyway" can change a share.

use super::*;

#[test]
fn a_mark_needs_the_number_and_a_share_with_no_sealed_key_is_refused() {
    let fake = fake("safety-number");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    opened(&mut source);
    // Not a person of the lists, and not the owner: nothing starts.
    assert!(!source.safety_start(TRIP, OTHERS, SafetyAsk::Number));
    assert!(!source.safety_start(TRIP, OWNER, SafetyAsk::Number));
    // A mark before any number is refused at the source.
    assert!(!source.safety_start(TRIP, INV_A, SafetyAsk::Matches));
    assert!(!source.safety_start(TRIP, INV_A, SafetyAsk::Differs));
    assert_eq!(source.safety_state(TRIP, INV_A), SafetyState::Idle);

    assert!(source.safety_wired());
    assert!(source.safety_start(TRIP, INV_A, SafetyAsk::Number));
    assert!(
        !source.safety_start(TRIP, INV_A, SafetyAsk::Number),
        "one call per person at a time"
    );
    assert!(settle(&mut source).is_empty());
    // The fake sealed nothing, so there is no number: a refusal in fixed words, never the engine's kind.
    assert!(matches!(source.safety_state(TRIP, INV_A), SafetyState::Refused(words) if !words.contains("_")));
    assert!(
        !source.members(TRIP).iter().any(|m| m.compared),
        "nobody compared anything"
    );
    assert!(changes(&fake.fake).is_empty(), "a number and a refusal send nothing");
}

#[test]
fn share_anyway_on_a_share_no_changed_key_stopped_changes_nothing() {
    let fake = fake("safety-anyway");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    opened(&mut source);
    assert!(source.safety_start(TRIP, INV_A, SafetyAsk::Anyway));
    assert!(settle(&mut source).is_empty());
    assert!(matches!(source.safety_state(TRIP, INV_A), SafetyState::Refused(_)));
    assert!(changes(&fake.fake).is_empty(), "no approval and no withdrawal left");
    source.safety_clear(TRIP, INV_A);
    assert_eq!(source.safety_state(TRIP, INV_A), SafetyState::Idle);
}

#[test]
fn what_a_session_ending_drops_is_not_kept() {
    let fake = fake("safety-forget");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    opened(&mut source);
    assert!(source.safety_start(TRIP, INV_A, SafetyAsk::Number));
    assert!(settle(&mut source).is_empty());
    source.forget();
    assert_eq!(source.safety_state(TRIP, INV_A), SafetyState::Idle);
    assert_eq!(source.key_changed_person(TRIP), None);
}
