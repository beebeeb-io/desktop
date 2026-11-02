//! The handle the app holds: the events it sends to the reconciler (a trigger, a removal, a
//! lock, the app becoming active) and the view it reads back.
//!
//! A removal answers every waiter queued for it (lead ruling T3-1) with what macOS kept when
//! the files had not reached the server (task 1882). Tests: `tests/reconciler.rs` for the
//! removal and lock answers, `tests/view.rs` for a handle whose reconciler is gone.

use super::*;

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
    pub(super) tx: mpsc::UnboundedSender<Event>,
    pub(super) view: watch::Receiver<FinderSetupView>,
    /// The core's hold, as of the last event the reconciler applied (a Lock is applied before it is acknowledged).
    pub(super) held: Arc<AtomicBool>,
}

/// What a person is told when the reconciler is not running: it was never started, or it stopped on a panic. The one
/// source for this sentence: the handle's calls return it, and so do the app's removal and lock when there is no
/// reconciler at all.
pub const NOT_RUNNING: &str = "Beebeeb’s Finder setup isn’t running. Quit and reopen Beebeeb to start it again.";

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
