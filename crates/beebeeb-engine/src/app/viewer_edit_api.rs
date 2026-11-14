//! Keep a file on this PC and save a text edit, for the native viewer window: the pieces the viewer's switch and
//! its editor ask of the engine, each for the session of the press (nothing changes unless the session in memory
//! is still that one).
//!
//! - [`Engine::edit_facts_for`]: the version the local state holds, whether the file is kept on this PC and
//!   whether the account may write it. Local state only, no network.
//! - [`Engine::save_text_for`]: the text as the file's next version, bound to the version it was edited from. It
//!   goes through the one write path every local edit takes (`EngineBridge::queue_file_provider_modify_from`: the staged
//!   copy, the queued upload, the sync's own conflict handling). A file whose version here is already past the
//!   base is [`SaveEnd::Conflict`] and nothing is written.
//! - [`Engine::save_text_copy_for`]: Keep both: the text as a new file next to the first, under the
//!   Keep-both name every conflict copy takes.
//!
//! BLOCKS: the state database and, for a save, up to [`LAND_WAIT`] for the upload to land: a worker's. Must not:
//! log a name, a path or a byte of the text. The text is held in an anonymous file the queue copies from, never
//! under a name of its own.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::engine_bridge::{EngineBridge, FinderWriteItemKind, FinderWriteOutcome, FinderWriteTarget, keep_both_name};
use crate::state_db::PinState;

/// How long a save waits for the upload to land before it says "stored here, sent later".
pub const LAND_WAIT: Duration = Duration::from_secs(20);
const LAND_POLL: Duration = Duration::from_millis(150);

/// A save that ended as [`SaveEnd::Queued`]: which upload carries it, from which base, for which session. Once that
/// upload has landed, the version it became is the viewer's base, so a second save from the same viewer is not a
/// conflict with the first. Keyed by file id; a session that moved on makes a record void.
struct QueuedSave {
    session: SessionGeneration,
    base: i64,
    op_id: String,
}

static QUEUED_SAVES: Mutex<Option<HashMap<String, QueuedSave>>> = Mutex::new(None);

fn remember_queued(file_id: &str, record: QueuedSave) {
    if let Ok(mut all) = QUEUED_SAVES.lock() {
        all.get_or_insert_with(HashMap::new).insert(file_id.to_string(), record);
    }
}

fn forget_queued(file_id: &str) {
    if let Ok(mut all) = QUEUED_SAVES.lock() {
        if let Some(map) = all.as_mut() {
            map.remove(file_id);
        }
    }
}

/// The base a save from `base` continues from: `base`, or, when this viewer's own queued save has landed since, the
/// version that save became (`current`, read by the caller from the local state).
fn base_after_own_landed_save(
    file_id: &str,
    session: SessionGeneration,
    base: i64,
    current: i64,
    op_is_gone: impl FnOnce(&str) -> bool,
) -> i64 {
    if current == base {
        return base;
    }
    let landed = QUEUED_SAVES.lock().ok().is_some_and(|all| {
        all.as_ref()
            .and_then(|map| map.get(file_id))
            .is_some_and(|q| q.session == session && q.base == base && current > base && op_is_gone(&q.op_id))
    });
    if landed {
        forget_queued(file_id);
        current
    } else {
        base
    }
}

/// What the local state says about a file the viewer has open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditFacts {
    /// The version the local state holds now (0: none recorded).
    pub version: i64,
    /// Kept on this PC (pinned, or inside a kept folder).
    pub kept: bool,
    /// The account may change the file.
    pub can_write: bool,
}

/// Why nothing was done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditRefused {
    /// No session, or the session moved on since the press.
    NotSignedIn,
    /// The local state is not this account's, or cannot be read.
    Unavailable,
    /// The account may not change this file.
    ReadOnly,
    /// The write was refused or failed; nothing is stored.
    Failed,
}

/// What a save came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveEnd {
    /// Stored and sent: the file's new version.
    Saved { version: i64 },
    /// Stored on this PC; the upload is not done yet (it is sent as soon as Beebeeb can be reached).
    Queued,
    /// The file is already past the version the edit began from; nothing was written.
    Conflict { theirs: i64 },
}

/// The file Keep both made.
#[derive(Clone, PartialEq, Eq)]
pub struct NewCopy {
    pub id: String,
    pub name: String,
}

impl<C: CredentialStores> Engine<C> {
    /// See the module's doc.
    pub fn edit_facts_for(&self, session: SessionGeneration, file_id: &str) -> Result<EditFacts, EditRefused> {
        let state = self.local_state().map_err(|_| EditRefused::Unavailable)?;
        if state.session() != session {
            return Err(EditRefused::NotSignedIn);
        }
        let read = state
            .with(|db| db.get_file_contract_state(file_id))
            .map_err(|_| EditRefused::NotSignedIn)?
            .map_err(|_| EditRefused::Unavailable)?;
        let contract = read.ok_or(EditRefused::Unavailable)?;
        Ok(EditFacts {
            version: contract.current_version,
            kept: contract.effective_pin_state() == PinState::Pinned,
            can_write: contract.can_write(),
        })
    }

    /// See the module's doc. `base` is the version the text was edited from.
    pub fn save_text_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        text: &str,
        base: i64,
    ) -> Result<SaveEnd, EditRefused> {
        let (token, key) = self.edit_credentials(session)?;
        let state = self.local_state_for_changes().map_err(|_| EditRefused::Unavailable)?;
        if state.session() != session {
            return Err(EditRefused::NotSignedIn);
        }
        let queued = state
            .with_shared(|db| {
                let contract = db.get_file_contract_state(file_id).ok().flatten()?;
                let row = db.get_file(file_id).ok().flatten()?;
                Some((contract, row))
            })
            .map_err(|_| EditRefused::NotSignedIn)?;
        let Some((contract, row)) = queued else {
            return Err(EditRefused::Unavailable);
        };
        if !contract.can_write() {
            return Err(EditRefused::ReadOnly);
        }
        // The viewer's own earlier save, once landed, moved the version: that is not another device.
        let base = base_after_own_landed_save(file_id, session, base, contract.current_version, |op_id| {
            state
                .with_shared(|db| db.get_operation(op_id).ok().is_some_and(|op| op.is_none()))
                .unwrap_or(false)
        });
        // Already past the version this edit began from: the person decides (never overwritten here).
        if contract.current_version != base {
            return Ok(SaveEnd::Conflict {
                theirs: contract.current_version,
            });
        }
        let path = row.path.trim_matches('/').to_string();
        let leaf = path.rsplit('/').next().unwrap_or(&path).to_string();
        let target = FinderWriteTarget {
            file_id: Some(file_id.to_string()),
            parent_id: contract.parent_id.clone(),
            filename: leaf,
            rel_path: Some(path),
            kind: FinderWriteItemKind::File,
            contents_path: Some("viewer-edit".to_string()),
            content_type: contract.content_type.clone(),
            base_version_identifier: Some(base.to_string()),
        };
        let held = anonymous_file(text).ok_or(EditRefused::Failed)?;
        let outcome = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                EngineBridge::new(self.identity, db, api)
                    .queue_file_provider_modify_from(target, Some(&held))
                    .map(|write| write.outcome)
            })
            .map_err(|_| EditRefused::NotSignedIn)?
            .map_err(|_| EditRefused::Failed)?;
        let FinderWriteOutcome::Queued { op_id, .. } = outcome else {
            return Err(EditRefused::Failed);
        };
        // Wait, bounded, for the upload to land: the op leaves the queue and the version moves past the base.
        let deadline = Instant::now() + LAND_WAIT;
        while self.is_syncing() && Instant::now() < deadline {
            let seen = state.with(|db| {
                let gone = db.get_operation(&op_id).ok().is_some_and(|op| op.is_none());
                let version = db
                    .get_file_contract_state(file_id)
                    .ok()
                    .flatten()
                    .map(|contract| contract.current_version);
                (gone, version)
            });
            match seen {
                Err(_) => return Err(EditRefused::NotSignedIn),
                Ok((true, Some(version))) if version > base => {
                    forget_queued(file_id);
                    return Ok(SaveEnd::Saved { version });
                }
                Ok(_) => std::thread::sleep(LAND_POLL),
            }
        }
        remember_queued(file_id, QueuedSave { session, base, op_id });
        Ok(SaveEnd::Queued)
    }

    /// See the module's doc: Keep both. `device` and `hhmm` name the copy as every conflict copy is named.
    pub fn save_text_copy_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        text: &str,
        device: &str,
        hhmm: &str,
    ) -> Result<NewCopy, EditRefused> {
        let (token, key) = self.edit_credentials(session)?;
        let state = self.local_state_for_changes().map_err(|_| EditRefused::Unavailable)?;
        if state.session() != session {
            return Err(EditRefused::NotSignedIn);
        }
        let read = state
            .with(|db| {
                let contract = db.get_file_contract_state(file_id).ok().flatten()?;
                let row = db.get_file(file_id).ok().flatten()?;
                let taken = db.child_names_lowercase(contract.parent_id.as_deref()).ok()?;
                Some((contract, row, taken))
            })
            .map_err(|_| EditRefused::NotSignedIn)?;
        let Some((contract, row, taken)) = read else {
            return Err(EditRefused::Unavailable);
        };
        if !contract.can_write() {
            return Err(EditRefused::ReadOnly);
        }
        let path = row.path.trim_matches('/').to_string();
        let (parent_rel, leaf) = match path.rsplit_once('/') {
            Some((parent, leaf)) => (Some(parent.to_string()), leaf.to_string()),
            None => (None, path.clone()),
        };
        let name = keep_both_name(&leaf, device, hhmm, &taken);
        let target = FinderWriteTarget {
            file_id: None,
            parent_id: contract.parent_id.clone(),
            filename: name.clone(),
            rel_path: Some(match parent_rel {
                Some(parent) => format!("{parent}/{name}"),
                None => name.clone(),
            }),
            kind: FinderWriteItemKind::File,
            contents_path: Some("viewer-edit".to_string()),
            content_type: contract.content_type.clone(),
            base_version_identifier: None,
        };
        let held = anonymous_file(text).ok_or(EditRefused::Failed)?;
        let outcome = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                EngineBridge::new(self.identity, db, api)
                    .queue_file_provider_create_from(target, Some(&held))
                    .map(|write| write.outcome)
            })
            .map_err(|_| EditRefused::NotSignedIn)?
            .map_err(|_| EditRefused::Failed)?;
        match outcome {
            FinderWriteOutcome::Queued { file_id: Some(id), .. } => Ok(NewCopy { id, name }),
            _ => Err(EditRefused::Failed),
        }
    }

    /// The token and the key of the session `session` is, copied while it still is.
    fn edit_credentials(&self, session: SessionGeneration) -> Result<(String, Zeroizing<[u8; 32]>), EditRefused> {
        self.person_session(|current, generation| {
            (generation == session).then(|| (current.token.clone(), Zeroizing::new(current.master_key)))
        })
        .ok()
        .flatten()
        .ok_or(EditRefused::NotSignedIn)
    }
}

/// `text` in a file with no name, read from the start.
fn anonymous_file(text: &str) -> Option<std::fs::File> {
    let mut file = tempfile::tempfile().ok()?;
    file.write_all(text.as_bytes()).ok()?;
    file.flush().ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    Some(file)
}

#[cfg(test)]
mod queued_save_tests {
    use super::*;

    fn queued(file: &str, session: u64, base: i64) {
        remember_queued(
            file,
            QueuedSave {
                session: SessionGeneration::for_test(session),
                base,
                op_id: "op-1".into(),
            },
        );
    }

    /// The viewer's own queued save, once its upload has landed, is the base of the next save: not a conflict.
    #[test]
    fn a_second_save_after_the_first_landed_continues_from_the_version_it_became() {
        queued("queued-own-1", 3, 4);
        let s = SessionGeneration::for_test(3);
        // Not landed yet (the op is still queued, the version is the base): the base stays.
        assert_eq!(base_after_own_landed_save("queued-own-1", s, 4, 4, |_| false), 4);
        // The version moved but the op is still queued: not ours to adopt.
        assert_eq!(base_after_own_landed_save("queued-own-1", s, 4, 5, |_| false), 4);
        // Landed: the op is gone and the version moved past the base.
        assert_eq!(
            base_after_own_landed_save("queued-own-1", s, 4, 5, |op| op == "op-1"),
            5
        );
        // Used once: the same landing is not adopted twice.
        assert_eq!(base_after_own_landed_save("queued-own-1", s, 4, 6, |_| true), 4);
    }

    /// Review of desktop#424 (engine P2-3): the viewer's queued save landed as base + 1, then another device saved
    /// base + 2 and the tick recorded it. The next save from the viewer is a conflict: only the version the viewer's
    /// own op became is adopted, never a version past it.
    #[test]
    fn a_version_past_the_one_the_own_save_became_stays_a_conflict() {
        queued("queued-own-3", 3, 4);
        let s = SessionGeneration::for_test(3);
        assert_eq!(
            base_after_own_landed_save("queued-own-3", s, 4, 6, |op| op == "op-1"),
            4,
            "another device's version after the own one is not adopted"
        );
    }

    /// Another device's save is never adopted: no record of this viewer's, another session, or another base.
    #[test]
    fn another_device_s_newer_version_stays_a_conflict() {
        let s = SessionGeneration::for_test(3);
        assert_eq!(base_after_own_landed_save("queued-own-none", s, 4, 5, |_| true), 4);
        queued("queued-own-2", 3, 4);
        assert_eq!(
            base_after_own_landed_save("queued-own-2", SessionGeneration::for_test(4), 4, 5, |_| true),
            4,
            "a session that moved on"
        );
        assert_eq!(
            base_after_own_landed_save("queued-own-2", s, 2, 5, |_| true),
            2,
            "another base"
        );
    }
}
