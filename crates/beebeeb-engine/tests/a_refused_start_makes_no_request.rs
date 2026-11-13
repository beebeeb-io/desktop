//! S1, invariants I8 and I10 (contract §6): this PC's local data belongs to another account; the start refuses with
//! `OtherAccountsData`, makes no request and changes nothing in the folder. Mutant: ignore the binding's refusal, so
//! an engine is made for the other account's data.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_refused_start_makes_no_request() {
    use beebeeb_engine::account_binding::Identity;
    use beebeeb_engine::app::SyncStartRefused;
    use beebeeb_engine::sync_view::Start;

    let s = common::setup();
    s.state_db_with_one_row(&Identity::new(Some("u-another"), Some("another@example.eu")));
    let folder = s.home.join("Beebeeb Test");
    std::fs::create_dir_all(folder.join("Plans")).unwrap();
    std::fs::write(folder.join("Plans").join("budget.ods"), b"twelve bytes").unwrap();
    s.sign_in();
    s.unlock();
    let before = common::listing(&folder);
    let from = s.fake.requests().len();

    assert_eq!(
        s.engine.start_sync(folder.clone()),
        Err(SyncStartRefused::OtherAccountsData)
    );
    // An engine that was made anyway would register a device and ask for a snapshot within a second.
    std::thread::sleep(std::time::Duration::from_secs(2));

    assert_eq!(s.routes_from(from), Vec::<String>::new(), "0 requests to the fake");
    assert_eq!(common::listing(&folder), before, "the folder is unchanged");
    assert!(!s.engine.is_syncing());
    assert_eq!(
        s.engine.view().start,
        Start::Refused(SyncStartRefused::OtherAccountsData)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
