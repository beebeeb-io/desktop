//! The No plan gate's memory: what the server last said about the plan, for the session in memory.
//!
//! The server is the only authority (`GET /api/v1/billing/subscription`, its `plan_state`). A call that does not
//! answer (offline, a 5xx, an answer that does not read) changes nothing: the last known state stays, and with none
//! known the gate never blocks. In memory only, and tied to the session generation, so a sign-out or another
//! account never inherits it.

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::signin::gate::has_no_plan;

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

    /// Ask the server for the plan of the unlocked session. `again` false: only once per session (the end of each
    /// transition asks; a server that is down costs one try, not one per transition). Skipped inside an async
    /// context, where the blocking call is not allowed, and when nobody is unlocked.
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
        let Ok(stamped) = self.account_subscription() else {
            return;
        };
        let plan = &stamped.value;
        let none = has_no_plan(
            plan.plan_state.as_deref(),
            Some(plan.plan.as_str()),
            Some(plan.status.as_str()),
            Some(plan.quota_bytes),
        );
        self.plan.lock().unwrap_or_else(|p| p.into_inner()).answer = Some((stamped.session, none));
    }
}
