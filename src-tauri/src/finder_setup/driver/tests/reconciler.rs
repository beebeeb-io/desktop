//! The reconciler loop: a check's operations, the retry schedule, Lock and sign-out
//! removal, the user-disabled poll, and the removal waiters' answers.
use super::*;

    #[tokio::test]
    async fn a_launch_runs_one_check_and_publishes_ready() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Launch).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        assert_eq!(
            ops(&record),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                Op::WaitStable(Duration::from_secs(10)),
                Op::FinishReady
            ]
        );
        let record = record.lock().unwrap();
        assert_eq!(
            record.published.first().map(|v| v.setup),
            Some(FinderSetup::Missing),
            "the first view is published at start"
        );
        assert_eq!(record.published.last().map(|v| v.setup), Some(FinderSetup::Ready));
        assert_eq!(
            serde_json::to_value(record.published.last().unwrap()).unwrap(),
            serde_json::to_value(handle.view()).unwrap(),
            "the event carries the very view `finder_setup_state` returns"
        );
        assert!(record.log.contains(&LifecycleEvent::Trigger(Trigger::Launch)));
        let to_ready = record
            .log
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    LifecycleEvent::Transition {
                        to: FinderSetup::Ready,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(to_ready, 1, "one log line per transition");
    }

    #[tokio::test]
    async fn sign_out_resolves_its_ack_after_the_removal() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Launch).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        handle
            .remove(Trigger::SignOut, Duration::from_secs(5))
            .await
            .expect("removed");
        assert_eq!(
            handle.view().setup,
            FinderSetup::Missing,
            "the view is published before the ack"
        );
        assert_eq!(ops(&record).last(), Some(&Op::RemoveDomain));
        assert_eq!(record.lock().unwrap().signed_out, vec![true]);
    }

    #[tokio::test]
    async fn an_event_sent_during_an_operation_waits_for_it_and_wins_over_the_rest_of_the_check() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let slot: Arc<Mutex<Option<FinderSetupHandle>>> = Arc::new(Mutex::new(None));
        let hook_slot = slot.clone();
        ports.during_add = Some(Box::new(move || {
            let handle = hook_slot.lock().unwrap().clone().expect("handle installed");
            handle.trigger(Trigger::SignOut).unwrap();
        }));
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        *slot.lock().unwrap() = Some(handle.clone());
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        for _ in 0..10_000 {
            if ops(&record).contains(&Op::RemoveDomain) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            ops(&record),
            vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::RemoveDomain]
        );
        assert_eq!(handle.view().setup, FinderSetup::Missing);
    }

    #[tokio::test]
    async fn transient_failures_follow_the_schedule_on_the_injected_clock() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        ports.add = Err(FpError::new(FILE_PROVIDER_DOMAIN, -2001, "not loaded"));
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Failed).await;
        let starts: Vec<u64> = record
            .lock()
            .unwrap()
            .ops
            .iter()
            .filter(|(_, op)| *op == Op::Observe)
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(starts, vec![0, 5, 15, 45]);
        let view = handle.view();
        assert_eq!(
            (view.reason, view.attempt, view.max_attempts),
            (Some(FinderFailureReason::ExtensionLoading), 4, 4)
        );
        assert_eq!(
            view.last_failure.as_ref().map(|f| (f.domain.as_str(), f.code, f.at)),
            Some((FILE_PROVIDER_DOMAIN, -2001, 1_791_291_909 + 45))
        );
        assert_eq!(record.lock().unwrap().persisted_failures.len(), 1);
    }

    #[tokio::test]
    async fn a_persisted_failure_is_in_the_first_view_and_ready_clears_it() {
        let saved = FailureRecord {
            reason: FinderFailureReason::FolderTaken,
            domain: COCOA_DOMAIN.into(),
            code: 516,
            at: 1_791_291_909,
        };
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            Some(saved.clone()),
            RetryPolicy::default(),
        );
        assert_eq!(
            handle.view().last_failure,
            Some(saved),
            "Copy details works across a relaunch, before any check"
        );
        tokio::spawn(task);
        handle.trigger(Trigger::TryAgain).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        assert_eq!(handle.view().last_failure, None);
        assert_eq!(record.lock().unwrap().persisted_failures, vec![None]);
    }

    #[tokio::test]
    async fn lock_is_acknowledged_and_holds_until_keys_arrive() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Launch).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        assert!(!handle.is_held(), "not held while signed in and unlocked");
        handle.lock(Duration::from_secs(5)).await.expect("acknowledged");
        assert!(
            handle.is_held(),
            "the handle says it is held once the lock is acknowledged (area A M4: Try again can say why)"
        );
        let before = ops(&record).len();
        handle.trigger(Trigger::TryAgain).unwrap();
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        assert_eq!(ops(&record).len(), before, "held: Try again does nothing after a lock");
        handle.trigger(Trigger::KeysArrived).unwrap();
        for _ in 0..10_000 {
            if ops(&record).len() > before {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(ops(&record).len() > before, "keys arriving lift the hold");
        assert!(!handle.is_held(), "and the handle says so");
    }

    #[tokio::test]
    async fn one_removal_answers_every_waiter_queued_for_it_and_runs_once() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        // Both are queued before the task polls once, so only a drain that applies EVERY queued
        // event before choosing the next operation turns them into one removal.
        let (repair_ack, repair_done) = oneshot::channel();
        let (sign_out_ack, sign_out_done) = oneshot::channel();
        handle
            .tx
            .send(Event::Remove {
                trigger: Trigger::Repair,
                ack: repair_ack,
            })
            .unwrap();
        handle
            .tx
            .send(Event::Remove {
                trigger: Trigger::SignOut,
                ack: sign_out_ack,
            })
            .unwrap();
        tokio::spawn(task);
        assert_eq!(answered(repair_done).await.0, Ok(()), "the Repair waiter is answered");
        assert_eq!(
            answered(sign_out_done).await.0,
            Ok(()),
            "the SignOut waiter is answered by the same removal"
        );
        assert_eq!(
            ops(&record),
            vec![Op::RemoveDomain],
            "one removal, and nothing after it: sign-out holds"
        );
        assert_eq!(handle.view().setup, FinderSetup::Missing);
        assert_eq!(record.lock().unwrap().signed_out, vec![true]);
    }

    /// F3 (task 1834 fix round 1): the driver tells the port that a removal is confirmed exactly when one worked.
    /// That call is what clears the persisted "removal owed" flag the engine start waits on, so a removal that fails
    /// must never make it.
    #[tokio::test]
    async fn a_confirmed_removal_reaches_the_port_and_a_failed_one_does_not() {
        for (remove, expected) in [(Ok(()), 1), (Err(FpError::new(COCOA_DOMAIN, 516, "fixture")), 0)] {
            let clock = FakeClock::new();
            let (mut ports, record) = FakePorts::new(&clock, DomainState::Enabled);
            ports.remove = remove;
            let (handle, task) = start(
                ports,
                clock.clone(),
                LaunchLocation::Applications,
                None,
                RetryPolicy::default(),
            );
            let (ack, done) = oneshot::channel();
            handle
                .tx
                .send(Event::Remove {
                    trigger: Trigger::Repair,
                    ack,
                })
                .unwrap();
            tokio::spawn(task);
            let _ = answered(done).await;
            assert_eq!(record.lock().unwrap().removals_confirmed, expected);
        }
    }

    /// Task 1882 × spec A: a Finder location the reconciler removed while files had not reached the server keeps
    /// them, and the folder macOS kept them in reaches whoever waits for the removal (sign-out, Repair), with a
    /// removal that worked and with one that failed (review M2). The port is not asked to show it as well.
    #[tokio::test]
    async fn test_1882_a_reconciler_removal_answers_its_waiter_with_the_kept_folder() {
        for remove in [Ok(()), Err(FpError::new(COCOA_DOMAIN, 516, "fixture"))] {
            let clock = FakeClock::new();
            let (mut ports, record) = FakePorts::new(&clock, DomainState::Enabled);
            ports.remove = remove.clone();
            ports.kept = kept_folder();
            let (handle, task) = start(
                ports,
                clock.clone(),
                LaunchLocation::Applications,
                None,
                RetryPolicy::default(),
            );
            tokio::spawn(task);
            let answer = handle.remove(Trigger::SignOut, Duration::from_secs(5)).await;
            let kept = match answer {
                Ok(removal) => {
                    assert!(remove.is_ok(), "a removal that worked answers Ok");
                    removal.kept_location("test")
                }
                Err(failure) => {
                    assert_eq!(failure.message, "NSCocoaErrorDomain 516", "domain and code, as before");
                    failure.kept_location("test")
                }
            };
            assert_eq!(
                kept.as_deref(),
                Some(KEPT_FOLDER),
                "{remove:?}: the waiter gets the folder"
            );
            assert!(
                record.lock().unwrap().kept_folders.is_empty(),
                "{remove:?}: the waiter shows it, so the port does not"
            );
        }
    }

    /// Task 1882 × spec A: a removal nobody waits for (here the account binding's Repair, sent as a plain
    /// trigger) still hands its kept folder on: to the port, which shows it the way the app-start sweep does.
    #[tokio::test]
    async fn test_1882_a_reconciler_removal_nobody_waits_for_hands_the_kept_folder_to_the_port() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        ports.kept = kept_folder();
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Repair).unwrap();
        for _ in 0..10_000 {
            if !record.lock().unwrap().kept_folders.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(
            record.lock().unwrap().kept_folders,
            vec![kept_folder()],
            "the folder reaches the port once"
        );
        assert!(ops(&record).contains(&Op::RemoveDomain));
    }

    /// Task 1882 × spec A: a waiter that stopped waiting (its sign-out timed out) never takes the folder with it.
    #[tokio::test]
    async fn test_1882_a_kept_folder_whose_waiter_gave_up_goes_to_the_port() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        ports.kept = kept_folder();
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        let (ack, done) = oneshot::channel();
        handle
            .tx
            .send(Event::Remove {
                trigger: Trigger::SignOut,
                ack,
            })
            .unwrap();
        drop(done);
        tokio::spawn(task);
        for _ in 0..10_000 {
            if !record.lock().unwrap().kept_folders.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(record.lock().unwrap().kept_folders, vec![kept_folder()]);
    }
    /// Fix round 3: a successful add reaches the port exactly once, so the persisted "domain gone" mark is cleared by
    /// it; a failed add does not.
    #[tokio::test]
    async fn a_successful_add_reaches_the_port_and_a_failed_one_does_not() {
        for (add, expected) in [(Ok(()), 1), (Err(FpError::new(COCOA_DOMAIN, 516, "fixture")), 0)] {
            let clock = FakeClock::new();
            let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
            ports.add = add;
            let (handle, task) = start(
                ports,
                clock.clone(),
                LaunchLocation::Applications,
                None,
                RetryPolicy::default(),
            );
            tokio::spawn(task);
            handle.trigger(Trigger::KeysArrived).unwrap();
            settle(&handle, |v| {
                v.setup == FinderSetup::Failed || v.setup == FinderSetup::Ready
            })
            .await;
            assert_eq!(record.lock().unwrap().domains_added, expected);
        }
    }

    #[tokio::test]
    async fn the_os_message_reaches_the_log_and_nothing_a_person_or_a_file_sees() {
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        ports.add = Err(FpError::new(COCOA_DOMAIN, 516, PATH_MESSAGE).with_underlying(POSIX_DOMAIN, 17));
        ports.remove = Err(FpError::new(COCOA_DOMAIN, 516, PATH_MESSAGE));
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Failed).await;
        {
            let record = record.lock().unwrap();
            for view in &record.published {
                assert_no_leak("a published view", &serde_json::to_string(view).unwrap());
            }
            assert_no_leak("the persisted failure", &format!("{:?}", record.persisted_failures));
            // The contrast that makes the checks above mean something: the message IS in the log
            // event, where `lifecycle_log` redacts it before it is written.
            assert!(
                record.log.iter().any(|e| matches!(e, LifecycleEvent::Transition { error: Some(error), .. } if error.message == PATH_MESSAGE)),
                "{:?}",
                record.log
            );
        }
        assert_eq!(
            handle.view().last_failure.map(|f| (f.domain, f.code)),
            Some((COCOA_DOMAIN.to_string(), 516))
        );
        assert_no_leak(
            "Copy details",
            &copy_details_text(&handle.view(), "0.8.12", "26.0", &[]),
        );

        let error = handle
            .remove(Trigger::SignOut, Duration::from_secs(5))
            .await
            .expect_err("the removal failed");
        assert_no_leak("the removal error", &error.message);
        assert_eq!(error.message, "NSCocoaErrorDomain 516", "domain and code, nothing else");
    }

    #[tokio::test]
    async fn a_repeated_failure_republishes_the_time_of_the_last_attempt() {
        // A disk image fails the same way every time and never passes through `Adding`, so the
        // core publishes nothing the second time: only the record's `at` moved.
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::DiskImage,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Failed).await;
        let first = handle.view().last_failure.expect("recorded").at;
        clock.advance(60);
        handle.trigger(Trigger::TryAgain).unwrap();
        settle(&handle, |v| v.last_failure.as_ref().is_some_and(|f| f.at > first)).await;
        assert_eq!(handle.view().last_failure.map(|f| f.at), Some(first + 60));
        assert_eq!(
            record.lock().unwrap().published.last().unwrap().last_failure,
            handle.view().last_failure,
            "the event agrees with the state"
        );
    }

    #[tokio::test]
    async fn the_user_disabled_poll_reads_only_while_a_window_is_visible_or_on_activation() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Disabled);
        let (domain, visible) = (ports.domain.clone(), ports.visible.clone());
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Launch).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::UserDisabled).await;
        assert_eq!(handle.view().reason, Some(FinderFailureReason::UserDisabled));
        for _ in 0..500 {
            tokio::task::yield_now().await;
        }
        assert_eq!(ops(&record), vec![Op::Observe], "no window, no poll");

        // The app becoming active reads once; the person has turned it on.
        *domain.lock().unwrap() = DomainState::Enabled;
        handle.app_activated();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        assert_eq!(
            ops(&record),
            vec![
                Op::Observe,
                Op::ReadDomain,
                Op::Observe,
                Op::StartEngine,
                Op::WaitStable(Duration::from_secs(2)),
                Op::FinishReady
            ],
            "one read, then exactly one check, and never an add"
        );

        // With a window on screen the timer reads by itself.
        *domain.lock().unwrap() = DomainState::Disabled;
        handle.trigger(Trigger::TryAgain).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::UserDisabled).await;
        let reads_before = ops(&record).iter().filter(|op| **op == Op::ReadDomain).count();
        *domain.lock().unwrap() = DomainState::Enabled;
        visible.store(true, Ordering::SeqCst);
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        assert!(
            ops(&record).iter().filter(|op| **op == Op::ReadDomain).count() > reads_before,
            "the visible window polled"
        );
        assert!(!ops(&record).contains(&Op::AddDomain));
    }

    /// Turned off in System Settings while Beebeeb runs: the running driver notices it without a relaunch and without
    /// any trigger, on the UserDisabled poll's own bound (§7): once when the app becomes active, and by the timer while
    /// a window is visible. It never adds, and it does not stop the session's engine.
    #[tokio::test]
    async fn a_domain_turned_off_while_ready_is_noticed_by_the_poll_without_a_relaunch() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let (domain, read, visible) = (ports.domain.clone(), ports.read.clone(), ports.visible.clone());
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Launch).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        let turned_off = |off: bool| {
            let state = if off {
                DomainState::Disabled
            } else {
                DomainState::Enabled
            };
            *domain.lock().unwrap() = state;
            *read.lock().unwrap() = state;
        };

        // Off, with no window on screen and no activation: nothing reads.
        turned_off(true);
        for _ in 0..500 {
            tokio::task::yield_now().await;
        }
        assert_eq!(handle.view().setup, FinderSetup::Ready);
        assert!(!ops(&record).contains(&Op::ReadDomain), "no window, no poll");

        // The app becomes active: one read, and the state follows macOS.
        handle.app_activated();
        settle(&handle, |v| v.setup == FinderSetup::UserDisabled).await;
        assert_eq!(handle.view().reason, Some(FinderFailureReason::UserDisabled));
        assert_eq!(
            ops(&record),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::WaitStable(Duration::from_secs(2)),
                Op::FinishReady,
                Op::ReadDomain
            ],
            "one read and nothing else: no check, no add, no engine stop"
        );
        {
            let record = record.lock().unwrap();
            assert_eq!(
                record
                    .log
                    .iter()
                    .filter(|e| matches!(e, LifecycleEvent::Trigger(_)))
                    .collect::<Vec<_>>(),
                vec![&LifecycleEvent::Trigger(Trigger::Launch)],
                "no trigger noticed it, the poll did"
            );
            assert!(record.log.iter().any(|e| matches!(
                e,
                LifecycleEvent::Transition {
                    from: FinderSetup::Ready,
                    to: FinderSetup::UserDisabled,
                    ..
                }
            )));
            assert_eq!(
                record.published.last().map(|v| v.setup),
                Some(FinderSetup::UserDisabled),
                "surfaces are told"
            );
        }

        // Back on, as a window gains focus and stays on screen: the UserDisabled poll runs exactly one check, which
        // only confirms.
        turned_off(false);
        visible.store(true, Ordering::SeqCst);
        handle.app_activated();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;

        // Off again, with that window still on screen: the timer reads by itself, no activation needed.
        let reads_before = ops(&record).iter().filter(|op| **op == Op::ReadDomain).count();
        turned_off(true);
        settle(&handle, |v| v.setup == FinderSetup::UserDisabled).await;
        assert!(
            ops(&record).iter().filter(|op| **op == Op::ReadDomain).count() > reads_before,
            "the visible window's timer read it"
        );
        assert_eq!(
            ops(&record).iter().filter(|op| **op == Op::Observe).count(),
            2,
            "the launch check and the one check after the flip back on"
        );
        assert!(!ops(&record).contains(&Op::AddDomain), "no flip adds");
        assert!(!ops(&record).contains(&Op::StopEngine));
    }

    /// F7 follow-up (review minor 2): while `Ready` with no Beebeeb window on screen the reconciler stops waking up.
    /// Every wake-up asks `window_visible` (a main-thread query in the app), so the count of those questions is the
    /// count of wake-ups. A window gaining focus resumes the poll: one read at once, and the next tick reads.
    #[tokio::test]
    async fn a_ready_driver_with_no_window_visible_stops_waking_until_a_window_gains_focus() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let (read, visible, asked) = (ports.read.clone(), ports.visible.clone(), ports.visible_asked.clone());
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Launch).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;
        for _ in 0..500 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            asked.load(Ordering::SeqCst),
            1,
            "one wake-up 3 s after Ready found no window, and set no new timer"
        );
        let slept_at = clock.secs();
        for _ in 0..500 {
            tokio::task::yield_now().await;
        }
        assert_eq!(asked.load(Ordering::SeqCst), 1, "still no wake-up");
        assert_eq!(clock.secs(), slept_at, "no timer was waited on");
        assert!(!ops(&record).contains(&Op::ReadDomain), "and nothing was read");

        // A window gains focus and stays on screen: one read at once, then the timer is back and its next tick reads.
        visible.store(true, Ordering::SeqCst);
        handle.app_activated();
        for _ in 0..10_000 {
            if ops(&record).iter().filter(|op| **op == Op::ReadDomain).count() >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        let reads: Vec<u64> = record
            .lock()
            .unwrap()
            .ops
            .iter()
            .filter(|(_, op)| *op == Op::ReadDomain)
            .map(|(at, _)| *at)
            .take(2)
            .collect();
        assert_eq!(
            reads,
            vec![slept_at, slept_at + 3],
            "the activation's read at once, then the next tick's 3 s later"
        );
        assert!(asked.load(Ordering::SeqCst) > 1, "the timer woke again");

        // F7 is kept: turned off while that window is on screen is noticed by the timer alone.
        *read.lock().unwrap() = DomainState::Disabled;
        settle(&handle, |v| v.setup == FinderSetup::UserDisabled).await;
        assert_eq!(
            ops(&record).iter().filter(|op| **op == Op::Observe).count(),
            1,
            "no check ran"
        );
        assert!(!ops(&record).contains(&Op::AddDomain));
    }


    #[tokio::test]
    async fn a_handle_says_so_when_its_reconciler_is_gone_or_silent_and_refuses_what_is_not_a_removal() {
        let view = FinderSetupView::initial(LaunchLocation::Applications);
        let (handle, rx) = FinderSetupHandle::for_test(view.clone());
        drop(rx);
        assert_eq!(
            handle
                .remove(Trigger::SignOut, Duration::from_secs(1))
                .await
                .unwrap_err()
                .message,
            NOT_RUNNING_SENTENCE
        );
        assert_eq!(
            handle.lock(Duration::from_secs(1)).await.unwrap_err(),
            NOT_RUNNING_SENTENCE
        );

        let (handle, _rx) = FinderSetupHandle::for_test(view.clone());
        assert!(
            handle
                .remove(Trigger::Repair, Duration::from_millis(20))
                .await
                .unwrap_err()
                .message
                .contains("did not remove")
        );
        assert!(
            handle
                .lock(Duration::from_millis(20))
                .await
                .unwrap_err()
                .contains("did not acknowledge the lock")
        );

        // Only sign-out and Repair are removals: anything else would leave a waiter nobody answers.
        let (handle, mut rx) = FinderSetupHandle::for_test(view.clone());
        assert!(handle.remove(Trigger::Launch, Duration::from_secs(1)).await.is_err());
        assert!(rx.try_recv().is_err(), "nothing was sent");

        assert_eq!(FinderSetupHandle::fixed(view.clone()).view(), view);
    }

    // ---- Task 7 fix round 1: the waiter is answered after the publish; a panicking port is seen ----

    #[tokio::test]
    async fn a_removal_waiter_is_answered_only_after_the_view_is_published() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::Enabled);
        let probe = ports.ack_probe.clone();
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::Launch).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Ready).await;

        // The fake `publish` looks at the waiter's receiver without awaiting it, so this holds on
        // any runtime: a caller woken by the ack can never read the view from before the removal.
        let (ack, done) = oneshot::channel();
        *probe.lock().unwrap() = Some(done);
        handle
            .tx
            .send(Event::Remove {
                trigger: Trigger::SignOut,
                ack,
            })
            .unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Missing).await;
        assert_eq!(
            record.lock().unwrap().ack_pending_at_publish,
            vec![true],
            "the view is published before the waiter is answered"
        );
        let done = probe.lock().unwrap().take().expect("the receiver is still there");
        assert_eq!(answered(done).await.0, Ok(()));
    }
