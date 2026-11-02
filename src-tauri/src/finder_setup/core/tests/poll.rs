//! The user-disabled and ready-domain polls, the launch-location reasons, and a persisted
//! failure cleared by Ready.
use super::*;

    #[test]
    fn user_disabled_poll_reads_only_while_visible_and_a_flip_runs_one_check() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        assert_eq!(sim.s.setup, UserDisabled);
        let three = Duration::from_secs(3);
        assert_eq!(sim.next(), Next::WakeAt(sim.t0 + three));
        sim.now = sim.t0 + three;
        sim.feed(Input::Tick { window_visible: false });
        assert_eq!(
            sim.next(),
            Next::WakeAt(sim.t0 + three * 2),
            "no read while no window is visible"
        );
        sim.now = sim.t0 + three * 2;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(sim.s.setup, UserDisabled);
        sim.feed(Input::AppActivated);
        assert_eq!(
            sim.next(),
            Next::Run(Op::ReadDomain),
            "once when the app becomes active"
        );
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert!(sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)));
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.count(Op::Observe), 2, "the flip runs exactly one check");
        assert_eq!(
            sim.count(Op::AddDomain),
            0,
            "the poll never adds; a registered, enabled domain is only confirmed (§5.5 step 3)"
        );
    }

    /// Turned off in System Settings while Beebeeb runs: while `Ready` the same read-only poll runs on the same bound
    /// as the UserDisabled one (§7: every 3 s while a window is visible, plus once when the app becomes active), and a
    /// read that says "turned off" lands `UserDisabled` by itself: no trigger, no check, no add, no engine stop.
    #[test]
    fn a_ready_domain_turned_off_while_running_lands_user_disabled_from_the_poll_alone() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        let ops_before = sim.kinds();
        let three = Duration::from_secs(3);
        let ready_at = sim.now;
        assert_eq!(sim.next(), Next::WakeAt(ready_at + three), "Ready keeps a poll timer");
        // A window is on screen. With none the timer stops (the test
        // `a_ready_core_with_no_window_visible_sets_no_timer_and_a_focus_resumes_the_poll`).
        sim.now = ready_at + three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert_eq!(sim.s.setup, Ready, "still on: nothing changes");
        assert_eq!(sim.next(), Next::WakeAt(ready_at + three * 2), "and reads again in 3 s");
        // The person turns Beebeeb off in System Settings; the visible window's timer reads it.
        sim.now = ready_at + three * 2;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.fx.clear();
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(
            (sim.s.setup, sim.s.reason, sim.s.last_error.clone()),
            (UserDisabled, Some(FinderFailureReason::UserDisabled), None)
        );
        assert!(
            sim.fx.iter().any(|e| matches!(
                e,
                Effect::Transition(t) if t.from == Ready && t.to == UserDisabled
                    && t.reason == Some(FinderFailureReason::UserDisabled)
            )),
            "one lifecycle line: {:?}",
            sim.fx
        );
        assert!(sim.fx.contains(&Effect::Publish), "surfaces refresh");
        assert!(
            !sim.fx.iter().any(|e| matches!(e, Effect::TriggerReceived(_))),
            "the poll alone noticed it: {:?}",
            sim.fx
        );
        assert_eq!(sim.kinds(), ops_before, "no check, no add, no engine stop");
        assert!(sim.s.is_settled());
        // From here it is the UserDisabled poll: turning it back on runs exactly one check, which only confirms.
        assert_eq!(sim.next(), Next::WakeAt(sim.now + three));
        sim.now += three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert!(sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)));
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.count(Op::Observe), 2, "the launch check, then the flip's one check");
        assert_eq!(sim.count(Op::AddDomain), 0, "neither the flip off nor the flip on adds");
        assert_eq!(sim.count(Op::StopEngine), 0);
    }

    /// The same, noticed when the app becomes active (§7's "plus once when the app becomes active").
    #[test]
    fn a_ready_domain_turned_off_while_running_is_read_once_when_the_app_becomes_active() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        sim.feed(Input::AppActivated);
        assert_eq!(
            sim.next(),
            Next::Run(Op::ReadDomain),
            "once when the app becomes active"
        );
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.count(Op::AddDomain), 0);
        assert_eq!(sim.count(Op::Observe), 1, "no check ran");
    }

    /// F7 follow-up (review minor 2): `Ready` is the usual resting state, so its poll must not wake the app every 3 s
    /// while no Beebeeb window is on screen. A tick that finds none sets no new timer; a window gaining focus (the app
    /// becoming active) reads once and arms the timer again, and the next tick reads. The bound F7 needs is kept: a
    /// window on screen is read every 3 s, and an activation reads once.
    #[test]
    fn a_ready_core_with_no_window_visible_sets_no_timer_and_a_focus_resumes_the_poll() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        let three = Duration::from_secs(3);
        let ready_at = sim.now;
        assert_eq!(
            sim.next(),
            Next::WakeAt(ready_at + three),
            "Ready lands with the timer set: a window may be on screen"
        );
        sim.now = ready_at + three;
        sim.feed(Input::Tick { window_visible: false });
        assert_eq!(sim.next(), Next::Idle, "no window on screen: no timer, so no wake-up");
        sim.now += Duration::from_secs(3600);
        assert_eq!(sim.next(), Next::Idle, "an hour later, still none");
        assert!(sim.s.is_settled());

        // A window gains focus: one read at once, and the timer is set again.
        sim.feed(Input::AppActivated);
        assert_eq!(
            sim.next(),
            Next::Run(Op::ReadDomain),
            "once when the app becomes active"
        );
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        let activated_at = sim.now;
        assert_eq!(
            sim.next(),
            Next::WakeAt(activated_at + three),
            "the focus set the timer again"
        );
        // The next tick, with the window on screen, reads, and F7 still holds: turned off is noticed.
        sim.now = activated_at + three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain), "the next tick reads");
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(
            (sim.s.setup, sim.s.reason),
            (UserDisabled, Some(FinderFailureReason::UserDisabled))
        );
        assert_eq!(
            sim.next(),
            Next::WakeAt(sim.now + three),
            "no check runs: what comes next is the UserDisabled poll's timer"
        );
    }

    /// While `Ready` the poll acts only on "turned off": a read that fails, or finds the domain not registered, is no
    /// evidence of anything and changes nothing (a missing domain is the next launch's to re-add, §5.5).
    #[test]
    fn the_ready_poll_ignores_every_answer_but_turned_off() {
        for read in [
            Ok(Enabled),
            Ok(NotRegistered),
            Err(FpError::app(app_code::OP_TIMEOUT, "ReadDomain did not answer")),
        ] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::Launch);
            sim.run(&World::ok(SIGNED_IN, Enabled));
            sim.fx.clear();
            sim.feed(Input::AppActivated);
            assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
            sim.feed(Input::Done(OpResult::Domain(read.clone())));
            assert_eq!((sim.s.setup, sim.s.reason), (Ready, None), "{read:?}");
            assert!(sim.fx.is_empty(), "{read:?}: {:?}", sim.fx);
            assert!(sim.s.is_settled(), "{read:?}");
        }
    }

    /// A Lock leaves `Ready` in place (it never changes what macOS says) and holds: no read, no timer, until keys
    /// arrive again.
    #[test]
    fn a_held_ready_core_does_not_poll() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        sim.feed(Input::AppActivated); // a read is pending
        sim.trigger(Trigger::Lock);
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(
            sim.next(),
            Next::Idle,
            "the pending read is dropped and no timer is left behind"
        );
        sim.now += Duration::from_secs(30);
        sim.feed(Input::Tick { window_visible: true });
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Idle);
    }

    #[test]
    fn a_disk_image_launch_reports_the_reason_before_any_check_and_never_adds() {
        let mut sim = Sim::new(DiskImage);
        assert_eq!(
            (sim.s.setup, sim.s.reason),
            (Missing, Some(FinderFailureReason::NotInApplications))
        );
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(sim.kinds(), vec![Op::Observe]);
        assert_eq!(
            (sim.s.setup, sim.s.reason),
            (Failed, Some(FinderFailureReason::NotInApplications))
        );
    }

    #[test]
    fn a_registered_domain_is_confirmed_even_from_a_disk_image() {
        // §5.5: the launch-location check applies only when the domain is not registered.
        let mut sim = Sim::new(DiskImage);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn ready_clears_a_persisted_failure() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        sim.fx.clear();
        sim.trigger(Trigger::TryAgain);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            (sim.s.setup, sim.s.reason, sim.s.last_error.clone()),
            (Ready, None, None)
        );
        assert!(sim.fx.contains(&Effect::PersistFailure(None)));
    }

    /// Fix round 1 (review): while `held` the poll must not read, must not log a flip, and must
    /// not leave a wake-up scheduled (a past `WakeAt` would spin the driver).
    #[test]
    fn a_held_user_disabled_core_does_not_poll_and_keys_arriving_resumes_it() {
        let disabled = World::ok(SIGNED_IN, Disabled);
        let three = Duration::from_secs(3);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&disabled);
        assert_eq!(sim.s.setup, UserDisabled);
        // The app was brought forward just before the lock: a read is already pending.
        sim.feed(Input::AppActivated);
        sim.trigger(Trigger::Lock);
        assert_eq!(
            sim.s.setup, UserDisabled,
            "lock never removes and never changes what macOS says"
        );
        assert_eq!(
            sim.next(),
            Next::Idle,
            "a pending read is dropped and no timer is left behind"
        );
        // Held: ticks with a visible window and the app coming forward read nothing.
        sim.now = sim.t0 + three;
        sim.feed(Input::Tick { window_visible: true });
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Idle);
        sim.now = sim.t0 + three * 5;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Idle);
        assert_eq!(sim.count(Op::ReadDomain), 0);
        assert!(
            !sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)),
            "{:?}",
            sim.fx
        );
        // Keys arriving lift the hold; the check lands UserDisabled again and the poll resumes.
        sim.trigger(Trigger::KeysArrived);
        sim.run(&disabled);
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.next(), Next::WakeAt(sim.now + three));
        sim.now += three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert!(sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)));
    }

    #[test]
    fn sign_out_drops_a_pending_user_disabled_read() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        sim.feed(Input::AppActivated); // a read is pending
        sim.ops.clear();
        sim.trigger(Trigger::SignOut);
        sim.run(&World::ok(SIGNED_OUT_BY_CHOICE, Disabled));
        assert_eq!(
            sim.kinds(),
            vec![Op::RemoveDomain],
            "no read of a domain that sign-out is removing"
        );
        assert_eq!(sim.s.setup, Missing);
    }

    /// Backoff is where a failing check spends most of its time (§7: up to 45 s of it).
    #[test]
    fn lock_during_backoff_cancels_the_retry_and_holds_until_keys_arrive() {
        let failing = World {
            add: Err(fp(-2001)),
            ..World::ok(SIGNED_IN, NotRegistered)
        };
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&failing); // Observe
        sim.step_op(&failing); // StartEngine
        sim.step_op(&failing); // AddDomain: -2001, a transient failure
        assert_eq!(sim.s.setup, Adding);
        assert_eq!(
            sim.next(),
            Next::WakeAt(sim.t0 + Duration::from_secs(5)),
            "the retry is waiting out its backoff"
        );
        sim.trigger(Trigger::Lock);
        assert_eq!(sim.s.setup, Missing);
        assert_eq!(sim.s.reason, None);
        assert_eq!(sim.next(), Next::Idle, "the backoff timer is gone");
        // The old deadline passes, and a TryAgain arrives: still nothing.
        sim.now = sim.t0 + Duration::from_secs(5);
        sim.feed(Input::Tick { window_visible: false });
        assert_eq!(sim.next(), Next::Idle, "no retry after the deadline");
        sim.now = sim.t0 + Duration::from_secs(60);
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.next(), Next::Idle, "TryAgain does not lift a lock");
        // Keys arriving run exactly one fresh check.
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.count(Op::Observe),
            2,
            "the cancelled check's first attempt, then exactly one more check"
        );
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn sign_in_after_a_sign_out_by_choice_clears_the_persisted_intent() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(
            SessionFacts {
                signed_out_by_choice: true,
                ..SIGNED_IN
            },
            NotRegistered,
        ));
        assert!(sim.fx.contains(&Effect::PersistSignedOutByChoice(false)));
    }
