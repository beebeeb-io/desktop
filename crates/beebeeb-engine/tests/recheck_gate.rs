//! w16-gates: `Engine::recheck_gate` runs `gate()` again and publishes it, so `engine.view().gate` follows the
//! credential store and the plan check the window's gate buttons ask about (Try again, Check again). It is not the
//! start path: `restore()` is never run again.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn recheck_gate_publishes_the_gate_it_reads_and_runs_no_restore() {
    use beebeeb_engine::signin::gate::GateState;

    let s = common::setup();
    s.sign_in();
    assert_eq!(s.engine.view().gate, GateState::NeedsRecoveryPhrase);

    // The published gate went stale (a store that did not answer, a plan that arrived): the view says so.
    s.engine
        .set_view_for_test(|view| view.gate = GateState::KeyStoreUnavailable);
    assert_eq!(s.engine.view().gate, GateState::KeyStoreUnavailable);

    let from = s.fake.requests().len();
    assert_eq!(s.engine.recheck_gate(), GateState::NeedsRecoveryPhrase);
    assert_eq!(s.engine.view().gate, GateState::NeedsRecoveryPhrase, "published");
    assert_eq!(
        s.routes_from(from),
        Vec::<String>::new(),
        "a re-check of the gate asks nobody"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
