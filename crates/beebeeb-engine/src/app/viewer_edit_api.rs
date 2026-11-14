//! Keep a file on this PC and save a text edit, for the native viewer window: the pieces the viewer's switch and
//! its editor ask of the engine, each for the session of the press (nothing changes unless the session in memory
//! is still that one).
//!
//! - [`Engine::edit_facts_for`]: the version the local state holds, whether the file is kept on this PC and
//!   whether the account may write it. Local state only, no network.
//! - [`Engine::save_text_for`]: the text as the file's next version, bound to the version it was edited from. It
//!   goes through the one write path every local edit takes (`EngineBridge::queue_finder_modify_from`: the staged
//!   copy, the queued upload, the sync's own conflict handling). A file whose version here is already past the
//!   base is [`SaveEnd::Conflict`] and nothing is written.
//! - [`Engine::save_text_copy_for`]: Keep both: the text as a new file next to the first, under the
//!   Keep-both name every conflict copy takes.
//!
//! BLOCKS: the state database and, for a save, up to [`LAND_WAIT`] for the upload to land: a worker's. Must not:
//! log a name, a path or a byte of the text. The text is held in an anonymous file the queue copies from, never
//! under a name of its own.

use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;
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
                EngineBridge::new(self.identity, db, api).queue_finder_modify_from(target, Some(&held))
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
                Ok((true, Some(version))) if version > base => return Ok(SaveEnd::Saved { version }),
                Ok(_) => std::thread::sleep(LAND_POLL),
            }
        }
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
                EngineBridge::new(self.identity, db, api).queue_finder_create_from(target, Some(&held))
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
        if !self.acct.session_unchanged_since(session) {
            return Err(EditRefused::NotSignedIn);
        }
        let guard = self.acct.session.lock().map_err(|_| EditRefused::NotSignedIn)?;
        let current = guard.as_ref().ok_or(EditRefused::NotSignedIn)?;
        if !self.acct.session_unchanged_since(session) {
            return Err(EditRefused::NotSignedIn);
        }
        Ok((current.token.clone(), Zeroizing::new(current.master_key)))
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
