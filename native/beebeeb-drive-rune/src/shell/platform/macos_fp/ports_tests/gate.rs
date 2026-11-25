//! The one bridge gate and the ops that run over it, on a fake bridge and a fake host. The tag's source pins on
//! `macos_ports.rs` do not carry over (the gate is now `bridge::Gate`, a mutex taken on the blocking thread); the rules
//! do: a stuck call holds the gate until the OS answers, a busy gate fails fast except for `Observe` and
//! `RemoveDomain`, and a removal that outlives its limit still shows its folder, once.
use super::super::bridge::GATE_TESTS;
use super::super::*;
use beebeeb_engine::finder_setup::core::KeptFolder as EngineKept;
use beebeeb_engine::finder_setup::error::APP_DOMAIN;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Instant;

/// A runtime that does not wait for a bridge call still stuck in the "OS" when the test is done (dropping a runtime
/// waits for its blocking threads; a real app's runtime outlives every call).
fn run<T>(future: impl Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = runtime.block_on(future);
    runtime.shutdown_background();
    out
}

#[derive(Default)]
struct FakeHost {
    surfaced: Mutex<Vec<Option<String>>>,
    removal_cleared: AtomicUsize,
    engine_calls: AtomicUsize,
}

impl FpHost for FakeHost {
    fn vault_unlocked(&self) -> bool {
        true
    }
    fn auth_present(&self) -> bool {
        true
    }
    fn removal_owed(&self) -> bool {
        false
    }
    fn config_path(&self) -> Option<PathBuf> {
        None
    }
    async fn start_engine(&self, _limit: Duration) -> Result<bool, FpError> {
        self.engine_calls.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
    async fn finish_ready(&self) -> Result<(), FpError> {
        Ok(())
    }
    async fn stop_engine(&self, _limit: Duration) -> Result<(), FpError> {
        Ok(())
    }
    fn publish(&self, _view: &FinderSetupView) {}
    fn window_visible(&self) -> bool {
        false
    }
    fn surface_kept_folder(&self, location: Option<String>) {
        self.surfaced.lock().unwrap().push(location);
    }
    fn clear_removal_owed(&self) {
        self.removal_cleared.fetch_add(1, Ordering::SeqCst);
    }
    fn clear_domain_gone(&self) {}
}

/// A bridge whose `add_domain` and `remove` block until released, and that counts what ran.
struct FakeBridge {
    release: Mutex<Option<mpsc::Receiver<()>>>,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    ran: AtomicUsize,
    kept: String,
}

impl FakeBridge {
    fn new(kept: &str) -> Arc<Self> {
        Arc::new(Self {
            release: Mutex::new(None),
            entered: Mutex::new(None),
            ran: AtomicUsize::new(0),
            kept: kept.to_string(),
        })
    }
    /// The next blocking call waits for `release`, and says it is in the OS on the returned receiver.
    fn stick(&self) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (release_tx, release_rx) = mpsc::channel();
        let (entered_tx, entered_rx) = mpsc::channel();
        *self.release.lock().unwrap() = Some(release_rx);
        *self.entered.lock().unwrap() = Some(entered_tx);
        (release_tx, entered_rx)
    }
    fn hold(&self) {
        self.ran.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        let release = self.release.lock().unwrap().take();
        if let Some(release) = release {
            let _ = release.recv();
        }
    }
}

impl FpBridge for FakeBridge {
    fn add_domain(&self) -> Result<(), FpError> {
        self.hold();
        Ok(())
    }
    fn domain_state(&self) -> Result<DomainState, FpError> {
        self.ran.fetch_add(1, Ordering::SeqCst);
        Ok(DomainState::Enabled)
    }
    fn wait_for_domain_ready(&self, _timeout: Duration) -> Result<(), FpError> {
        self.ran.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn remove(&self) -> Result<DomainRemoval, RemovalFailure> {
        self.hold();
        Ok(DomainRemoval {
            kept: KeptFolder::Kept {
                path: self.kept.clone(),
                contents_checked: true,
                empty: false,
            },
        })
    }
}

fn ports(wait_ms: u64, bridge: &Arc<FakeBridge>, host: &Arc<FakeHost>) -> MacosPorts<FakeHost, FakeBridge> {
    MacosPorts::with_bridge(host.clone(), bridge.clone(), Duration::from_millis(wait_ms))
}

fn is_busy(error: &FpError) -> bool {
    (error.domain.as_str(), error.code) == (APP_DOMAIN, app_code::OP_TIMEOUT)
}

fn serial() -> std::sync::MutexGuard<'static, ()> {
    GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner())
}

#[test]
fn a_stuck_bridge_call_holds_the_gate_so_the_next_call_fails_fast_and_runs_nothing() {
    let _serial = serial();
    let (bridge, host) = (FakeBridge::new("/k"), Arc::new(FakeHost::default()));
    let (release, entered) = bridge.stick();
    let mut p = ports(200, &bridge, &host);
    let held = std::thread::spawn({
        let bridge = bridge.clone();
        move || {
            let _gate = Gate::try_enter().unwrap();
            bridge.hold();
        }
    });
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("the call is in the OS");
    let started = Instant::now();
    let answer = run(p.run(Op::ReadDomain));
    let OpResult::Domain(Err(error)) = answer else {
        panic!("{answer:?}")
    };
    assert!(is_busy(&error), "{error:?}");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "fails at once, never queues"
    );
    assert_eq!(bridge.ran.load(Ordering::SeqCst), 1, "only the stuck call ran");
    release.send(()).unwrap();
    held.join().unwrap();
    assert!(matches!(
        run(p.run(Op::ReadDomain)),
        OpResult::Domain(Ok(DomainState::Enabled))
    ));
}

#[test]
fn the_gate_is_released_when_the_stuck_call_returns_not_when_it_was_cut_off() {
    let _serial = serial();
    let (bridge, host) = (FakeBridge::new("/k"), Arc::new(FakeHost::default()));
    let (release, entered) = bridge.stick();
    let mut p = ports(50, &bridge, &host);
    // AddDomain's limit is 20 s; drive the cut-off with `within` at 30 ms over the same gated call.
    let cut = run(async {
        let b = bridge.clone();
        within(
            Op::AddDomain,
            Duration::from_millis(30),
            gated(None, move || b.add_domain()),
        )
        .await
    });
    entered.recv_timeout(Duration::from_secs(5)).expect("in the OS");
    let error = cut.expect_err("cut off");
    assert!(is_busy(&error), "{error:?}");
    let OpResult::Added(Err(next)) = run(p.run(Op::AddDomain)) else {
        panic!("refused while held")
    };
    assert!(is_busy(&next));
    assert_eq!(bridge.ran.load(Ordering::SeqCst), 1, "the refused call never ran");
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Gate::try_enter().is_err() {
        assert!(Instant::now() < deadline, "the gate frees once the OS call returns");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(run(p.run(Op::AddDomain)), OpResult::Added(Ok(()))));
}

#[test]
fn observe_waits_a_little_for_the_gate_and_gives_up_busy_after_the_wait() {
    let _serial = serial();
    let (bridge, host) = (FakeBridge::new("/k"), Arc::new(FakeHost::default()));
    let mut p = ports(1000, &bridge, &host);
    let (taken_tx, taken_rx) = mpsc::channel();
    let release = std::thread::spawn(move || {
        let holder = Gate::try_enter().unwrap();
        taken_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        drop(holder);
    });
    taken_rx.recv().unwrap();
    let started = Instant::now();
    let OpResult::Observed(seen) = run(p.run(Op::Observe)) else {
        panic!("observed")
    };
    assert_eq!(
        seen.domain,
        Ok(DomainState::Enabled),
        "the holder returned inside the wait"
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    release.join().unwrap();

    let mut short = ports(120, &bridge, &host);
    let _held = Gate::try_enter().unwrap();
    let started = Instant::now();
    let OpResult::Observed(seen) = run(short.run(Op::Observe)) else {
        panic!("observed")
    };
    assert!(is_busy(&seen.domain.expect_err("busy")));
    assert!(started.elapsed() >= Duration::from_millis(100) && started.elapsed() < Duration::from_secs(5));
    assert_eq!(bridge.ran.load(Ordering::SeqCst), 1, "the busy read ran nothing");
}

#[test]
fn only_observe_and_remove_wait_the_others_fail_fast() {
    let _serial = serial();
    let (bridge, host) = (FakeBridge::new("/k"), Arc::new(FakeHost::default()));
    let mut p = ports(2000, &bridge, &host);
    let _held = Gate::try_enter().unwrap();
    for op in [Op::AddDomain, Op::ReadDomain, Op::WaitStable(Duration::from_secs(1))] {
        let started = Instant::now();
        let busy = match run(p.run(op)) {
            OpResult::Added(r) | OpResult::Stable(r) => r.err(),
            OpResult::Domain(r) => r.err(),
            other => panic!("{other:?}"),
        };
        assert!(busy.as_ref().is_some_and(is_busy), "{op:?}: {busy:?}");
        assert!(started.elapsed() < Duration::from_millis(500), "{op:?} must not wait");
    }
    assert_eq!(bridge.ran.load(Ordering::SeqCst), 0);
}

#[test]
fn the_engine_ops_never_touch_the_bridge_gate() {
    let _serial = serial();
    let (bridge, host) = (FakeBridge::new("/k"), Arc::new(FakeHost::default()));
    let mut p = ports(50, &bridge, &host);
    let _held = Gate::try_enter().unwrap();
    assert!(matches!(run(p.run(Op::StartEngine)), OpResult::EngineStarted(Ok(true))));
    assert!(matches!(run(p.run(Op::FinishReady)), OpResult::Finished(Ok(()))));
    assert!(matches!(run(p.run(Op::StopEngine)), OpResult::EngineStopped(Ok(()))));
    assert_eq!(host.engine_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_removal_in_time_answers_the_reconciler_with_its_kept_folder_and_surfaces_nothing_itself() {
    let _serial = serial();
    let (bridge, host) = (FakeBridge::new("/kept/here"), Arc::new(FakeHost::default()));
    let mut p = ports(500, &bridge, &host);
    let answer = run(p.run(Op::RemoveDomain));
    assert_eq!(
        answer,
        OpResult::Removed(
            Ok(()),
            EngineKept::Kept {
                path: "/kept/here".into(),
                contents_checked: true,
                empty: false
            }
        )
    );
    assert!(
        host.surfaced.lock().unwrap().is_empty(),
        "the driver hands the folder on, not the late sink"
    );
}

#[test]
fn a_removal_that_finishes_after_the_limit_surfaces_its_folder_once_and_frees_the_gate_after() {
    let _serial = serial();
    let (bridge, host) = (FakeBridge::new("/kept/late"), Arc::new(FakeHost::default()));
    let (release, entered) = bridge.stick();
    let (answer_tx, answer_rx) = oneshot::channel();
    let late_host = host.clone();
    let b = bridge.clone();
    let cut = run(async {
        within(
            Op::RemoveDomain,
            Duration::from_millis(30),
            gated(
                Some(Duration::from_millis(500)),
                answering(
                    answer_tx,
                    move || b.remove(),
                    move |kept| surface_kept(&*late_host, kept, "test"),
                ),
            ),
        )
        .await
    });
    entered.recv_timeout(Duration::from_secs(5)).expect("in the OS");
    let error = cut.expect_err("cut off at 30 ms");
    assert!(is_busy(&error), "{error:?}");
    let answer = removal_answer(Err(error), answer_rx);
    let OpResult::Removed(Err(told), kept) = answer else {
        panic!("{answer:?}")
    };
    assert!(is_busy(&told));
    assert_eq!(kept, EngineKept::Nothing, "the late sink owns the folder now");
    assert!(
        host.surfaced.lock().unwrap().is_empty(),
        "nothing yet: the OS call has not returned"
    );
    assert!(Gate::try_enter().is_err(), "still held by the OS call");
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.surfaced.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the late sink ran");
        std::thread::sleep(Duration::from_millis(5));
    }
    while Gate::try_enter().is_err() {
        assert!(Instant::now() < deadline, "the gate frees after the late sink");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        *host.surfaced.lock().unwrap(),
        vec![Some("/kept/late".to_string())],
        "exactly once"
    );
}

#[test]
fn a_failed_removal_carries_no_os_text_and_keeps_the_folder() {
    let (answer_tx, answer_rx) = oneshot::channel();
    let _ = answer_tx.send(Err(RemovalFailure {
        message: "could not remove /Users/sam/Secret".into(),
        kept: KeptFolder::Unknown,
    }));
    let OpResult::Removed(Err(error), kept) = removal_answer(Ok(()), answer_rx) else {
        panic!("failed")
    };
    assert!(
        !error.message.contains("sam") && !error.message.contains("Secret"),
        "{error:?}"
    );
    assert_eq!(kept, EngineKept::Unknown);
}

#[test]
fn the_ports_persist_through_the_host_and_clear_the_owed_flags() {
    let (bridge, host) = (FakeBridge::new("/k"), Arc::new(FakeHost::default()));
    let mut p = ports(50, &bridge, &host);
    p.domain_removal_confirmed();
    assert_eq!(host.removal_cleared.load(Ordering::SeqCst), 1);
    p.kept_folder(EngineKept::Kept {
        path: "/k".into(),
        contents_checked: false,
        empty: false,
    });
    p.kept_folder(EngineKept::Unknown);
    assert_eq!(*host.surfaced.lock().unwrap(), vec![Some("/k".to_string()), None]);
}
