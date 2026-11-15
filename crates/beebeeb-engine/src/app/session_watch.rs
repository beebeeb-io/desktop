//! Task 2535: the account's session watch. A PC whose vault is open closes it when Beebeeb ended the session elsewhere
//! ("Sign out of all devices"), also when no sync runner feeds the run of refusals (`AuthHealth`).
//!
//! The run is fed by the engine's own loop: the sync tick, the heartbeat, and the paused tick's check. With no runner
//! (its start was refused: the sync folder is gone, the registration failed, the local data is another account's or
//! its owner is unknown; or the runner ended) none of them runs, so nothing counted the refusals and the keys stayed
//! in memory. This watch fills that gap: ONE task per engine, started with the engine's runtime, that every period
//! asks Beebeeb whether it still accepts the session in memory, but only while
//! - a session is in memory (signed in, vault open; a locked screen keeps the session, and is watched too),
//! - the session is not held as ended already (the session watcher closes it; nothing more is asked), and
//! - none of the engine's own feeds noted an outcome within the period ([`AuthHealth::fed_within`]).
//!
//! The answer counts as the engine's own calls do (`AuthHealth::note_watch_answer`): a 401 adds one, an answer clears
//! the run, offline, a timeout or a 5xx leaves it. A suspended account's 403 is not a 401, so it never counts
//! (decision 2531). Three refusals in a row hold the session as ended, and the session watcher
//! (`Engine::watch_session`) closes the vault. The watch's answers do not count as a feed, so with nothing else
//! running it checks every period: an ended session is held as ended after three periods.
//!
//! The paused tick's check (`runner.rs`) gives up after 2 s. On a slow link it then notes nothing, nothing fed the
//! run, and this watch checks in the next period with the API client's own timeout. The check is raced against a
//! session that moved (a lock, a sign-out, a new sign-in): the request is dropped at once and nothing is counted. The
//! engine's stop drops the engine's runtime, and this task and a check in flight with it.
//!
//! Must not: log a token, an id or an address; send anything while no session is in memory; count an answer for a
//! session that is no longer the one in memory; start more than one task per engine.
//!
//! Tests: `session_watch_tests.rs`.

use std::sync::Arc;
use std::time::Duration;

use zeroize::Zeroizing;

use crate::account::{AccountRuntime, SessionGeneration};
use crate::api_client::ApiClient;
use crate::session::Session;

/// How often the watch looks, and how recent a feed of the engine's own must be to keep it quiet.
pub(crate) const SESSION_WATCH_PERIOD: Duration = Duration::from_secs(30);

/// How often a check in flight looks whether the session it was asked for is still the one in memory.
const MOVED_POLL: Duration = Duration::from_millis(50);

/// Start the engine's one watch on its runtime, every [`SESSION_WATCH_PERIOD`] (`Engine::open`, once per engine). It
/// runs until the runtime is dropped with the engine.
pub(super) fn start(runtime: &tokio::runtime::Runtime, acct: &Arc<AccountRuntime>, api_base: &str) {
    let watch = SessionWatch::new(acct.clone(), api_base.to_string(), SESSION_WATCH_PERIOD);
    runtime.spawn(watch.run());
}

/// What one step of the watch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// No session in memory (signed out, or the vault is closed): nothing was asked.
    NoSession,
    /// The session is held as ended already: nothing more was asked.
    AlreadyEnded,
    /// One of the engine's own feeds noted an outcome within the period: nothing was asked.
    Fed,
    /// Beebeeb answered and accepts the session: the run was cleared.
    Accepted,
    /// Beebeeb refused the session (401): one more in the run.
    Refused,
    /// No answer that says anything about the session (offline, a timeout, a 5xx, a 403): the run is as it was.
    NoAnswer,
    /// The session moved on while the check was in flight: the check was dropped and nothing was counted.
    Moved,
}

/// The watch of one account's session.
pub(super) struct SessionWatch {
    acct: Arc<AccountRuntime>,
    api_base: String,
    period: Duration,
}

impl SessionWatch {
    pub(super) fn new(acct: Arc<AccountRuntime>, api_base: String, period: Duration) -> Self {
        Self { acct, api_base, period }
    }

    /// One step every period, the first one period after the start. The next period starts when a step ends, so a
    /// check that ran long is never followed by a burst of them, and there is one check at a time.
    pub(super) async fn run(self) {
        loop {
            tokio::time::sleep(self.period).await;
            self.step().await;
        }
    }

    /// One look: the check, when one is due (see the module's doc).
    pub(super) async fn step(&self) -> Step {
        if self.acct.auth_health.is_expired() {
            return Step::AlreadyEnded;
        }
        if self.acct.auth_health.fed_within(self.period) {
            return Step::Fed;
        }
        let session = self.with_session(|session, turn| (Zeroizing::new(session.token.clone()), turn));
        let Some((token, turn)) = session else {
            return Step::NoSession;
        };
        // No vault key: the check decrypts nothing. The client's copy of the token is wiped when it is dropped.
        let api = ApiClient::new(self.api_base.clone(), String::from(token.as_str()), [0; 32]);
        let answer = tokio::select! {
            biased;
            () = self.moved_from(&token, turn) => None,
            answer = api.session_check() => Some(answer),
        };
        drop(api);
        let Some(answer) = answer else {
            tracing::debug!("session watch: dropped, the session moved on");
            return Step::Moved;
        };
        let step = match &answer {
            Ok(()) => Step::Accepted,
            Err(error) if crate::runner::is_session_refusal(error) => Step::Refused,
            Err(_) => Step::NoAnswer,
        };
        let counted = self
            .with_session(|session, now| {
                let still = now == turn && session.token.as_str() == token.as_str();
                if still {
                    self.acct.auth_health.note_watch_answer(answer.as_ref().err());
                }
                still
            })
            .unwrap_or(false);
        if !counted {
            tracing::debug!("session watch: dropped, the session moved on");
            return Step::Moved;
        }
        match step {
            Step::Accepted => tracing::debug!("session watch: checked"),
            Step::Refused => tracing::debug!("session watch: refused"),
            _ => tracing::debug!("session watch: no answer"),
        }
        step
    }

    /// Ends when the session in memory is no longer the one `turn` and `token` name (a lock, a sign-out, a new
    /// sign-in, a new token): the check in flight is then for a session that is gone.
    async fn moved_from(&self, token: &str, turn: SessionGeneration) {
        loop {
            let still = self
                .with_session(|session, now| now == turn && session.token.as_str() == token)
                .unwrap_or(false);
            if !still {
                return;
            }
            tokio::time::sleep(MOVED_POLL).await;
        }
    }

    /// The watch's one read of the session in memory: `with` runs on it in one hold of its lock, with the session's
    /// generation of now. `None`: no session in memory. The generation moves before a session is replaced or
    /// removed, under the engine's session-write lock, so a caller compares the token as well as the generation.
    fn with_session<T>(&self, with: impl FnOnce(&Session, SessionGeneration) -> T) -> Option<T> {
        let session = self
            .acct
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        session
            .as_ref()
            .map(|session| with(session, self.acct.session_generation()))
    }
}

#[cfg(test)]
#[path = "session_watch_tests.rs"]
mod tests;
