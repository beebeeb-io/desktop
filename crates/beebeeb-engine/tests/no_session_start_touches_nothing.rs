//! S1, invariant I6 (contract §6): signed in without the 12 words there are no keys, and a start then touches
//! nothing: no folder, no `state.db`, no request. Mutant: open the local state (step 7) before the keys are read.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn no_session_start_touches_nothing() {
    use beebeeb_engine::app::SyncStart;
    use beebeeb_engine::sync_view::Start;

    let s = common::setup();
    s.sign_in();
    let from = s.fake.requests().len();
    let folder = s.home.join("Beebeeb Test");

    assert_eq!(s.engine.start_sync(folder.clone()), Ok(SyncStart::NoSession));

    assert!(!folder.exists(), "no folder");
    assert!(!s.state_db().exists(), "no state.db");
    assert_eq!(common::listing(&s.state_dir), [], "nothing in the state folder");
    assert_eq!(s.routes_from(from), Vec::<String>::new(), "0 requests");
    assert_eq!(s.engine.view().start, Start::NotAsked);
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
