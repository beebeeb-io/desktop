//! What the host keeps of an editing session so that a helper that dies can be replaced: the bytes the document was
//! opened with, and the last snapshot (or save) the helper handed over. Both stay in memory and are wiped when dropped
//! or replaced. A new document has no opened bytes.
//!
//! Seam for W4 (`w04-host-held-spill`): above `SPILL_ABOVE_BYTES` the snapshot goes to sealed scratch instead of
//! memory. Only [`Held::store_snapshot`] and the readers below change then; nothing else touches the buffers.

use std::time::SystemTime;

use zeroize::Zeroizing;

struct Snapshot {
    bytes: Zeroizing<Vec<u8>>,
    #[allow(dead_code)]
    revision: u64,
    at: SystemTime,
}

pub(super) struct Held {
    opened: Option<Zeroizing<Vec<u8>>>,
    opened_at: SystemTime,
    snapshot: Option<Snapshot>,
}

impl Held {
    /// `opened`: the document's bytes as they were opened (`None` for a new document).
    pub(super) fn new(opened: Option<Zeroizing<Vec<u8>>>) -> Held {
        Held {
            opened,
            opened_at: SystemTime::now(),
            snapshot: None,
        }
    }

    /// Keeps `bytes` (revision `revision`) as the newest state; the one before is wiped.
    pub(super) fn store_snapshot(&mut self, bytes: &[u8], revision: u64) {
        self.snapshot = Some(Snapshot {
            bytes: Zeroizing::new(bytes.to_vec()),
            revision,
            at: SystemTime::now(),
        });
    }

    pub(super) fn has_snapshot(&self) -> bool {
        self.snapshot.is_some()
    }

    /// When the state a restart opens is from: the snapshot's time, else when the document was opened.
    pub(super) fn at(&self) -> SystemTime {
        self.snapshot.as_ref().map_or(self.opened_at, |snapshot| snapshot.at)
    }

    /// A copy of the snapshot, if there is one.
    pub(super) fn snapshot_copy(&self) -> Option<Zeroizing<Vec<u8>>> {
        self.snapshot.as_ref().map(|snapshot| snapshot.bytes.clone())
    }

    /// What a new helper opens: the last snapshot, else the opened bytes. `None`: a new document with no snapshot.
    pub(super) fn restart_bytes(&self) -> Option<Zeroizing<Vec<u8>>> {
        self.snapshot_copy().or_else(|| self.opened.clone())
    }
}
