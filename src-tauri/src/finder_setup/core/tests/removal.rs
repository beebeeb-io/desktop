//! Sign-out and Repair: the check is cancelled, the removal runs exactly once, and the
//! hold lasts until keys arrive; an owed removal runs before the check starts.
use super::*;

    #[test]
    fn sign_out_cancels_the_check_in_flight_removes_and_holds_until_keys_arrive() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe
        sim.step_op(&world); // StartEngine
        sim.step_op(&world); // AddDomain
        sim.trigger(Trigger::SignOut);
        sim.run(&world);
        assert_eq!(
            sim.kinds(),
            vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::RemoveDomain]
        );
        assert_eq!(sim.s.setup, Missing);
        assert!(sim.fx.contains(&Effect::RemoveFinished(Ok(()))));
        assert!(sim.fx.contains(&Effect::PersistSignedOutByChoice(true)));
        // Held: nothing re-adds, whatever else arrives.
        sim.trigger(Trigger::TryAgain);
        sim.feed(Input::AppActivated);
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Idle);
        // Keys arriving again lift the hold.
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Ready);
    }

    /// Repair always runs its one fresh check, also after a removal that failed (§5.3 (6)). Its failed removal is not a
    /// failure to show on its own: it is recorded on the way as `Missing` with its error, never as `Failed`, and the
    /// fresh check decides what the person sees.
    #[test]
    fn a_repair_whose_removal_fails_still_checks_once_and_never_lands_failed_on_the_way() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        sim.ops.clear();
        sim.fx.clear();
        let world = World {
            remove: Err(fp(516)),
            ..World::ok(SIGNED_IN, Enabled)
        };
        sim.trigger(Trigger::Repair);
        assert_eq!(sim.step_op(&world), Op::RemoveDomain);
        assert_eq!(sim.s.setup, Missing, "the failed removal is recorded on the way");
        assert!(
            !sim.fx
                .iter()
                .any(|effect| matches!(effect, Effect::PersistFailure(Some(_)))),
            "and is not a failure of its own: {:?}",
            sim.fx
        );
        sim.run(&world);
        assert_eq!(sim.count(Op::RemoveDomain), 1, "one removal");
        assert_eq!(sim.count(Op::Observe), 1, "then its one fresh check");
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn repair_removes_then_checks_exactly_once() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        sim.ops.clear();
        sim.trigger(Trigger::Repair);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.kinds(),
            vec![
                Op::RemoveDomain,
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                WAIT_10,
                Op::FinishReady
            ]
        );
        assert_eq!(sim.s.setup, Ready);
        assert!(sim.fx.contains(&Effect::RemoveFinished(Ok(()))));
    }

    // ---- Task 10 fix round 1 (F3): a removal that is OWED before any engine starts ----
    /// A removal that completed is what releases a start that waits on it; one that failed is not. The core says
    /// so with an effect, and the driver turns it into the port call that clears the persisted flag.
    #[test]
    fn only_a_confirmed_removal_releases_the_owed_flag() {
        for (remove, confirmed) in [(Ok(()), true), (Err(fp(516)), false)] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::Repair);
            sim.run(&World {
                remove,
                ..World::ok(SIGNED_OUT_BY_401, NotRegistered)
            });
            assert_eq!(sim.fx.contains(&Effect::DomainRemovalConfirmed), confirmed);
        }
        // A sign-out removal is a removal too.
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::SignOut);
        sim.run(&World::ok(SIGNED_OUT_BY_CHOICE, NotRegistered));
        assert!(sim.fx.contains(&Effect::DomainRemovalConfirmed));
    }

    /// Fix round 3: a successful `addDomain` is the moment the domain is known registered again, so the core says
    /// so with an effect; a failed add says nothing (the domain may not be there).
    #[test]
    fn only_a_successful_add_says_the_domain_is_registered() {
        for (add, confirmed) in [(Ok(()), true), (Err(fp(-2011)), false)] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::KeysArrived);
            sim.run(&World {
                add,
                ..World::ok(SIGNED_IN, NotRegistered)
            });
            assert_eq!(
                sim.fx.contains(&Effect::DomainAddConfirmed),
                confirmed,
                "add result decides"
            );
        }
        // A check that finds the domain already registered never adds, so it confirms nothing.
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert!(!sim.fx.contains(&Effect::DomainAddConfirmed), "no add, no confirmation");
    }

    /// An Observe that finds the domain absent also releases it, and only while something is owed (so an ordinary
    /// check never touches the database).
    #[test]
    fn an_observe_that_finds_the_domain_absent_releases_the_owed_flag() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&World::ok(SIGNED_IN_OWING, NotRegistered));
        assert!(
            sim.fx.contains(&Effect::DomainRemovalConfirmed),
            "the domain is not there, so nothing is owed any more"
        );
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&World::ok(SIGNED_IN, NotRegistered));
        assert!(
            !sim.fx.contains(&Effect::DomainRemovalConfirmed),
            "nothing owed, nothing to release"
        );
    }

    /// With a removal owed and the domain still there, the check removes it BEFORE it goes on to start the engine, and
    /// only then runs its fresh check.
    #[test]
    fn an_owed_removal_runs_before_the_check_starts_the_engine() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        let owing = World::ok(SIGNED_IN_OWING, Enabled);
        assert_eq!(sim.step_op(&owing), Op::Observe);
        assert_eq!(
            sim.step_op(&owing),
            Op::RemoveDomain,
            "the removal comes before StartEngine"
        );
        assert!(sim.fx.contains(&Effect::DomainRemovalConfirmed));
        // The flag is now clear (the driver's port did that), and the domain is gone.
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.kinds()[..4],
            [Op::Observe, Op::RemoveDomain, Op::Observe, Op::StartEngine]
        );
        assert_eq!(sim.s.setup, Ready);
    }

    /// A removal that fails does not loop: no immediate re-check (which would remove again, and again). It lands, and
    /// the next trigger tries once more. It lands in `Failed`, never a resting `Missing`: the person is signed in and
    /// unlocked, the previous domain is still there and no engine may start, so the view says so and offers "Try
    /// again" (§13.1: `Missing` is never a resting state while wanted is present).
    #[test]
    fn an_owed_removal_that_fails_does_not_loop() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        let failing = World {
            remove: Err(fp(516)),
            ..World::ok(SIGNED_IN_OWING, Enabled)
        };
        sim.run(&failing);
        assert_eq!(
            sim.kinds(),
            vec![Op::Observe, Op::RemoveDomain],
            "one removal, then it settles"
        );
        assert!(
            !sim.fx.contains(&Effect::DomainRemovalConfirmed),
            "a failed removal releases nothing"
        );
        let reason = policy::classify(&fp(516));
        assert_eq!(
            (sim.s.setup, sim.s.reason, sim.s.last_error.clone()),
            (Failed, Some(reason), Some(fp(516))),
            "a signed-in Mac with the old domain still there is a failure with one action, not a resting Missing"
        );
        assert_eq!(sim.s.max_attempts, 1, "one attempt per trigger");
        assert!(
            sim.fx.contains(&Effect::PersistFailure(Some((reason, fp(516))))),
            "recorded like every other failure"
        );
        sim.trigger(Trigger::TryAgain);
        sim.run(&failing);
        assert_eq!(sim.count(Op::RemoveDomain), 2, "the next trigger removes once more");
        assert_eq!(sim.s.setup, Failed, "and still says so when it fails again");
    }

    #[test]
    fn lock_cancels_the_check_and_holds_until_keys_arrive() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe → Adding
        assert_eq!(sim.s.setup, Adding);
        sim.trigger(Trigger::Lock);
        assert_eq!(sim.s.setup, Missing);
        assert_eq!(sim.next(), Next::Idle, "no StartEngine after a lock");
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.next(), Next::Idle);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Ready);
    }
