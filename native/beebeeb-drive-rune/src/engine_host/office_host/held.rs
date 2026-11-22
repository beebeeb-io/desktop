//! What the host keeps of an editing session so that a helper that dies can be replaced: the bytes the document was
//! opened with, and the last snapshot (or save) the helper handed over. Both stay in memory and are wiped when dropped
//! or replaced. A new document has no opened bytes.
//!
//! Above `SPILL_ABOVE_BYTES` the opened bytes and the snapshot go to sealed scratch (the engine's
//! `office_scratch`, key in memory only) instead of memory; below it nothing touches the disk. A `Held` made without
//! a store ([`Held::new`]) keeps everything in memory. If the store refuses, the bytes stay in memory.

use std::time::SystemTime;

use beebeeb_engine::app::office_scratch::{ScratchFile, ScratchStore};
use zeroize::Zeroizing;

/// Working copies above this many bytes go to scratch (64 MiB).
pub(super) const SPILL_ABOVE_BYTES: usize = 64 * 1024 * 1024;

/// Bytes in memory, or sealed on disk.
enum Bytes {
    Memory(Zeroizing<Vec<u8>>),
    Scratch(ScratchFile),
}

impl Bytes {
    fn keep(bytes: &[u8], scratch: Option<&ScratchStore>, limit: usize) -> Bytes {
        if bytes.len() > limit
            && let Some(file) = scratch.and_then(|store| store.put(bytes).ok())
        {
            return Bytes::Scratch(file);
        }
        Bytes::Memory(Zeroizing::new(bytes.to_vec()))
    }

    fn copy(&self) -> Option<Zeroizing<Vec<u8>>> {
        match self {
            Bytes::Memory(bytes) => Some(bytes.clone()),
            Bytes::Scratch(file) => file.read().ok(),
        }
    }
}

struct Snapshot {
    bytes: Bytes,
    #[allow(dead_code)]
    revision: u64,
    at: SystemTime,
}

pub(super) struct Held {
    opened: Option<Bytes>,
    opened_at: SystemTime,
    snapshot: Option<Snapshot>,
    scratch: Option<ScratchStore>,
    limit: usize,
}

impl Held {
    /// `opened`: the document's bytes as they were opened (`None` for a new document).
    pub(super) fn new(opened: Option<Zeroizing<Vec<u8>>>) -> Held {
        Held::with_scratch(opened, None)
    }

    /// [`Held::new`] with the session's scratch store: copies above `SPILL_ABOVE_BYTES` go there.
    pub(super) fn with_scratch(opened: Option<Zeroizing<Vec<u8>>>, scratch: Option<ScratchStore>) -> Held {
        Held::with_limit(opened, scratch, SPILL_ABOVE_BYTES)
    }

    fn with_limit(opened: Option<Zeroizing<Vec<u8>>>, scratch: Option<ScratchStore>, limit: usize) -> Held {
        let opened = opened.map(|bytes| Bytes::keep(&bytes, scratch.as_ref(), limit));
        Held {
            opened,
            opened_at: SystemTime::now(),
            snapshot: None,
            scratch,
            limit,
        }
    }

    /// Keeps `bytes` (revision `revision`) as the newest state; the one before is wiped.
    pub(super) fn store_snapshot(&mut self, bytes: &[u8], revision: u64) {
        self.snapshot = Some(Snapshot {
            bytes: Bytes::keep(bytes, self.scratch.as_ref(), self.limit),
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
        self.snapshot.as_ref().and_then(|snapshot| snapshot.bytes.copy())
    }

    /// What a new helper opens: the last snapshot, else the opened bytes. `None`: a new document with no snapshot.
    pub(super) fn restart_bytes(&self) -> Option<Zeroizing<Vec<u8>>> {
        self.snapshot_copy()
            .or_else(|| self.opened.as_ref().and_then(Bytes::copy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beebeeb_engine::account::SessionGeneration;

    const MARKER: &[u8] = b"TOP-SECRET-MARKER-77aa";

    /// A store for tests: a real engine is not needed, the engine's own seam makes one per state dir.
    fn store(dir: &std::path::Path) -> ScratchStore {
        beebeeb_engine::app::office_scratch::store_for_test(dir, SessionGeneration::for_test(1))
    }

    fn doc(len: usize) -> Vec<u8> {
        let mut v = MARKER.repeat(len / MARKER.len() + 1);
        v.truncate(len);
        v
    }

    fn scratch_has_marker(dir: &std::path::Path) -> bool {
        let Ok(rd) = std::fs::read_dir(dir.join("office-scratch")) else {
            return false;
        };
        rd.flatten().any(|e| {
            std::fs::read(e.path())
                .unwrap()
                .windows(MARKER.len())
                .any(|w| w == MARKER)
        })
    }

    fn file_count(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir.join("office-scratch"))
            .map(|r| r.count())
            .unwrap_or(0)
    }

    #[test]
    fn above_the_threshold_it_spills_sealed_and_reads_back_equal() {
        let tmp = tempfile::tempdir().unwrap();
        let opened = doc(300_000);
        let snap = doc(400_000);
        let mut held = Held::with_limit(Some(Zeroizing::new(opened.clone())), Some(store(tmp.path())), 100_000);
        assert_eq!(&**held.restart_bytes().unwrap(), &opened[..]);
        held.store_snapshot(&snap, 1);
        assert_eq!(file_count(tmp.path()), 2);
        assert!(!scratch_has_marker(tmp.path()));
        assert_eq!(&**held.restart_bytes().unwrap(), &snap[..]);
        assert_eq!(&**held.snapshot_copy().unwrap(), &snap[..]);
        held.store_snapshot(&opened, 2); // the older snapshot's file goes
        assert_eq!(file_count(tmp.path()), 2);
        drop(held);
        assert_eq!(file_count(tmp.path()), 0);
    }

    #[test]
    fn below_the_threshold_no_file_is_written() {
        let tmp = tempfile::tempdir().unwrap();
        let mut held = Held::with_limit(Some(Zeroizing::new(doc(1000))), Some(store(tmp.path())), 100_000);
        held.store_snapshot(&doc(5000), 1);
        assert_eq!(file_count(tmp.path()), 0);
        assert_eq!(held.restart_bytes().unwrap().len(), 5000);
    }

    #[test]
    fn after_the_session_ends_nothing_is_readable_or_left() {
        let tmp = tempfile::tempdir().unwrap();
        let mut held = Held::with_limit(None, Some(store(tmp.path())), 100);
        held.store_snapshot(&doc(10_000), 1);
        beebeeb_engine::app::office_scratch::end_for_test(tmp.path());
        assert!(held.restart_bytes().is_none());
        assert_eq!(file_count(tmp.path()), 0);
    }
}
