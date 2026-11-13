//! S1, invariant I7 (contract §6, step 5 of §4.2): the OS says another provider registered the folder. The start
//! refuses with `FolderOfAnotherProvider` before anything is written: the folder is unchanged, there is no
//! `state.db`, and nothing was registered. Through `Engine::start_sync_with`, so it runs on every OS but macOS.
//! Mutant: drop step 5.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn another_providers_folder_is_refused_before_anything_is_written() {
    use beebeeb_engine::app::SyncStartRefused;
    use beebeeb_engine::sync_root_owner::Registration;
    use beebeeb_engine::sync_view::Start;

    let s = common::setup();
    s.sign_in();
    s.unlock();
    let folder = s.home.join("Beebeeb Test");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("theirs.txt"), b"another app's file").unwrap();
    let before = common::listing(&folder);
    let port = common::Port {
        registration: Registration::Provider("Other".into()),
        ..common::Port::free()
    };

    let refused = s.engine.start_sync_with(folder.clone(), &port);

    assert_eq!(refused, Err(SyncStartRefused::FolderOfAnotherProvider));
    assert_eq!(common::listing(&folder), before, "no folder change");
    assert!(!s.state_db().exists(), "no state.db");
    assert_eq!(port.registers(), 0, "nothing was registered");
    assert_eq!(
        s.engine.view().start,
        Start::Refused(SyncStartRefused::FolderOfAnotherProvider)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
