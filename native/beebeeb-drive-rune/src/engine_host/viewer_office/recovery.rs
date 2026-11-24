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

/// The view's engine, as the recovery store reaches it (the trait's calls; the engine's own `Blobs` impl is the
/// concrete one, this one is what the source holds).
pub(crate) struct EngineBlobs<E: ViewerEngine>(pub(crate) Arc<E>);

impl<E: ViewerEngine> Blobs for EngineBlobs<E> {
    fn put(
        &self,
        session: SessionGeneration,
        file_id: &str,
        base: i64,
        bytes: &Zeroizing<Vec<u8>>,
    ) -> Result<(), RecoveryRefused> {
        self.0.recovery_put(session, file_id, base, bytes)
    }

    fn list(&self, session: SessionGeneration) -> Result<Vec<RecoveryEntry>, RecoveryRefused> {
        self.0.recovery_list(session)
    }

    fn take(&self, session: SessionGeneration, file_id: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryRefused> {
        self.0.recovery_take(session, file_id)
    }

    fn discard(&self, session: SessionGeneration, file_id: &str) -> Result<(), RecoveryRefused> {
        self.0.recovery_discard(session, file_id)
    }
}

/// Where one editing session's snapshots go: the file's recovery blob.
pub(crate) struct Tap {
    recovery: Recovery,
    file_id: String,
    base: i64,
}

impl Tap {
    pub(crate) fn new(recovery: Recovery, file_id: &str, base: i64) -> Tap {
        Tap { recovery, file_id: file_id.to_string(), base }
    }

    /// Keep `bytes` as the blob. A refusal only means the safety net is missing; the document goes on.
    pub(crate) fn keep(&self, bytes: &Zeroizing<Vec<u8>>) {
        let _ = self.recovery.keep(&self.file_id, self.base, bytes);
    }
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    /// The recovery store for the session now. `None`: no session.
    fn office_recovery_store(&self) -> Option<Recovery> {
        self.engine
            .has_session()
            .then(|| Recovery::new(Arc::new(EngineBlobs(self.engine.clone())), self.engine.session_turn()))
    }

    /// When this PC's kept editing of `file_id` is from (the offer). Only for the editor of that file.
    pub(in crate::engine_host::viewer) fn office_recovery(&self, file_id: &str) -> Option<RecoveryOffer> {
        self.office_recovery_store()?.offer(file_id)
    }

    /// The account's kept editings, newest first, for the notice after unlock (the shell's hook calls this).
    pub(in crate::engine_host::viewer) fn office_recovery_pending(&self) -> Vec<Pending> {
        self.office_recovery_store().map(|store| store.pending()).unwrap_or_default()
    }

    /// Drop the kept editing of `file_id`.
    pub(in crate::engine_host::viewer) fn office_discard_recovery(&mut self, file_id: &str) {
        if let Some(store) = self.office_recovery_store() {
            store.discard(file_id);
        }
    }

    /// Open the kept editing of `file_id` in the editor instead of the file's own bytes: a new session starts on the
    /// blob (the old one ends), and the editor is told `Restored{at}`. The blob stays until a save or a discard.
    /// `false`: not the open editor of this file, a save is on its way, or there is no blob.
    pub(in crate::engine_host::viewer) fn office_restore(&mut self, file_id: &str) -> bool {
        let Some(store) = self.office_recovery_store() else {
            return false;
        };
        let session = self.engine.session_turn();
        let account = self.engine.account();
        let busy = self.changes.busy || self.office.work.is_saving();
        let Some(shown) = self
            .office
            .shown
            .as_mut()
            .filter(|shown| !busy && shown.file_id == file_id && !shown.ended && shown.session == session)
            .filter(|shown| shown.account == account && shown.view.is_some())
        else {
            return false;
        };
        let Some(kind) = shown.edit else {
            return false;
        };
        let (Some(offer), Some(bytes)) = (store.offer(file_id), store.restore(file_id)) else {
            return false;
        };
        // The restored edits come from the blob's version: a save is sent from it, so a newer server version answers
        // Conflict instead of being overwritten.
        let blob_base = store.base_of(file_id).unwrap_or(shown.base);
        shown.restored_base = (blob_base != shown.base).then_some(blob_base);
        let waker = self.engine.clone();
        let wake: crate::engine_host::office_host::Wake = Arc::new(move || waker.wake());
        let scratch = self.engine.office_scratch(session);
        let mut view = (self.office.edit_opener)(&shown.helper, kind, bytes, wake, scratch);
        view.set_kept(shown.kept);
        view.set_recovery(store, file_id, blob_base);
        view.push_answer(EditAnswer::Restored { at: offer.at });
        if let Some(mut old) = shown.view.replace(view) {
            old.close(CloseWhy::Closed);
        }
        true
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

    /// The door a lock seals through: `file_id`'s blob, for this session (a refusal only means no safety net).
    pub(crate) fn sink(&self, file_id: &str, base: i64) -> crate::engine_host::office_host::FlushSink {
        let (blobs, session, file_id) = (self.blobs.clone(), self.session, file_id.to_string());
        Arc::new(move |bytes| {
            let _ = blobs.put(session, &file_id, base, bytes);
        })
    }

    /// When the file's blob is from, if there is one.
    pub(crate) fn offer(&self, file_id: &str) -> Option<RecoveryOffer> {
        let entries = self.blobs.list(self.session).ok()?;
        let entry = entries.iter().find(|entry| entry.file_id == file_id)?;
        Some(RecoveryOffer { at: clock(entry.saved_at) })
    }

    /// The version the file's blob was edited from.
    pub(crate) fn base_of(&self, file_id: &str) -> Option<i64> {
        let entries = self.blobs.list(self.session).ok()?;
        entries.iter().find(|entry| entry.file_id == file_id).map(|entry| entry.base)
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
    #[cfg(test)]
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
