//! The single Finder reconciler task (spec §5.4, §9 `finder_setup::driver`). It receives events,
//! runs the one operation the core asks for at a time, and owns the timer. Between two
//! operations it first applies every event that arrived, so a sign-out or a lock always wins
//! over the rest of a check. Generic over `Ports` (OS, engine, app) and `Clock`.
//!
//! Children: `state` (the published view), `handle` (the handle and the events it sends),
//! `ports` (the OS/engine/app seam and the clock), `limits` (one time limit per operation),
//! `check` (the loop, the retry schedule and the removal waiters), `details` (the "Copy details"
//! text). The tests under `tests/` are split by area; their fakes stay in `mod tests` below.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::FutureExt as _;
use tokio::sync::{mpsc, oneshot, watch};

use super::core::{self, CoreState, Effect, Input, Next, Op, OpResult, Trigger};
use super::error::{FailureRecord, FpError, app_code};
use super::launch_location::LaunchLocation;
use super::policy::RetryPolicy;
use crate::finder_removal::{DomainRemoval, KeptFolder, RemovalFailure};
use crate::lifecycle_log::{self, LifecycleEvent};
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

mod check;
mod details;
mod handle;
mod limits;
mod ports;
mod state;

// Feeds the test module's `use super::*` (the "Copy details" text builders).
#[cfg(test)]
use details::*;

// The re-exports below are the surface; nothing else in this file uses the children directly.
pub use details::copy_details;
pub use handle::{Event, FinderSetupHandle, RemoveAnswer};
pub use ports::{Clock, Ports};
pub use state::FinderSetupView;
// Started and asked about only by the macOS reconciler wiring (`lib.rs`).
#[cfg(target_os = "macos")]
pub use check::spawn;
#[cfg(target_os = "macos")]
pub use handle::NOT_RUNNING;
#[cfg(target_os = "macos")]
pub use state::FINDER_SETUP_CHANGED_EVENT;
// `op_limit`/`within` time the macOS ports and are read by the driver's tests.
#[cfg(any(test, target_os = "macos"))]
pub use limits::{op_limit, within};
#[cfg(target_os = "macos")]
pub use ports::SystemClock;
// `start` builds the task in the driver's own tests.
#[cfg(test)]
pub use check::start;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::core::{DomainState, Observation, SessionFacts};
    use crate::finder_setup::error::{COCOA_DOMAIN, FILE_PROVIDER_DOMAIN, POSIX_DOMAIN};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// What an NSError message can look like: it names a person's folder and a file.
    const PATH_MESSAGE: &str = "The file \u{201c}/Users/sam/Secret Folder/tax.pdf\u{201d} could not be added.";

    /// The sentence a person sees when the reconciler is not running, pinned as text: changing the copy changes
    /// these tests too.
    const NOT_RUNNING_SENTENCE: &str =
        "Beebeeb’s Finder setup isn’t running. Quit and reopen Beebeeb to start it again.";

    fn assert_no_leak(what: &str, text: &str) {
        for leaked in ["/Users/sam", "Secret Folder", "tax.pdf", "could not be added"] {
            assert!(!text.contains(leaked), "{what} leaked {leaked:?}: {text}");
        }
    }

    #[derive(Clone)]
    struct FakeClock {
        t0: Instant,
        now: Arc<Mutex<Instant>>,
    }

    impl FakeClock {
        fn new() -> Self {
            let t0 = Instant::now();
            Self {
                t0,
                now: Arc::new(Mutex::new(t0)),
            }
        }
        fn secs(&self) -> u64 {
            (*self.now.lock().unwrap() - self.t0).as_secs()
        }
        fn advance(&self, secs: u64) {
            *self.now.lock().unwrap() += Duration::from_secs(secs);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }
        fn unix_now(&self) -> i64 {
            1_791_291_909 + self.secs() as i64
        }
        fn sleep_until(&self, at: Instant) -> impl Future<Output = ()> + Send {
            {
                let mut now = self.now.lock().unwrap();
                if at > *now {
                    *now = at;
                }
            }
            std::future::ready(())
        }
    }

    #[derive(Default)]
    struct Record {
        ops: Vec<(u64, Op)>,
        published: Vec<FinderSetupView>,
        log: Vec<LifecycleEvent>,
        persisted_failures: Vec<Option<FailureRecord>>,
        signed_out: Vec<bool>,
        /// How many times the core told the port a removal is confirmed (the persisted "removal owed" flag is
        /// cleared by exactly this call).
        removals_confirmed: u32,
        /// How many times the core told the port the domain was registered (the persisted "domain gone" mark is
        /// cleared by exactly this call).
        domains_added: u32,
        /// For each publish while `FakePorts::ack_probe` held a receiver: whether it was still
        /// empty (the removal waiter had not been answered yet).
        ack_pending_at_publish: Vec<bool>,
        /// Task 1882: what the driver handed the port to show, for removals no waiter took.
        kept_folders: Vec<KeptFolder>,
    }

    type AckProbe = Arc<Mutex<Option<oneshot::Receiver<RemoveAnswer>>>>;

    struct FakePorts {
        clock: FakeClock,
        record: Arc<Mutex<Record>>,
        /// What `Observe` reads. A test flips it to play the person turning the extension on.
        domain: Arc<Mutex<DomainState>>,
        /// What `ReadDomain` (the read after an add, and the poll) answers.
        read: Arc<Mutex<DomainState>>,
        add: Result<(), FpError>,
        remove: Result<(), FpError>,
        /// Task 1882: what the removal kept (with its answer, `remove`).
        kept: KeptFolder,
        /// What stopping the engine a check started answers.
        stop: Result<(), FpError>,
        /// Whether a Beebeeb window is on screen (`Ports::window_visible`).
        visible: Arc<AtomicBool>,
        /// How many times the driver asked `window_visible`: once per poll wake-up.
        visible_asked: Arc<AtomicUsize>,
        /// Runs inside `AddDomain`, before it returns: an event that arrives mid-operation.
        during_add: Option<Box<dyn FnMut() + Send>>,
        /// This operation panics (inside its future) instead of answering.
        panic_on: Option<Op>,
        /// After the panic, `publish`, `log` and `persist_failure` panic too: a port that is wrecked.
        wrecked: bool,
        has_panicked: Arc<AtomicBool>,
        /// A removal waiter's receiver, looked at on every publish without being awaited.
        ack_probe: AckProbe,
    }

    impl FakePorts {
        fn new(clock: &FakeClock, domain: DomainState) -> (Self, Arc<Mutex<Record>>) {
            let record = Arc::new(Mutex::new(Record::default()));
            let ports = Self {
                clock: clock.clone(),
                record: record.clone(),
                domain: Arc::new(Mutex::new(domain)),
                read: Arc::new(Mutex::new(DomainState::Enabled)),
                add: Ok(()),
                remove: Ok(()),
                kept: KeptFolder::default(),
                stop: Ok(()),
                visible: Arc::new(AtomicBool::new(false)),
                visible_asked: Arc::new(AtomicUsize::new(0)),
                during_add: None,
                panic_on: None,
                wrecked: false,
                has_panicked: Arc::new(AtomicBool::new(false)),
                ack_probe: Arc::new(Mutex::new(None)),
            };
            (ports, record)
        }
    }

    impl FakePorts {
        fn wrecked_check(&self, what: &str) {
            if self.wrecked && self.has_panicked.load(Ordering::SeqCst) {
                panic!("{what} on a wrecked port");
            }
        }
    }

    impl Ports for FakePorts {
        fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send {
            self.record.lock().unwrap().ops.push((self.clock.secs(), op));
            let blow_up = self.panic_on == Some(op);
            if blow_up {
                self.has_panicked.store(true, Ordering::SeqCst);
            }
            let result = match op {
                Op::Observe => OpResult::Observed(Observation {
                    facts: SessionFacts {
                        vault_unlocked: true,
                        auth_present: true,
                        signed_out_by_choice: false,
                        removal_owed: false,
                    },
                    domain: Ok(*self.domain.lock().unwrap()),
                }),
                Op::StartEngine => OpResult::EngineStarted(Ok(true)),
                Op::AddDomain => {
                    if let Some(hook) = self.during_add.as_mut() {
                        hook();
                    }
                    OpResult::Added(self.add.clone())
                }
                Op::ReadDomain => OpResult::Domain(Ok(*self.read.lock().unwrap())),
                Op::WaitStable(_) => OpResult::Stable(Ok(())),
                Op::FinishReady => OpResult::Finished(Ok(())),
                Op::StopEngine => OpResult::EngineStopped(self.stop.clone()),
                Op::RemoveDomain => OpResult::Removed(self.remove.clone(), self.kept.clone()),
            };
            async move {
                if blow_up {
                    // The payload names a person's file, as a real one could.
                    panic!("the port blew up: {PATH_MESSAGE}");
                }
                result
            }
        }
        fn publish(&mut self, view: &FinderSetupView) {
            self.wrecked_check("publish");
            if let Some(receiver) = self.ack_probe.lock().unwrap().as_mut() {
                let pending = matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Empty));
                self.record.lock().unwrap().ack_pending_at_publish.push(pending);
            }
            self.record.lock().unwrap().published.push(view.clone());
        }
        fn log(&mut self, event: LifecycleEvent) {
            self.wrecked_check("log");
            self.record.lock().unwrap().log.push(event);
        }
        fn persist_failure(&mut self, record: Option<FailureRecord>) {
            self.wrecked_check("persist_failure");
            self.record.lock().unwrap().persisted_failures.push(record);
        }
        fn persist_signed_out_by_choice(&mut self, value: bool) {
            self.record.lock().unwrap().signed_out.push(value);
        }
        fn domain_removal_confirmed(&mut self) {
            self.record.lock().unwrap().removals_confirmed += 1;
        }
        fn domain_added(&mut self) {
            self.record.lock().unwrap().domains_added += 1;
        }
        fn kept_folder(&mut self, kept: KeptFolder) {
            self.record.lock().unwrap().kept_folders.push(kept);
        }
        fn window_visible(&self) -> bool {
            self.visible_asked.fetch_add(1, Ordering::SeqCst);
            self.visible.load(Ordering::SeqCst)
        }
    }

    async fn settle(handle: &FinderSetupHandle, done: impl Fn(&FinderSetupView) -> bool) {
        for _ in 0..10_000 {
            if done(&handle.view()) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the driver did not reach the expected view: {:?}", handle.view());
    }

    fn ops(record: &Arc<Mutex<Record>>) -> Vec<Op> {
        record.lock().unwrap().ops.iter().map(|(_, op)| *op).collect()
    }

    /// An answer the driver owes, or a failure instead of a hang.
    async fn answered<T>(rx: oneshot::Receiver<T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("the waiter was not answered within 5 s")
            .expect("the driver dropped the waiter without answering")
    }

    fn failed_view() -> FinderSetupView {
        FinderSetupView {
            setup: FinderSetup::Failed,
            reason: Some(FinderFailureReason::FolderTaken),
            launch_location: LaunchLocation::Applications,
            attempt: 1,
            max_attempts: 1,
            last_failure: Some(FailureRecord {
                reason: FinderFailureReason::FolderTaken,
                domain: "NSCocoaErrorDomain".into(),
                code: 516,
                at: 1_791_291_909,
            }),
        }
    }


    const KEPT_FOLDER: &str = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";

    fn kept_folder() -> KeptFolder {
        KeptFolder::Kept {
            path: KEPT_FOLDER.to_string(),
            contents_checked: true,
            empty: false,
        }
    }

    mod ops;
    mod panics;
    mod reconciler;
    mod view;
}