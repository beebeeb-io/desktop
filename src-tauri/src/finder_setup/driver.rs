//! The single Finder reconciler task (spec §5.4, §9 `finder_setup::driver`). It receives events,
//! runs the one operation the core asks for at a time, and owns the timer. Between two
//! operations it first applies every event that arrived, so a sign-out or a lock always wins
//! over the rest of a check. Generic over `Ports` (OS, engine, app) and `Clock`.

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

pub const FINDER_SETUP_CHANGED_EVENT: &str = "finder-setup-changed";

/// What `finder_setup_state` returns and `finder-setup-changed` carries (spec §5.2, §9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FinderSetupView {
    pub setup: FinderSetup,
    pub reason: Option<FinderFailureReason>,
    pub launch_location: LaunchLocation,
    pub attempt: u8,
    pub max_attempts: u8,
    pub last_failure: Option<FailureRecord>,
}

impl FinderSetupView {
    /// The one rule for what `reason` a person is told (lead ruling T3-4, corrected 2026-10-07):
    /// - `Failed` and `UserDisabled`: the core's reason, a verdict.
    /// - `Missing`: only `NotInApplications`. That is spec device check D7 ("opened from the
    ///   mounted dmg: `not_in_applications` reported by `finder_setup_state` before sign-in"), the
    ///   reason the core already holds from the launch location before any check runs, and what
    ///   `finderSetupPresentation` reads when `setup === 'missing'`. Any other reason a `Missing`
    ///   core holds is not something to tell.
    /// - `Adding`: never. The core keeps the reason of the attempt it is silently retrying, and
    ///   publishing it would flash a failure that is not one yet.
    /// - `Ready`: never.
    ///
    /// The match is exhaustive so a new state has to choose.
    pub fn of(core: &CoreState, last_failure: Option<FailureRecord>) -> Self {
        let reason = match core.setup {
            FinderSetup::Failed | FinderSetup::UserDisabled => core.reason,
            FinderSetup::Missing => core
                .reason
                .filter(|reason| *reason == FinderFailureReason::NotInApplications),
            FinderSetup::Ready | FinderSetup::Adding => None,
        };
        Self {
            setup: core.setup,
            reason,
            launch_location: core.launch,
            attempt: core.attempt,
            max_attempts: core.max_attempts,
            last_failure,
        }
    }

    pub fn initial(launch: LaunchLocation) -> Self {
        Self::of(&CoreState::new(launch), None)
    }
}

/// What a removal answers whoever waits for it (sign-out, Repair): its result, and what macOS kept of the files that
/// had not reached the server (task 1882), also when the removal failed (review M2).
pub type RemoveAnswer = (Result<(), FpError>, KeptFolder);

pub enum Event {
    Trigger(Trigger),
    /// Sign-out or Repair: `ack` resolves once the domain removal ran.
    Remove {
        trigger: Trigger,
        ack: oneshot::Sender<RemoveAnswer>,
    },
    /// `ack` resolves once any check is cancelled and the hold is set.
    Lock {
        ack: oneshot::Sender<()>,
    },
    AppActivated,
}

#[derive(Clone)]
pub struct FinderSetupHandle {
    tx: mpsc::UnboundedSender<Event>,
    view: watch::Receiver<FinderSetupView>,
    /// The core's hold, as of the last event the reconciler applied (a Lock is applied before it is acknowledged).
    held: Arc<AtomicBool>,
}

const NOT_RUNNING: &str = "the Finder reconciler is not running (it starts again when Beebeeb is reopened)";

impl FinderSetupHandle {
    /// `Err` when the reconciler is not running (it stopped on a panic, see `start`): a caller
    /// that offers a person "Try again" can say so instead of doing nothing.
    pub fn trigger(&self, trigger: Trigger) -> Result<(), String> {
        self.tx
            .send(Event::Trigger(trigger))
            .map_err(|_| NOT_RUNNING.to_string())
    }

    /// Nothing to tell a person when it fails: the app being active is not their request.
    pub fn app_activated(&self) {
        if self.tx.send(Event::AppActivated).is_err() {
            tracing::warn!("finder setup: the app became active but the reconciler is not running");
        }
    }

    /// Sign-out or Repair: resolves once the domain removal ran. The error is the OS error's
    /// domain and code only (`FpError::redacted`, never its message): callers hand it to a Tauri
    /// command result. Task 1882: either way the answer carries what macOS kept of the files that had
    /// not reached the server, so the caller can name the folder.
    pub async fn remove(&self, trigger: Trigger, timeout: Duration) -> Result<DomainRemoval, RemovalFailure> {
        // Only these two make the core answer a waiter; any other trigger would leave `ack`
        // queued until some later removal resolved it.
        if !matches!(trigger, Trigger::SignOut | Trigger::Repair) {
            return Err(format!("{} is not a removal", trigger.as_str()).into());
        }
        let (ack, done) = oneshot::channel();
        self.tx
            .send(Event::Remove { trigger, ack })
            .map_err(|_| RemovalFailure::from(NOT_RUNNING.to_string()))?;
        match tokio::time::timeout(timeout, done).await {
            Ok(Ok((Ok(()), kept))) => Ok(DomainRemoval { kept }),
            Ok(Ok((Err(error), kept))) => Err(RemovalFailure {
                message: error.redacted(),
                kept,
            }),
            Ok(Err(_)) => Err("the Finder reconciler stopped before removing".to_string().into()),
            Err(_) => Err(format!("the Finder reconciler did not remove within {}s", timeout.as_secs()).into()),
        }
    }

    pub async fn lock(&self, timeout: Duration) -> Result<(), String> {
        let (ack, done) = oneshot::channel();
        self.tx.send(Event::Lock { ack }).map_err(|_| NOT_RUNNING.to_string())?;
        match tokio::time::timeout(timeout, done).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("the Finder reconciler stopped".to_string()),
            Err(_) => Err(format!(
                "the Finder reconciler did not acknowledge the lock within {}s",
                timeout.as_secs()
            )),
        }
    }

    pub fn view(&self) -> FinderSetupView {
        self.view.borrow().clone()
    }

    /// Held by a sign-out or a Lock until keys arrive again: "Try again" cannot add Beebeeb to Finder now, and a caller
    /// that offers it says why instead of doing nothing (area A M4).
    pub fn is_held(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }

    /// A handle whose view never changes and whose events go nowhere.
    #[cfg(test)]
    pub fn fixed(view: FinderSetupView) -> Self {
        Self::for_test(view).0
    }

    /// A handle plus the receiving end of its channel, for tests that answer events by hand.
    #[cfg(test)]
    pub fn for_test(view: FinderSetupView) -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (_view_tx, view) = watch::channel(view);
        (
            Self {
                tx,
                view,
                held: Arc::default(),
            },
            rx,
        )
    }

    /// Tests that answer events by hand set the hold the way the reconciler would.
    #[cfg(test)]
    pub fn set_held_for_test(&self, held: bool) {
        self.held.store(held, Ordering::SeqCst);
    }
}

pub trait Ports: Send + 'static {
    fn run(&mut self, op: Op) -> impl Future<Output = OpResult> + Send;
    fn publish(&mut self, view: &FinderSetupView);
    fn log(&mut self, event: LifecycleEvent);
    fn persist_failure(&mut self, record: Option<FailureRecord>);
    fn persist_signed_out_by_choice(&mut self, value: bool);
    /// The domain is confirmed gone (a removal worked, or an `Observe` found none while a removal was owed): clear
    /// the persisted "removal owed" flag. A port with nothing persisted has nothing to do.
    fn domain_removal_confirmed(&mut self) {}
    /// The domain was registered (`addDomain` succeeded): the persisted "domain gone" mark no longer holds, so clear
    /// it. A port with nothing persisted has nothing to do.
    fn domain_added(&mut self) {}
    /// Task 1882: a removal nobody took the answer of (an owed removal at launch, the account binding's Repair, or a
    /// sign-out or Repair that stopped waiting) left `kept`. The port shows the person a kept folder the way the
    /// app-start sweep does: saved for the Settings › Sync row, then the alert. `kept` may be nothing.
    fn kept_folder(&mut self, kept: KeptFolder);
    fn window_visible(&self) -> bool;
}

/// The schedule's clock (§7: "the schedule and the clock … are injected in tests").
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
    fn unix_now(&self) -> i64;
    fn sleep_until(&self, at: Instant) -> impl Future<Output = ()> + Send;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_default()
    }

    fn sleep_until(&self, at: Instant) -> impl Future<Output = ()> + Send {
        tokio::time::sleep_until(tokio::time::Instant::from_std(at))
    }
}

/// How long a port may take to answer one operation before the driver gives up on it (lead
/// ruling T7-1). Several ObjC primitives wait `DISPATCH_TIME_FOREVER` for `fileproviderd`, and a
/// stuck one must not hang the reconciler and, behind it, sign-out or lock. Real calls answer in
/// milliseconds; these are the point where "slow" has become "not coming".
///
/// - `Observe`, `ReadDomain`: one `getDomains` round trip to `fileproviderd`.
/// - `StartEngine`: spawn plus the 3 s wait for the engine's socket and the engine slot's lock.
/// - `AddDomain`: `addDomain` may have to register the extension, the slowest honest call here.
/// - `WaitStable(w)`: the bridge enforces `w` on its own clock (spec: at most 10 s). The extra 3 s
///   only backs up a bridge that fails to return at all.
/// - `FinishReady`: saving the sync folder and taking the engine slot's lock.
/// - `StopEngine`: the engine slot's lock plus its own 3 s + 2 s abort windows.
/// - `RemoveDomain`: `removeDomain`, which sign-out waits for.
pub fn op_limit(op: Op) -> Duration {
    let s = Duration::from_secs;
    match op {
        Op::Observe | Op::ReadDomain => s(10),
        Op::StartEngine | Op::FinishReady | Op::StopEngine | Op::RemoveDomain => s(15),
        Op::AddDomain => s(20),
        Op::WaitStable(wait) => wait + s(3),
    }
}

fn op_name(op: Op) -> &'static str {
    match op {
        Op::Observe => "Observe",
        Op::StartEngine => "StartEngine",
        Op::AddDomain => "AddDomain",
        Op::ReadDomain => "ReadDomain",
        Op::WaitStable(_) => "WaitStable",
        Op::FinishReady => "FinishReady",
        Op::StopEngine => "StopEngine",
        Op::RemoveDomain => "RemoveDomain",
    }
}

/// Run one port call for at most `limit`. A call that is not back by then is dropped (a blocking
/// bridge call keeps its thread until the OS answers, but nothing waits for it any more) and
/// becomes `OP_TIMEOUT`, which classifies as `timeout`: it retries on the schedule and ends in a
/// sentence and an action, never in a reconciler that sits on one call forever. The text names
/// the operation and the limit only.
pub async fn within<T>(op: Op, limit: Duration, call: impl Future<Output = Result<T, FpError>>) -> Result<T, FpError> {
    match tokio::time::timeout(limit, call).await {
        Ok(result) => result,
        Err(_) => Err(FpError::app(
            app_code::OP_TIMEOUT,
            format!("{} did not answer within {limit:?}", op_name(op)),
        )),
    }
}

struct Driver {
    core: CoreState,
    last_failure: Option<FailureRecord>,
    /// Every sign-out or Repair waiting for a removal. A removal answers all of them (lead
    /// ruling T3-1): a Repair and a sign-out can be queued behind the same `RemoveDomain`.
    remove_acks: Vec<oneshot::Sender<RemoveAnswer>>,
    view_tx: watch::Sender<FinderSetupView>,
    /// Shared with every handle: the core's hold after each input.
    held: Arc<AtomicBool>,
    policy: RetryPolicy,
}

impl Driver {
    fn publish<P: Ports>(&mut self, ports: &mut P) {
        let view = FinderSetupView::of(&self.core, self.last_failure.clone());
        self.view_tx.send_replace(view.clone());
        ports.publish(&view);
    }

    fn event<P: Ports, C: Clock>(&mut self, event: Event, ports: &mut P, clock: &C) {
        match event {
            Event::Trigger(trigger) => self.input(Input::Trigger(trigger), ports, clock),
            Event::Remove { trigger, ack } => {
                self.remove_acks.push(ack);
                self.input(Input::Trigger(trigger), ports, clock);
            }
            Event::Lock { ack } => {
                self.input(Input::Trigger(Trigger::Lock), ports, clock);
                let _ = ack.send(());
            }
            Event::AppActivated => self.input(Input::AppActivated, ports, clock),
        }
    }

    /// Feed the core one input and carry out what it asks for. The view is published once, after
    /// every effect: the core's own `Publish` effects all describe its final state, and so does a
    /// change to the saved failure that the core did not consider a transition (the same failure
    /// twice: only `at` moved, and "Copy details" shows it).
    ///
    /// Removal waiters are answered LAST, after that publish. The core emits `Publish` before
    /// `RemoveFinished`, but this loop runs the effects in order and publishes at its end, so
    /// answering inside the loop woke a `remove()` caller (on a multi-thread runtime, at once)
    /// that could still read the view from before the removal.
    fn input<P: Ports, C: Clock>(&mut self, input: Input, ports: &mut P, clock: &C) {
        // Task 1882: what a removal kept travels beside the core, which decides nothing from it. A waiter gets it with
        // the removal's answer; when no waiter takes it, the port shows it to the person (below).
        let mut kept = match &input {
            Input::Done(OpResult::Removed(_, kept)) => Some(kept.clone()),
            _ => None,
        };
        let (core, effects) = core::step(self.core.clone(), input, clock.now(), &self.policy);
        self.core = core;
        self.held.store(self.core.is_held(), Ordering::SeqCst);
        let failure_before = self.last_failure.clone();
        let mut publish = false;
        let mut removals = Vec::new();
        for effect in effects {
            match effect {
                Effect::Publish => publish = true,
                Effect::TriggerReceived(trigger) => ports.log(LifecycleEvent::Trigger(trigger)),
                Effect::Transition(t) => ports.log(LifecycleEvent::Transition {
                    from: t.from,
                    to: t.to,
                    reason: t.reason,
                    error: t.error,
                    attempt: t.attempt,
                    max_attempts: t.max_attempts,
                }),
                Effect::PersistFailure(Some((reason, error))) => {
                    // Domain and code only: the message goes to the lifecycle log and nowhere else.
                    let record = FailureRecord {
                        reason,
                        domain: error.domain,
                        code: error.code,
                        at: clock.unix_now(),
                    };
                    self.last_failure = Some(record.clone());
                    ports.persist_failure(Some(record));
                }
                Effect::PersistFailure(None) => {
                    if self.last_failure.take().is_some() {
                        ports.persist_failure(None);
                    }
                }
                Effect::PersistSignedOutByChoice(value) => ports.persist_signed_out_by_choice(value),
                Effect::DomainRemovalConfirmed => ports.domain_removal_confirmed(),
                Effect::DomainAddConfirmed => ports.domain_added(),
                Effect::RemoveFinished(result) => removals.push(result),
            }
        }
        if publish || self.last_failure != failure_before {
            self.publish(ports);
        }
        for result in removals {
            let answer: RemoveAnswer = (result, kept.take().unwrap_or_default());
            let mut taken = false;
            for ack in self.remove_acks.drain(..) {
                taken |= ack.send(answer.clone()).is_ok();
            }
            if !taken {
                kept = Some(answer.1);
            }
        }
        if let Some(kept) = kept {
            ports.kept_folder(kept);
        }
    }
}

pub fn start<P: Ports, C: Clock>(
    ports: P,
    clock: C,
    launch: LaunchLocation,
    persisted: Option<FailureRecord>,
    policy: RetryPolicy,
) -> (FinderSetupHandle, impl Future<Output = ()> + Send + 'static) {
    let (tx, rx) = mpsc::unbounded_channel();
    let core = CoreState::new(launch);
    let (view_tx, view) = watch::channel(FinderSetupView::of(&core, persisted.clone()));
    let held = Arc::new(AtomicBool::new(core.is_held()));
    let driver = Driver {
        core,
        last_failure: persisted,
        remove_acks: Vec::new(),
        view_tx,
        held: held.clone(),
        policy,
    };
    (FinderSetupHandle { tx, view, held }, run(driver, ports, clock, rx))
}

/// The reconciler as a task that cannot die unseen. The loop borrows the driver and the ports
/// instead of owning them, so when anything in it panics (a port, the core) both are still here to
/// report with. A `JoinHandle` supervisor could not do that: the ports die with the task.
async fn run<P: Ports, C: Clock>(mut driver: Driver, mut ports: P, clock: C, mut rx: mpsc::UnboundedReceiver<Event>) {
    let outcome = AssertUnwindSafe(drive(&mut driver, &mut ports, &clock, &mut rx))
        .catch_unwind()
        .await;
    if outcome.is_err() {
        stopped_by_panic(&mut driver, &mut ports, &clock);
    }
    // `rx` is dropped here: from now on `trigger`, `remove` and `lock` say "not running", and a
    // removal waiter that was queued gets its channel closed instead of an answer. No restart:
    // the next launch of the app starts a fresh driver.
}

/// What a person and support see when the reconciler panicked: one failed `unknown` view (so
/// there is a sentence and an action instead of "Adding…" forever), a lifecycle line, the saved
/// failure for "Copy details" after a relaunch. Fixed text only: the panic payload can name a path,
/// and the default panic hook has already printed it to stderr.
fn stopped_by_panic<P: Ports, C: Clock>(driver: &mut Driver, ports: &mut P, clock: &C) {
    tracing::error!(
        "finder setup: the Finder reconciler panicked and has stopped; it starts again when Beebeeb is reopened"
    );
    let error = FpError::app(app_code::RECONCILER_PANIC, "the Finder reconciler panicked");
    let from = driver.core.setup;
    let attempt = driver.core.attempt;
    let max_attempts = driver.core.max_attempts.max(attempt);
    let record = FailureRecord {
        reason: FinderFailureReason::Unknown,
        domain: error.domain.clone(),
        code: error.code,
        at: clock.unix_now(),
    };
    let view = FinderSetupView {
        setup: FinderSetup::Failed,
        reason: Some(FinderFailureReason::Unknown),
        launch_location: driver.core.launch,
        attempt,
        max_attempts,
        last_failure: Some(record.clone()),
    };
    // The handle first: it needs nothing from the ports, which may be what panicked.
    driver.last_failure = Some(record.clone());
    driver.view_tx.send_replace(view.clone());
    guarded("log", || {
        ports.log(LifecycleEvent::Transition {
            from,
            to: FinderSetup::Failed,
            reason: Some(FinderFailureReason::Unknown),
            error: Some(error),
            attempt,
            max_attempts,
        })
    });
    guarded("persist_failure", || ports.persist_failure(Some(record)));
    guarded("publish", || ports.publish(&view));
}

/// A port that panicked once may panic again: report the stop to as many of them as will listen.
fn guarded(what: &str, call: impl FnOnce()) {
    if std::panic::catch_unwind(AssertUnwindSafe(call)).is_err() {
        tracing::error!("finder setup: {what} panicked while the stopped reconciler was being reported");
    }
}

async fn drive<P: Ports, C: Clock>(
    driver: &mut Driver,
    ports: &mut P,
    clock: &C,
    rx: &mut mpsc::UnboundedReceiver<Event>,
) {
    driver.publish(ports);
    loop {
        // Cooperative: an instant fake clock must not starve the runtime.
        tokio::task::yield_now().await;
        // Every event that arrived during the last operation is applied BEFORE the next one is
        // chosen: this is what makes sign-out and lock win over the rest of a check (§5.3).
        while let Ok(event) = rx.try_recv() {
            driver.event(event, ports, clock);
        }
        match core::next_op(&mut driver.core, clock.now(), &driver.policy) {
            Next::Run(op) => {
                let result = ports.run(op).await;
                driver.input(Input::Done(result), ports, clock);
            }
            Next::WakeAt(at) => {
                tokio::select! {
                    biased;
                    event = rx.recv() => match event {
                        Some(event) => driver.event(event, ports, clock),
                        None => return,
                    },
                    () = clock.sleep_until(at) => {
                        let window_visible = ports.window_visible();
                        driver.input(Input::Tick { window_visible }, ports, clock);
                    }
                }
            }
            Next::Idle => match rx.recv().await {
                Some(event) => driver.event(event, ports, clock),
                None => return,
            },
        }
    }
}

/// `start`, on Tauri's runtime. The task is supervised inside the future `start` returns (see
/// `run`), so a panic in a port reaches the handle's view whichever way the future is spawned.
pub fn spawn<P: Ports, C: Clock>(
    ports: P,
    clock: C,
    launch: LaunchLocation,
    persisted: Option<FailureRecord>,
    policy: RetryPolicy,
) -> FinderSetupHandle {
    let (handle, task) = start(ports, clock, launch, persisted, policy);
    tauri::async_runtime::spawn(task);
    handle
}

/// The text of "Copy details" (spec §6.2): app version, macOS version, reason, NSError domain +
/// code, attempt count, the time of the last attempt, and the lifecycle log's tail (§8). Never a
/// path, a file name, the email or the account id: the view carries none, and the tail is already
/// redacted. Private on purpose: callers use [`copy_details`], which reads the tail itself, so no
/// caller can hand it a short or an empty one.
fn copy_details_text(
    view: &FinderSetupView,
    app_version: &str,
    macos_version: &str,
    lifecycle_tail: &[String],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Beebeeb {} on macOS {}",
        lifecycle_log::token(app_version),
        lifecycle_log::token(macos_version)
    );
    let _ = writeln!(out, "Finder setup: {}", view.setup.as_str());
    let reason = view.reason.or(view.last_failure.as_ref().map(|f| f.reason));
    let _ = writeln!(out, "Reason: {}", reason.map_or("none", FinderFailureReason::as_str));
    match &view.last_failure {
        Some(failure) => {
            let _ = writeln!(out, "Error: {} {}", lifecycle_log::token(&failure.domain), failure.code);
            let at = chrono::DateTime::from_timestamp(failure.at, 0)
                .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let _ = writeln!(out, "Last attempt: {at}");
        }
        None => {
            let _ = writeln!(out, "Error: none");
        }
    }
    let _ = writeln!(
        out,
        "Attempts: {} of {}",
        view.attempt,
        view.max_attempts.max(view.attempt)
    );
    let _ = writeln!(out, "Launched from: {}", view.launch_location.as_str());
    let _ = writeln!(out, "\nLifecycle log (last {} lines):", lifecycle_tail.len());
    for line in lifecycle_tail {
        let _ = writeln!(out, "{line}");
    }
    out
}

/// The one "Copy details" there is, for the commands to call: the view plus the last
/// `TAIL_LINES` lines of the real lifecycle log (lead ruling T5-b).
pub fn copy_details(view: &FinderSetupView, app_version: &str, macos_version: &str) -> String {
    copy_details_from(view, app_version, macos_version, lifecycle_log::tail)
}

fn copy_details_from(
    view: &FinderSetupView,
    app_version: &str,
    macos_version: &str,
    tail: impl FnOnce(usize) -> Vec<String>,
) -> String {
    copy_details_text(view, app_version, macos_version, &tail(lifecycle_log::TAIL_LINES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::core::{DomainState, Observation, SessionFacts};
    use crate::finder_setup::error::{COCOA_DOMAIN, FILE_PROVIDER_DOMAIN, POSIX_DOMAIN};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// What an NSError message can look like: it names a person's folder and a file.
    const PATH_MESSAGE: &str = "The file \u{201c}/Users/sam/Secret Folder/tax.pdf\u{201d} could not be added.";

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
        add: Result<(), FpError>,
        remove: Result<(), FpError>,
        /// Task 1882: what the removal kept (with its answer, `remove`).
        kept: KeptFolder,
        /// What stopping the engine a check started answers.
        stop: Result<(), FpError>,
        /// Whether a Beebeeb window is on screen (`Ports::window_visible`).
        visible: Arc<AtomicBool>,
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
                add: Ok(()),
                remove: Ok(()),
                kept: KeptFolder::default(),
                stop: Ok(()),
                visible: Arc::new(AtomicBool::new(false)),
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
                Op::ReadDomain => OpResult::Domain(Ok(DomainState::Enabled)),
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

    #[test]
    fn copy_details_names_what_support_needs_and_never_a_path() {
        let view = FinderSetupView {
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
        };
        let tail = vec!["2026-10-06T13:05:09Z trigger keys_arrived".to_string()];
        let text = copy_details_text(&view, "0.8.12-alpha.1", "26.0", &tail);
        let header = &text[..text.find("Lifecycle log").expect("the log section")];
        assert_eq!(
            header,
            "Beebeeb 0.8.12-alpha.1 on macOS 26.0\nFinder setup: failed\nReason: folder_taken\nError: NSCocoaErrorDomain 516\nLast attempt: 2026-10-06T13:05:09Z\nAttempts: 1 of 1\nLaunched from: applications\n\n"
        );
        assert!(
            text.ends_with("Lifecycle log (last 1 lines):\n2026-10-06T13:05:09Z trigger keys_arrived\n"),
            "{text}"
        );
        assert!(!header.contains('/') && !header.contains('@'), "{header}");
    }

    // ---- Task 7 additions: lead rulings T3-1, T3-4, T14-c3, T1-4, T5-b, plus the driver's own seams ----

    #[test]
    fn the_view_is_exactly_the_json_the_typescript_parses() {
        // The brief's sample, as text: the contract with `src/finderSetup.ts` (`parseFinderSetupView`).
        let sample = r#"{"setup":"failed","reason":"folder_taken","launch_location":"applications","attempt":1,"max_attempts":1,"last_failure":{"reason":"folder_taken","domain":"NSCocoaErrorDomain","code":516,"at":1791291909}}"#;
        let json = serde_json::to_value(failed_view()).unwrap();
        assert_eq!(json, serde_json::from_str::<serde_json::Value>(sample).unwrap());

        let object = json.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "attempt",
                "last_failure",
                "launch_location",
                "max_attempts",
                "reason",
                "setup"
            ],
            "no extra and no missing key"
        );
        assert_eq!(object["setup"], "failed");
        assert_eq!(object["reason"], "folder_taken");
        assert_eq!(object["launch_location"], "applications");
        let failure = object["last_failure"].as_object().unwrap();
        let mut failure_keys: Vec<&str> = failure.keys().map(String::as_str).collect();
        failure_keys.sort_unstable();
        assert_eq!(failure_keys, ["at", "code", "domain", "reason"]);
        // Numbers are JSON numbers, never strings.
        for (name, value) in [
            ("attempt", &object["attempt"]),
            ("max_attempts", &object["max_attempts"]),
            ("code", &failure["code"]),
            ("at", &failure["at"]),
        ] {
            assert!(value.is_i64(), "{name} must be a JSON number: {value}");
        }

        // Absent values are present-and-null, the same JSON `finder_setup_state` returns at start.
        assert_eq!(
            serde_json::to_value(FinderSetupView::initial(LaunchLocation::Elsewhere)).unwrap(),
            serde_json::json!({
                "setup": "missing", "reason": null, "launch_location": "elsewhere",
                "attempt": 0, "max_attempts": 0, "last_failure": null
            })
        );

        // `launch_location` is exactly one of four strings, and every setup state keeps its wire name.
        for (location, wire) in [
            (LaunchLocation::Applications, "applications"),
            (LaunchLocation::Translocated, "translocated"),
            (LaunchLocation::DiskImage, "disk_image"),
            (LaunchLocation::Elsewhere, "elsewhere"),
        ] {
            assert_eq!(
                serde_json::to_value(FinderSetupView::initial(location)).unwrap()["launch_location"],
                wire
            );
        }
        for (setup, wire) in [
            (FinderSetup::Ready, "ready"),
            (FinderSetup::Missing, "missing"),
            (FinderSetup::Adding, "adding"),
            (FinderSetup::Failed, "failed"),
            (FinderSetup::UserDisabled, "user_disabled"),
        ] {
            let mut core = CoreState::new(LaunchLocation::Applications);
            core.setup = setup;
            assert_eq!(
                serde_json::to_value(FinderSetupView::of(&core, None)).unwrap()["setup"],
                wire
            );
        }
    }

    #[test]
    fn a_view_carries_a_reason_when_failed_or_user_disabled_or_missing_for_the_launch_location() {
        // Every state against every reason the core could be holding (lead ruling T3-4, corrected).
        for setup in [
            FinderSetup::Ready,
            FinderSetup::Missing,
            FinderSetup::Adding,
            FinderSetup::Failed,
            FinderSetup::UserDisabled,
        ] {
            for reason in FinderFailureReason::ALL {
                let mut core = CoreState::new(LaunchLocation::Applications);
                core.setup = setup;
                core.reason = Some(reason);
                let expected = match setup {
                    FinderSetup::Failed | FinderSetup::UserDisabled => Some(reason),
                    FinderSetup::Missing => (reason == FinderFailureReason::NotInApplications).then_some(reason),
                    FinderSetup::Ready | FinderSetup::Adding => None,
                };
                assert_eq!(
                    FinderSetupView::of(&core, None).reason,
                    expected,
                    "{setup:?} holding {reason:?}"
                );
            }
            let mut core = CoreState::new(LaunchLocation::Applications);
            core.setup = setup;
            core.reason = None;
            assert_eq!(
                FinderSetupView::of(&core, None).reason,
                None,
                "{setup:?} holding no reason"
            );
        }
    }

    #[test]
    fn a_disk_image_or_translocated_launch_is_missing_for_not_in_applications_before_any_check() {
        // Spec D7: "opened from the mounted dmg: `not_in_applications` reported by
        // `finder_setup_state` before sign-in". The core knows it at construction.
        for location in [LaunchLocation::DiskImage, LaunchLocation::Translocated] {
            let view = FinderSetupView::initial(location);
            assert_eq!(
                (view.setup, view.reason),
                (FinderSetup::Missing, Some(FinderFailureReason::NotInApplications)),
                "{location:?}"
            );
        }
        for location in [LaunchLocation::Applications, LaunchLocation::Elsewhere] {
            let view = FinderSetupView::initial(location);
            assert_eq!((view.setup, view.reason), (FinderSetup::Missing, None), "{location:?}");
        }
    }

    #[tokio::test]
    async fn a_dmg_launch_publishes_not_in_applications_at_start_with_no_trigger() {
        let clock = FakeClock::new();
        let (ports, record) = FakePorts::new(&clock, DomainState::NotRegistered);
        let (handle, task) = start(
            ports,
            clock.clone(),
            LaunchLocation::DiskImage,
            None,
            RetryPolicy::default(),
        );
        assert_eq!(
            handle.view().reason,
            Some(FinderFailureReason::NotInApplications),
            "the first read, before the task runs"
        );
        tokio::spawn(task);
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let record = record.lock().unwrap();
        let first = record.published.first().expect("published at start");
        assert_eq!(
            (first.setup, first.reason, first.launch_location),
            (
                FinderSetup::Missing,
                Some(FinderFailureReason::NotInApplications),
                LaunchLocation::DiskImage
            )
        );
        assert!(
            record.ops.is_empty(),
            "nothing ran: the notice needs no sign-in and no check"
        );
    }

    #[tokio::test]
    async fn a_silent_retry_never_publishes_its_reason() {
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
        let record = record.lock().unwrap();
        assert!(
            record
                .published
                .iter()
                .any(|v| v.setup == FinderSetup::Adding && v.attempt == 3),
            "the third attempt's retry was published as Adding: {:?}",
            record.published
        );
        for view in &record.published {
            if !matches!(view.setup, FinderSetup::Failed | FinderSetup::UserDisabled) {
                assert_eq!(
                    view.reason, None,
                    "a {:?} view must not carry a reason: {view:?}",
                    view.setup
                );
            }
        }
        assert_eq!(
            record.published.last().and_then(|v| v.reason),
            Some(FinderFailureReason::ExtensionLoading)
        );
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

    const KEPT_FOLDER: &str = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";

    fn kept_folder() -> KeptFolder {
        KeptFolder::Kept {
            path: KEPT_FOLDER.to_string(),
            contents_checked: true,
            empty: false,
        }
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

    #[test]
    fn copy_details_reads_the_last_200_lifecycle_lines() {
        let asked = std::cell::Cell::new(0);
        let text = copy_details_from(&failed_view(), "0.8.12", "26.0", |n| {
            asked.set(n);
            vec![
                "2026-10-06T13:05:09Z trigger launch".to_string(),
                "2026-10-06T13:05:10Z trigger try_again".to_string(),
            ]
        });
        assert_eq!(asked.get(), 200);
        assert!(
            text.ends_with("Lifecycle log (last 2 lines):\n2026-10-06T13:05:09Z trigger launch\n2026-10-06T13:05:10Z trigger try_again\n"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_handle_says_so_when_its_reconciler_is_gone_or_silent_and_refuses_what_is_not_a_removal() {
        let view = FinderSetupView::initial(LaunchLocation::Applications);
        let (handle, rx) = FinderSetupHandle::for_test(view.clone());
        drop(rx);
        assert!(
            handle
                .remove(Trigger::SignOut, Duration::from_secs(1))
                .await
                .unwrap_err()
                .message
                .contains("not running")
        );
        assert!(
            handle
                .lock(Duration::from_secs(1))
                .await
                .unwrap_err()
                .contains("not running")
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
        assert!(handle.trigger(Trigger::TryAgain).unwrap_err().contains("not running"));
        assert!(
            handle
                .remove(Trigger::SignOut, Duration::from_secs(5))
                .await
                .unwrap_err()
                .message
                .contains("not running")
        );
        assert!(
            handle
                .lock(Duration::from_secs(5))
                .await
                .unwrap_err()
                .contains("not running")
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
        let outcome = tokio::time::timeout(Duration::from_secs(10), async {
            settle(&handle, |v| v.setup == FinderSetup::Failed).await
        })
        .await;
        assert!(
            outcome.is_ok(),
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
}
