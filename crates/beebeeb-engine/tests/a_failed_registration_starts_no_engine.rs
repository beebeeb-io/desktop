//! S1, invariant I7 (contract §6, step 9 of §4.2): the OS refuses to register the folder. The start refuses with
//! `RegistrationFailed` and Windows' code, no engine exists, the view says the start was refused, and the fake gets
//! no request. Through `Engine::start_sync_with`, so it runs on every OS but macOS. Mutant: ignore the
//! registration's error.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_failed_registration_starts_no_engine() {
    use beebeeb_engine::app::SyncStartRefused;
    use beebeeb_engine::sync_view::Start;

    /// `HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_INVALID_REQUEST)`, an invented refusal.
    const CODE: i32 = 0x8007_017C_u32 as i32;
    let s = common::setup();
    s.sign_in();
    s.unlock();
    let port = common::Port {
        register: Err(Some(CODE)),
        ..common::Port::free()
    };
    let from = s.fake.requests().len();

    let refused = s.engine.start_sync_with(s.home.join("Beebeeb Test"), &port);

    let kind = SyncStartRefused::RegistrationFailed { code: Some(CODE) };
    assert_eq!(refused, Err(kind));
    assert_eq!(port.registers(), 1);
    assert!(!s.engine.is_syncing(), "the engine slot is empty");
    let view = s.engine.view();
    assert_eq!(view.start, Start::Refused(kind));
    assert_eq!(view.attempt, 1, "one refused attempt");
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(s.routes_from(from), Vec::<String>::new(), "no request");
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
