//! S1, invariant I11 (contract §6): after sync ran, a lock stops the engine and forgets the keys in memory. Then the
//! view does not say it runs, a start has no session, a file operation is refused, and the fake gets no request.
//! Mutant: `lock` keeps the session in memory.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn locked_writes_nothing() {
    use beebeeb_engine::app::SyncStart;
    use beebeeb_engine::sync_view::Start;

    let s = common::setup();
    s.sign_in();
    s.unlock();
    let folder = s.home.join("Beebeeb Test");
    assert_eq!(s.engine.start_sync(folder.clone()), Ok(SyncStart::Started));
    assert_eq!(common::wait_for_rows(&s.state_db(), 7), 7, "sync ran");

    s.engine.lock();
    let from = s.fake.requests().len();

    assert_ne!(s.engine.view().start, Start::Running);
    assert!(
        !s.engine.is_syncing(),
        "the files source's local() refuses: no engine runs"
    );
    assert_eq!(s.engine.start_sync(folder), Ok(SyncStart::NoSession));
    assert!(
        s.engine
            .trash_items_for(s.engine.session_turn(), &["a0000000-0000-4000-8000-000000000003"])
            .is_err(),
        "a file operation needs the session"
    );
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(s.routes_from(from), Vec::<String>::new(), "0 requests after the lock");
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
