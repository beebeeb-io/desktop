//! The state and its vocabulary: `Trigger`, the `SessionFacts` an observation carries, the
//! `Op` the driver runs, the `Effect`s a step emits, and `CoreState` itself.
//!
//! Must not: decide what comes next (`decide`) or land a result (`landing`).

use super::*;

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
pub(super) enum Wait {
    AfterAdd,
    ConfirmExisting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Landing {
    Failed,
    UserDisabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Observe,
    StartEngine { add: bool },
    Add,
    ReadAfterAdd,
    Wait(Wait),
    Finish,
    StopEngine(Landing),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
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
    pub(super) phase: Phase,
    pub(super) awaiting: bool,
    pub(super) check_started: Option<Instant>,
    pub(super) check_started_engine: bool,
    /// A trigger arrived during a check: run exactly one more afterwards (§5.4).
    pub(super) check_again: bool,
    /// Set by sign-out and lock; only `Launch` or `KeysArrived` clears it. While held no check
    /// starts, so nothing re-adds the domain or starts the engine in the gap between the
    /// reconciler finishing and the caller clearing the session. The poll (see [`polls`]) is off
    /// while held.
    pub(super) held: bool,
    pub(super) poll_due: Option<Instant>,
    pub(super) poll_pending: bool,
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
