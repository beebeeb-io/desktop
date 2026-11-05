    use super::*;
    use crate::account::{AccountId, AccountRuntime};
    use crate::finder_setup::error::{APP_DOMAIN, app_code};
    use crate::runner::EngineRunner;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn account() -> Arc<AccountRuntime> {
        Arc::new(AccountRuntime::new(AccountId("finder-engine-test".to_string())))
    }

    /// An engine task that never looks at its cancel signal, as one stuck mid-call would: `abort()`
    /// waits out its 3 s graceful window, then forces the task down. Slower than any limit below.
    fn stubborn_engine(ticks: Arc<AtomicUsize>) -> EngineRunner {
        let task = tokio::spawn(async move {
            loop {
                ticks.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        EngineRunner::for_test_with_task(task)
    }

    /// The finding: the limit once wrapped the whole stop, so a stop that had already taken the
    /// engine out of its slot was dropped mid-`abort()`, and the engine task kept running with no
    /// handle and no slot. The limit now covers the wait for the slot only.
    #[tokio::test]
    async fn a_stop_that_has_taken_the_engine_runs_to_completion_past_the_limit_and_the_engine_does_not_survive() {
        let acct = account();
        let ticks = Arc::new(AtomicUsize::new(0));
        *acct.engine.lock().await = Some(stubborn_engine(ticks.clone()));
        let started = Instant::now();

        let stopped = stop_check_engine(&acct, Duration::from_millis(50)).await;

        assert_eq!(
            stopped,
            Ok(()),
            "a limit of 50 ms must not cut a stop short once it holds the engine"
        );
        assert!(
            started.elapsed() >= Duration::from_secs(3),
            "abort() waited out its graceful window: {:?}",
            started.elapsed()
        );
        assert!(acct.engine.lock().await.is_none(), "the engine left its slot");
        assert!(!acct.engine_stop_unconfirmed.load(Ordering::SeqCst));
        let after = ticks.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            ticks.load(Ordering::SeqCst),
            after,
            "the engine task is gone: nothing is still ticking"
        );
    }

    #[tokio::test]
    async fn a_stop_with_no_engine_in_the_slot_is_a_stop() {
        assert_eq!(stop_check_engine(&account(), Duration::from_millis(50)).await, Ok(()));
    }

    /// The one thing the limit does cut off: the wait for the slot. The engine is not touched, so it
    /// is still where sign-out and lock look for it.
    #[tokio::test]
    async fn a_stop_that_cannot_take_the_slot_reports_it_and_leaves_the_engine_tracked() {
        let acct = account();
        let ticks = Arc::new(AtomicUsize::new(0));
        *acct.engine.lock().await = Some(stubborn_engine(ticks.clone()));
        let held = acct.engine.lock().await; // somebody else is starting, locking or signing out
        let error = stop_check_engine(&acct, Duration::from_millis(50))
            .await
            .expect_err("the slot is held");
        assert_eq!((error.domain.as_str(), error.code), (APP_DOMAIN, app_code::OP_TIMEOUT));
        assert!(held.is_some(), "the engine was not touched");
        drop(held);
        assert!(
            !acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
            "nothing was lost, so the gate stays open"
        );
        // The engine is still tracked: dropping its runner (as sign-out's abort would) ends the task.
        drop(acct.engine.lock().await.take());
    }

    /// `abort()` that cannot confirm the task ended: it leaves an empty slot, so the Bug-A2 flag is the
    /// only thing that keeps sign-out refused. The reconciler sets it and says so.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stop_that_cannot_confirm_the_engine_is_gone_closes_the_sign_out_gate_and_says_so() {
        let acct = account();
        // No await point: tokio cannot force-abort it. A wall-clock deadline ends the thread.
        let deadline = Instant::now() + Duration::from_secs(8);
        let task = tokio::task::spawn_blocking(move || {
            while Instant::now() < deadline {
                std::hint::black_box(());
            }
        });
        *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));

        let error = stop_check_engine(&acct, Duration::from_secs(1))
            .await
            .expect_err("the stop cannot be confirmed");

        assert_eq!(
            (error.domain.as_str(), error.code),
            (APP_DOMAIN, app_code::ENGINE_STOP_UNCONFIRMED)
        );
        assert!(
            acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
            "sign-out must stay refused until a restart"
        );
        assert!(acct.engine.lock().await.is_none());
    }
