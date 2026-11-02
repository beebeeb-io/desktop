//! The view the app reads: what `finder_setup_state` returns and what `finder-setup-changed`
//! carries (spec §5.2, §9), plus the one rule deciding which `reason` a person is told.
//!
//! Owns: the published shape and the reason filter. Must not: decide a state or a transition
//! (see `super::core`). Tests: `tests/view.rs`.

use super::*;

pub const FINDER_SETUP_CHANGED_EVENT: &str = "finder-setup-changed";

/// What `finder_setup_state` returns and `finder-setup-changed` carries (spec §5.2, §9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FinderSetupView {
    pub setup: FinderSetup,
    pub reason: Option<FinderFailureReason>,
    pub launch_location: LaunchLocation,
    pub attempt: u8,
    pub max_attempts: u8,
    pub last_failure: Option<FailureRecord>,
}

impl FinderSetupView {
    /// The one rule for what `reason` a person is told (lead ruling T3-4, corrected 2026-10-07):
    /// - `Failed` and `UserDisabled`: the core's reason, a verdict.
    /// - `Missing`: only `NotInApplications`. That is spec device check D7 ("opened from the
    ///   mounted dmg: `not_in_applications` reported by `finder_setup_state` before sign-in"), the
    ///   reason the core already holds from the launch location before any check runs, and what
    ///   `finderSetupPresentation` reads when `setup === 'missing'`. Any other reason a `Missing`
    ///   core holds is not something to tell.
    /// - `Adding`: never. The core keeps the reason of the attempt it is silently retrying, and
    ///   publishing it would flash a failure that is not one yet.
    /// - `Ready`: never.
    ///
    /// The match is exhaustive so a new state has to choose.
    pub fn of(core: &CoreState, last_failure: Option<FailureRecord>) -> Self {
        let reason = match core.setup {
            FinderSetup::Failed | FinderSetup::UserDisabled => core.reason,
            FinderSetup::Missing => core
                .reason
                .filter(|reason| *reason == FinderFailureReason::NotInApplications),
            FinderSetup::Ready | FinderSetup::Adding => None,
        };
        Self {
            setup: core.setup,
            reason,
            launch_location: core.launch,
            attempt: core.attempt,
            max_attempts: core.max_attempts,
            last_failure,
        }
    }

    pub fn initial(launch: LaunchLocation) -> Self {
        Self::of(&CoreState::new(launch), None)
    }
}
