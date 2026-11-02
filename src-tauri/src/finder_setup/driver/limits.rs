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

pub(super) fn op_name(op: Op) -> &'static str {
    match op {
        Op::Observe => "Observe",
        Op::StartEngine => "StartEngine",
        Op::AddDomain => "AddDomain",
        Op::ReadDomain => "ReadDomain",
        Op::WaitStable(_) => "WaitStable",
        Op::FinishReady => "FinishReady",
        Op::StopEngine => "StopEngine",
        Op::RemoveDomain => "RemoveDomain",
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
            format!("{} did not answer within {limit:?}", op_name(op)),
        )),
    }
}
