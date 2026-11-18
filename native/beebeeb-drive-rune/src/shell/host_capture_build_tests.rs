//! Which build hides its secret screens from capture: the real app on every one of them, the test build on none
//! (it is what a release is recorded from). `poll_capture` decides from `State::identity`, the identity the
//! window was started with; these tests name the identity they mean, so they pass under either build.

use super::*;
use crate::shell::identity::AppIdentity;

/// A signed-out window of `identity`, with its first poll done.
fn window_of(identity: AppIdentity) -> (Arc<FixtureEngine>, Arc<FakePlatform>, Runtime<State>) {
    let engine = Arc::new(FixtureEngine::signed_out());
    let platform = Arc::new(FakePlatform::default());
    let mut state = state::init_with(Shell::fixture_on(engine.clone(), platform.clone()));
    state.identity = identity;
    (engine, platform, runtime(state))
}

/// Sign-in, every sign-up screen (the password, the recovery words shown and typed back), the recovery phrase
/// of a new PC and the lock screen. Returns how many screens it saw.
fn walk_the_secret_screens(
    engine: &FixtureEngine,
    rt: &mut Runtime<State>,
    mut on_secret_screen: impl FnMut(&Runtime<State>, &str),
) -> usize {
    use crate::frames::signin::create::FRAME_IDS;
    let mut seen = 0;
    let mut at = 1.0;
    let mut check = |rt: &mut Runtime<State>, what: &str| {
        at += 1.0;
        rt.frame(at);
        assert!(
            registry::route_shows_secret(&rt.state.shell.app),
            "{what} is a secret screen"
        );
        on_secret_screen(rt, what);
        seen += 1;
    };
    check(rt, "sign-in");
    for frame in FRAME_IDS {
        rt.update(|s, _| s.signin = crate::frames::signin::scene(frame));
        check(rt, frame);
    }
    engine.set_gate(GateState::NeedsRecoveryPhrase);
    check(rt, "the recovery phrase");
    engine.set_gate(GateState::Locked);
    check(rt, "the lock screen");
    seen
}

#[test]
fn only_the_test_build_leaves_its_secret_screens_capturable() {
    assert!(hides_secrets_from_capture(AppIdentity::Real));
    assert!(!hides_secrets_from_capture(AppIdentity::Test));
    assert!(
        hides_secrets_from_capture(AppIdentity::GOLDEN),
        "the fixture windows of the other capture tests are the real build"
    );
}

/// The test build: no exclusion call at all, on any secret screen, and nothing left to retry.
#[test]
fn the_test_build_never_asks_the_platform_for_capture_exclusion() {
    let (engine, platform, mut rt) = window_of(AppIdentity::Test);
    let seen = walk_the_secret_screens(&engine, &mut rt, |rt, what| {
        assert!(!rt.state.shell.capture_excluded, "{what} is hidden from a recording");
        assert_eq!(rt.state.shell.capture_exclusion_retries, 0, "{what}");
    });
    assert!(seen > 10, "{seen} screens");
    assert!(
        platform.capture_exclusions.lock().unwrap().is_empty(),
        "the platform was never called"
    );
    // A platform that would fail is never reached either.
    platform
        .capture_exclusion_fails
        .store(true, std::sync::atomic::Ordering::SeqCst);
    engine.set_gate(GateState::Locked);
    rt.frame(100.0);
    assert_eq!(rt.state.shell.capture_exclusion_retries, 0);
}

/// The real build, over the same walk: excluded on every secret screen, by one call that is never lifted
/// between them.
#[test]
fn the_real_build_excludes_every_secret_screen() {
    let (engine, platform, mut rt) = window_of(AppIdentity::Real);
    let seen = walk_the_secret_screens(&engine, &mut rt, |rt, what| {
        assert!(rt.state.shell.capture_excluded, "{what} can be captured");
    });
    assert!(seen > 10, "{seen} screens");
    assert_eq!(platform.capture_exclusions.lock().unwrap().clone(), vec![true]);
}

/// A link password typed in the Share dialog of the test build: no call either (the real build:
/// `a_links_password_field_excludes_the_window_from_capture_while_it_shows`).
#[test]
fn the_test_build_does_not_exclude_a_link_password() {
    let (_engine, platform, mut rt) = fixture();
    rt.state.identity = AppIdentity::Test;
    files::share_with_a_password(&mut rt);
    settle(&mut rt);
    assert!(crate::frames::files::share::shows_secret(&rt.state));
    assert!(!rt.state.shell.capture_excluded);
    assert!(platform.capture_exclusions.lock().unwrap().is_empty());
}
