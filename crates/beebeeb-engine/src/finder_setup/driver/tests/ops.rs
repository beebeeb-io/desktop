//! Operation time limits: every `Op` has a named limit, and a call that never returns
//! becomes a redacted timeout instead of a hang.
use super::*;

    // ---- Task 8, lead ruling T7-1: every port operation has a time limit ----

    const EVERY_OP: [Op; 8] = [
        Op::Observe,
        Op::StartEngine,
        Op::AddDomain,
        Op::ReadDomain,
        Op::WaitStable(Duration::from_secs(10)),
        Op::FinishReady,
        Op::StopEngine,
        Op::RemoveDomain,
    ];

    #[test]
    fn every_operation_has_a_named_limit_and_stabilization_keeps_its_own_wait() {
        let s = Duration::from_secs;
        assert_eq!(op_limit(Op::Observe), s(10));
        assert_eq!(op_limit(Op::StartEngine), s(15));
        assert_eq!(op_limit(Op::AddDomain), s(20));
        assert_eq!(op_limit(Op::ReadDomain), s(10));
        assert_eq!(op_limit(Op::FinishReady), s(15));
        assert_eq!(op_limit(Op::StopEngine), s(15));
        assert_eq!(op_limit(Op::RemoveDomain), s(15));
        // The bridge waits for stabilization on its own clock (spec: at most 10 s); the driver's
        // limit is only the backstop for a bridge that never returns, so it adds a margin.
        assert_eq!(op_limit(Op::WaitStable(s(10))), s(13));
        assert_eq!(op_limit(Op::WaitStable(s(2))), s(5));
        assert!(
            RetryPolicy::default().stabilize_after_add <= s(10),
            "spec §7: stabilization stays at 10 s or less"
        );
        for op in EVERY_OP {
            assert!(op_limit(op) > Duration::ZERO && op_limit(op) <= s(30), "{op:?}");
        }
    }

    #[tokio::test]
    async fn a_call_that_never_returns_is_cut_off_at_its_limit_with_a_redacted_timeout_error() {
        // The test's own 5 s guard turns a `within` that does not cut off into a red assertion, not a hang.
        let guarded = |call: Duration, op: Op| async move {
            tokio::time::timeout(
                Duration::from_secs(5),
                within(op, call, std::future::pending::<Result<(), FpError>>()),
            )
            .await
            .expect("`within` did not cut the call off at its limit (the test's 5 s guard fired)")
        };
        let started = Instant::now();
        let error = guarded(Duration::from_millis(40), Op::AddDomain)
            .await
            .expect_err("a call that never returns is cut off");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cut off at its limit, not waited for: {:?}",
            started.elapsed()
        );
        assert_eq!(
            (error.domain.as_str(), error.code),
            (crate::finder_setup::error::APP_DOMAIN, app_code::OP_TIMEOUT)
        );
        assert_eq!(error.message, "AddDomain did not answer within 40ms");
        assert_eq!(
            crate::finder_setup::policy::classify(&error),
            FinderFailureReason::Timeout
        );
        assert_eq!(error.redacted(), "io.beebeeb.app 6");

        // Every operation is cut off the same way, whatever its limit.
        for op in EVERY_OP {
            let error = guarded(Duration::from_millis(5), op).await.expect_err("cut off");
            assert_eq!(error.code, app_code::OP_TIMEOUT, "{op:?}");
        }

        // A call that answers in time is passed through untouched, and so is its own error.
        assert_eq!(
            within(Op::Observe, Duration::from_secs(5), async { Ok::<_, FpError>(7) }).await,
            Ok(7)
        );
        let own = FpError::new(FILE_PROVIDER_DOMAIN, -2001, "not loaded");
        assert_eq!(
            within(Op::AddDomain, Duration::from_secs(5), async {
                Err::<(), _>(own.clone())
            })
            .await,
            Err(own)
        );
    }

    /// `FakePorts`, except that `AddDomain` is a call that never returns, bounded by `within` the
    /// way `MacosPorts` bounds each of its bridge calls.
    struct HangOnAdd {
        inner: FakePorts,
        limit: Duration,
    }

    impl Ports for HangOnAdd {
        fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send {
            let limit = self.limit;
            let inner = self.inner.run(op);
            async move {
                if op == Op::AddDomain {
                    OpResult::Added(within(op, limit, std::future::pending::<Result<(), FpError>>()).await)
                } else {
                    inner.await
                }
            }
        }
        fn publish(&mut self, view: &FinderSetupView) {
            self.inner.publish(view)
        }
        fn log(&mut self, event: LifecycleEvent) {
            self.inner.log(event)
        }
        fn persist_failure(&mut self, record: Option<FailureRecord>) {
            self.inner.persist_failure(record)
        }
        fn persist_signed_out_by_choice(&mut self, value: bool) {
            self.inner.persist_signed_out_by_choice(value)
        }
        fn domain_removal_confirmed(&mut self) {
            self.inner.domain_removal_confirmed()
        }
        fn domain_added(&mut self) {
            self.inner.domain_added()
        }
        fn kept_folder(&mut self, kept: KeptFolder) {
            self.inner.kept_folder(kept)
        }
        fn window_visible(&self) -> bool {
            self.inner.window_visible()
        }
    }

    #[tokio::test]
    async fn an_add_that_never_returns_ends_in_failed_timeout_after_the_schedule_not_in_a_hang() {
        let clock = FakeClock::new();
        let (inner, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let ports = HangOnAdd {
            inner,
            limit: Duration::from_millis(20),
        };
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::Applications,
            None,
            RetryPolicy::default(),
        );
        tokio::spawn(task);
        handle.trigger(Trigger::KeysArrived).unwrap();
        let waited = Instant::now();
        // Awaits the published view itself, not a number of scheduler turns: each attempt ends on a real
        // 20 ms timer, and a fast machine runs many turns before the first one fires.
        let mut views = handle.view.clone();
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            views.wait_for(|v| v.setup == FinderSetup::Failed),
        )
        .await
        .map(|seen| seen.is_ok());
        assert!(
            matches!(outcome, Ok(true)),
            "the reconciler hung on an add that never returned: {:?}",
            handle.view()
        );
        assert!(waited.elapsed() < Duration::from_secs(10));
        let view = handle.view();
        assert_eq!(
            (view.reason, view.attempt, view.max_attempts),
            (Some(FinderFailureReason::Timeout), 4, 4),
            "a timeout retries 4 times like the spec says"
        );
        let failure = view.last_failure.expect("recorded");
        assert_eq!(
            (failure.reason, failure.domain.as_str(), failure.code),
            (
                FinderFailureReason::Timeout,
                crate::finder_setup::error::APP_DOMAIN,
                app_code::OP_TIMEOUT
            )
        );
        let starts: Vec<u64> = record
            .lock()
            .unwrap()
            .ops
            .iter()
            .filter(|(_, op)| *op == Op::Observe)
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(
            starts,
            vec![0, 5, 15, 45],
            "the schedule counts from the check's start on the injected clock"
        );
        assert_no_leak("the failed view", &serde_json::to_string(&handle.view()).unwrap());
    }
