//! S1, the happy path (contract §8 S1): sign in, the 12 words, then a start on a folder that does not exist yet. The
//! start makes the folder, an engine runs, the view says so, and the first pull puts the account's 7 items in
//! `state.db`. Mutant: remove step 8 (the start no longer makes the folder).

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn phrase_then_start_syncs_7_items() {
    use beebeeb_engine::app::SyncStart;
    use beebeeb_engine::sync_view::Start;

    let s = common::setup();
    s.sign_in();
    s.unlock();
    let folder = s.home.join("Beebeeb Test");
    assert!(!folder.exists(), "precondition: the folder does not exist");

    assert_eq!(s.engine.start_sync(folder.clone()), Ok(SyncStart::Started));
    assert!(folder.is_dir(), "the start made the folder before the engine");
    assert_eq!(s.engine.view().start, Start::Running);
    let rows = common::wait_for_rows(&s.state_db(), 7);
    s.engine.stop();
    assert_eq!(rows, 7, "state.db has the account's 7 items");
    assert_eq!(
        s.engine.view().start,
        Start::NotAsked,
        "a stopped engine is not running"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
