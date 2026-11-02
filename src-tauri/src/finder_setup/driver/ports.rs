//! The seam between the reconciler and the app: `Ports` is where one operation runs and where
//! its log, failure and view go; `Clock` is the schedule's clock, injected in tests (spec §7).
//!
//! Must not: decide a step, a retry or a transition (that is `super::core`).

use super::*;

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
