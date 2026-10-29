//! Spec 2026-10-06 §5 (`finder_setup::core`): what the Finder reconciler decides. Pure: no OS,
//! no clock, no I/O. The driver feeds one `Input` at a time and runs the one `Op` that
//! `next_op` asks for. While that op runs nothing else is fed (events wait in the driver's
//! channel), so an add and a remove can never overlap (§5.4).

use std::time::{Duration, Instant};

use super::error::{FpError, app_code};
use super::launch_location::LaunchLocation;
use super::policy::{self, RetryPolicy};
use crate::finder_removal::KeptFolder;
use crate::surfaces::phase::{FinderFailureReason, FinderSetup};

/// What starts a check (§5.3), the two removals, and lock. Also the lifecycle log's
/// "trigger received" vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Launch,
    KeysArrived,
    UserEnabledFlipped,
    TryAgain,
    SignOut,
    Repair,
    Lock,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::KeysArrived => "keys_arrived",
            Self::UserEnabledFlipped => "user_enabled_flipped",
            Self::TryAgain => "try_again",
            Self::SignOut => "sign_out",
            Self::Repair => "repair",
            Self::Lock => "lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    Present,
    Absent,
    NoAction,
}

/// The facts `Wanted` is derived from (§5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionFacts {
    /// This Mac has the keys in memory (`AccountRuntime::session` is `Some`).
    pub vault_unlocked: bool,
    /// A session token is stored (`AppState::auth_present`).
    pub auth_present: bool,
    /// The last sign-out was the person's choice and no sign-in followed
    /// (`DesktopConfig::finder_signed_out_by_choice`). A startup 401 that discards a revoked
    /// token does NOT set it, so ruling R2 keeps Beebeeb in Finder (plan "Spec issues" 2).
    pub signed_out_by_choice: bool,
    /// A domain removal is OWED before any engine may start (task 1834 fix round 1, F3): an account reset on this
    /// Mac, or a sign-out whose removal was not confirmed, left the Finder domain of the previous account in
    /// place. Persisted in `state.db`; the engine start refuses while it is set. Only a confirmed removal, or an
    /// `Observe` that finds the domain absent, releases it (`Effect::DomainRemovalConfirmed`).
    pub removal_owed: bool,
}

/// §5.1. A revoked session (`auth_expired`) is not an input at all: with the keys in memory it
/// is `Present` (R2).
pub fn wanted(facts: SessionFacts) -> Wanted {
    if facts.vault_unlocked {
        Wanted::Present
    } else if !facts.auth_present && facts.signed_out_by_choice {
        Wanted::Absent
    } else {
        Wanted::NoAction
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainState {
    Enabled,
    Disabled,
    NotRegistered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub facts: SessionFacts,
    pub domain: Result<DomainState, FpError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Observe,
    StartEngine,
    AddDomain,
    ReadDomain,
    WaitStable(Duration),
    FinishReady,
    StopEngine,
    RemoveDomain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpResult {
    Observed(Observation),
    /// `Ok(true)`: this op started a new engine; `Ok(false)`: one was already running.
    EngineStarted(Result<bool, FpError>),
    Added(Result<(), FpError>),
    Domain(Result<DomainState, FpError>),
    Stable(Result<(), FpError>),
    Finished(Result<(), FpError>),
    /// `Err`: the stop could not take the engine slot, or could not confirm the engine task
    /// ended. The engine may still be alive; the core never claims otherwise.
    EngineStopped(Result<(), FpError>),
    /// The removal's answer, and (task 1882) what macOS kept of the files that had not reached the server, also
    /// when the removal failed (review M2). The core decides nothing from the folder; the driver hands it on.
    Removed(Result<(), FpError>, KeptFolder),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Trigger(Trigger),
    /// A timer the core asked for (`Next::WakeAt`) fired.
    Tick {
        window_visible: bool,
    },
    AppActivated,
    Done(OpResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub from: FinderSetup,
    pub to: FinderSetup,
    pub reason: Option<FinderFailureReason>,
    pub error: Option<FpError>,
    pub attempt: u8,
    pub max_attempts: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// The published view changed: emit `finder-setup-changed`.
    Publish,
    TriggerReceived(Trigger),
    Transition(Transition),
    /// `Some` when a check ends in `Failed`, `None` when it ends in `Ready`.
    PersistFailure(Option<(FinderFailureReason, FpError)>),
    PersistSignedOutByChoice(bool),
    /// A removal requested by sign-out or Repair ran: resolve whoever waits for it.
    RemoveFinished(Result<(), FpError>),
    /// The domain is confirmed gone: a removal succeeded, or an `Observe` found it absent while a removal was owed.
    /// The driver clears the persisted "removal owed" flag (the engine start waits on it).
    DomainRemovalConfirmed,
    /// `addDomain` succeeded: the domain is registered again, so it is no longer known gone. The driver clears the
    /// persisted "domain gone" mark that a confirmed sign-out left.
    DomainAddConfirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Run(Op),
    WakeAt(Instant),
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wait {
    AfterAdd,
    ConfirmExisting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landing {
    Failed,
    UserDisabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Observe,
    StartEngine { add: bool },
    Add,
    ReadAfterAdd,
    Wait(Wait),
    Finish,
    StopEngine(Landing),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Running(Step),
    Backoff(Instant),
    /// `requested`: sign-out or Repair waits for it. `then_check`: Repair's "then one fresh check".
    Removing {
        requested: bool,
        then_check: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreState {
    pub setup: FinderSetup,
    pub reason: Option<FinderFailureReason>,
    pub last_error: Option<FpError>,
    pub attempt: u8,
    pub max_attempts: u8,
    pub launch: LaunchLocation,
    phase: Phase,
    awaiting: bool,
    check_started: Option<Instant>,
    check_started_engine: bool,
    /// A trigger arrived during a check: run exactly one more afterwards (§5.4).
    check_again: bool,
    /// Set by sign-out and lock; only `Launch` or `KeysArrived` clears it. While held no check
    /// starts, so nothing re-adds the domain or starts the engine in the gap between the
    /// reconciler finishing and the caller clearing the session. The poll (see [`polls`]) is off
    /// while held.
    held: bool,
    poll_due: Option<Instant>,
    poll_pending: bool,
}

impl CoreState {
    pub fn new(launch: LaunchLocation) -> Self {
        Self {
            setup: FinderSetup::Missing,
            reason: missing_reason(launch),
            last_error: None,
            attempt: 0,
            max_attempts: 0,
            launch,
            phase: Phase::Idle,
            awaiting: false,
            check_started: None,
            check_started_engine: false,
            check_again: false,
            held: false,
            poll_due: None,
            poll_pending: false,
        }
    }

    /// Held by a sign-out or a Lock until keys arrive again: no check starts, and "Try again" does nothing.
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Nothing runs and nothing is scheduled except, possibly, the poll.
    pub fn is_settled(&self) -> bool {
        self.phase == Phase::Idle && !self.awaiting && !self.poll_pending
    }
}

/// §7: the read-only `userEnabled` poll (every `user_disabled_poll` while a window is visible, plus once when the app
/// becomes active) runs while `UserDisabled`, to notice it turned back on, and while `Ready`, to notice it turned off
/// in System Settings while Beebeeb runs. Never while held. Only between checks: the caller checks the phase.
/// While `Ready`, the usual resting state, its timer also stops once a tick finds no window on screen, so the app is
/// not woken every 3 s with every window hidden; the app becoming active reads once and sets it again (`on_tick`).
fn polls(s: &CoreState) -> bool {
    !s.held && matches!(s.setup, FinderSetup::UserDisabled | FinderSetup::Ready)
}

/// From a disk image or a translocated copy the reason is known before any check (§6.2:
/// "exposed in `finder_setup_state` before sign-in").
fn missing_reason(launch: LaunchLocation) -> Option<FinderFailureReason> {
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

fn on_trigger(s: &mut CoreState, trigger: Trigger, now: Instant, fx: &mut Vec<Effect>) {
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

fn on_tick(s: &mut CoreState, window_visible: bool, now: Instant, policy: &RetryPolicy) {
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

fn request_check(s: &mut CoreState, now: Instant) {
    if s.phase == Phase::Idle {
        start_check(s, now);
    } else {
        s.check_again = true;
    }
}

fn start_check(s: &mut CoreState, now: Instant) {
    s.phase = Phase::Running(Step::Observe);
    s.check_started = Some(now);
    s.check_started_engine = false;
    s.attempt = 1;
    s.max_attempts = 0;
    s.poll_pending = false;
}

fn on_done(s: &mut CoreState, result: OpResult, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
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

fn on_check_result(
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

fn on_observed(s: &mut CoreState, observation: Observation, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
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

fn attempt_failed(s: &mut CoreState, error: FpError, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
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
fn end_check(s: &mut CoreState, landing: Landing, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    if s.check_started_engine {
        s.phase = Phase::Running(Step::StopEngine(landing));
    } else {
        land_terminal(s, landing, now, policy, fx);
    }
}

fn land_terminal(s: &mut CoreState, landing: Landing, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
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

fn land_ready(s: &mut CoreState, now: Instant, policy: &RetryPolicy, fx: &mut Vec<Effect>) {
    s.last_error = None;
    s.check_started_engine = false; // the engine now belongs to the signed-in session
    fx.push(Effect::PersistFailure(None));
    set_state(s, FinderSetup::Ready, None, None, fx);
    s.poll_due = Some(now + policy.user_disabled_poll);
    finish_check(s, now);
}

fn land_missing(s: &mut CoreState, error: Option<FpError>, now: Instant, fx: &mut Vec<Effect>) {
    let reason = missing_reason(s.launch);
    set_state(s, FinderSetup::Missing, reason, error, fx);
    finish_check(s, now);
}

fn finish_check(s: &mut CoreState, now: Instant) {
    s.phase = Phase::Idle;
    s.check_started = None;
    if s.check_again && !s.held {
        s.check_again = false;
        start_check(s, now);
    }
}

/// Every transition writes one log line and publishes once (§5.5 step 6).
fn set_state(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::error::{BRIDGE_DOMAIN, COCOA_DOMAIN, FILE_PROVIDER_DOMAIN, bridge_code};
    use DomainState::{Disabled, Enabled, NotRegistered};
    use FinderSetup::{Adding, Failed, Missing, Ready, UserDisabled};
    use LaunchLocation::{Applications, DiskImage};

    const SIGNED_IN: SessionFacts = SessionFacts {
        vault_unlocked: true,
        auth_present: true,
        signed_out_by_choice: false,
        removal_owed: false,
    };
    const KEYS_MISSING: SessionFacts = SessionFacts {
        vault_unlocked: false,
        auth_present: true,
        signed_out_by_choice: false,
        removal_owed: false,
    };
    const SIGNED_OUT_BY_CHOICE: SessionFacts = SessionFacts {
        vault_unlocked: false,
        auth_present: false,
        signed_out_by_choice: true,
        removal_owed: false,
    };
    const SIGNED_OUT_BY_401: SessionFacts = SessionFacts {
        vault_unlocked: false,
        auth_present: false,
        signed_out_by_choice: false,
        removal_owed: false,
    };

    fn fp(code: i64) -> FpError {
        FpError::new(FILE_PROVIDER_DOMAIN, code, "fixture")
    }

    fn stabilization_timeout() -> FpError {
        FpError::new(BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT, "fixture")
    }

    /// What the fake OS answers. `wait_secs` is how long one `WaitStable` takes on the fake
    /// clock; every other op takes no time.
    #[derive(Clone)]
    struct World {
        facts: SessionFacts,
        domain: Result<DomainState, FpError>,
        add: Result<(), FpError>,
        after_add: Result<DomainState, FpError>,
        stable: Result<(), FpError>,
        /// What stopping the engine this check started answers.
        stop: Result<(), FpError>,
        /// What removing the domain answers.
        remove: Result<(), FpError>,
        wait_secs: u64,
    }

    impl World {
        fn ok(facts: SessionFacts, domain: DomainState) -> Self {
            Self {
                facts,
                domain: Ok(domain),
                add: Ok(()),
                after_add: Ok(Enabled),
                stable: Ok(()),
                stop: Ok(()),
                remove: Ok(()),
                wait_secs: 0,
            }
        }
    }

    fn answer(world: &World, op: Op) -> OpResult {
        match op {
            Op::Observe => OpResult::Observed(Observation {
                facts: world.facts,
                domain: world.domain.clone(),
            }),
            Op::StartEngine => OpResult::EngineStarted(Ok(true)),
            Op::AddDomain => OpResult::Added(world.add.clone()),
            Op::ReadDomain => OpResult::Domain(world.after_add.clone()),
            Op::WaitStable(_) => OpResult::Stable(world.stable.clone()),
            Op::FinishReady => OpResult::Finished(Ok(())),
            Op::StopEngine => OpResult::EngineStopped(world.stop.clone()),
            Op::RemoveDomain => OpResult::Removed(world.remove.clone(), KeptFolder::default()),
        }
    }

    /// A miniature driver on an injected clock.
    struct Sim {
        s: CoreState,
        t0: Instant,
        now: Instant,
        policy: RetryPolicy,
        ops: Vec<(u64, Op)>,
        fx: Vec<Effect>,
    }

    impl Sim {
        fn new(launch: LaunchLocation) -> Self {
            let t0 = Instant::now();
            Self {
                s: CoreState::new(launch),
                t0,
                now: t0,
                policy: RetryPolicy::default(),
                ops: Vec::new(),
                fx: Vec::new(),
            }
        }
        fn secs(&self) -> u64 {
            (self.now - self.t0).as_secs()
        }
        fn feed(&mut self, input: Input) {
            let (s, fx) = step(self.s.clone(), input, self.now, &self.policy);
            self.s = s;
            self.fx.extend(fx);
        }
        fn trigger(&mut self, trigger: Trigger) {
            self.feed(Input::Trigger(trigger));
        }
        fn next(&mut self) -> Next {
            next_op(&mut self.s, self.now, &self.policy)
        }
        fn step_op(&mut self, world: &World) -> Op {
            match self.next() {
                Next::Run(op) => {
                    self.ops.push((self.secs(), op));
                    let result = answer(world, op);
                    if matches!(op, Op::WaitStable(_)) {
                        self.now += Duration::from_secs(world.wait_secs);
                    }
                    self.feed(Input::Done(result));
                    op
                }
                other => panic!("expected an operation, got {other:?}"),
            }
        }
        /// Answer ops from `world` (and let backoff timers fire) until the core settles.
        fn run(&mut self, world: &World) {
            for _ in 0..500 {
                if self.s.is_settled() {
                    return;
                }
                match self.next() {
                    Next::Idle => return,
                    Next::WakeAt(at) => {
                        self.now = self.now.max(at);
                        self.feed(Input::Tick { window_visible: false });
                    }
                    Next::Run(op) => {
                        self.ops.push((self.secs(), op));
                        let result = answer(world, op);
                        if matches!(op, Op::WaitStable(_)) {
                            self.now += Duration::from_secs(world.wait_secs);
                        }
                        self.feed(Input::Done(result));
                    }
                }
            }
            panic!("the core did not settle: {:?}", self.s);
        }
        fn kinds(&self) -> Vec<Op> {
            self.ops.iter().map(|(_, op)| *op).collect()
        }
        fn count(&self, op: Op) -> usize {
            self.ops.iter().filter(|(_, o)| *o == op).count()
        }
        fn attempt_starts(&self) -> Vec<u64> {
            self.ops
                .iter()
                .filter(|(_, op)| *op == Op::Observe)
                .map(|(t, _)| *t)
                .collect()
        }
    }

    const WAIT_10: Op = Op::WaitStable(Duration::from_secs(10));
    const WAIT_2: Op = Op::WaitStable(Duration::from_secs(2));

    #[test]
    fn wanted_follows_section_5_1() {
        assert_eq!(wanted(SIGNED_IN), Wanted::Present);
        assert_eq!(
            wanted(SessionFacts {
                vault_unlocked: true,
                auth_present: false,
                signed_out_by_choice: true,
                removal_owed: false
            }),
            Wanted::Present
        );
        assert_eq!(wanted(KEYS_MISSING), Wanted::NoAction);
        assert_eq!(wanted(SIGNED_OUT_BY_CHOICE), Wanted::Absent);
        assert_eq!(
            wanted(SIGNED_OUT_BY_401),
            Wanted::NoAction,
            "R2: a startup 401 is not a sign-out by choice"
        );
    }

    #[test]
    fn present_and_not_registered_adds_then_lands_ready() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.kinds(),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                WAIT_10,
                Op::FinishReady
            ]
        );
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.s.reason, None);
        assert!(sim.fx.contains(&Effect::PersistFailure(None)));
    }

    #[test]
    fn present_and_registered_confirms_without_adding_and_ready_never_flickers() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.kinds(), vec![Op::Observe, Op::StartEngine, WAIT_2, Op::FinishReady]);
        assert_eq!(sim.s.setup, Ready);
        // A second launch-like trigger on a Ready domain never publishes Adding.
        sim.fx.clear();
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert!(
            !sim.fx
                .iter()
                .any(|e| matches!(e, Effect::Transition(t) if t.to == Adding)),
            "{:?}",
            sim.fx
        );
        assert_eq!(sim.count(Op::AddDomain), 0);
    }

    #[test]
    fn present_and_turned_off_lands_user_disabled_without_adding() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        assert_eq!(sim.kinds(), vec![Op::Observe]);
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.s.reason, Some(FinderFailureReason::UserDisabled));
    }

    #[test]
    fn turned_off_right_after_add_stops_the_engine_it_started() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            after_add: Ok(Disabled),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(
            sim.kinds(),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                Op::StopEngine
            ]
        );
        assert_eq!(sim.s.setup, UserDisabled);
    }

    /// Task 8 fix round 1: a stop that could not take the engine slot, or could not confirm the
    /// engine is gone, must not be reported as a stop. The core keeps "this check started an
    /// engine", still lands with the check's OWN reason (a person is never shown a stop failure as
    /// the cause), and writes the stop failure to the lifecycle log like any failed attempt.
    #[test]
    fn a_stop_that_did_not_finish_is_logged_never_claimed_and_the_check_still_lands_with_its_own_reason() {
        for stop_error in [
            FpError::app(
                app_code::ENGINE_STOP_UNCONFIRMED,
                "the sync engine did not confirm it stopped",
            ),
            FpError::app(app_code::OP_TIMEOUT, "StopEngine did not answer within 15s"),
        ] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::KeysArrived);
            sim.run(&World {
                after_add: Ok(Disabled),
                stop: Err(stop_error.clone()),
                ..World::ok(SIGNED_IN, NotRegistered)
            });
            assert_eq!(
                sim.kinds(),
                vec![
                    Op::Observe,
                    Op::StartEngine,
                    Op::AddDomain,
                    Op::ReadDomain,
                    Op::StopEngine
                ],
                "{stop_error:?}"
            );
            assert_eq!(
                (sim.s.setup, sim.s.reason),
                (UserDisabled, Some(FinderFailureReason::UserDisabled)),
                "the check's own reason"
            );
            assert!(
                sim.s.check_started_engine,
                "a stop that did not finish is not a stop: {stop_error:?}"
            );
            let logged: Vec<&Transition> = sim
                .fx
                .iter()
                .filter_map(|e| match e {
                    Effect::Transition(t) if t.error.as_ref() == Some(&stop_error) => Some(t),
                    _ => None,
                })
                .collect();
            assert_eq!(logged.len(), 1, "the stop failure is logged once: {:?}", sim.fx);
            assert_eq!(
                (logged[0].from, logged[0].to),
                (Adding, Adding),
                "log-only: it changes nothing a person sees"
            );
        }
        // The contrast: a stop that finished clears it, and logs nothing.
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            after_add: Ok(Disabled),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert!(!sim.s.check_started_engine);
        assert!(
            !sim.fx
                .iter()
                .any(|e| matches!(e, Effect::Transition(t) if t.from == t.to)),
            "{:?}",
            sim.fx
        );
    }

    #[test]
    fn domain_disabled_from_add_is_user_disabled_not_failed() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            add: Err(fp(-2011)),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.count(Op::AddDomain), 1, "-2011 is not retried");
    }

    #[test]
    fn a_failed_read_after_add_is_not_evidence_and_still_waits() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            after_add: Err(fp(-1000)),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(
            sim.kinds(),
            vec![
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                WAIT_10,
                Op::FinishReady
            ]
        );
        assert_eq!(sim.s.setup, Ready);
    }

    /// §13.1 "table tests for every trigger × every observation".
    #[test]
    fn every_trigger_against_every_observation_ends_in_a_state_a_person_can_act_on() {
        let observations: Vec<(&str, World, FinderSetup, usize)> = vec![
            // (name, world, final state, RemoveDomain count)
            (
                "signed in, not registered",
                World::ok(SIGNED_IN, NotRegistered),
                Ready,
                0,
            ),
            ("signed in, registered", World::ok(SIGNED_IN, Enabled), Ready, 0),
            ("signed in, turned off", World::ok(SIGNED_IN, Disabled), UserDisabled, 0),
            (
                "signed in, turned off right after add",
                World {
                    after_add: Ok(Disabled),
                    ..World::ok(SIGNED_IN, NotRegistered)
                },
                UserDisabled,
                0,
            ),
            (
                "signed in, folder taken",
                World {
                    add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")),
                    ..World::ok(SIGNED_IN, NotRegistered)
                },
                Failed,
                0,
            ),
            (
                "signed in, lookup keeps failing",
                World {
                    domain: Err(fp(-2001)),
                    ..World::ok(SIGNED_IN, Enabled)
                },
                Failed,
                0,
            ),
            (
                "signed in, never stable",
                World {
                    stable: Err(stabilization_timeout()),
                    ..World::ok(SIGNED_IN, NotRegistered)
                },
                Failed,
                0,
            ),
            (
                "signed in, the previous domain's owed removal fails",
                World {
                    remove: Err(fp(516)),
                    ..World::ok(SIGNED_IN_OWING, Enabled)
                },
                Failed,
                1,
            ),
            ("keys missing", World::ok(KEYS_MISSING, Enabled), Missing, 0),
            (
                "signed out by choice, still registered",
                World::ok(SIGNED_OUT_BY_CHOICE, Enabled),
                Missing,
                1,
            ),
            (
                "signed out by choice, gone",
                World::ok(SIGNED_OUT_BY_CHOICE, NotRegistered),
                Missing,
                0,
            ),
            (
                "revoked at startup (401), registered",
                World::ok(SIGNED_OUT_BY_401, Enabled),
                Missing,
                0,
            ),
        ];
        for trigger in [Trigger::Launch, Trigger::KeysArrived, Trigger::TryAgain] {
            for (name, world, expected, removes) in &observations {
                let mut sim = Sim::new(Applications);
                sim.trigger(trigger);
                sim.run(world);
                assert_eq!(sim.s.setup, *expected, "{trigger:?} / {name}");
                assert_eq!(
                    sim.count(Op::RemoveDomain),
                    *removes,
                    "{trigger:?} / {name}: removes only after a sign-out by choice or for an owed removal"
                );
                if wanted(world.facts) == Wanted::Present {
                    assert_ne!(
                        sim.s.setup, Missing,
                        "{trigger:?} / {name}: Missing is not a resting state while wanted is present"
                    );
                } else {
                    assert_eq!(
                        sim.count(Op::AddDomain),
                        0,
                        "{trigger:?} / {name}: never adds unless wanted"
                    );
                }
            }
        }
    }

    #[test]
    fn merged_triggers_produce_exactly_one_follow_up_check() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe
        for _ in 0..5 {
            sim.trigger(Trigger::TryAgain);
        }
        sim.trigger(Trigger::Launch);
        sim.run(&world);
        assert_eq!(
            sim.count(Op::Observe),
            2,
            "six triggers during one check → exactly one follow-up"
        );
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn sign_out_cancels_the_check_in_flight_removes_and_holds_until_keys_arrive() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe
        sim.step_op(&world); // StartEngine
        sim.step_op(&world); // AddDomain
        sim.trigger(Trigger::SignOut);
        sim.run(&world);
        assert_eq!(
            sim.kinds(),
            vec![Op::Observe, Op::StartEngine, Op::AddDomain, Op::RemoveDomain]
        );
        assert_eq!(sim.s.setup, Missing);
        assert!(sim.fx.contains(&Effect::RemoveFinished(Ok(()))));
        assert!(sim.fx.contains(&Effect::PersistSignedOutByChoice(true)));
        // Held: nothing re-adds, whatever else arrives.
        sim.trigger(Trigger::TryAgain);
        sim.feed(Input::AppActivated);
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Idle);
        // Keys arriving again lift the hold.
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Ready);
    }

    /// Repair always runs its one fresh check, also after a removal that failed (§5.3 (6)). Its failed removal is not a
    /// failure to show on its own: it is recorded on the way as `Missing` with its error, never as `Failed`, and the
    /// fresh check decides what the person sees.
    #[test]
    fn a_repair_whose_removal_fails_still_checks_once_and_never_lands_failed_on_the_way() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        sim.ops.clear();
        sim.fx.clear();
        let world = World {
            remove: Err(fp(516)),
            ..World::ok(SIGNED_IN, Enabled)
        };
        sim.trigger(Trigger::Repair);
        assert_eq!(sim.step_op(&world), Op::RemoveDomain);
        assert_eq!(sim.s.setup, Missing, "the failed removal is recorded on the way");
        assert!(
            !sim.fx
                .iter()
                .any(|effect| matches!(effect, Effect::PersistFailure(Some(_)))),
            "and is not a failure of its own: {:?}",
            sim.fx
        );
        sim.run(&world);
        assert_eq!(sim.count(Op::RemoveDomain), 1, "one removal");
        assert_eq!(sim.count(Op::Observe), 1, "then its one fresh check");
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn repair_removes_then_checks_exactly_once() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        sim.ops.clear();
        sim.trigger(Trigger::Repair);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.kinds(),
            vec![
                Op::RemoveDomain,
                Op::Observe,
                Op::StartEngine,
                Op::AddDomain,
                Op::ReadDomain,
                WAIT_10,
                Op::FinishReady
            ]
        );
        assert_eq!(sim.s.setup, Ready);
        assert!(sim.fx.contains(&Effect::RemoveFinished(Ok(()))));
    }

    // ---- Task 10 fix round 1 (F3): a removal that is OWED before any engine starts ----

    const SIGNED_IN_OWING: SessionFacts = SessionFacts {
        removal_owed: true,
        ..SIGNED_IN
    };

    /// A removal that completed is what releases a start that waits on it; one that failed is not. The core says
    /// so with an effect, and the driver turns it into the port call that clears the persisted flag.
    #[test]
    fn only_a_confirmed_removal_releases_the_owed_flag() {
        for (remove, confirmed) in [(Ok(()), true), (Err(fp(516)), false)] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::Repair);
            sim.run(&World {
                remove,
                ..World::ok(SIGNED_OUT_BY_401, NotRegistered)
            });
            assert_eq!(sim.fx.contains(&Effect::DomainRemovalConfirmed), confirmed);
        }
        // A sign-out removal is a removal too.
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::SignOut);
        sim.run(&World::ok(SIGNED_OUT_BY_CHOICE, NotRegistered));
        assert!(sim.fx.contains(&Effect::DomainRemovalConfirmed));
    }

    /// Fix round 3: a successful `addDomain` is the moment the domain is known registered again, so the core says
    /// so with an effect; a failed add says nothing (the domain may not be there).
    #[test]
    fn only_a_successful_add_says_the_domain_is_registered() {
        for (add, confirmed) in [(Ok(()), true), (Err(fp(-2011)), false)] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::KeysArrived);
            sim.run(&World {
                add,
                ..World::ok(SIGNED_IN, NotRegistered)
            });
            assert_eq!(
                sim.fx.contains(&Effect::DomainAddConfirmed),
                confirmed,
                "add result decides"
            );
        }
        // A check that finds the domain already registered never adds, so it confirms nothing.
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert!(!sim.fx.contains(&Effect::DomainAddConfirmed), "no add, no confirmation");
    }

    /// An Observe that finds the domain absent also releases it, and only while something is owed (so an ordinary
    /// check never touches the database).
    #[test]
    fn an_observe_that_finds_the_domain_absent_releases_the_owed_flag() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&World::ok(SIGNED_IN_OWING, NotRegistered));
        assert!(
            sim.fx.contains(&Effect::DomainRemovalConfirmed),
            "the domain is not there, so nothing is owed any more"
        );
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&World::ok(SIGNED_IN, NotRegistered));
        assert!(
            !sim.fx.contains(&Effect::DomainRemovalConfirmed),
            "nothing owed, nothing to release"
        );
    }

    /// With a removal owed and the domain still there, the check removes it BEFORE it goes on to start the engine, and
    /// only then runs its fresh check.
    #[test]
    fn an_owed_removal_runs_before_the_check_starts_the_engine() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        let owing = World::ok(SIGNED_IN_OWING, Enabled);
        assert_eq!(sim.step_op(&owing), Op::Observe);
        assert_eq!(
            sim.step_op(&owing),
            Op::RemoveDomain,
            "the removal comes before StartEngine"
        );
        assert!(sim.fx.contains(&Effect::DomainRemovalConfirmed));
        // The flag is now clear (the driver's port did that), and the domain is gone.
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.kinds()[..4],
            [Op::Observe, Op::RemoveDomain, Op::Observe, Op::StartEngine]
        );
        assert_eq!(sim.s.setup, Ready);
    }

    /// A removal that fails does not loop: no immediate re-check (which would remove again, and again). It lands, and
    /// the next trigger tries once more. It lands in `Failed`, never a resting `Missing`: the person is signed in and
    /// unlocked, the previous domain is still there and no engine may start, so the view says so and offers "Try
    /// again" (§13.1: `Missing` is never a resting state while wanted is present).
    #[test]
    fn an_owed_removal_that_fails_does_not_loop() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        let failing = World {
            remove: Err(fp(516)),
            ..World::ok(SIGNED_IN_OWING, Enabled)
        };
        sim.run(&failing);
        assert_eq!(
            sim.kinds(),
            vec![Op::Observe, Op::RemoveDomain],
            "one removal, then it settles"
        );
        assert!(
            !sim.fx.contains(&Effect::DomainRemovalConfirmed),
            "a failed removal releases nothing"
        );
        let reason = policy::classify(&fp(516));
        assert_eq!(
            (sim.s.setup, sim.s.reason, sim.s.last_error.clone()),
            (Failed, Some(reason), Some(fp(516))),
            "a signed-in Mac with the old domain still there is a failure with one action, not a resting Missing"
        );
        assert_eq!(sim.s.max_attempts, 1, "one attempt per trigger");
        assert!(
            sim.fx.contains(&Effect::PersistFailure(Some((reason, fp(516))))),
            "recorded like every other failure"
        );
        sim.trigger(Trigger::TryAgain);
        sim.run(&failing);
        assert_eq!(sim.count(Op::RemoveDomain), 2, "the next trigger removes once more");
        assert_eq!(sim.s.setup, Failed, "and still says so when it fails again");
    }

    #[test]
    fn lock_cancels_the_check_and_holds_until_keys_arrive() {
        let world = World::ok(SIGNED_IN, NotRegistered);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&world); // Observe → Adding
        assert_eq!(sim.s.setup, Adding);
        sim.trigger(Trigger::Lock);
        assert_eq!(sim.s.setup, Missing);
        assert_eq!(sim.next(), Next::Idle, "no StartEngine after a lock");
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.next(), Next::Idle);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Ready);
    }

    /// §7 on an injected clock: transient failures → exactly 4 attempts at 0/5/15/45 s.
    #[test]
    fn transient_failures_make_four_attempts_at_0_5_15_45() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            add: Err(fp(-2001)),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(sim.attempt_starts(), vec![0, 5, 15, 45]);
        assert_eq!(sim.s.setup, Failed);
        assert_eq!(sim.s.reason, Some(FinderFailureReason::ExtensionLoading));
        assert_eq!((sim.s.attempt, sim.s.max_attempts), (4, 4));
        assert_eq!(
            sim.count(Op::StopEngine),
            1,
            "the engine this check started is stopped once"
        );
        // The person saw Adding throughout: one Adding publish, then one Failed.
        let lands: Vec<FinderSetup> = sim
            .fx
            .iter()
            .filter_map(|e| match e {
                Effect::Transition(t) if t.from != t.to => Some(t.to),
                _ => None,
            })
            .collect();
        assert_eq!(lands, vec![Adding, Failed]);
    }

    #[test]
    fn a_slow_attempt_delays_the_next_start() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            stable: Err(stabilization_timeout()),
            wait_secs: 10,
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(sim.attempt_starts(), vec![0, 10, 20, 45]);
        assert_eq!(sim.secs(), 55, "ends within about 55 s");
        assert_eq!(sim.s.reason, Some(FinderFailureReason::Timeout));
    }

    #[test]
    fn unknown_makes_two_attempts_and_not_retryable_reasons_make_one() {
        let mut unknown = Sim::new(Applications);
        unknown.trigger(Trigger::KeysArrived);
        unknown.run(&World {
            add: Err(FpError::new("SomeDomain", 1, "x")),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(unknown.attempt_starts(), vec![0, 5]);
        assert_eq!(
            (unknown.s.setup, unknown.s.reason),
            (Failed, Some(FinderFailureReason::Unknown))
        );

        let taken = FpError::new(COCOA_DOMAIN, 516, "exists");
        let mut folder = Sim::new(Applications);
        folder.trigger(Trigger::KeysArrived);
        folder.run(&World {
            add: Err(taken.clone()),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        assert_eq!(folder.attempt_starts(), vec![0]);
        assert_eq!(
            (folder.s.setup, folder.s.reason),
            (Failed, Some(FinderFailureReason::FolderTaken))
        );
        assert!(
            folder
                .fx
                .contains(&Effect::PersistFailure(Some((FinderFailureReason::FolderTaken, taken))))
        );
    }

    #[test]
    fn nothing_retries_after_failed_without_a_trigger_and_try_again_restarts_the_budget() {
        let world = World {
            add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")),
            ..World::ok(SIGNED_IN, NotRegistered)
        };
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&world);
        assert_eq!(sim.s.setup, Failed);
        sim.feed(Input::Tick { window_visible: true });
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Idle, "bringing the app forward does not retry");
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.s.attempt, 1, "a fresh budget");
        sim.run(&world);
        assert_eq!(sim.count(Op::AddDomain), 2);
    }

    #[test]
    fn user_disabled_poll_reads_only_while_visible_and_a_flip_runs_one_check() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        assert_eq!(sim.s.setup, UserDisabled);
        let three = Duration::from_secs(3);
        assert_eq!(sim.next(), Next::WakeAt(sim.t0 + three));
        sim.now = sim.t0 + three;
        sim.feed(Input::Tick { window_visible: false });
        assert_eq!(
            sim.next(),
            Next::WakeAt(sim.t0 + three * 2),
            "no read while no window is visible"
        );
        sim.now = sim.t0 + three * 2;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(sim.s.setup, UserDisabled);
        sim.feed(Input::AppActivated);
        assert_eq!(
            sim.next(),
            Next::Run(Op::ReadDomain),
            "once when the app becomes active"
        );
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert!(sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)));
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.count(Op::Observe), 2, "the flip runs exactly one check");
        assert_eq!(
            sim.count(Op::AddDomain),
            0,
            "the poll never adds; a registered, enabled domain is only confirmed (§5.5 step 3)"
        );
    }

    /// Turned off in System Settings while Beebeeb runs: while `Ready` the same read-only poll runs on the same bound
    /// as the UserDisabled one (§7: every 3 s while a window is visible, plus once when the app becomes active), and a
    /// read that says "turned off" lands `UserDisabled` by itself: no trigger, no check, no add, no engine stop.
    #[test]
    fn a_ready_domain_turned_off_while_running_lands_user_disabled_from_the_poll_alone() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        let ops_before = sim.kinds();
        let three = Duration::from_secs(3);
        let ready_at = sim.now;
        assert_eq!(sim.next(), Next::WakeAt(ready_at + three), "Ready keeps a poll timer");
        // A window is on screen. With none the timer stops (the test
        // `a_ready_core_with_no_window_visible_sets_no_timer_and_a_focus_resumes_the_poll`).
        sim.now = ready_at + three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert_eq!(sim.s.setup, Ready, "still on: nothing changes");
        assert_eq!(sim.next(), Next::WakeAt(ready_at + three * 2), "and reads again in 3 s");
        // The person turns Beebeeb off in System Settings; the visible window's timer reads it.
        sim.now = ready_at + three * 2;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.fx.clear();
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(
            (sim.s.setup, sim.s.reason, sim.s.last_error.clone()),
            (UserDisabled, Some(FinderFailureReason::UserDisabled), None)
        );
        assert!(
            sim.fx.iter().any(|e| matches!(
                e,
                Effect::Transition(t) if t.from == Ready && t.to == UserDisabled
                    && t.reason == Some(FinderFailureReason::UserDisabled)
            )),
            "one lifecycle line: {:?}",
            sim.fx
        );
        assert!(sim.fx.contains(&Effect::Publish), "surfaces refresh");
        assert!(
            !sim.fx.iter().any(|e| matches!(e, Effect::TriggerReceived(_))),
            "the poll alone noticed it: {:?}",
            sim.fx
        );
        assert_eq!(sim.kinds(), ops_before, "no check, no add, no engine stop");
        assert!(sim.s.is_settled());
        // From here it is the UserDisabled poll: turning it back on runs exactly one check, which only confirms.
        assert_eq!(sim.next(), Next::WakeAt(sim.now + three));
        sim.now += three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert!(sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)));
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(sim.count(Op::Observe), 2, "the launch check, then the flip's one check");
        assert_eq!(sim.count(Op::AddDomain), 0, "neither the flip off nor the flip on adds");
        assert_eq!(sim.count(Op::StopEngine), 0);
    }

    /// The same, noticed when the app becomes active (§7's "plus once when the app becomes active").
    #[test]
    fn a_ready_domain_turned_off_while_running_is_read_once_when_the_app_becomes_active() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        sim.feed(Input::AppActivated);
        assert_eq!(
            sim.next(),
            Next::Run(Op::ReadDomain),
            "once when the app becomes active"
        );
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.count(Op::AddDomain), 0);
        assert_eq!(sim.count(Op::Observe), 1, "no check ran");
    }

    /// F7 follow-up (review minor 2): `Ready` is the usual resting state, so its poll must not wake the app every 3 s
    /// while no Beebeeb window is on screen. A tick that finds none sets no new timer; a window gaining focus (the app
    /// becoming active) reads once and arms the timer again, and the next tick reads. The bound F7 needs is kept: a
    /// window on screen is read every 3 s, and an activation reads once.
    #[test]
    fn a_ready_core_with_no_window_visible_sets_no_timer_and_a_focus_resumes_the_poll() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
        let three = Duration::from_secs(3);
        let ready_at = sim.now;
        assert_eq!(
            sim.next(),
            Next::WakeAt(ready_at + three),
            "Ready lands with the timer set: a window may be on screen"
        );
        sim.now = ready_at + three;
        sim.feed(Input::Tick { window_visible: false });
        assert_eq!(sim.next(), Next::Idle, "no window on screen: no timer, so no wake-up");
        sim.now += Duration::from_secs(3600);
        assert_eq!(sim.next(), Next::Idle, "an hour later, still none");
        assert!(sim.s.is_settled());

        // A window gains focus: one read at once, and the timer is set again.
        sim.feed(Input::AppActivated);
        assert_eq!(
            sim.next(),
            Next::Run(Op::ReadDomain),
            "once when the app becomes active"
        );
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        let activated_at = sim.now;
        assert_eq!(
            sim.next(),
            Next::WakeAt(activated_at + three),
            "the focus set the timer again"
        );
        // The next tick, with the window on screen, reads, and F7 still holds: turned off is noticed.
        sim.now = activated_at + three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain), "the next tick reads");
        sim.feed(Input::Done(OpResult::Domain(Ok(Disabled))));
        assert_eq!(
            (sim.s.setup, sim.s.reason),
            (UserDisabled, Some(FinderFailureReason::UserDisabled))
        );
        assert_eq!(
            sim.next(),
            Next::WakeAt(sim.now + three),
            "no check runs: what comes next is the UserDisabled poll's timer"
        );
    }

    /// While `Ready` the poll acts only on "turned off": a read that fails, or finds the domain not registered, is no
    /// evidence of anything and changes nothing (a missing domain is the next launch's to re-add, §5.5).
    #[test]
    fn the_ready_poll_ignores_every_answer_but_turned_off() {
        for read in [
            Ok(Enabled),
            Ok(NotRegistered),
            Err(FpError::app(app_code::OP_TIMEOUT, "ReadDomain did not answer")),
        ] {
            let mut sim = Sim::new(Applications);
            sim.trigger(Trigger::Launch);
            sim.run(&World::ok(SIGNED_IN, Enabled));
            sim.fx.clear();
            sim.feed(Input::AppActivated);
            assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
            sim.feed(Input::Done(OpResult::Domain(read.clone())));
            assert_eq!((sim.s.setup, sim.s.reason), (Ready, None), "{read:?}");
            assert!(sim.fx.is_empty(), "{read:?}: {:?}", sim.fx);
            assert!(sim.s.is_settled(), "{read:?}");
        }
    }

    /// A Lock leaves `Ready` in place (it never changes what macOS says) and holds: no read, no timer, until keys
    /// arrive again.
    #[test]
    fn a_held_ready_core_does_not_poll() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        sim.feed(Input::AppActivated); // a read is pending
        sim.trigger(Trigger::Lock);
        assert_eq!(sim.s.setup, Ready);
        assert_eq!(
            sim.next(),
            Next::Idle,
            "the pending read is dropped and no timer is left behind"
        );
        sim.now += Duration::from_secs(30);
        sim.feed(Input::Tick { window_visible: true });
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Idle);
    }

    #[test]
    fn a_disk_image_launch_reports_the_reason_before_any_check_and_never_adds() {
        let mut sim = Sim::new(DiskImage);
        assert_eq!(
            (sim.s.setup, sim.s.reason),
            (Missing, Some(FinderFailureReason::NotInApplications))
        );
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(sim.kinds(), vec![Op::Observe]);
        assert_eq!(
            (sim.s.setup, sim.s.reason),
            (Failed, Some(FinderFailureReason::NotInApplications))
        );
    }

    #[test]
    fn a_registered_domain_is_confirmed_even_from_a_disk_image() {
        // §5.5: the launch-location check applies only when the domain is not registered.
        let mut sim = Sim::new(DiskImage);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Enabled));
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn ready_clears_a_persisted_failure() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World {
            add: Err(FpError::new(COCOA_DOMAIN, 516, "exists")),
            ..World::ok(SIGNED_IN, NotRegistered)
        });
        sim.fx.clear();
        sim.trigger(Trigger::TryAgain);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            (sim.s.setup, sim.s.reason, sim.s.last_error.clone()),
            (Ready, None, None)
        );
        assert!(sim.fx.contains(&Effect::PersistFailure(None)));
    }

    /// Fix round 1 (review): while `held` the poll must not read, must not log a flip, and must
    /// not leave a wake-up scheduled (a past `WakeAt` would spin the driver).
    #[test]
    fn a_held_user_disabled_core_does_not_poll_and_keys_arriving_resumes_it() {
        let disabled = World::ok(SIGNED_IN, Disabled);
        let three = Duration::from_secs(3);
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&disabled);
        assert_eq!(sim.s.setup, UserDisabled);
        // The app was brought forward just before the lock: a read is already pending.
        sim.feed(Input::AppActivated);
        sim.trigger(Trigger::Lock);
        assert_eq!(
            sim.s.setup, UserDisabled,
            "lock never removes and never changes what macOS says"
        );
        assert_eq!(
            sim.next(),
            Next::Idle,
            "a pending read is dropped and no timer is left behind"
        );
        // Held: ticks with a visible window and the app coming forward read nothing.
        sim.now = sim.t0 + three;
        sim.feed(Input::Tick { window_visible: true });
        sim.feed(Input::AppActivated);
        assert_eq!(sim.next(), Next::Idle);
        sim.now = sim.t0 + three * 5;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Idle);
        assert_eq!(sim.count(Op::ReadDomain), 0);
        assert!(
            !sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)),
            "{:?}",
            sim.fx
        );
        // Keys arriving lift the hold; the check lands UserDisabled again and the poll resumes.
        sim.trigger(Trigger::KeysArrived);
        sim.run(&disabled);
        assert_eq!(sim.s.setup, UserDisabled);
        assert_eq!(sim.next(), Next::WakeAt(sim.now + three));
        sim.now += three;
        sim.feed(Input::Tick { window_visible: true });
        assert_eq!(sim.next(), Next::Run(Op::ReadDomain));
        sim.feed(Input::Done(OpResult::Domain(Ok(Enabled))));
        assert!(sim.fx.contains(&Effect::TriggerReceived(Trigger::UserEnabledFlipped)));
    }

    #[test]
    fn sign_out_drops_a_pending_user_disabled_read() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::Launch);
        sim.run(&World::ok(SIGNED_IN, Disabled));
        sim.feed(Input::AppActivated); // a read is pending
        sim.ops.clear();
        sim.trigger(Trigger::SignOut);
        sim.run(&World::ok(SIGNED_OUT_BY_CHOICE, Disabled));
        assert_eq!(
            sim.kinds(),
            vec![Op::RemoveDomain],
            "no read of a domain that sign-out is removing"
        );
        assert_eq!(sim.s.setup, Missing);
    }

    /// Backoff is where a failing check spends most of its time (§7: up to 45 s of it).
    #[test]
    fn lock_during_backoff_cancels_the_retry_and_holds_until_keys_arrive() {
        let failing = World {
            add: Err(fp(-2001)),
            ..World::ok(SIGNED_IN, NotRegistered)
        };
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.step_op(&failing); // Observe
        sim.step_op(&failing); // StartEngine
        sim.step_op(&failing); // AddDomain: -2001, a transient failure
        assert_eq!(sim.s.setup, Adding);
        assert_eq!(
            sim.next(),
            Next::WakeAt(sim.t0 + Duration::from_secs(5)),
            "the retry is waiting out its backoff"
        );
        sim.trigger(Trigger::Lock);
        assert_eq!(sim.s.setup, Missing);
        assert_eq!(sim.s.reason, None);
        assert_eq!(sim.next(), Next::Idle, "the backoff timer is gone");
        // The old deadline passes, and a TryAgain arrives: still nothing.
        sim.now = sim.t0 + Duration::from_secs(5);
        sim.feed(Input::Tick { window_visible: false });
        assert_eq!(sim.next(), Next::Idle, "no retry after the deadline");
        sim.now = sim.t0 + Duration::from_secs(60);
        sim.trigger(Trigger::TryAgain);
        assert_eq!(sim.next(), Next::Idle, "TryAgain does not lift a lock");
        // Keys arriving run exactly one fresh check.
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(SIGNED_IN, NotRegistered));
        assert_eq!(
            sim.count(Op::Observe),
            2,
            "the cancelled check's first attempt, then exactly one more check"
        );
        assert_eq!(sim.s.setup, Ready);
    }

    #[test]
    fn sign_in_after_a_sign_out_by_choice_clears_the_persisted_intent() {
        let mut sim = Sim::new(Applications);
        sim.trigger(Trigger::KeysArrived);
        sim.run(&World::ok(
            SessionFacts {
                signed_out_by_choice: true,
                ..SIGNED_IN
            },
            NotRegistered,
        ));
        assert!(sim.fx.contains(&Effect::PersistSignedOutByChoice(false)));
    }
}
