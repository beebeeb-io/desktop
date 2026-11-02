//! The reconciler loop and its lifecycle: apply every waiting event before choosing the next
//! operation, run at most one operation at a time, and answer removal waiters only after the
//! view is published. Also owns the retry schedule, the panic stop and the Tauri spawn.
//!
//! Must not: decide a state or an effect (see `super::core`), or run an operation outside
//! `super::limits`. Tests: `tests/reconciler.rs`, `tests/panics.rs`.

use super::*;

pub(super) struct Driver {
    pub(super) core: CoreState,
    pub(super) last_failure: Option<FailureRecord>,
    /// Every sign-out or Repair waiting for a removal. A removal answers all of them (lead
    /// ruling T3-1): a Repair and a sign-out can be queued behind the same `RemoveDomain`.
    pub(super) remove_acks: Vec<oneshot::Sender<RemoveAnswer>>,
    pub(super) view_tx: watch::Sender<FinderSetupView>,
    /// Shared with every handle: the core's hold after each input.
    pub(super) held: Arc<AtomicBool>,
    pub(super) policy: RetryPolicy,
}

impl Driver {
    pub(super) fn publish<P: Ports>(&mut self, ports: &mut P) {
        let view = FinderSetupView::of(&self.core, self.last_failure.clone());
        self.view_tx.send_replace(view.clone());
        ports.publish(&view);
    }

    pub(super) fn event<P: Ports, C: Clock>(&mut self, event: Event, ports: &mut P, clock: &C) {
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
    pub(super) fn input<P: Ports, C: Clock>(&mut self, input: Input, ports: &mut P, clock: &C) {
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
pub(super) async fn run<P: Ports, C: Clock>(mut driver: Driver, mut ports: P, clock: C, mut rx: mpsc::UnboundedReceiver<Event>) {
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
pub(super) fn stopped_by_panic<P: Ports, C: Clock>(driver: &mut Driver, ports: &mut P, clock: &C) {
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
pub(super) fn guarded(what: &str, call: impl FnOnce()) {
    if std::panic::catch_unwind(AssertUnwindSafe(call)).is_err() {
        tracing::error!("finder setup: {what} panicked while the stopped reconciler was being reported");
    }
}

pub(super) async fn drive<P: Ports, C: Clock>(
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
