//! What one finished operation means: `on_done`, `on_observed` and the attempt budget's
//! accounting. A finished check hands its landing to `landing`.
//!
//! Must not: choose the next operation (`decide`). Tests: `tests/steps.rs`.

use super::*;

pub(super) fn on_done(s: &mut CoreState, result: OpResult, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    match (s.phase, result) {
        (Phase::Idle, OpResult::Domain(read)) => match (s.setup, read) {
            // The UserDisabled poll. It never adds; a flip to enabled runs exactly one check.
            (FinderSetup::UserDisabled, Ok(DomainState::Enabled)) => {
                fx.push(Effect::TriggerReceived(Trigger::UserEnabledFlipped));
                if !s.held {
                    start_check(s, now);
                }
            }
            // The Ready poll: turned off in System Settings while Beebeeb runs. Not a check, so it adds nothing and
            // stops nothing: the running engine belongs to the signed-in session, and the flip back on runs the one
            // check that confirms the domain and reuses that engine. Any other answer (still on, not listed, a failed
            // read) is no evidence of anything; a missing domain is re-added by the next launch (§5.5).
            (FinderSetup::Ready, Ok(DomainState::Disabled)) if !s.held => {
                s.last_error = None;
                set_state(
                    s,
                    FinderSetup::UserDisabled,
                    Some(FinderFailureReason::UserDisabled),
                    None,
                    fx,
                );
                s.poll_due = Some(now + policy.user_disabled_poll);
            }
            _ => {}
        },
        (Phase::Removing { requested, then_check }, OpResult::Removed(result, _)) => {
            let error = result.as_ref().err().cloned();
            let removed = error.is_none();
            s.phase = Phase::Idle;
            s.check_started = None;
            match error {
                // An owed removal that the check itself asked for (`then_check` without `requested`) failed: the person is
                // signed in and unlocked, the previous domain is still registered, and no engine may start before it is
                // gone. That is a failure with one sentence and "Try again" (which removes once more), never a resting
                // `Missing` (§13.1). One attempt per trigger: nothing runs again until the next one.
                Some(error) if then_check && !requested => {
                    s.reason = Some(policy::classify(&error));
                    s.last_error = Some(error);
                    s.max_attempts = 1;
                    land_terminal(s, Landing::Failed, now, policy, fx);
                }
                error => {
                    let reason = missing_reason(s.launch);
                    set_state(s, FinderSetup::Missing, reason, error, fx);
                }
            }
            if removed {
                fx.push(Effect::DomainRemovalConfirmed);
            }
            if requested {
                fx.push(Effect::RemoveFinished(result));
            }
            // Repair (`requested`) always runs its one fresh check. A removal the check itself asked for (an owed
            // one, `then_check` without `requested`) runs the fresh check only when the removal worked: a failed
            // one would otherwise be observed again and removed again, and again.
            let check_after = then_check && (requested || removed);
            if (check_after || s.check_again) && !s.held {
                s.check_again = false;
                start_check(s, now);
            }
        }
        (Phase::Running(step), result) => on_check_result(s, step, result, now, policy, fx),
        (phase, result) => tracing::warn!(?phase, ?result, "finder setup: unexpected result, ignored"),
    }
}

pub(super) fn on_check_result(
    s: &mut CoreState,
    step: Step,
    result: OpResult,
    now: Instant,
    policy: &RetryPolicy,
    fx: &mut Vec<Effect>,
) {
    match (step, result) {
        (Step::Observe, OpResult::Observed(observation)) => on_observed(s, observation, now, policy, fx),
        (Step::StartEngine { add }, OpResult::EngineStarted(Ok(newly_started))) => {
            s.check_started_engine |= newly_started;
            s.phase = Phase::Running(if add {
                Step::Add
            } else {
                Step::Wait(Wait::ConfirmExisting)
            });
        }
        (Step::Add, OpResult::Added(Ok(()))) => {
            fx.push(Effect::DomainAddConfirmed);
            s.phase = Phase::Running(Step::ReadAfterAdd);
        }
        (Step::ReadAfterAdd, OpResult::Domain(Ok(DomainState::Disabled))) => {
            s.reason = Some(FinderFailureReason::UserDisabled);
            end_check(s, Landing::UserDisabled, now, policy, fx);
        }
        // Enabled, not listed yet, or a failed read: wait for stabilization. A failed read is
        // never evidence of anything (the 1524 rule `decide_install_step` used to pin).
        (Step::ReadAfterAdd, OpResult::Domain(_)) => s.phase = Phase::Running(Step::Wait(Wait::AfterAdd)),
        (Step::Wait(_), OpResult::Stable(Ok(()))) => s.phase = Phase::Running(Step::Finish),
        (Step::Finish, OpResult::Finished(Ok(()))) => land_ready(s, now, policy, fx),
        (Step::StopEngine(landing), OpResult::EngineStopped(stopped)) => {
            match stopped {
                Ok(()) => s.check_started_engine = false,
                // The stop could not take the engine slot, or could not confirm the engine task
                // ended: the engine may still be alive, so this never says it stopped
                // (`check_started_engine` stays). The check still lands with its OWN reason, since a
                // stop failure is not why Finder setup failed; the failure goes to the lifecycle log
                // the way a failed attempt does, and changes nothing a person sees.
                Err(error) => fx.push(Effect::Transition(Transition {
                    from: s.setup,
                    to: s.setup,
                    reason: Some(policy::classify(&error)),
                    error: Some(error),
                    attempt: s.attempt,
                    max_attempts: s.max_attempts.max(s.attempt),
                })),
            }
            land_terminal(s, landing, now, policy, fx);
        }
        (
            _,
            OpResult::EngineStarted(Err(error))
            | OpResult::Added(Err(error))
            | OpResult::Stable(Err(error))
            | OpResult::Finished(Err(error)),
        ) => attempt_failed(s, error, now, policy, fx),
        (step, result) => tracing::warn!(?step, ?result, "finder setup: unexpected result, ignored"),
    }
}

pub(super) fn on_observed(s: &mut CoreState, observation: Observation, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    let Observation { facts, domain } = observation;
    let wanted = wanted(facts);
    if wanted == Wanted::Present && facts.signed_out_by_choice {
        fx.push(Effect::PersistSignedOutByChoice(false));
    }
    // The domain is not there, so whatever was owed has been paid.
    if facts.removal_owed && domain == Ok(DomainState::NotRegistered) {
        fx.push(Effect::DomainRemovalConfirmed);
    }
    match (wanted, domain) {
        (Wanted::Absent, Ok(DomainState::Enabled | DomainState::Disabled)) => {
            s.phase = Phase::Removing {
                requested: false,
                then_check: false,
            };
        }
        // A removal is owed and the previous account's domain is still there: remove it before anything else
        // (the engine start refuses until it is gone), then run a fresh check.
        (Wanted::Present, Ok(DomainState::Enabled | DomainState::Disabled)) if facts.removal_owed => {
            s.phase = Phase::Removing {
                requested: false,
                then_check: true,
            };
        }
        (Wanted::Absent, Err(error)) => land_missing(s, Some(error), now, fx),
        (Wanted::Absent, Ok(DomainState::NotRegistered)) | (Wanted::NoAction, _) => land_missing(s, None, now, fx),
        (Wanted::Present, Ok(DomainState::Disabled)) => {
            s.reason = Some(FinderFailureReason::UserDisabled);
            end_check(s, Landing::UserDisabled, now, policy, fx);
        }
        (Wanted::Present, Ok(DomainState::Enabled)) => {
            if !matches!(s.setup, FinderSetup::Ready | FinderSetup::Adding) {
                set_state(s, FinderSetup::Adding, None, None, fx);
            }
            s.phase = Phase::Running(Step::StartEngine { add: false });
        }
        (Wanted::Present, Ok(DomainState::NotRegistered)) => {
            if s.launch.allows_add() {
                if s.setup != FinderSetup::Adding {
                    set_state(s, FinderSetup::Adding, None, None, fx);
                }
                s.phase = Phase::Running(Step::StartEngine { add: true });
            } else {
                s.reason = Some(FinderFailureReason::NotInApplications);
                s.max_attempts = 1;
                s.last_error = Some(FpError::app(
                    app_code::LAUNCH_LOCATION,
                    "Beebeeb was opened from a disk image or a translocated copy",
                ));
                end_check(s, Landing::Failed, now, policy, fx);
            }
        }
        (Wanted::Present, Err(error)) => attempt_failed(s, error, now, policy, fx),
    }
}

pub(super) fn attempt_failed(s: &mut CoreState, error: FpError, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    let reason = policy::classify(&error);
    s.reason = Some(reason);
    s.max_attempts = policy.max_attempts(reason);
    s.last_error = Some(error.clone());
    if reason == FinderFailureReason::UserDisabled {
        return end_check(s, Landing::UserDisabled, now, policy, fx);
    }
    let started = s.check_started.unwrap_or(now);
    match policy.next_attempt_at(reason, s.attempt, started) {
        Some(at) => {
            // Adding → Adding: a silent retry, logged with its reason and attempt n of N.
            fx.push(Effect::Transition(Transition {
                from: s.setup,
                to: FinderSetup::Adding,
                reason: Some(reason),
                error: Some(error),
                attempt: s.attempt,
                max_attempts: s.max_attempts,
            }));
            s.setup = FinderSetup::Adding;
            fx.push(Effect::Publish);
            s.phase = Phase::Backoff(at.max(now));
        }
        None => end_check(s, Landing::Failed, now, policy, fx),
    }
}

/// Stop the engine this check started (if any), then land.
pub(super) fn end_check(s: &mut CoreState, landing: Landing, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    if s.check_started_engine {
        s.phase = Phase::Running(Step::StopEngine(landing));
    } else {
        land_terminal(s, landing, now, policy, fx);
    }
}
