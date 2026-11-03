//! How long a port may take to answer one operation before the driver gives up on it (lead
//! ruling T7-1): one named limit per `Op`, and `within` turning a call that never returns into
//! a redacted `OP_TIMEOUT` instead of a reconciler that sits on it forever.
//!
//! Tests: `tests/ops.rs`.

use super::*;

/// How long a port may take to answer one operation before the driver gives up on it (lead
/// ruling T7-1). Several ObjC primitives wait `DISPATCH_TIME_FOREVER` for `fileproviderd`, and a
/// stuck one must not hang the reconciler and, behind it, sign-out or lock. Real calls answer in
/// milliseconds; these are the point where "slow" has become "not coming".
///
/// - `Observe`, `ReadDomain`: one `getDomains` round trip to `fileproviderd`.
/// - `StartEngine`: spawn plus the 3 s wait for the engine's socket and the engine slot's lock.
/// - `AddDomain`: `addDomain` may have to register the extension, the slowest honest call here.
/// - `WaitStable(w)`: the bridge enforces `w` on its own clock (spec: at most 10 s). The extra 3 s
///   only backs up a bridge that fails to return at all.
/// - `FinishReady`: saving the sync folder and taking the engine slot's lock.
/// - `StopEngine`: the engine slot's lock plus its own 3 s + 2 s abort windows.
/// - `RemoveDomain`: `removeDomain`, which sign-out waits for.
pub fn op_limit(op: Op) -> Duration {
    let s = Duration::from_secs;
    match op {
        Op::Observe | Op::ReadDomain => s(10),
        Op::StartEngine | Op::FinishReady | Op::StopEngine | Op::RemoveDomain => s(15),
        Op::AddDomain => s(20),
        Op::WaitStable(wait) => wait + s(3),
    }
}

/// The step one operation performs (spec §5.3), named once: the word a timeout sentence uses and the
/// order a check walks. A check of a present domain walks `Observe`, `StartEngine`, `AddDomain`,
/// `ReadDomain`, `WaitStable`, `FinishReady`; a removal walks `StopEngine`, `RemoveDomain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinderSetupStep {
    Observe,
    StartEngine,
    AddDomain,
    ReadDomain,
    WaitStable,
    FinishReady,
    StopEngine,
    RemoveDomain,
}

impl FinderSetupStep {
    pub(super) fn of(op: Op) -> Self {
        match op {
            Op::Observe => Self::Observe,
            Op::StartEngine => Self::StartEngine,
            Op::AddDomain => Self::AddDomain,
            Op::ReadDomain => Self::ReadDomain,
            Op::WaitStable(_) => Self::WaitStable,
            Op::FinishReady => Self::FinishReady,
            Op::StopEngine => Self::StopEngine,
            Op::RemoveDomain => Self::RemoveDomain,
        }
    }

    /// The word the timeout sentence has always used for this step.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "Observe",
            Self::StartEngine => "StartEngine",
            Self::AddDomain => "AddDomain",
            Self::ReadDomain => "ReadDomain",
            Self::WaitStable => "WaitStable",
            Self::FinishReady => "FinishReady",
            Self::StopEngine => "StopEngine",
            Self::RemoveDomain => "RemoveDomain",
        }
    }
}

/// Run one port call for at most `limit`. A call that is not back by then is dropped (a blocking
/// bridge call keeps its thread until the OS answers, but nothing waits for it any more) and
/// becomes `OP_TIMEOUT`, which classifies as `timeout`: it retries on the schedule and ends in a
/// sentence and an action, never in a reconciler that sits on one call forever. The text names
/// the operation and the limit only.
pub async fn within<T>(op: Op, limit: Duration, call: impl Future<Output = Result<T, FpError>>) -> Result<T, FpError> {
    match tokio::time::timeout(limit, call).await {
        Ok(result) => result,
        Err(_) => Err(FpError::app(
            app_code::OP_TIMEOUT,
            format!("{} did not answer within {limit:?}", FinderSetupStep::of(op).as_str()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One row per operation: the step and the word the timeout sentence has always used.
    #[test]
    fn finder_setup_step_names_every_operation_table_driven() {
        for (op, step, name) in [
            (Op::Observe, FinderSetupStep::Observe, "Observe"),
            (Op::StartEngine, FinderSetupStep::StartEngine, "StartEngine"),
            (Op::AddDomain, FinderSetupStep::AddDomain, "AddDomain"),
            (Op::ReadDomain, FinderSetupStep::ReadDomain, "ReadDomain"),
            (
                Op::WaitStable(Duration::from_secs(2)),
                FinderSetupStep::WaitStable,
                "WaitStable",
            ),
            (Op::FinishReady, FinderSetupStep::FinishReady, "FinishReady"),
            (Op::StopEngine, FinderSetupStep::StopEngine, "StopEngine"),
            (Op::RemoveDomain, FinderSetupStep::RemoveDomain, "RemoveDomain"),
        ] {
            assert_eq!(FinderSetupStep::of(op), step, "{op:?}");
            assert_eq!(step.as_str(), name, "{op:?}");
        }
    }

    /// The order a check of a present domain walks (spec §5.3), pinned as the story the driver tells.
    #[test]
    fn the_finder_setup_step_order_is_observe_start_add_read_wait_finish() {
        let walked = [
            Op::Observe,
            Op::StartEngine,
            Op::AddDomain,
            Op::ReadDomain,
            Op::WaitStable(Duration::from_secs(10)),
            Op::FinishReady,
        ];
        assert_eq!(
            walked.map(FinderSetupStep::of),
            [
                FinderSetupStep::Observe,
                FinderSetupStep::StartEngine,
                FinderSetupStep::AddDomain,
                FinderSetupStep::ReadDomain,
                FinderSetupStep::WaitStable,
                FinderSetupStep::FinishReady,
            ]
        );
    }
}
