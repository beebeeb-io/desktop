//! Task 2530 (wave 2, slice S3): a way out of a parked upload, chosen on This PC's Sync history › Problems row, for the
//! session a person pressed under and no other ([`Engine::settle_parked_for`]). Declared from `owned_state.rs`, the
//! module that hands This PC its local state.
//!
//! - Keep this PC's, Keep the other, Keep both: `EngineBridge::resolve_parked` (`Local`, `Remote`, `Both`), against the
//!   file's version the row was read at. Keep the other first copies this PC's bytes into "Changes not sent" in the app's
//!   data folder (`sign_out::kept_names::kept_root`, an event folder named for the time, the file's path under it, as the
//!   macOS keep lays it out), checks the copy is whole, and only then lets the write go.
//! - Remove from the list: `StateDb::remove_parked_without_bytes`, only while no bytes of the change are on this PC.
//! - Try again is not here: it is the row's Try again (`StateDb::retry_unsent_change`, then `try_parked_again`).
//!
//! Every choice checks the write again inside the transaction that acts on it; one that moved is [`Settled::Moved`],
//! with nothing changed. A session that moved on is [`SettleRefused::NotSignedIn`], with nothing done.
//!
//! BLOCKS on the state database and a file copy: a worker's. Must not: log a name, a path, an id, a token or a key.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::app::{CredentialStores, Engine, LocalStateRefused};
use crate::engine_bridge::{EngineBridge, ParkedChoice, ParkedResolution};
use crate::state_db::RemoveParked;

#[cfg(test)]
#[path = "parked_exit_api_tests.rs"]
mod tests;

/// What the person picked on the row (the engine's own list of the row's ways out, `drive_v2::status::Exit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkedPick {
    KeepThisPcs,
    KeepOther,
    KeepBoth,
    RemoveFromList,
}

/// What a pick did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// Keep this PC's: the same change is sent again, on the version shown.
    SentAgain,
    /// Keep the other: this PC's bytes are kept in "Changes not sent" (`kept`), and the file reads Beebeeb's version.
    KeptOther { kept: PathBuf },
    /// Keep both: this PC's bytes are queued as a new file beside the original.
    KeptBoth,
    /// Remove from the list: the change is gone from the list; the file on Beebeeb is as it was.
    Removed,
    /// The change moved since the row was read (sent, settled, another change queued, the file past the version
    /// shown): nothing changed.
    Moved,
}

/// Why a pick was not applied. Nothing changed in any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleRefused {
    /// The press's session is gone or moved on.
    NotSignedIn,
    /// The account's local state is not handed out.
    NoLocalState,
    /// This PC's bytes could not be kept, so the write did not go.
    CopyFailed,
    /// Bytes of the change are on this PC, so it was not removed.
    BytesHere,
    /// The work failed; the change is still listed as it was.
    Failed,
}

fn local(refusal: LocalStateRefused) -> SettleRefused {
    match refusal {
        LocalStateRefused::SessionChanged
        | LocalStateRefused::SessionEnded
        | LocalStateRefused::SignedOut
        | LocalStateRefused::Locked => SettleRefused::NotSignedIn,
        _ => SettleRefused::NoLocalState,
    }
}

/// Keep `source`, a parked write's staged copy, at `<root>/<event>/<the file's path>` (`rel`, the vault path): a new
/// event folder (`event`, then `event (2)`…), the path's folders inside it, a file never written over. The copy is
/// flushed, and its length is checked against the source's before it is handed back; a copy that failed takes its
/// half-made event folder with it (only copies of bytes still in the queue are in it). A plain copy on every platform:
/// `sign_out::kept_writer` is the macOS sign-out's (an APFS clone) and is not built elsewhere.
pub(crate) fn keep_in_changes_not_sent(
    root: &Path,
    event: &str,
    op_id: &str,
    rel: &str,
    source: &Path,
) -> io::Result<PathBuf> {
    use crate::sign_out::kept_names::{KeptRole, kept_relative_path};
    let source_meta = std::fs::symlink_metadata(source)?;
    if !source_meta.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the staged copy is not a file",
        ));
    }
    std::fs::create_dir_all(root)?;
    let mut n = 1u32;
    let event_dir = loop {
        let name = if n == 1 {
            event.to_string()
        } else {
            format!("{event} ({n})")
        };
        let candidate = root.join(name);
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    };
    let kept = (|| -> io::Result<PathBuf> {
        let components = kept_relative_path(Some(rel), op_id, KeptRole::Newest);
        let (folders, name) = components.split_at(components.len() - 1);
        let folder = folders.iter().fold(event_dir.clone(), |path, part| path.join(part));
        std::fs::create_dir_all(&folder)?;
        let kept = folder.join(&name[0]);
        let mut from = std::fs::File::open(source)?;
        let mut to = std::fs::OpenOptions::new().write(true).create_new(true).open(&kept)?;
        io::copy(&mut from, &mut to)?;
        to.flush()?;
        to.sync_all()?;
        drop(to);
        // Lead, 2026-11-15 14:06: the copy is there, whole, before anything acts.
        if std::fs::metadata(&kept)?.len() != source_meta.len() {
            return Err(io::Error::other("the kept copy is not whole"));
        }
        Ok(kept)
    })();
    if kept.is_err() {
        let _ = std::fs::remove_dir_all(&event_dir);
    }
    kept
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES: apply `pick` to the parked change `op_id`, for the session a person pressed under and no other, against
    /// `shown_version` (the file's version the row was read at; a Keep without it is `Moved`). `device` names this PC
    /// in a Keep both copy's name (the window's own name for this PC; the engine caps it). See the module's doc.
    pub fn settle_parked_for(
        &self,
        session: SessionGeneration,
        op_id: &str,
        pick: ParkedPick,
        shown_version: Option<i64>,
        device: &str,
    ) -> Result<Settled, SettleRefused> {
        let root = crate::sign_out::kept_names::kept_root_for_this_app(self.identity);
        self.settle_parked_with(session, op_id, pick, shown_version, device, root.as_deref())
    }

    /// [`Self::settle_parked_for`] with the "Changes not sent" folder given (`None`: none can be named, so Keep the
    /// other refuses).
    pub(crate) fn settle_parked_with(
        &self,
        session: SessionGeneration,
        op_id: &str,
        pick: ParkedPick,
        shown_version: Option<i64>,
        device: &str,
        kept_root: Option<&Path>,
    ) -> Result<Settled, SettleRefused> {
        if self.acct.session.is_poisoned() {
            return Err(SettleRefused::NotSignedIn);
        }
        let (token, key) = self
            .acct
            .while_session_is(&self.writes, session, |current| {
                (current.token.clone(), Zeroizing::new(current.master_key))
            })
            .ok_or(SettleRefused::NotSignedIn)?;
        let state = self.local_state_for_changes().map_err(local)?;
        if state.session() != session {
            return Err(SettleRefused::NotSignedIn);
        }
        let choice = match pick {
            ParkedPick::RemoveFromList => {
                return match state.with(|db| db.remove_parked_without_bytes(op_id)).map_err(local)? {
                    Ok(RemoveParked::Removed) => Ok(Settled::Removed),
                    Ok(RemoveParked::Moved) => Ok(Settled::Moved),
                    Ok(RemoveParked::BytesHere) => Err(SettleRefused::BytesHere),
                    Err(_) => Err(SettleRefused::Failed),
                };
            }
            ParkedPick::KeepThisPcs => ParkedChoice::Local,
            ParkedPick::KeepOther => ParkedChoice::Remote,
            ParkedPick::KeepBoth => ParkedChoice::Both,
        };
        let hhmm = chrono::Local::now().format("%H.%M").to_string();
        let event = crate::sign_out::kept_names::event_folder_name(chrono::Local::now());
        let mut copy_failed = false;
        let done = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                let mut keep_copy = |source: &Path, rel: &str| -> anyhow::Result<PathBuf> {
                    let kept = kept_root
                        .ok_or_else(|| io::Error::other("no folder for kept changes"))
                        .and_then(|root| keep_in_changes_not_sent(root, &event, op_id, rel, source));
                    copy_failed = kept.is_err();
                    Ok(kept?)
                };
                bridge.resolve_parked(op_id, choice, shown_version, &mut keep_copy, device, &hhmm)
            })
            .map_err(local)?;
        let settled = match done {
            Ok(ParkedResolution::Rebased { .. }) => Settled::SentAgain,
            Ok(ParkedResolution::KeptOther { kept_copy }) => Settled::KeptOther {
                kept: PathBuf::from(kept_copy),
            },
            Ok(ParkedResolution::KeptBoth { .. }) => Settled::KeptBoth,
            Ok(ParkedResolution::Moved) => return Ok(Settled::Moved),
            Err(_) if copy_failed => return Err(SettleRefused::CopyFailed),
            Err(_) => return Err(SettleRefused::Failed),
        };
        self.request_resnapshot();
        Ok(settled)
    }
}
