//! The check itself: what each (trigger, observation) pair wants, the operations it asks
//! for, and the state each result lands in.
use super::*;

    #[test]
    fn wanted_follows_section_5_1() {
        assert_eq!(wanted(SIGNED_IN), Wanted::Present);
        assert_eq!(
            wanted(SessionFacts {
                vault_unlocked: true,
                auth_present: false,
                signed_out_by_choice: true,
                removal_owed: false
            }),
            Wanted::Present
        );
        assert_eq!(wanted(KEYS_MISSING), Wanted::NoAction);
        assert_eq!(wanted(SIGNED_OUT_BY_CHOICE), Wanted::Absent);
        assert_eq!(
            wanted(SIGNED_OUT_BY_401),
            Wanted::NoAction,
            "R2: a startup 401 is not a sign-out by choice"
        );
    }

    #[test]
    fn present_and_not_registered_adds_then_lands_ready() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.kinds(),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                WAIT_10,
                Op::FinishReady
            ]
        );
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.s.reason, None);
        assert!(sim.fx.contains(&Effect::PersistFailure(None)));
    }

    #[test]
    fn present_and_registered_confirms_without_adding_and_ready_never_flickers() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.kinds(), vec![Op::Observe, Op::StartEngine, WAIT_2, Op::FinishReady]);
        assert_eq!(sim.s.setup, Ready);
        // A second launch-like trigger on a Ready domain never publishes Adding.
        sim.fx.clear();
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert!(
            !sim.fx
                .iter()
                .any(|e| matches!(e, Effect::Transition(t) if t.to == Adding)),
            "{:?}",
            sim.fx
        );
        assert_eq!(sim.count(Op::AddDomain), 0);
    }

    #[test]
    fn present_and_turned_off_lands_user_disabled_without_adding() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        assert_eq!(sim.kinds(), vec![Op::Observe]);
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.s.reason, Some(FinderFailureReason::UserDisabled));
    }

    #[test]
    fn turned_off_right_after_add_stops_the_engine_it_started() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            after_add: Ok(Disabled),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(
            sim.kinds(),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                Op::StopEngine
            ]
        );
        assert_eq!(sim.s.setup, UserDisabled);
    }

    /// Task 8 fix round 1: a stop that could not take the engine slot, or could not confirm the
    /// engine is gone, must not be reported as a stop. The core keeps "this check started an
    /// engine", still lands with the check's OWN reason (a person is never shown a stop failure as
    /// the cause), and writes the stop failure to the lifecycle log like any failed attempt.
    #[test]
    fn a_stop_that_did_not_finish_is_logged_never_claimed_and_the_check_still_lands_with_its_own_reason() {
        for stop_error in [
            FpError::app(
                app_code::ENGINE_STOP_UNCONFIRMED,
                "the sync engine did not confirm it stopped",
            ),
            FpError::app(app_code::OP_TIMEOUT, "StopEngine did not answer within 15s"),
        ] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::KeysArrived);
            sim.run(&World {
                after_add: Ok(Disabled),
                stop: Err(stop_error.clone()),
                ..World::ok(SIGNED_IN, NotRegistered)
            });
            assert_eq!(
                sim.kinds(),
                vec![
                    Op::Observe,
                    Op::StartEngine,
                    Op::AddDomain,
                    Op::ReadDomain,
                    Op::StopEngine
                ],
                "{stop_error:?}"
            );
            assert_eq!(
                (sim.s.setup, sim.s.reason),
                (UserDisabled, Some(FinderFailureReason::UserDisabled)),
                "the check's own reason"
            );
            assert!(
                sim.s.check_started_engine,
                "a stop that did not finish is not a stop: {stop_error:?}"
            );
            let logged: Vec<&Transition> = sim
                .fx
                .iter()
                .filter_map(|e| match e {
                    Effect::Transition(t) if t.error.as_ref() == Some(&stop_error) => Some(t),
                    _ => None,
                })
                .collect();
            assert_eq!(logged.len(), 1, "the stop failure is logged once: {:?}", sim.fx);
            assert_eq!(
                (logged[0].from, logged[0].to),
                (Adding, Adding),
                "log-only: it changes nothing a person sees"
            );
        }
        // The contrast: a stop that finished clears it, and logs nothing.
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            after_add: Ok(Disabled),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert!(!sim.s.check_started_engine);
        assert!(
            !sim.fx
                .iter()
                .any(|e| matches!(e, Effect::Transition(t) if t.from == t.to)),
            "{:?}",
            sim.fx
        );
    }

    #[test]
    fn domain_disabled_from_add_is_user_disabled_not_failed() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            add: Err(fp(-2011)),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.count(Op::AddDomain), 1, "-2011 is not retried");
    }

    #[test]
    fn a_failed_read_after_add_is_not_evidence_and_still_waits() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            after_add: Err(fp(-1000)),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(
            sim.kinds(),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                WAIT_10,
                Op::FinishReady
            ]
        );
        assert_eq!(sim.s.setup, Ready);
    }

    /// §13.1 "table tests for every trigger × every observation".
    #[test]
    fn every_trigger_against_every_observation_ends_in_a_state_a_person_can_act_on() {
        let observations: Vec<(&str, World, FinderSetup, usize)> = vec![
            // (name, world, final state, RemoveDomain count)
            (
                "signed in, not registered",
                World::ok(SIGNED_IN, NotRegistered),
                Ready,
                0,
            ),
            ("signed in, registered", World::ok(SIGNED_IN, Enabled), Ready, 0),
            ("signed in, turned off", World::ok(SIGNED_IN, Disabled), UserDisabled, 0),
            (
                "signed in, turned off right after add",
                World {
                    after_add: Ok(Disabled),
                    ..World::ok(SIGNED_IN, NotRegistered)
                },
                UserDisabled,
                0,
            ),
            (
                "signed in, folder taken",
                World {
                    add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")),
                    ..World::ok(SIGNED_IN, NotRegistered)
                },
                Failed,
                0,
            ),
            (
                "signed in, lookup keeps failing",
                World {
                    domain: Err(fp(-2001)),
                    ..World::ok(SIGNED_IN, Enabled)
                },
                Failed,
                0,
            ),
            (
                "signed in, never stable",
                World {
                    stable: Err(stabilization_timeout()),
                    ..World::ok(SIGNED_IN, NotRegistered)
                },
                Failed,
                0,
            ),
            (
                "signed in, the previous domain's owed removal fails",
                World {
                    remove: Err(fp(516)),
                    ..World::ok(SIGNED_IN_OWING, Enabled)
                },
                Failed,
                1,
            ),
            ("keys missing", World::ok(KEYS_MISSING, Enabled), Missing, 0),
            (
                "signed out by choice, still registered",
                World::ok(SIGNED_OUT_BY_CHOICE, Enabled),
                Missing,
                1,
            ),
            (
                "signed out by choice, gone",
                World::ok(SIGNED_OUT_BY_CHOICE, NotRegistered),
                Missing,
                0,
            ),
            (
                "revoked at startup (401), registered",
                World::ok(SIGNED_OUT_BY_401, Enabled),
                Missing,
                0,
            ),
        ];
        for trigger in [Trigger::Launch, Trigger::KeysArrived, Trigger::TryAgain] {
            for (name, world, expected, removes) in &observations {
                let mut sim = Sim::new(Applications);
                sim.trigger(trigger);
                sim.run(world);
                assert_eq!(sim.s.setup, *expected, "{trigger:?} / {name}");
                assert_eq!(
                    sim.count(Op::RemoveDomain),
                    *removes,
                    "{trigger:?} / {name}: removes only after a sign-out by choice or for an owed removal"
                );
                if wanted(world.facts) == Wanted::Present {
                    assert_ne!(
                        sim.s.setup, Missing,
                        "{trigger:?} / {name}: Missing is not a resting state while wanted is present"
                    );
                } else {
                    assert_eq!(
                        sim.count(Op::AddDomain),
                        0,
                        "{trigger:?} / {name}: never adds unless wanted"
                    );
                }
            }
        }
    }

    #[test]
    fn merged_triggers_produce_exactly_one_follow_up_check() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe
        for _ in 0..5 {
            sim.trigger(Trigger::TryAgain);
        }
        sim.trigger(Trigger::Launch);
        sim.run(&world);
        assert_eq!(
            sim.count(Op::Observe),
            2,
            "six triggers during one check → exactly one follow-up"
        );
        assert_eq!(sim.s.setup, Ready);
    }
