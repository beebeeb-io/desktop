//! S1, invariant I9 (contract §6, step 6 of §4.2): this PC holds the account's own local data, and its sync folder is
//! gone. The start refuses with `FolderMissing` and does not make the folder again, empty. Through
//! `Engine::start_sync_with`, so it runs on every OS but macOS. Mutant: drop step 6.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_vanished_folder_with_account_data_is_refused() {
    use beebeeb_engine::app::SyncStartRefused;

    let s = common::setup();
    s.state_db_with_one_row(&s.this_account());
    s.sign_in();
    s.unlock();
    let folder = s.home.join("Beebeeb Test");
    assert!(!folder.exists(), "precondition: the folder is gone");
    let port = common::Port::free();

    let refused = s.engine.start_sync_with(folder.clone(), &port);

    assert_eq!(refused, Err(SyncStartRefused::FolderMissing));
    assert!(!folder.exists(), "the folder is not made again");
    assert_eq!(port.registers(), 0, "nothing was registered");
    assert!(!s.engine.is_syncing());
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
