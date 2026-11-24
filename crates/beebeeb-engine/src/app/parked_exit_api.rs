//! Task 2530: a way out of a parked upload, chosen on This PC's Sync history › Problems row, for the
//! session a person pressed under and no other ([`Engine::settle_parked_for`]). Declared from `owned_state.rs`, the
//! module that hands This PC its local state.
//!
//! - Keep this PC's, Keep the other, Keep both: `EngineBridge::resolve_parked` (`Local`, `Remote`, `Both`), against the
//!   file's version the row was read at. Keep the other first copies this PC's bytes into "Changes not sent" (on
//!   Windows and Linux the account's own folder in the engine's local state folder, `changes_not_sent_in`; on macOS
//!   the sign-out keep's place), an event folder named for the time, the file's path under it, as the macOS keep lays
//!   it out; it checks the copy is listed whole under the name meant, and only then lets the write go.
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
/// flushed, and it counts only once its folder lists it, inside the event folder, under the name meant, at the
/// source's length ([`kept_where_meant`]); a copy that failed takes its half-made event folder with it (only copies of
/// bytes still in the queue are in it). A plain copy on every platform: `sign_out::kept_writer` is the macOS
/// sign-out's (an APFS clone) and is not built elsewhere.
///
/// Data review of #458 (P1-1): a path that could lead out of the folder (a drive prefix, a parent step in either
/// separator) is refused before anything is made; on Windows each part is first made a name Windows takes
/// ([`part_windows_takes`]).
pub(crate) fn keep_in_changes_not_sent(
    root: &Path,
    event: &str,
    op_id: &str,
    rel: &str,
    source: &Path,
) -> io::Result<PathBuf> {
    use crate::sign_out::kept_names::{KeptRole, kept_relative_path};
    crate::path_safety::reject_unsafe_rel_path(rel)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the file's path cannot be kept as it is"))?;
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
        let components: Vec<String> = kept_relative_path(Some(rel), op_id, KeptRole::Newest)
            .iter()
            .map(|part| part_for_this_os(part))
            .collect();
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
        kept_where_meant(&event_dir, &kept, &name[0], source_meta.len())?;
        Ok(kept)
    })();
    if kept.is_err() {
        let _ = std::fs::remove_dir_all(&event_dir);
    }
    kept
}

/// `<state dir>/Changes not sent/<account digest>`: one account's folder of kept changes, named, as Keep there's
/// `replaced_folder_in`, by a digest of the account's user id (its email when no id is known), never the id itself.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) fn changes_not_sent_in(state_dir: &Path, account: &crate::account_binding::Identity) -> Option<PathBuf> {
    let who = account.user_id.as_deref().or(account.email.as_deref())?;
    Some(
        state_dir
            .join(crate::sign_out::kept_names::KEPT_FOLDER_NAME)
            .join(crate::app::conflicts_api::digest_name(who)),
    )
}

/// A kept copy counts only when it is inside `event_dir`, its own name is `name`, and its folder lists it under that
/// name as a file of `len` bytes. A name the file system reads another way is not listed so (on NTFS `a:b.txt` is a
/// hidden stream of a file `a`, whose length a look at the path reports all the same), and is refused.
fn kept_where_meant(event_dir: &Path, kept: &Path, name: &str, len: u64) -> io::Result<()> {
    let not_meant = || io::Error::other("the kept copy is not where it was meant to be");
    if !kept.starts_with(event_dir) {
        return Err(not_meant());
    }
    let own_name = kept.file_name().ok_or_else(not_meant)?;
    if own_name != std::ffi::OsStr::new(name) {
        return Err(not_meant());
    }
    let folder = kept.parent().ok_or_else(not_meant)?;
    let listed = std::fs::read_dir(folder)?
        .flatten()
        .find(|entry| entry.file_name() == own_name)
        .ok_or_else(not_meant)?;
    let meta = listed.metadata()?;
    if !meta.is_file() || meta.len() != len {
        return Err(io::Error::other("the kept copy is not whole"));
    }
    Ok(())
}

#[cfg(windows)]
fn part_for_this_os(part: &str) -> String {
    part_windows_takes(part)
}

#[cfg(not(windows))]
fn part_for_this_os(part: &str) -> String {
    part.to_string()
}

/// One part of a kept path as Windows takes it, the way `known_folder::segment` makes a device name a folder name:
/// each character Windows refuses in a name (`\ / : * ? " < > |`, the control characters) becomes `_`, the trailing
/// dots and spaces Windows would drop go, and a device name (`NUL`, `CON.txt`, `COM1`, `LPT²`…) gets a `_` in front. A
/// part left empty is `_`. Pure; tested on every platform, used on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn part_windows_takes(part: &str) -> String {
    use crate::sign_out::kept_names::{MAX_NAME_BYTES, cut_name};
    let mapped: String = part
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = mapped.trim_end_matches(['.', ' ']);
    let mut out = if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    };
    let stem = out
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|tail| {
            matches!(
                tail,
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    };
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT") {
        out.insert(0, '_');
    }
    if out.len() > MAX_NAME_BYTES {
        out = cut_name(&out).trim_end_matches(['.', ' ']).to_string();
    }
    out
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
        let root = self.changes_not_sent_root();
        self.settle_parked_with(session, op_id, pick, shown_version, device, root.as_deref())
    }

    /// Where Keep the other keeps this PC's bytes. Windows and Linux (data review of #458, P2-2): the signed-in
    /// account's own folder in the engine's local state folder ([`changes_not_sent_in`]): on Windows that is under
    /// `%LOCALAPPDATA%`, never the roaming folder a domain profile copies off the PC, and beside Keep there's copies.
    /// `None` without a signed-in account (Keep the other then refuses). macOS: the sign-out keep's place (§7.1).
    fn changes_not_sent_root(&self) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            crate::sign_out::kept_names::kept_root_for_this_app(self.identity)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.session_identity()
                .ok()
                .and_then(|account| changes_not_sent_in(&self.config.state_dir, &account))
        }
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
