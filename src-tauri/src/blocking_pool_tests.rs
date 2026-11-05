    use super::on_blocking_pool;
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread")]
    async fn test_1882_r4_blocking_work_does_not_hold_the_runtime_thread() {
        let (release, released) = std::sync::mpsc::channel::<()>();
        let job = tokio::spawn(on_blocking_pool(move || {
            released.recv_timeout(Duration::from_secs(2)).is_ok()
        }));
        // Let the job start; the runtime thread must still be free to run this and send the release.
        tokio::task::yield_now().await;
        release.send(()).expect("the job is waiting");
        let saw_release = job.await.expect("job task").expect("pool");
        assert!(
            saw_release,
            "the blocking job ran off the runtime thread and saw the release"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_1882_r4_blocking_work_returns_its_value_and_runs_on_another_thread() {
        let here = std::thread::current().id();
        let there = on_blocking_pool(|| std::thread::current().id()).await.expect("pool");
        assert_ne!(
            here, there,
            "the work ran on the blocking pool, not the caller's thread"
        );
    }
