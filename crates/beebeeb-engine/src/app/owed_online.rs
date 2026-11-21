//! The network coming back pays what the engine owes the server (LD-0028). The engine reports `State::Offline` and
//! then another state when it reaches the server again; [`OnlineWatch`] sees that change in the host's events and
//! [`OwedPayingHost`] starts [`super::session_revoke::pay_owed_detached`] on it. Carries no path, id or token.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::engine_status::State;
use crate::host::{ConflictNotice, EngineEvent, EngineHost, Notice};
use crate::keychain::owed_revocations::OwedRevocations;
use crate::transfer_progress::TransferBoard;

/// Remembers that the last reported state was `Offline`.
#[derive(Default)]
pub(crate) struct OnlineWatch(AtomicBool);

impl OnlineWatch {
    /// Feed each reported state. `true` exactly when this one ends an offline stretch.
    pub(crate) fn note(&self, state: State) -> bool {
        if state == State::Offline {
            self.0.store(true, Ordering::SeqCst);
            false
        } else {
            self.0.swap(false, Ordering::SeqCst)
        }
    }
}

/// The host the handle wraps around the one it was given: it forwards everything and pays the owed list when the
/// network comes back.
pub(crate) struct OwedPayingHost {
    inner: Arc<dyn EngineHost>,
    watch: OnlineWatch,
    runtime: tokio::runtime::Handle,
    client: reqwest::Client,
    base: String,
    owed: Arc<OwedRevocations>,
}

impl OwedPayingHost {
    pub(crate) fn new(
        inner: Arc<dyn EngineHost>,
        runtime: tokio::runtime::Handle,
        client: reqwest::Client,
        base: String,
        owed: Arc<OwedRevocations>,
    ) -> Self {
        Self {
            inner,
            watch: OnlineWatch::default(),
            runtime,
            client,
            base,
            owed,
        }
    }
}

impl EngineHost for OwedPayingHost {
    fn event(&self, event: EngineEvent) {
        if let EngineEvent::Status(status) = &event {
            if self.watch.note(status.status.state) {
                let _enter = self.runtime.enter();
                super::session_revoke::pay_owed_detached(self.client.clone(), self.base.clone(), self.owed.clone());
            }
        }
        self.inner.event(event);
    }

    fn notify(&self, notice: Notice) -> Result<(), String> {
        self.inner.notify(notice)
    }

    fn conflict_opens_window(&self) -> bool {
        self.inner.conflict_opens_window()
    }

    fn open_conflict(&self, conflict: &ConflictNotice) -> Result<(), String> {
        self.inner.open_conflict(conflict)
    }

    fn transfer_board(&self) -> Option<Arc<TransferBoard>> {
        self.inner.transfer_board()
    }

    fn file_provider(&self) -> Option<Arc<dyn crate::file_provider_port::FileProviderPort>> {
        self.inner.file_provider()
    }

    fn now(&self) -> std::time::SystemTime {
        self.inner.now()
    }

    fn start_report(&self, report: crate::sync_view::StartReport) {
        self.inner.start_report(report);
    }

    fn wake(&self) {
        self.inner.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_state_after_offline_is_the_network_coming_back() {
        let watch = OnlineWatch::default();
        assert!(!watch.note(State::Syncing), "never offline: nothing to pay for");
        assert!(!watch.note(State::Offline));
        assert!(!watch.note(State::Offline));
        assert!(watch.note(State::Syncing), "the first state after offline");
        assert!(!watch.note(State::Syncing), "once per offline stretch");
    }
}
