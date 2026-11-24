//! The retry budget: four attempts at 0/5/15/45 s, what is retryable, and Try again
//! starting a fresh budget.
use super::*;

    /// §7 on an injected clock: transient failures → exactly 4 attempts at 0/5/15/45 s.
    #[test]
    fn transient_failures_make_four_attempts_at_0_5_15_45() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            add: Err(fp(-2001)),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(sim.attempt_starts(), vec![0, 5, 15, 45]);
        assert_eq!(sim.s.setup, Failed);
        assert_eq!(sim.s.reason, Some(FinderFailureReason::ExtensionLoading));
        assert_eq!((sim.s.attempt, sim.s.max_attempts), (4, 4));
        assert_eq!(
            sim.count(Op::StopEngine),
            1,
            "the engine this check started is stopped once"
        );
        // The person saw Adding throughout: one Adding publish, then one Failed.
        let lands: Vec<FinderSetup> = sim
            .fx
            .iter()
            .filter_map(|e| match e {
                Effect::Transition(t) if t.from != t.to => Some(t.to),
                _ => None,
            })
            .collect();
        assert_eq!(lands, vec![Adding, Failed]);
    }

    #[test]
    fn a_slow_attempt_delays_the_next_start() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            stable: Err(stabilization_timeout()),
            wait_secs: 10,
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(sim.attempt_starts(), vec![0, 10, 20, 45]);
        assert_eq!(sim.secs(), 55, "ends within about 55 s");
        assert_eq!(sim.s.reason, Some(FinderFailureReason::Timeout));
    }

    #[test]
    fn unknown_makes_two_attempts_and_not_retryable_reasons_make_one() {
        let mut unknown = Sim::new(Applications);
        unknown.trigger(Trigger::KeysArrived);
        unknown.run(&World {
            add: Err(FpError::new("SomeDomain", 1, "x")),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(unknown.attempt_starts(), vec![0, 5]);
        assert_eq!(
            (unknown.s.setup, unknown.s.reason),
            (Failed, Some(FinderFailureReason::Unknown))
        );

        let taken = FpError::new(COCOA_DOMAIN, 516, "exists");
        let mut folder = Sim::new(Applications);
        folder.trigger(Trigger::KeysArrived);
        folder.run(&World {
            add: Err(taken.clone()),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(folder.attempt_starts(), vec![0]);
        assert_eq!(
            (folder.s.setup, folder.s.reason),
            (Failed, Some(FinderFailureReason::FolderTaken))
        );
        assert!(
            folder
                .fx
                .contains(&Effect::PersistFailure(Some((FinderFailureReason::FolderTaken, taken))))
        );
    }

    #[test]
    fn nothing_retries_after_failed_without_a_trigger_and_try_again_restarts_the_budget() {
        let world = World {
            add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")),
            ..World::ok(SIGNED_IN, NotRegistered)
        };
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Failed);
        sim.feed(Input::Tick { window_visible: true });
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Idle, "bringing the app forward does not retry");
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.s.attempt, 1, "a fresh budget");
        sim.run(&world);
        assert_eq!(sim.count(Op::AddDomain), 2);
    }
