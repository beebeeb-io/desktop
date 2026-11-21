//! w16-gates: the No plan gate when the server cannot be reached from the start. `Engine::gate` blocks Files only when the SERVER says the account has no active
//! plan (`plan_state` none or lapsed). A server that cannot be reached never blocks on its own: the last known
//! state is used. `recheck_gate` asks the server for the plan again and runs no restore.
//! (One engine per test process: the state folder is resolved once, so one test per file.)

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_server_that_cannot_say_never_blocks_when_nothing_is_known() {
    use beebeeb_engine::signin::gate::GateState;

    let s = common::setup();
    s.account.set_plan_state("none");
    s.account.set_plan_unreachable(true);
    s.sign_in();
    s.unlock();
    assert_eq!(s.engine.view().gate, GateState::Ready);
    assert_eq!(s.engine.recheck_gate(), GateState::Ready);
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
