//! The No plan gate's memory: what the server last said about the plan, for the session in memory.
//!
//! The server is the only authority (`GET /api/v1/billing/subscription`, its `plan_state`). A call that does not
//! answer (offline, a 5xx, an answer that does not read) changes nothing: the last known state stays, and with none
//! known the gate never blocks. In memory only, and tied to the session generation, so a sign-out or another
//! account never inherits it.

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::signin::gate::{GateState, has_no_plan};

#[derive(Default)]
pub(super) struct PlanKnown {
    /// The session whose plan was asked once at its start, answered or not (offline asks again on Check again).
    asked: Option<SessionGeneration>,
    /// The last answer the server gave for a session: whether it has no active plan.
    answer: Option<(SessionGeneration, bool)>,
}

impl<C: CredentialStores> Engine<C> {
    pub(super) fn plan_known_none(&self) -> bool {
        let now = self.acct.session_generation();
        let known = self.plan.lock().unwrap_or_else(|p| p.into_inner());
        matches!(known.answer, Some((generation, none)) if generation == now && none)
    }

    /// Ask the server for the plan of the unlocked session. `again` false (the end of each transition, so the start
    /// path): only once per session, IN THE BACKGROUND with a timeout, so a slow or silent server never holds up a
    /// start or the first pull; the answer is applied when it lands. `again` true (Check again, a button the person
    /// pressed): asks now and waits. Skipped inside an async context, where the blocking call is not allowed, and
    /// when nobody is unlocked.
    pub(super) fn ask_plan(&self, again: bool) {
        if tokio::runtime::Handle::try_current().is_ok() || !self.unlocked() {
            return;
        }
        let generation = self.acct.session_generation();
        {
            let mut known = self.plan.lock().unwrap_or_else(|p| p.into_inner());
            if !again && known.asked == Some(generation) {
                return;
            }
            known.asked = Some(generation);
        }
        if again {
            let Ok(stamped) = self.account_subscription() else {
                return;
            };
            self.plan.lock().unwrap_or_else(|p| p.into_inner()).answer =
                Some((stamped.session, none_of(&stamped.value)));
            return;
        }
        let Ok((mut token, session)) = self.token_of_this_session() else {
            return;
        };
        let api = ApiClient::new(self.config.api_base.clone(), std::mem::take(&mut *token), [0; 32]);
        let (plan, acct, view, host) = (
            self.plan.clone(),
            self.acct.clone(),
            self.view.clone(),
            self.host.clone(),
        );
        self.runtime.spawn(async move {
            // No answer in time changes nothing: the gate never blocks without one.
            let Ok(Ok(subscription)) = tokio::time::timeout(PLAN_TIMEOUT, api.subscription()).await else {
                return;
            };
            let none = none_of(&subscription);
            let mut known = plan.lock().unwrap_or_else(|p| p.into_inner());
            // The answer of an earlier session is not this one's.
            if acct.session_generation() != session {
                return;
            }
            known.answer = Some((session, none));
            drop(known);
            // The gate was Ready when the transition published; a no-plan answer blocks Files now.
            let mut changed = false;
            view.write(|v| {
                if none && v.gate == GateState::Ready {
                    v.gate = GateState::NoPlan;
                    changed = true;
                }
            });
            if changed {
                host.wake();
            }
        });
    }
}

/// How long the background plan question waits before it gives up (the safe default: not blocked).
const PLAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn none_of(plan: &crate::account_dto::Subscription) -> bool {
    has_no_plan(
        plan.plan_state.as_deref(),
        Some(plan.plan.as_str()),
        Some(plan.status.as_str()),
        Some(plan.quota_bytes),
    )
}
