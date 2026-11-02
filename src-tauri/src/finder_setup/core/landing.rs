//! Where a check ends: the terminal `FinderSetup` states, the persisted failure, the hold
//! that a sign-out or Lock sets, and `finish_check`.
//!
//! Must not: run an operation (`decide` asks for it) or read the OS.
//! Tests: `tests/steps.rs`, `tests/removal.rs`.

use super::*;

pub(super) fn land_terminal(s: &mut CoreState, landing: Landing, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    match landing {
        Landing::Failed => {
            let reason = s.reason.unwrap_or(FinderFailureReason::Unknown);
            let error = s
                .last_error
                .clone()
                .unwrap_or_else(|| FpError::app(app_code::UNEXPECTED_BRIDGE_RETURN, "failed without an error"));
            fx.push(Effect::PersistFailure(Some((reason, error.clone()))));
            set_state(s, FinderSetup::Failed, Some(reason), Some(error), fx);
        }
        Landing::UserDisabled => {
            let error = s.last_error.clone();
            set_state(
                s,
                FinderSetup::UserDisabled,
                Some(FinderFailureReason::UserDisabled),
                error,
                fx,
            );
            s.poll_due = Some(now + policy.user_disabled_poll);
        }
    }
    finish_check(s, now);
}

pub(super) fn land_ready(s: &mut CoreState, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    s.last_error = None;
    s.check_started_engine = false; // the engine now belongs to the signed-in session
    fx.push(Effect::PersistFailure(None));
    set_state(s, FinderSetup::Ready, None, None, fx);
    s.poll_due = Some(now + policy.user_disabled_poll);
    finish_check(s, now);
}

pub(super) fn land_missing(s: &mut CoreState, error: Option<FpError>, now: Instant, fx: &mut Vec<Effect>) {
    let reason = missing_reason(s.launch);
    set_state(s, FinderSetup::Missing, reason, error, fx);
    finish_check(s, now);
}

pub(super) fn finish_check(s: &mut CoreState, now: Instant) {
    s.phase = Phase::Idle;
    s.check_started = None;
    if s.check_again && !s.held {
        s.check_again = false;
        start_check(s, now);
    }
}

/// Every transition writes one log line and publishes once (§5.5 step 6).
pub(super) fn set_state(
    s: &mut CoreState,
    to: FinderSetup,
    reason: Option<FinderFailureReason>,
    error: Option<FpError>,
    fx: &mut Vec<Effect>,
) {
    if s.setup == to && s.reason == reason {
        return;
    }
    fx.push(Effect::Transition(Transition {
        from: s.setup,
        to,
        reason,
        error,
        attempt: s.attempt,
        max_attempts: s.max_attempts.max(s.attempt),
    }));
    s.setup = to;
    s.reason = reason;
    fx.push(Effect::Publish);
}
