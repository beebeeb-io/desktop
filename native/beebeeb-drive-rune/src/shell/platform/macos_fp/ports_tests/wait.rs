//! Dm-87 (RC16: setup timed out three times in a row after 60 s each on a Mac at load 300, while the domain existed
//! and served reads): the wait for the domain on a fake bridge. Times are scaled down a hundred times (60 s is
//! 600 ms); no test makes an FFI call.
use super::super::bridge::GATE_TESTS;
use super::super::*;
use beebeeb_engine::finder_setup::driver::SystemClock;
use beebeeb_engine::finder_setup::launch_location::LaunchLocation;
use beebeeb_engine::finder_setup::policy::RetryPolicy;
use beebeeb_engine::surfaces::phase::FinderSetup;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn serial() -> std::sync::MutexGuard<'static, ()> {
    GATE_TESTS.lock().unwrap_or_else(|p| p.into_inner())
}

/// A host that is signed in, with an engine that starts.
struct Host;

impl FpHost for Host {
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
    fn surface_kept_folder(&self, _location: Option<String>) {}
    fn clear_removal_owed(&self) {}
    fn clear_domain_gone(&self) {}
}

/// An existing, enabled domain whose stabilization never answers in time (it waits out each slice) and whose
/// extension never connects. Its root gets a place on disk `available_after` from `born`; a nudge connects the
/// extension when `nudge_connects`.
struct Bridge {
    born: Instant,
    available_after: Duration,
    nudge_connects: bool,
    connected: AtomicBool,
    nudges: AtomicUsize,
    stabilizations: AtomicUsize,
}

impl Bridge {
    fn new(available_after: Duration) -> Arc<Self> {
        Arc::new(Self {
            born: Instant::now(),
            available_after,
            nudge_connects: false,
            connected: AtomicBool::new(false),
            nudges: AtomicUsize::new(0),
            stabilizations: AtomicUsize::new(0),
        })
    }
}

impl FpBridge for Bridge {
    fn add_domain(&self) -> Result<(), FpError> {
        Ok(())
    }
    fn domain_state(&self) -> Result<DomainState, FpError> {
        Ok(DomainState::Enabled)
    }
    fn wait_for_domain_ready(&self, timeout: Duration) -> Result<(), FpError> {
        use beebeeb_engine::finder_setup::error::{BRIDGE_DOMAIN, bridge_code};
        self.stabilizations.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(timeout);
        Err(FpError::new(
            BRIDGE_DOMAIN,
            bridge_code::STABILIZATION_TIMEOUT,
            "Timed out waiting for the Beebeeb File Provider domain to become available",
        ))
    }
    fn extension_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
    fn root_in_place(&self) -> bool {
        self.born.elapsed() >= self.available_after
    }
    fn nudge(&self) {
        self.nudges.fetch_add(1, Ordering::SeqCst);
        if self.nudge_connects {
            self.connected.store(true, Ordering::SeqCst);
        }
    }
    fn remove(&self) -> Result<DomainRemoval, RemovalFailure> {
        unreachable!("nothing is removed")
    }
}

fn run<T>(future: impl Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = runtime.block_on(future);
    runtime.shutdown_background();
    out
}

/// A domain that already serves reads is ready at the first look: no stabilization is waited for.
#[test]
fn a_domain_that_already_serves_reads_is_ready_at_once() {
    let _serial = serial();
    let bridge = Bridge::new(Duration::ZERO);
    let mut p = MacosPorts::with_bridge(Arc::new(Host), bridge.clone(), Duration::from_millis(50));
    let started = Instant::now();
    let answer = run(p.run(Op::WaitStable(Duration::from_secs(60))));
    assert!(matches!(answer, OpResult::Stable(Ok(()))), "{answer:?}");
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert_eq!(bridge.stabilizations.load(Ordering::SeqCst), 0);
    assert_eq!(bridge.nudges.load(Ordering::SeqCst), 0, "nothing to wake");
}

/// An extension that is slow to launch is woken once per wait, and its connection ends the wait.
#[test]
fn a_slow_extension_is_nudged_once_and_its_connection_ends_the_wait() {
    let _serial = serial();
    let bridge = Arc::new(Bridge {
        nudge_connects: true,
        ..Arc::into_inner(Bridge::new(Duration::from_secs(3600))).unwrap()
    });
    let mut p = MacosPorts::with_bridge(Arc::new(Host), bridge.clone(), Duration::from_millis(50));
    let answer = run(p.run(Op::WaitStable(Duration::from_millis(600))));
    assert!(matches!(answer, OpResult::Stable(Ok(()))), "{answer:?}");
    assert_eq!(bridge.nudges.load(Ordering::SeqCst), 1);
    assert_eq!(
        bridge.stabilizations.load(Ordering::SeqCst),
        1,
        "one slice, then the look saw the extension"
    );
}

/// RC16 scaled: the domain becomes available after 90 s, longer than one attempt's 60 s. The setup stays "adding"
/// through the first attempt's timeout, never "failed", and ends ready.
#[test]
fn a_domain_that_becomes_available_after_ninety_seconds_ends_ready_not_failed() {
    let _serial = serial();
    let bridge = Bridge::new(Duration::from_millis(900));
    let ports = MacosPorts::with_bridge(Arc::new(Host), bridge.clone(), Duration::from_millis(50));
    let policy = RetryPolicy {
        confirm_existing: Duration::from_millis(600),
        ..RetryPolicy::default()
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut seen = Vec::new();
    let ready = runtime.block_on(async {
        let handle =
            beebeeb_engine::finder_setup::driver::spawn(ports, SystemClock, LaunchLocation::Applications, None, policy);
        handle
            .trigger(beebeeb_engine::finder_setup::core::Trigger::Launch)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let setup = handle.view().setup;
            if seen.last() != Some(&setup) {
                seen.push(setup);
            }
            if setup == FinderSetup::Ready {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    });
    runtime.shutdown_background();
    assert!(ready, "never ready: {seen:?}");
    assert!(!seen.contains(&FinderSetup::Failed), "{seen:?}");
    assert!(
        bridge.born.elapsed() >= Duration::from_millis(900),
        "ready only once the domain was available"
    );
    assert!(
        bridge.stabilizations.load(Ordering::SeqCst) >= 1,
        "the first attempt waited its slices"
    );
}

/// The wait's line in the lifecycle log: fixed words and numbers, each step's time.
#[test]
fn the_wait_line_says_how_long_each_step_took() {
    let steps = WaitSteps {
        end: Some(WaitEnd::RootInPlace),
        total_ms: 2150,
        looks: 2,
        domain_ms: 40,
        enabled: true,
        extension: false,
        root_ms: 12,
        root_in_place: true,
        slices: 1,
        stabilize_ms: 2000,
        nudged: true,
    };
    assert_eq!(
        steps.line(),
        "wait end=ready_root_in_place total_ms=2150 looks=2 domain_ms=40 enabled=yes extension=no root_ms=12 \
         root_in_place=yes slices=1 stabilize_ms=2000 nudged=yes"
    );
}
