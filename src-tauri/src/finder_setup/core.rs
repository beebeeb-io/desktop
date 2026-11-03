//! Spec 2026-10-06 §5 (`finder_setup::core`): what the Finder reconciler decides. Pure: no OS,
//! no clock, no I/O. The driver feeds one `Input` at a time and runs the one `Op` that
//! `next_op` asks for. While that op runs nothing else is fed (events wait in the driver's
//! channel), so an add and a remove can never overlap (§5.4).
//!
//! Children: `types` (the state and its vocabulary), `decide` (what a trigger, a tick or a
//! result asks for next), `results` (what one finished operation means), `landing` (the
//! terminal states and the lock hold). The tests under `tests/` are split by area; their
//! harness stays in `mod tests` below.

use std::time::{Duration, Instant};

use super::error::{FpError, app_code};
use super::launch_location::LaunchLocation;
use super::policy::{self, RetryPolicy};
use crate::finder_removal::KeptFolder;
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

mod decide;
mod landing;
mod results;
mod types;

use decide::*;
use landing::*;
use results::*;
use types::*;

pub use decide::{next_op, step};
pub use types::{Trigger, Wanted, wanted, DomainState, Observation, Op, OpResult, Input, Transition, Effect, Next, CoreState};
// Built by the macOS ports and read by this module's own tests.
#[cfg(any(test, target_os = "macos"))]
pub use types::SessionFacts;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::error::{BRIDGE_DOMAIN, COCOA_DOMAIN, FILE_PROVIDER_DOMAIN, bridge_code};
    use DomainState::{Disabled, Enabled, NotRegistered};
    use FinderSetup::{Adding, Failed, Missing, Ready, UserDisabled};
    use LaunchLocation::{Applications, DiskImage};

    const SIGNED_IN: SessionFacts = SessionFacts {
        vault_unlocked: true,
        auth_present: true,
        signed_out_by_choice: false,
        removal_owed: false,
    };
    const KEYS_MISSING: SessionFacts = SessionFacts {
        vault_unlocked: false,
        auth_present: true,
        signed_out_by_choice: false,
        removal_owed: false,
    };
    const SIGNED_OUT_BY_CHOICE: SessionFacts = SessionFacts {
        vault_unlocked: false,
        auth_present: false,
        signed_out_by_choice: true,
        removal_owed: false,
    };
    const SIGNED_OUT_BY_401: SessionFacts = SessionFacts {
        vault_unlocked: false,
        auth_present: false,
        signed_out_by_choice: false,
        removal_owed: false,
    };

    fn fp(code: i64) -> FpError {
        FpError::new(FILE_PROVIDER_DOMAIN, code, "fixture")
    }

    fn stabilization_timeout() -> FpError {
        FpError::new(BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT, "fixture")
    }

    /// What the fake OS answers. `wait_secs` is how long one `WaitStable` takes on the fake
    /// clock; every other op takes no time.
    #[derive(Clone)]
    struct World {
        facts: SessionFacts,
        domain: Result<DomainState, FpError>,
        add: Result<(), FpError>,
        after_add: Result<DomainState, FpError>,
        stable: Result<(), FpError>,
        /// What stopping the engine this check started answers.
        stop: Result<(), FpError>,
        /// What removing the domain answers.
        remove: Result<(), FpError>,
        wait_secs: u64,
    }

    impl World {
        fn ok(facts: SessionFacts, domain: DomainState) -> Self {
            Self {
                facts,
                domain: Ok(domain),
                add: Ok(()),
                after_add: Ok(Enabled),
                stable: Ok(()),
                stop: Ok(()),
                remove: Ok(()),
                wait_secs: 0,
            }
        }
    }

    fn answer(world: &World, op: Op) -> OpResult {
        match op {
            Op::Observe => OpResult::Observed(Observation {
                facts: world.facts,
                domain: world.domain.clone(),
            }),
            Op::StartEngine => OpResult::EngineStarted(Ok(true)),
            Op::AddDomain => OpResult::Added(world.add.clone()),
            Op::ReadDomain => OpResult::Domain(world.after_add.clone()),
            Op::WaitStable(_) => OpResult::Stable(world.stable.clone()),
            Op::FinishReady => OpResult::Finished(Ok(())),
            Op::StopEngine => OpResult::EngineStopped(world.stop.clone()),
            Op::RemoveDomain => OpResult::Removed(world.remove.clone(), KeptFolder::default()),
        }
    }

    /// A miniature driver on an injected clock.
    struct Sim {
        s: CoreState,
        t0: Instant,
        now: Instant,
        policy: RetryPolicy,
        ops: Vec<(u64, Op)>,
        fx: Vec<Effect>,
    }

    impl Sim {
        fn new(launch: LaunchLocation) -> Self {
            let t0 = Instant::now();
            Self {
                s: CoreState::new(launch),
                t0,
                now: t0,
                policy: RetryPolicy::default(),
                ops: Vec::new(),
                fx: Vec::new(),
            }
        }
        fn secs(&self) -> u64 {
            (self.now - self.t0).as_secs()
        }
        fn feed(&mut self, input: Input) {
            let (s, fx) = step(self.s.clone(), input, self.now, &self.policy);
            self.s = s;
            self.fx.extend(fx);
        }
        fn trigger(&mut self, trigger: Trigger) {
            self.feed(Input::Trigger(trigger));
        }
        fn next(&mut self) -> Next {
            next_op(&mut self.s, self.now, &self.policy)
        }
        fn step_op(&mut self, world: &World) -> Op {
            match self.next() {
                Next::Run(op) => {
                    self.ops.push((self.secs(), op));
                    let result = answer(world, op);
                    if matches!(op, Op::WaitStable(_)) {
                        self.now += Duration::from_secs(world.wait_secs);
                    }
                    self.feed(Input::Done(result));
                    op
                }
                other => panic!("expected an operation, got {other:?}"),
            }
        }
        /// Answer ops from `world` (and let backoff timers fire) until the core settles.
        fn run(&mut self, world: &World) {
            for _ in 0..500 {
                if self.s.is_settled() {
                    return;
                }
                match self.next() {
                    Next::Idle => return,
                    Next::WakeAt(at) => {
                        self.now = self.now.max(at);
                        self.feed(Input::Tick { window_visible: false });
                    }
                    Next::Run(op) => {
                        self.ops.push((self.secs(), op));
                        let result = answer(world, op);
                        if matches!(op, Op::WaitStable(_)) {
                            self.now += Duration::from_secs(world.wait_secs);
                        }
                        self.feed(Input::Done(result));
                    }
                }
            }
            panic!("the core did not settle: {:?}", self.s);
        }
        fn kinds(&self) -> Vec<Op> {
            self.ops.iter().map(|(_, op)| *op).collect()
        }
        fn count(&self, op: Op) -> usize {
            self.ops.iter().filter(|(_, o)| *o == op).count()
        }
        fn attempt_starts(&self) -> Vec<u64> {
            self.ops
                .iter()
                .filter(|(_, op)| *op == Op::Observe)
                .map(|(t, _)| *t)
                .collect()
        }
    }

    const WAIT_10: Op = Op::WaitStable(Duration::from_secs(10));
    const WAIT_2: Op = Op::WaitStable(Duration::from_secs(2));


    const SIGNED_IN_OWING: SessionFacts = SessionFacts {
        removal_owed: true,
        ..SIGNED_IN
    };

    mod poll;
    mod removal;
    mod retry;
    mod steps;
}
