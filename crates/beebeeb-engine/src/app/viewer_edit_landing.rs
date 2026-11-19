//! What became of a viewer save's own queued upload, one look at the local state (w26, v5 bug 8). A save answers
//! "saved" only for a landing the server confirmed; a save the server refused as stale (409 on the upload's init: the
//! file moved on while the person edited) is a conflict the person decides, never "saved".
//!
//! BLOCKS: the state database. Must not: log a name, a path or a byte.

use std::path::Path;

use super::{QueuedSave, landed_version};
use crate::state_db::StateDb;

/// The class a stale-base park carries (`ParkReason::StaleBase`).
const STALE_BASE: &str = "stale_base";

/// One look at `queued`'s fate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fate {
    /// The upload landed as this version (the one its own write recorded).
    Landed(i64),
    /// The server's version moved past the base; the save was taken out of the queue (nothing is sent, the bytes stay
    /// with the editor) and the person decides. `theirs`: the version the file is at now.
    Conflict { theirs: i64 },
    /// The save will not be sent by itself (its tries are used up for another reason): not saved.
    Failed,
    /// Parked on a stale base, but this PC has not yet seen the newer version (the pull brings it): not saved yet.
    ParkedStale,
    /// The op left the queue and no landing is recorded for this save's write.
    Gone,
    /// Still queued or in flight.
    Waiting,
}

/// The fate of `queued`, saved from `base`. A stale-base park is retired here (the same transaction as Keep the other:
/// the op, its session row and the file's held write go), so a Keep mine that follows queues behind nothing.
pub(super) fn look(db: &StateDb, file_id: &str, queued: &QueuedSave, base: i64) -> Result<Fate, rusqlite::Error> {
    if let Some(version) = landed_version(db, file_id, queued).filter(|version| *version > base) {
        return Ok(Fate::Landed(version));
    }
    let Some(op) = db.get_operation(&queued.op_id)? else {
        return Ok(Fate::Gone);
    };
    if op.attempts < op.max_attempts {
        return Ok(Fate::Waiting);
    }
    if op.last_error.as_deref() != Some(STALE_BASE) {
        return Ok(Fate::Failed);
    }
    let theirs = db.get_file_contract_state(file_id)?.map_or(0, |contract| contract.current_version);
    if theirs <= base {
        return Ok(Fate::ParkedStale);
    }
    match db.drop_parked(&queued.op_id, theirs)? {
        Some(released) => {
            if let Some(path) = released {
                crate::staged_payload::remove_or_log(db, Path::new(&path));
            }
            Ok(Fate::Conflict { theirs })
        }
        // Not settleable now (a later op of the file, a folder on the move): the save was refused and is still parked,
        // never "saved".
        None => Ok(Fate::ParkedStale),
    }
}
