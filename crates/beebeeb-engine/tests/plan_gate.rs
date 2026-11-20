//! w16-gates: the No plan gate. `Engine::gate` blocks Files only when the SERVER says the account has no active
//! plan (`plan_state` none or lapsed). A server that cannot be reached never blocks on its own: the last known
//! state is used. `recheck_gate` asks the server for the plan again and runs no restore.
//! (One engine per test process: the state folder is resolved once, so one test per file.)

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn the_gate_follows_what_the_server_says_and_the_last_known_state_when_it_cannot_say() {
    use beebeeb_engine::signin::gate::GateState;

    let s = common::setup();
    s.account.set_plan_state("none");
    s.sign_in();
    s.unlock();
    // The first ask is in the background: the gate follows when the answer lands.
    assert!(
        common::wait_until(10, || s.engine.view().gate == GateState::NoPlan),
        "no plan: blocked"
    );

    // Unreachable, last known none: stays blocked.
    s.account.set_plan_unreachable(true);
    assert_eq!(s.engine.recheck_gate(), GateState::NoPlan);

    // The plan arrived: Check again unblocks, with one ask for the plan and no restore.
    s.account.set_plan_unreachable(false);
    s.account.set_plan_state("active");
    let from = s.fake.requests().len();
    assert_eq!(s.engine.recheck_gate(), GateState::Ready);
    assert_eq!(s.engine.view().gate, GateState::Ready, "published");
    assert_eq!(s.routes_from(from), vec!["GET /api/v1/billing/subscription".to_string()]);

    // Unreachable, last known active: not blocked, whatever the server would say.
    s.account.set_plan_unreachable(true);
    s.account.set_plan_state("none");
    assert_eq!(s.engine.recheck_gate(), GateState::Ready);

    // The server again: lapsed blocks; trial and free do not.
    s.account.set_plan_unreachable(false);
    s.account.set_plan_state("lapsed");
    assert_eq!(s.engine.recheck_gate(), GateState::NoPlan);
    for state in ["trial", "free"] {
        s.account.set_plan_state(state);
        assert_eq!(s.engine.recheck_gate(), GateState::Ready, "{state}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
