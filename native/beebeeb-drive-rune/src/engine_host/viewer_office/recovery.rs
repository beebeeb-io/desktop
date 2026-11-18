//! The source's door to the engine's recovery store (`beebeeb_engine::app::office_recovery`): the snapshots the
//! editing session's helper hands over ([`Purpose::Snapshot`]) are kept as encrypted blobs on this PC, one per file, and
//! given back after an app crash. Child of `viewer_office.rs`.
//!
//! - **Keep:** [`Recovery::keep`] stores a snapshot's bytes (sealed by the engine under the account's key, bound to the
//!   file id) over the file's previous blob. [`Recovery::offer`] says when a file's blob is from; [`Recovery::restore`]
//!   hands the bytes back (the blob stays until a save or a discard, so a crash after a restore loses nothing);
//!   [`Recovery::pending`] lists the account's blobs (the notice after unlock).
//! - **Delete:** a save ([`Recovery::saved`]), a discard ([`Recovery::discard`]) and sign-out (the engine wipes the
//!   account's folder itself, `wipe_office_recovery_at_session_end`).
//! - **Session:** every call is made for the session the view was opened under; the engine refuses another one and
//!   another account's blob never opens (the blob is bound to the account's key).
//!
//! BLOCKS: the disk, on the window's thread for a blob of a document (small next to a decrypt; the host's snapshot
//! cadence is seconds). Must not: log a name, a path or a byte.

#![allow(dead_code)] // the source's Office arm calls it (w05-wire-shell)

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use beebeeb_engine::app::office_recovery::{RecoveryEntry, RecoveryRefused};
use beebeeb_engine::app::{CredentialStores, Engine};

use super::*;
use crate::frames::viewers::source::RecoveryOffer;

/// The engine's recovery calls, as this module makes them (the app's: [`Engine`]; a test's fake).
pub(crate) trait Blobs: Send + Sync {
    fn put(
        &self,
        session: SessionGeneration,
        file_id: &str,
        base: i64,
        bytes: &Zeroizing<Vec<u8>>,
    ) -> Result<(), RecoveryRefused>;
    fn list(&self, session: SessionGeneration) -> Result<Vec<RecoveryEntry>, RecoveryRefused>;
    fn take(&self, session: SessionGeneration, file_id: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryRefused>;
    fn discard(&self, session: SessionGeneration, file_id: &str) -> Result<(), RecoveryRefused>;
}

impl<C: CredentialStores> Blobs for Engine<C> {
    fn put(
        &self,
        session: SessionGeneration,
        file_id: &str,
        base: i64,
        bytes: &Zeroizing<Vec<u8>>,
    ) -> Result<(), RecoveryRefused> {
        self.put_office_snapshot_for(session, file_id, base, bytes)
    }

    fn list(&self, session: SessionGeneration) -> Result<Vec<RecoveryEntry>, RecoveryRefused> {
        self.office_snapshots_for(session)
    }

    fn take(&self, session: SessionGeneration, file_id: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryRefused> {
        self.take_office_snapshot_for(session, file_id)
    }

    fn discard(&self, session: SessionGeneration, file_id: &str) -> Result<(), RecoveryRefused> {
        self.discard_office_snapshot_for(session, file_id)
    }
}

/// A file with a blob, as the notice after unlock names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pending {
    pub(crate) file_id: String,
    /// The blob's time of day, "14:32".
    pub(crate) at: String,
}

/// The recovery store for one session's view.
pub(crate) struct Recovery {
    blobs: Arc<dyn Blobs>,
    session: SessionGeneration,
}

impl Recovery {
    pub(crate) fn new(blobs: Arc<dyn Blobs>, session: SessionGeneration) -> Self {
        Recovery { blobs, session }
    }

    /// Keep a snapshot of `file_id` (edited from version `base`) as its one blob. `false`: not kept (the disk or the
    /// session refused); the document goes on, only the safety net is missing.
    pub(crate) fn keep(&self, file_id: &str, base: i64, bytes: &Zeroizing<Vec<u8>>) -> bool {
        self.blobs.put(self.session, file_id, base, bytes).is_ok()
    }

    /// When the file's blob is from, if there is one.
    pub(crate) fn offer(&self, file_id: &str) -> Option<RecoveryOffer> {
        let entries = self.blobs.list(self.session).ok()?;
        let entry = entries.iter().find(|entry| entry.file_id == file_id)?;
        Some(RecoveryOffer { at: clock(entry.saved_at) })
    }

    /// The bytes of the file's blob. The blob stays until [`Recovery::saved`] or [`Recovery::discard`].
    pub(crate) fn restore(&self, file_id: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.blobs.take(self.session, file_id).ok()
    }

    /// Delete the file's blob (Discard).
    pub(crate) fn discard(&self, file_id: &str) {
        let _ = self.blobs.discard(self.session, file_id);
    }

    /// A save landed: the blob is no longer needed.
    pub(crate) fn saved(&self, file_id: &str) {
        self.discard(file_id);
    }

    /// The account's blobs, newest first (empty when the session does not allow it).
    pub(crate) fn pending(&self) -> Vec<Pending> {
        let mut entries = self.blobs.list(self.session).unwrap_or_default();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.saved_at));
        entries
            .into_iter()
            .map(|entry| Pending {
                at: clock(entry.saved_at),
                file_id: entry.file_id,
            })
            .collect()
    }
}

fn clock(saved_at: i64) -> String {
    clock_words(UNIX_EPOCH + Duration::from_secs(saved_at.max(0) as u64))
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
