//! What arrives and what it asks for next: `step`, `next_op`, the trigger and tick handling,
//! and starting a check (spec §5.1, §5.3).
//!
//! Must not: run an operation or write anything; it only returns effects.
//! Tests: `tests/steps.rs`, `tests/retry.rs`, `tests/poll.rs`.

use super::*;

/// §7: the read-only `userEnabled` poll (every `user_disabled_poll` while a window is visible, plus once when the app
/// becomes active) runs while `UserDisabled`, to notice it turned back on, and while `Ready`, to notice it turned off
/// in System Settings while Beebeeb runs. Never while held. Only between checks: the caller checks the phase.
/// While `Ready`, the usual resting state, its timer also stops once a tick finds no window on screen, so the app is
/// not woken every 3 s with every window hidden; the app becoming active reads once and sets it again (`on_tick`).
pub(super) fn polls(s: &CoreState) -> bool {
    !s.held && matches!(s.setup, FinderSetup::UserDisabled | FinderSetup::Ready)
}

/// From a disk image or a translocated copy the reason is known before any check (§6.2:
/// "exposed in `finder_setup_state` before sign-in").
pub(super) fn missing_reason(launch: LaunchLocation) -> Option<FinderFailureReason> {
    if launch.allows_add() {
        None
    } else {
        Some(FinderFailureReason::NotInApplications)
    }
}

pub fn step(mut state: CoreState, input: Input, now: Instant, policy: &RetryPolicy) -> (CoreState, Vec<Effect>) {
    let mut fx = Vec::new();
    match input {
        Input::Trigger(trigger) => on_trigger(&mut state, trigger, now, &mut fx),
        Input::Tick { window_visible } => on_tick(&mut state, window_visible, now, policy),
        Input::AppActivated => {
            if state.phase == Phase::Idle && polls(&state) {
                state.poll_pending = true;
                // A `Ready` poll whose timer stopped while no window was on screen starts again here.
                if state.poll_due.is_none() {
                    state.poll_due = Some(now + policy.user_disabled_poll);
                }
            }
        }
        Input::Done(result) => {
            state.awaiting = false;
            on_done(&mut state, result, now, policy, &mut fx);
        }
    }
    (state, fx)
}

pub fn next_op(state: &mut CoreState, now: Instant, policy: &RetryPolicy) -> Next {
    if state.awaiting {
        tracing::warn!("finder setup: asked for the next operation while one is running");
        return Next::Idle;
    }
    let op = match state.phase {
        Phase::Removing { .. } => Op::RemoveDomain,
        Phase::Running(step) => match step {
            Step::Observe => Op::Observe,
            Step::StartEngine { .. } => Op::StartEngine,
            Step::Add => Op::AddDomain,
            Step::ReadAfterAdd => Op::ReadDomain,
            Step::Wait(Wait::AfterAdd) => Op::WaitStable(policy.stabilize_after_add),
            Step::Wait(Wait::ConfirmExisting) => Op::WaitStable(policy.confirm_existing),
            Step::Finish => Op::FinishReady,
            Step::StopEngine(_) => Op::StopEngine,
        },
        Phase::Backoff(until) => {
            if now < until {
                return Next::WakeAt(until);
            }
            state.attempt = state.attempt.saturating_add(1);
            state.phase = Phase::Running(Step::Observe);
            Op::Observe
        }
        Phase::Idle => {
            if state.poll_pending {
                state.poll_pending = false;
                Op::ReadDomain
            } else if polls(state) {
                // While held there is no poll, so no timer either: a `WakeAt` in the past that
                // `on_tick` ignores would spin the driver. A `Ready` poll with no window on screen has
                // no timer either (`on_tick`): only an event (the app becoming active, a trigger) wakes it.
                return state.poll_due.map_or(Next::Idle, Next::WakeAt);
            } else {
                return Next::Idle;
            }
        }
    };
    state.awaiting = true;
    Next::Run(op)
}

pub(super) fn on_trigger(s: &mut CoreState, trigger: Trigger, now: Instant, fx: &mut Vec<Effect>) {
    fx.push(Effect::TriggerReceived(trigger));
    match trigger {
        Trigger::SignOut => {
            s.held = true;
            s.poll_pending = false;
            s.check_again = false;
            s.check_started_engine = false; // clear_session_impl stops the engine itself
            // Sign-out tells this reconciler FIRST, before anything else, and only then stops the engine, so the
            // reconciler cannot start one in the gap. It skips the engine stop when nothing is signed in (no auth
            // flag, no session in memory and no Keychain session), and it refuses, leaving the engine in its slot,
            // while an earlier stop was never confirmed: then every start refuses until a restart, and the
            // restart's `Launch` re-adds Finder. Every engine start reads its keys under the engine slot
            // (`spawn_bound_engine`), so a start that waited for the slot behind a sign-out finds none.
            // How a sign-in and a sign-out are ordered against each other is Task 12's.
            fx.push(Effect::PersistSignedOutByChoice(true));
            s.phase = Phase::Removing {
                requested: true,
                then_check: false,
            };
        }
        Trigger::Repair => {
            s.check_again = false;
            s.check_started_engine = false; // reset_macos_integration stops the engine itself
            // `reset_macos_integration` takes the engine slot and stops the engine BEFORE it sends this trigger,
            // so a check whose `FinishReady` is in flight can start one more engine before the removal; it
            // belongs to the signed-in session, and the check that follows the removal reuses it (`StartEngine`
            // answers `Ok(false)`). Every stop goes through `stop_engine_in_slot` (lib.rs), which sets the flag
            // that makes a start refuse when a stop could not be confirmed, and Repair then adds a "Restart
            // Beebeeb" warning. The engine start's account reset (`bind_data_to_session`) sends this trigger too,
            // without stopping anything: the engine it leaves belongs to the session as well.
            s.phase = Phase::Removing {
                requested: true,
                then_check: true,
            };
        }
        Trigger::Lock => {
            s.held = true;
            s.poll_pending = false;
            s.check_again = false;
            if matches!(s.phase, Phase::Running(_) | Phase::Backoff(_)) {
                // No operation is in flight (the driver feeds events only between operations);
                // lock_vault stops any engine this check started right after this returns.
                // VERIFIED (Task 9, 2026-10-07): `lock_vault` sends `Event::Lock` and waits for its
                // acknowledgement, which the driver sends after this runs, then takes the engine slot
                // and stops whatever is in it. If the acknowledgement never comes, `lock_vault` still
                // stops the engine and clears the keys, and a late `StartEngine` reads the keys under
                // the slot and finds none, so no engine starts.
                s.phase = Phase::Idle;
                s.check_started = None;
                s.check_started_engine = false;
                if s.setup == FinderSetup::Adding {
                    let reason = missing_reason(s.launch);
                    set_state(s, FinderSetup::Missing, reason, None, fx);
                }
            }
        }
        Trigger::Launch | Trigger::KeysArrived => {
            s.held = false;
            request_check(s, now);
        }
        Trigger::TryAgain | Trigger::UserEnabledFlipped => {
            if !s.held {
                request_check(s, now);
            }
        }
    }
}

pub(super) fn on_tick(s: &mut CoreState, window_visible: bool, now: Instant, policy: &RetryPolicy) {
    // Backoff expiry needs nothing here: `next_op` compares `now` with the deadline.
    // Held (sign-out or lock): no poll until keys arrive, or it would log a flip it cannot act on.
    if s.phase != Phase::Idle || !polls(s) {
        return;
    }
    if s.poll_due.is_some_and(|due| now >= due) {
        if window_visible {
            s.poll_due = Some(now + policy.user_disabled_poll);
            s.poll_pending = true;
        } else if s.setup == FinderSetup::Ready {
            // No Beebeeb window on screen: no read, and no next wake-up either. `Ready` is where the app rests, so a
            // timer here would wake it every 3 s for as long as it runs. A window becoming visible gains focus (every
            // `show()` is followed by `set_focus()`), and `AppActivated` reads once and sets the timer again.
            s.poll_due = None;
        } else {
            s.poll_due = Some(now + policy.user_disabled_poll);
        }
    }
}

pub(super) fn request_check(s: &mut CoreState, now: Instant) {
    if s.phase == Phase::Idle {
        start_check(s, now);
    } else {
        s.check_again = true;
    }
}

pub(super) fn start_check(s: &mut CoreState, now: Instant) {
    s.phase = Phase::Running(Step::Observe);
    s.check_started = Some(now);
    s.check_started_engine = false;
    s.attempt = 1;
    s.max_attempts = 0;
    s.poll_pending = false;
}
