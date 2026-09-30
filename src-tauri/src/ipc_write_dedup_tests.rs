//! Unit tests for the write-queue idempotency table (task 1684). They drive the
//! table with a counting closure and an injected clock, so "enqueued once" is an
//! exact count and TTL expiry needs no sleeping. The socket-level tests, which
//! count real queued operations, live in `ipc_socket_framing_tests.rs`.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

type Dedup = Arc<WriteDedup<String>>;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_time()
        .build()
        .unwrap()
}

#[derive(Clone)]
struct Clock(Arc<Mutex<Instant>>);

impl Clock {
    fn new() -> Self {
        Clock(Arc::new(Mutex::new(Instant::now())))
    }
    fn advance(&self, by: Duration) {
        let mut t = self.0.lock().unwrap();
        *t += by;
    }
    fn reader(&self) -> impl Fn() -> Instant + Send + Sync + 'static {
        let inner = Arc::clone(&self.0);
        move || *inner.lock().unwrap()
    }
}

/// Run one request whose work increments `runs` and returns `value`.
async fn call(
    dedup: &Dedup,
    clock: &Clock,
    key: &str,
    fingerprint: &str,
    runs: &Arc<AtomicUsize>,
    value: &str,
) -> String {
    let runs = Arc::clone(runs);
    let value = value.to_string();
    dedup
        .run(
            key,
            fingerprint,
            clock.reader(),
            |_| true,
            |v: &String| !v.starts_with("error"),
            move || {
                runs.fetch_add(1, Ordering::SeqCst);
                value
            },
            "panic".to_string(),
        )
        .await
}

#[test]
fn two_concurrent_requests_with_one_key_run_the_work_exactly_once() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::new();
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));

    let (first, second) = rt.block_on(async {
        // The leader parks inside the work until released, so the second request
        // is guaranteed to arrive while the first is in flight.
        let leader = {
            let dedup = dedup.clone();
            let clock = clock.clone();
            let runs = Arc::clone(&runs);
            let release_rx = Arc::clone(&release_rx);
            tokio::spawn(async move {
                dedup
                    .run(
                        "k",
                        "create|a.txt",
                        clock.reader(),
                        |_| true,
                        |_: &String| true,
                        move || {
                            runs.fetch_add(1, Ordering::SeqCst);
                            release_rx.lock().unwrap().recv().unwrap();
                            "queued-by-leader".to_string()
                        },
                        "panic".to_string(),
                    )
                    .await
            })
        };
        // Wait until the leader is inside its work.
        let deadline = Instant::now() + Duration::from_secs(5);
        while runs.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "leader never started");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let follower = {
            let dedup = dedup.clone();
            let clock = clock.clone();
            let runs = Arc::clone(&runs);
            tokio::spawn(async move { call(&dedup, &clock, "k", "create|a.txt", &runs, "queued-by-follower").await })
        };
        // The follower must be parked, not finished and not running its own work.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !follower.is_finished(),
            "an in-flight repeat must wait for the first result"
        );
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "an in-flight repeat must not start a second run"
        );
        release_tx.send(()).unwrap();
        (leader.await.unwrap(), follower.await.unwrap())
    });

    assert_eq!(runs.load(Ordering::SeqCst), 1, "exactly one run for one key");
    assert_eq!(first, "queued-by-leader");
    assert_eq!(second, "queued-by-leader", "the repeat must get the FIRST result");
}

#[test]
fn a_repeat_after_completion_returns_the_stored_result_without_running_again() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::new();
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    rt.block_on(async {
        let first = call(&dedup, &clock, "k", "fp", &runs, "result-1").await;
        let again = call(&dedup, &clock, "k", "fp", &runs, "result-2").await;
        assert_eq!(first, "result-1");
        assert_eq!(again, "result-1", "a repeat within the TTL returns the stored result");
    });
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn different_keys_each_run_their_own_work() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::new();
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    rt.block_on(async {
        let a = call(&dedup, &clock, "key-a", "fp", &runs, "a").await;
        let b = call(&dedup, &clock, "key-b", "fp", &runs, "b").await;
        assert_eq!((a.as_str(), b.as_str()), ("a", "b"));
    });
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn a_stored_result_expires_exactly_at_the_ttl() {
    let rt = rt();
    let ttl = Duration::from_secs(60);
    let dedup: Dedup = WriteDedup::with_limits(ttl, 16);
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    rt.block_on(async {
        assert_eq!(call(&dedup, &clock, "k", "fp", &runs, "first").await, "first");
        clock.advance(ttl - Duration::from_secs(1));
        assert_eq!(
            call(&dedup, &clock, "k", "fp", &runs, "second").await,
            "first",
            "one second before the TTL the result is still served"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        clock.advance(Duration::from_secs(1));
        assert_eq!(
            call(&dedup, &clock, "k", "fp", &runs, "third").await,
            "third",
            "at the TTL the entry is gone and the work runs again"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    });
}

#[test]
fn a_key_reused_for_a_different_request_shape_is_not_merged() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::new();
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    rt.block_on(async {
        let a = call(&dedup, &clock, "k", "create|a.txt", &runs, "for-a").await;
        let b = call(&dedup, &clock, "k", "create|b.txt", &runs, "for-b").await;
        assert_eq!(a, "for-a");
        assert_eq!(
            b, "for-b",
            "a colliding key for another file must still do its own work"
        );
        // and it did not clobber the original entry
        let a_again = call(&dedup, &clock, "k", "create|a.txt", &runs, "unused").await;
        assert_eq!(a_again, "for-a");
    });
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn a_stored_result_that_fails_validation_is_dropped_and_the_work_runs_again() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::new();
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    rt.block_on(async {
        assert_eq!(call(&dedup, &clock, "k", "fp", &runs, "old").await, "old");
        let runs2 = Arc::clone(&runs);
        let redone = dedup
            .run(
                "k",
                "fp",
                clock.reader(),
                |cached: &String| cached != "old", // the world moved on: "old" is stale
                |_: &String| true,
                move || {
                    runs2.fetch_add(1, Ordering::SeqCst);
                    "new".to_string()
                },
                "panic".to_string(),
            )
            .await;
        assert_eq!(redone, "new");
        assert_eq!(call(&dedup, &clock, "k", "fp", &runs, "unused").await, "new");
    });
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn a_failed_result_is_shared_with_waiters_but_not_remembered() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::new();
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    rt.block_on(async {
        let failed = call(&dedup, &clock, "k", "fp", &runs, "error: copy failed").await;
        assert!(failed.starts_with("error"));
        // The retry after a failed attempt must actually try again.
        let retried = call(&dedup, &clock, "k", "fp", &runs, "queued").await;
        assert_eq!(retried, "queued");
        assert_eq!(dedup.len(), 1);
    });
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn a_waiter_takes_over_when_the_leader_panics() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::new();
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let (leader_out, waiter_out) = rt.block_on(async {
        let leader = {
            let dedup = dedup.clone();
            let clock = clock.clone();
            let runs = Arc::clone(&runs);
            let release_rx = Arc::clone(&release_rx);
            tokio::spawn(async move {
                dedup
                    .run(
                        "k",
                        "fp",
                        clock.reader(),
                        |_| true,
                        |_: &String| true,
                        move || -> String {
                            runs.fetch_add(1, Ordering::SeqCst);
                            release_rx.lock().unwrap().recv().unwrap();
                            panic!("copy blew up");
                        },
                        "leader-panicked".to_string(),
                    )
                    .await
            })
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while runs.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "leader never started");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let waiter = {
            let dedup = dedup.clone();
            let clock = clock.clone();
            let runs = Arc::clone(&runs);
            tokio::spawn(async move { call(&dedup, &clock, "k", "fp", &runs, "waiter-result").await })
        };
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!waiter.is_finished(), "the waiter must be parked behind the leader");
        release_tx.send(()).unwrap();
        (leader.await.unwrap(), waiter.await.unwrap())
    });
    assert_eq!(leader_out, "leader-panicked");
    assert_eq!(waiter_out, "waiter-result", "a waiter must not hang or inherit a panic");
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[test]
fn the_table_is_bounded_and_evicts_the_oldest_finished_entry() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::with_limits(Duration::from_secs(3600), 2);
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    rt.block_on(async {
        call(&dedup, &clock, "k1", "fp", &runs, "r1").await;
        clock.advance(Duration::from_secs(1));
        call(&dedup, &clock, "k2", "fp", &runs, "r2").await;
        clock.advance(Duration::from_secs(1));
        call(&dedup, &clock, "k3", "fp", &runs, "r3").await;
        assert_eq!(dedup.len(), 2, "the cap must hold");
        // k1 (oldest) was evicted: a repeat runs again. k3 is still remembered.
        assert_eq!(call(&dedup, &clock, "k3", "fp", &runs, "unused").await, "r3");
        assert_eq!(call(&dedup, &clock, "k1", "fp", &runs, "r1-again").await, "r1-again");
    });
    assert_eq!(runs.load(Ordering::SeqCst), 4);
}

#[test]
fn a_table_full_of_in_flight_work_fails_open_instead_of_refusing_the_write() {
    let rt = rt();
    let dedup: Dedup = WriteDedup::with_limits(Duration::from_secs(3600), 1);
    let clock = Clock::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    rt.block_on(async {
        let parked = {
            let dedup = dedup.clone();
            let clock = clock.clone();
            let runs = Arc::clone(&runs);
            let release_rx = Arc::clone(&release_rx);
            tokio::spawn(async move {
                dedup
                    .run(
                        "busy",
                        "fp",
                        clock.reader(),
                        |_| true,
                        |_: &String| true,
                        move || {
                            runs.fetch_add(1, Ordering::SeqCst);
                            release_rx.lock().unwrap().recv().unwrap();
                            "busy-done".to_string()
                        },
                        "panic".to_string(),
                    )
                    .await
            })
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while runs.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "leader never started");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // A different key finds the table full of in-flight work: it still runs.
        let other = call(&dedup, &clock, "other", "fp", &runs, "other-result").await;
        assert_eq!(other, "other-result");
        release_tx.send(()).unwrap();
        assert_eq!(parked.await.unwrap(), "busy-done");
    });
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}
