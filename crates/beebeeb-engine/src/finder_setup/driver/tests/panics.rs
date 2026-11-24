//! A port that panics (report it and stop, never claim the stop finished) and the
//! tracing capture the panic tests need.
use super::*;

    /// Captures what `tracing` writes while a test holds `set_default`.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    #[tokio::test]
    async fn a_panicking_port_ends_in_one_failed_unknown_view_and_a_handle_that_says_so() {
        let captured = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_ansi(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        ports.panic_on = Some(Op::AddDomain);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        let join = tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Failed).await;
        join.await.expect("the panic was caught: the task ended normally");

        let view = handle.view();
        assert_eq!(
            (view.setup, view.reason, view.launch_location),
            (
                FinderSetup::Failed,
                Some(FinderFailureReason::Unknown),
                LaunchLocation::Applications
            )
        );
        let failure = view.last_failure.clone().expect("the person can copy what happened");
        assert_eq!(
            (failure.reason, failure.domain.as_str(), failure.code),
            (
                FinderFailureReason::Unknown,
                crate::finder_setup::error::APP_DOMAIN,
                crate::finder_setup::error::app_code::RECONCILER_PANIC
            )
        );
        {
            let record = record.lock().unwrap();
            assert_eq!(record.published.last(), Some(&view), "the event carries the final view");
            assert_eq!(
                record.persisted_failures.last(),
                Some(&Some(failure)),
                "it survives a relaunch like any failure"
            );
            let failures: Vec<_> = record
                .log
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        LifecycleEvent::Transition {
                            to: FinderSetup::Failed,
                            ..
                        }
                    )
                })
                .collect();
            assert_eq!(failures.len(), 1, "one lifecycle line: {:?}", record.log);
            match failures[0] {
                LifecycleEvent::Transition {
                    from,
                    reason,
                    error: Some(error),
                    ..
                } => {
                    assert_eq!(
                        (*from, *reason),
                        (FinderSetup::Adding, Some(FinderFailureReason::Unknown))
                    );
                    assert_eq!(
                        error.message, "the Finder reconciler panicked",
                        "a fixed sentence, never the panic payload"
                    );
                }
                other => panic!("{other:?}"),
            }
            assert_no_leak("the lifecycle log", &format!("{:?}", record.log));
        }
        let traced = captured.text();
        assert!(
            traced.contains("ERROR") && traced.contains("the Finder reconciler panicked"),
            "{traced}"
        );
        assert_no_leak("tracing", &traced);
        assert_no_leak("the final view", &serde_json::to_string(&view).unwrap());
        assert!(copy_details_text(&view, "0.8.12", "26.0", &[]).contains("Error: io.beebeeb.app 5"));

        // It does not come back, and nothing hangs or pretends.
        assert_eq!(handle.trigger(Trigger::TryAgain).unwrap_err(), NOT_RUNNING_SENTENCE);
        assert_eq!(
            handle
                .remove(Trigger::SignOut, Duration::from_secs(5))
                .await
                .unwrap_err()
                .message,
            NOT_RUNNING_SENTENCE
        );
        assert_eq!(
            handle.lock(Duration::from_secs(5)).await.unwrap_err(),
            NOT_RUNNING_SENTENCE
        );
        assert_eq!(handle.view(), view, "the view stays the final one");

        // A port that is wrecked by the panic (publish, log and persist all panic from now on)
        // must not stop the final view from reaching the handle, nor take the supervisor down.
        // It is the second half of THIS test on purpose: tracing's callsite interest is process
        // wide, so a second test hitting the supervisor's callsites on another thread while a
        // subscriber is captured here made the capture miss the line (3 of 4 runs).
        let (mut ports, _record) = FakePorts::new(&clock, DomainState::NotRegistered);
        ports.panic_on = Some(Op::AddDomain);
        ports.wrecked = true;
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        let join = tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        settle(&handle, |v| v.setup == FinderSetup::Failed).await;
        join.await
            .expect("publish, log and persist all panicked after the first panic, and the supervisor survived");
        assert_eq!(handle.view().reason, Some(FinderFailureReason::Unknown));
        assert!(handle.remove(Trigger::SignOut, Duration::from_secs(5)).await.is_err());
    }

    // ---- Task 8 fix round 1: a stop that did not finish is reported, never claimed ----

    #[tokio::test]
    async fn a_stop_that_did_not_finish_is_logged_and_the_check_still_ends_with_its_own_reason() {
        let stop_error = FpError::app(
            app_code::ENGINE_STOP_UNCONFIRMED,
            "the sync engine did not confirm it stopped",
        );
        let clock = FakeClock::new();
        let (mut ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        // The folder is taken: not retryable, so the check ends after one attempt, and it started
        // an engine (`StartEngine` answers `Ok(true)`), so the end of the check stops it.
        ports.add = Err(FpError::new(COCOA_DOMAIN, 516, PATH_MESSAGE));
        ports.stop = Err(stop_error.clone());
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
        let view = handle.view();
        assert_eq!(
            view.reason,
            Some(FinderFailureReason::FolderTaken),
            "a person is shown the check's reason, not the stop's"
        );
        assert_eq!(
            view.last_failure.as_ref().map(|f| (f.reason, f.code)),
            Some((FinderFailureReason::FolderTaken, 516))
        );
        assert_eq!(ops(&record).last(), Some(&Op::StopEngine));
        let record = record.lock().unwrap();
        let logged = record
            .log
            .iter()
            .filter(|e| matches!(e, LifecycleEvent::Transition { error: Some(error), .. } if *error == stop_error))
            .count();
        assert_eq!(logged, 1, "the stop failure is in the lifecycle log: {:?}", record.log);
        assert_no_leak("the failed view", &serde_json::to_string(&view).unwrap());
    }
