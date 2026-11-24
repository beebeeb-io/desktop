//! S1, step 4 of §4.2 (contract): a live process holds this app's engine lock. The start refuses with `StateInUse`
//! before `state.db` is opened: its file is not touched (modification time) and the start's adoption mark is not
//! written (`PRAGMA user_version` stays 0). Through `Engine::start_sync_with`, so it runs on every OS but macOS.
//! Mutant: step 4 after step 7.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn state_in_use_is_refused_before_state_db_opens() {
    use beebeeb_engine::app::SyncStartRefused;
    use beebeeb_engine::state_db::StateDb;

    let s = common::setup();
    std::fs::create_dir_all(&s.state_dir).unwrap();
    drop(StateDb::open(s.state_db()).unwrap());
    let user_version = || {
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let db = rusqlite::Connection::open_with_flags(s.state_db(), flags).unwrap();
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let modified = || std::fs::metadata(s.state_db()).unwrap().modified().unwrap();
    assert_eq!(user_version(), 0, "precondition: no start marked it yet");
    let before = modified();
    s.sign_in();
    s.unlock();
    // The file system's clock may be coarse: a write after this would show.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let port = common::Port {
        lock_held: true,
        ..common::Port::free()
    };

    let refused = s.engine.start_sync_with(s.home.join("Beebeeb Test"), &port);

    assert_eq!(refused, Err(SyncStartRefused::StateInUse));
    assert_eq!(modified(), before, "state.db was not written");
    assert_eq!(user_version(), 0, "state.db was not opened by the start");
    assert_eq!(port.registers(), 0);
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
