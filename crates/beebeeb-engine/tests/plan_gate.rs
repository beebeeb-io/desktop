//! w16-gates: the No plan gate. `Engine::gate` blocks Files only when the SERVER says the account has no active
//! plan (`plan_state` none or lapsed). A server that cannot be reached, or an answer that does not read, never
//! blocks on its own: the last known state is used. `recheck_gate` asks the server for the plan again and runs no
//! restore.

mod common;

#[cfg(not(target_os = "macos"))]
mod plan {
    use super::common;
    use beebeeb_engine::signin::gate::GateState;

    fn ready(state: &str) -> common::Setup {
        let s = common::setup();
        s.account.set_plan_state(state);
        s.sign_in();
        s.unlock();
        s
    }

    #[test]
    fn no_active_plan_is_the_no_plan_gate() {
        for state in ["none", "lapsed"] {
            let s = ready(state);
            assert_eq!(s.engine.view().gate, GateState::NoPlan, "{state}");
        }
    }

    #[test]
    fn an_active_plan_is_no_gate() {
        for state in ["active", "trial", "free"] {
            let s = ready(state);
            assert_eq!(s.engine.view().gate, GateState::Ready, "{state}");
        }
    }

    #[test]
    fn unreachable_with_last_known_active_does_not_block() {
        let s = ready("active");
        s.account.set_plan_unreachable(true);
        s.account.set_plan_state("none");
        assert_eq!(s.engine.recheck_gate(), GateState::Ready);
    }

    #[test]
    fn unreachable_with_last_known_none_stays_blocked() {
        let s = ready("none");
        assert_eq!(s.engine.view().gate, GateState::NoPlan);
        s.account.set_plan_unreachable(true);
        assert_eq!(s.engine.recheck_gate(), GateState::NoPlan);
    }

    #[test]
    fn unreachable_from_the_start_never_blocks() {
        let s = common::setup();
        s.account.set_plan_state("none");
        s.account.set_plan_unreachable(true);
        s.sign_in();
        s.unlock();
        assert_eq!(s.engine.view().gate, GateState::Ready);
    }

    #[test]
    fn recheck_after_the_plan_became_active_unblocks_and_runs_no_restore() {
        let s = ready("none");
        assert_eq!(s.engine.view().gate, GateState::NoPlan);
        s.account.set_plan_state("active");
        let from = s.fake.requests().len();
        assert_eq!(s.engine.recheck_gate(), GateState::Ready);
        assert_eq!(s.engine.view().gate, GateState::Ready, "published");
        assert_eq!(s.routes_from(from), vec!["GET /api/v1/billing/subscription".to_string()]);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
