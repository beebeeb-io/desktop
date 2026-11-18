//! Keep a file on this PC and save a text edit, for the native viewer window: the pieces the viewer's switch and
//! its editor ask of the engine, each for the session of the press (nothing changes unless the session in memory
//! is still that one).
//!
//! - [`Engine::edit_facts_for`]: the version the local state holds, whether the file is kept on this PC and
//!   whether the account may write it. Local state only, no network.
//! - [`Engine::save_text_for`]: the text as the file's next version, bound to the version it was edited from. It
//!   goes through the one write path every local edit takes (`EngineBridge::queue_file_provider_modify_from`: the staged
//!   copy, the queued upload, the sync's own conflict handling). A file whose version here is already past the
//!   base is [`SaveEnd::Conflict`] and nothing is written. A save that would be queued after an unsent write of the
//!   file that is not its own opening's is [`EditRefused::EarlierChangeNotSent`] ([`EditRefused::EarlierChangeHeld`]
//!   when that write is held back), and nothing is written.
//! - [`Engine::save_text_copy_for`]: Keep both: the text as a new file next to the first, under the
//!   Keep-both name every conflict copy takes.
//!
//! BLOCKS: the state database and, for a save, up to [`LAND_WAIT`] for the upload to land: a worker's. Must not:
//! log a name, a path or a byte of the text. The text is held in an anonymous file the queue copies from, never
//! under a name of its own.

use std::collections::{BTreeMap, HashMap};
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::engine_bridge::{
    BehindAnotherWrite, EngineBridge, FinderWriteItemKind, FinderWriteOutcome, FinderWriteTarget, keep_both_name,
};
use crate::state_db::PinState;
use crate::write_token::ChainAfter;

/// How long a save waits for the upload to land before it says "stored here, sent later".
pub const LAND_WAIT: Duration = Duration::from_secs(20);
const LAND_POLL: Duration = Duration::from_millis(150);

/// A save that ended as [`SaveEnd::Queued`]: which upload carries it, from which base, for which session and which
/// opening (`epoch`), under which write. Once that upload has landed, the version it became (the landing records it
/// for that write) is that opening's own: see [`ViewerSaves`].
#[derive(Clone)]
struct QueuedSave {
    session: SessionGeneration,
    /// The opening the save began under ([`Engine::viewer_opening`]).
    epoch: u64,
    base: i64,
    op_id: String,
    /// The write's token (`FpWrite::token`): the landing records the version it became under this write only.
    token: Option<String>,
}

/// What one opening of a file in the viewer saved of it: the latest save answered [`SaveEnd::Queued`] whose landing
/// is not known yet, and every version its saves became, with the base each was saved from. Each version is one
/// write's own landing, so keeping them is safe: a version another device made is never one of them.
///
/// The record belongs to ONE opening (`epoch`, delta checks of desktop#432): a new opening of the file
/// ([`Engine::viewer_opening`]) replaces it with an empty one of a new epoch, and every write to it
/// ([`remember_queued`], [`note_landed`], the read in [`own_versions`]) is refused unless it carries the record's own
/// session and epoch. A save or a read of an earlier opening, however late it runs, can never create, extend or change
/// the record of a later one: the check and the write are under one lock, and an epoch is never handed out twice.
pub(crate) struct ViewerSaves {
    session: SessionGeneration,
    epoch: u64,
    queued: Option<QueuedSave>,
    /// The version a save became → the base it was saved from.
    became: BTreeMap<i64, i64>,
}

/// The viewer's records of one account's runtime (`AccountRuntime::viewer_saves`), by file id. Emptied whenever its
/// session moves (a lock, a sign-out, a sign-in: `advance_session_generation_holding_session_write_lock`).
#[derive(Default)]
pub(crate) struct ViewerSavesStore(Mutex<HashMap<String, ViewerSaves>>);

impl ViewerSavesStore {
    /// Every record goes.
    pub(crate) fn clear(&self) {
        if let Ok(mut all) = self.0.lock() {
            all.clear();
        }
    }
}

/// The last epoch handed out, process-wide: never moved back, so an epoch is never used twice.
static EPOCHS: AtomicU64 = AtomicU64::new(0);

/// A new opening of `file_id` for `session`: the next epoch, and an empty record only that epoch writes. The records
/// of other sessions go (they can never be written again).
fn open_saves(store: &ViewerSavesStore, file_id: &str, session: SessionGeneration) -> u64 {
    let epoch = EPOCHS.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut all) = store.0.lock() {
        all.retain(|_, saves| saves.session == session);
        all.insert(
            file_id.to_string(),
            ViewerSaves {
                session,
                epoch,
                queued: None,
                became: BTreeMap::new(),
            },
        );
    }
    epoch
}

/// `change` on the record of `file_id`, only while it is the record of `session` and `epoch`: never one created
/// here, never another opening's. `false`: refused.
fn with_saves(
    store: &ViewerSavesStore,
    file_id: &str,
    session: SessionGeneration,
    epoch: u64,
    change: impl FnOnce(&mut ViewerSaves),
) -> bool {
    let Ok(mut all) = store.0.lock() else {
        return false;
    };
    match all
        .get_mut(file_id)
        .filter(|saves| saves.session == session && saves.epoch == epoch)
    {
        Some(saves) => {
            change(saves);
            true
        }
        None => false,
    }
}

/// The latest queued save of its opening (an earlier one of the same opening can no longer be told apart once a
/// later write holds the file).
fn remember_queued(store: &ViewerSavesStore, file_id: &str, record: QueuedSave) -> bool {
    let (session, epoch) = (record.session, record.epoch);
    with_saves(store, file_id, session, epoch, |saves| saves.queued = Some(record))
}

/// `queued` landed as `version`: from now on `version` is its opening's own (noted before the record is let go, so
/// no read finds neither). Refused for any other opening.
fn note_landed(store: &ViewerSavesStore, file_id: &str, queued: &QueuedSave, version: i64) -> bool {
    with_saves(store, file_id, queued.session, queued.epoch, |saves| {
        saves.became.insert(version, queued.base);
        if saves.queued.as_ref().is_some_and(|q| q.op_id == queued.op_id) {
            saves.queued = None;
        }
    })
}

/// `what` of the record of `file_id`, copied out, while it is the record of `session` and `epoch`.
fn record_of<T>(
    store: &ViewerSavesStore,
    file_id: &str,
    session: SessionGeneration,
    epoch: u64,
    what: impl FnOnce(&ViewerSaves) -> T,
) -> Option<T> {
    let all = store.0.lock().ok()?;
    let saves = all.get(file_id)?;
    (saves.session == session && saves.epoch == epoch).then(|| what(saves))
}

/// The versions the saves of the opening `epoch` made of `file_id` became, each with its base. Its queued save, when
/// its landing is known now (`landed_as`, asked with the record copied out: no lock is held while the local state is
/// read), is noted first, under that same opening only.
fn own_versions(
    store: &ViewerSavesStore,
    file_id: &str,
    session: SessionGeneration,
    epoch: u64,
    landed_as: impl Fn(&QueuedSave) -> Option<i64>,
) -> BTreeMap<i64, i64> {
    if let Some(record) = record_of(store, file_id, session, epoch, |saves| saves.queued.clone()).flatten() {
        if let Some(version) = landed_as(&record).filter(|version| *version > record.base) {
            note_landed(store, file_id, &record, version);
        }
    }
    record_of(store, file_id, session, epoch, |saves| saves.became.clone()).unwrap_or_default()
}

/// The version that landing recorded for `queued`'s write, from the local state: once its op has left the queue,
/// while the file's held write is still this save's. `None` when that is not known: still queued, a later save's
/// write, or an op that left the queue without landing.
fn landed_version(db: &crate::state_db::StateDb, file_id: &str, queued: &QueuedSave) -> Option<i64> {
    let token = queued.token.as_deref()?;
    let gone = db.get_operation(&queued.op_id).ok().is_some_and(|op| op.is_none());
    if !gone {
        return None;
    }
    let held = db.held_write(file_id).ok().flatten()?;
    if held.token() != token {
        return None;
    }
    held.version
}

/// The base a save from `base` continues from: `base`, or, when `current` (read by the caller from the local state)
/// is a version a save of this same opening (`epoch`) made from `base` became, that version. Anything else stays
/// `base`, so the save answers Conflict: another device's version after the own one (review of desktop#424), an op
/// that left the queue without landing, a save of an earlier opening (delta checks of desktop#432).
fn base_after_own_landed_save(
    store: &ViewerSavesStore,
    file_id: &str,
    session: SessionGeneration,
    epoch: u64,
    base: i64,
    current: i64,
    landed_as: impl Fn(&QueuedSave) -> Option<i64>,
) -> i64 {
    if current <= base {
        return base;
    }
    let own = own_versions(store, file_id, session, epoch, landed_as);
    if own.get(&current) == Some(&base) {
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
    /// The version the local state holds is one the saves of the asking opening became (a save answered
    /// [`SaveEnd::Queued`] included, once it landed): not another device's, and not another opening's.
    pub own: bool,
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
    /// An earlier change to this file on this PC that is not this opening's own (an earlier opening's save, a File
    /// Provider save) has not been sent yet. Queued after it, this save would replace it unasked once both land
    /// (review of desktop#432, delta 3). Nothing is stored; the edit stays with the viewer.
    EarlierChangeNotSent,
    /// As [`Self::EarlierChangeNotSent`], but the earlier change to this file on this PC is held back (parked: its
    /// write used up its tries; or a held edit of task 2524): it is not sent until it is settled in Sync history.
    /// Nothing is stored; the edit stays with the viewer.
    EarlierChangeHeld,
}

/// What a save came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveEnd {
    /// Stored and sent: the file's new version.
    Saved { version: i64 },
    /// Stored on this PC; the upload is not done yet (it is sent as soon as Beebeeb can be reached). `base`: the
    /// version it was saved from (the one this save continued from, which the view's text is then based on).
    Queued { base: i64 },
    /// The file is already past the version the edit began from; nothing was written.
    Conflict { theirs: i64 },
}

#[path = "viewer_edit_held.rs"]
mod viewer_edit_held;

/// The file Keep both made.
#[derive(Clone, PartialEq, Eq)]
pub struct NewCopy {
    pub id: String,
    pub name: String,
}

impl<C: CredentialStores> Engine<C> {
    /// See the module's doc. `opening`: the epoch of the viewer's opening that asks ([`Self::viewer_opening`]).
    pub fn edit_facts_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        opening: u64,
    ) -> Result<EditFacts, EditRefused> {
        let state = self.local_state().map_err(|_| EditRefused::Unavailable)?;
        if state.session() != session {
            return Err(EditRefused::NotSignedIn);
        }
        // The version first, then whether it is one the view's own saves became: a landing between the two reads
        // makes the version older than the landing, never another device's.
        let read = state
            .with(|db| {
                let contract = db.get_file_contract_state(file_id)?;
                let own = contract.as_ref().is_some_and(|contract| {
                    own_versions(&self.acct.viewer_saves, file_id, session, opening, |queued| {
                        landed_version(db, file_id, queued)
                    })
                    .contains_key(&contract.current_version)
                });
                Ok::<_, rusqlite::Error>((contract, own))
            })
            .map_err(|_| EditRefused::NotSignedIn)?
            .map_err(|_| EditRefused::Unavailable)?;
        let (contract, own) = read;
        let contract = contract.ok_or(EditRefused::Unavailable)?;
        Ok(EditFacts {
            version: contract.current_version,
            kept: contract.effective_pin_state() == PinState::Pinned,
            can_write: contract.can_write(),
            own,
        })
    }

    /// A new opening of `file_id` in the viewer (open, Try again, Show it), for `session`: its epoch, which it passes
    /// with every save and every read of the version. It holds none of the earlier openings' saves, so none of them is
    /// its own (delta checks of desktop#432). While an earlier opening's save is still unsent (queued, or held back),
    /// a save of the new opening is refused ([`EditRefused::EarlierChangeNotSent`]), never queued after it. Once that
    /// save has landed, the version it became is announced as a newer version, and a save from the new opening's
    /// version answers Conflict, never continued from it. No write of an earlier opening reaches its record, whenever
    /// it runs.
    pub fn viewer_opening(&self, session: SessionGeneration, file_id: &str) -> u64 {
        open_saves(&self.acct.viewer_saves, file_id, session)
    }

    /// See the module's doc. `base` is the version the text was edited from; `opening`, the epoch of the viewer's
    /// opening that saves ([`Self::viewer_opening`]).
    pub fn save_text_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        text: &str,
        base: i64,
        opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        self.save_bytes_for(session, file_id, text.as_bytes(), base, opening)
    }

    /// [`Self::save_text_for`] for any bytes (the Office save shares it: `office_edit_api`). One body for both, so
    /// the opening epoch, the Conflict answer, the chain rule and the land wait can never differ between them.
    pub(in crate::app) fn save_bytes_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &[u8],
        base: i64,
        opening: u64,
    ) -> Result<SaveEnd, EditRefused> {
        self.save_bytes_sealed_for(session, file_id, bytes, base, opening, None)
    }

    /// [`Self::save_bytes_for`]; `seal_kind` `Some(kind)` stages the bytes sealed (an Office save: `office_edit_api`).
    pub(in crate::app) fn save_bytes_sealed_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &[u8],
        base: i64,
        opening: u64,
        seal_kind: Option<u8>,
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
        // The viewer's own earlier save, once landed, moved the version to the one it became: that is not another
        // device. A version past it is.
        let saves = &self.acct.viewer_saves;
        let base = base_after_own_landed_save(
            saves,
            file_id,
            session,
            opening,
            base,
            contract.current_version,
            |queued| {
                state
                    .with_shared(|db| landed_version(&db, file_id, queued))
                    .ok()
                    .flatten()
            },
        );
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
        // A sealed save never puts the plaintext in a file of its own: the one staged copy is the sealed blob.
        let held = match seal_kind {
            Some(_) => None,
            None => Some(anonymous_file(bytes).ok_or(EditRefused::Failed)?),
        };
        // Delta 3 of desktop#432: queued after this opening's own latest unsent save alone. Another unsent write of
        // the file (an earlier opening's save, a File Provider save) is not what the text on screen is based on:
        // after it, both would land one after the other and nobody would be asked. The accept checks it inside its
        // own transaction, so a write queued after this look cannot slip in.
        let own_write = record_of(saves, file_id, session, opening, |saves| {
            saves
                .queued
                .as_ref()
                .and_then(|queued| queued.token.as_deref())
                .and_then(crate::write_token::parse_token)
                .map(|token| token.write_id)
        })
        .flatten();
        let chain = own_write.map_or(ChainAfter::Nothing, ChainAfter::Only);
        let (outcome, write_token) = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let queued = EngineBridge::new(self.identity, db.clone(), api)
                    .with_chain_after(chain)
                    .queue_modify_for_save(target, held.as_ref(), bytes, seal_kind)
                    .map(|write| (write.outcome, write.token));
                // Refused behind an earlier write: ONE read, after the refused accept, picks which refusal it is (held
                // back, or not sent yet). For the words alone: nothing was stored either way, so a change between the
                // accept and this read can change only the words.
                queued.map_err(|error| {
                    if error.downcast_ref::<BehindAnotherWrite>().is_none() {
                        EditRefused::Failed
                    } else if viewer_edit_held::earlier_change_is_held(&db, file_id) {
                        EditRefused::EarlierChangeHeld
                    } else {
                        EditRefused::EarlierChangeNotSent
                    }
                })
            })
            .map_err(|_| EditRefused::NotSignedIn)??;
        let FinderWriteOutcome::Queued { op_id, .. } = outcome else {
            return Err(EditRefused::Failed);
        };
        let queued = QueuedSave {
            session,
            epoch: opening,
            base,
            op_id,
            token: write_token,
        };
        // Remembered before the wait (review of desktop#432): a landing at any moment from here on is told apart as
        // this viewer's own by its read of the version (`EditFacts::own`), also one between the last look below and
        // the Queued answer.
        remember_queued(saves, file_id, queued.clone());
        // Wait, bounded, for the upload to land: the op leaves the queue, and the version it became is the one the
        // landing recorded for this save's write (never a version read off the row, which may be another device's).
        let deadline = Instant::now() + LAND_WAIT;
        while self.is_syncing() && Instant::now() < deadline {
            match state.with(|db| landed_version(db, file_id, &queued)) {
                Err(_) => return Err(EditRefused::NotSignedIn),
                Ok(Some(version)) if version > base => {
                    // Noted, never forgotten: a read between here and the answer finds it the view's own.
                    note_landed(saves, file_id, &queued, version);
                    return Ok(SaveEnd::Saved { version });
                }
                Ok(_) => std::thread::sleep(LAND_POLL),
            }
        }
        Ok(SaveEnd::Queued { base })
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
        self.save_bytes_copy_for(session, file_id, text.as_bytes(), device, hhmm)
    }

    /// [`Self::save_text_copy_for`] for any bytes (shared with the Office save's Keep both).
    pub(in crate::app) fn save_bytes_copy_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        bytes: &[u8],
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
        let held = anonymous_file(bytes).ok_or(EditRefused::Failed)?;
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
    pub(in crate::app) fn edit_credentials(&self, session: SessionGeneration) -> Result<(String, Zeroizing<[u8; 32]>), EditRefused> {
        self.person_session(|current, generation| {
            (generation == session).then(|| (current.token.clone(), Zeroizing::new(current.master_key)))
        })
        .ok()
        .flatten()
        .ok_or(EditRefused::NotSignedIn)
    }
}

/// `bytes` in a file with no name, read from the start.
pub(in crate::app) fn anonymous_file(bytes: &[u8]) -> Option<std::fs::File> {
    let mut file = tempfile::tempfile().ok()?;
    file.write_all(bytes).ok()?;
    file.flush().ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    Some(file)
}

#[cfg(test)]
mod queued_save_tests {
    use super::*;
    use crate::state_db::{
        AcceptOutcome, ClaimOutcome, FileEntry, FileStatus, FinderAccept, FinderAcceptKind, ItemKind, LandingInput,
        PendingOperation, StateDb, StateDbError,
    };

    /// A record of the opening `epoch` of `session`: a queued save from `base` under op `op`.
    fn save(session: SessionGeneration, epoch: u64, base: i64, op: &str) -> QueuedSave {
        QueuedSave {
            session,
            epoch,
            base,
            op_id: op.into(),
            token: Some(format!("token-{op}")),
        }
    }

    /// A session no real engine of this test binary reaches.
    fn session() -> SessionGeneration {
        SessionGeneration::for_test(1_000_003)
    }

    /// The viewer's own queued save, once its upload has landed, is the base of the next save: not a conflict.
    #[test]
    fn a_second_save_after_the_first_landed_continues_from_the_version_it_became() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let e = open_saves(&store, "f", s);
        assert!(remember_queued(&store, "f", save(s, e, 4, "op-1")));
        // Not landed yet (the op is still queued, the version is the base): the base stays.
        assert_eq!(base_after_own_landed_save(&store, "f", s, e, 4, 4, |_| None), 4);
        // The version moved but the save has not landed (no version recorded for its write): not ours to adopt.
        assert_eq!(base_after_own_landed_save(&store, "f", s, e, 4, 5, |_| None), 4);
        // Landed as 5, and 5 is the version now: adopted.
        let landed = |q: &QueuedSave| (q.op_id == "op-1").then_some(5);
        assert_eq!(base_after_own_landed_save(&store, "f", s, e, 4, 5, landed), 5);
        // A version past it is not.
        assert_eq!(base_after_own_landed_save(&store, "f", s, e, 4, 6, |_| Some(6)), 4);
    }

    /// Review of desktop#424 (engine P2-3): the viewer's queued save landed as base + 1, then another device saved
    /// base + 2 and the tick recorded it. The next save from the viewer is a conflict: only the version the viewer's
    /// own op became is adopted, never a version past it.
    #[test]
    fn a_version_past_the_one_the_own_save_became_stays_a_conflict() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let e = open_saves(&store, "f", s);
        remember_queued(&store, "f", save(s, e, 4, "op-1"));
        assert_eq!(
            base_after_own_landed_save(&store, "f", s, e, 4, 6, |_| Some(5)),
            4,
            "another device's version after the own one is not adopted"
        );
    }

    /// Another device's save is never adopted: no record of this viewer's, another session, or another base.
    #[test]
    fn another_device_s_newer_version_stays_a_conflict() {
        let (store, s) = (ViewerSavesStore::default(), session());
        assert_eq!(base_after_own_landed_save(&store, "none", s, 1, 4, 5, |_| Some(5)), 4);
        let e = open_saves(&store, "f", s);
        remember_queued(&store, "f", save(s, e, 4, "op-1"));
        let other = SessionGeneration::for_test(1_000_004);
        assert_eq!(
            base_after_own_landed_save(&store, "f", other, e, 4, 5, |_| Some(5)),
            4,
            "a session that moved on"
        );
        assert_eq!(
            base_after_own_landed_save(&store, "f", s, e, 2, 5, |_| Some(5)),
            2,
            "another base"
        );
    }

    /// Delta check of #432, P2 (a): the save found its landing and answers Saved. From the moment it found it, a
    /// read of the version finds it the view's own, also before the answer reached the view (the landing itself is
    /// not asked again: `|_| None`).
    #[test]
    fn a_read_between_the_landing_found_and_the_saved_answer_finds_it_own() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let e = open_saves(&store, "f", s);
        let record = save(s, e, 4, "op-saved");
        remember_queued(&store, "f", record.clone());
        // What `save_text_for` does when its land wait finds the version, just before it answers Saved.
        assert!(note_landed(&store, "f", &record, 5));
        assert_eq!(own_versions(&store, "f", s, e, |_| None).get(&5), Some(&4));
    }

    /// Delta check of #432, P2 (b): the first queued save landed as 5, and the next save continued from it and waits
    /// to land. Every read in that wait finds 5 the view's own: no banner about the person's own save.
    #[test]
    fn two_saves_in_a_row_keep_the_first_ones_version_their_own() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let e = open_saves(&store, "f", s);
        remember_queued(&store, "f", save(s, e, 4, "op-first"));
        let landed = |q: &QueuedSave| (q.op_id == "op-first").then_some(5);
        // The second save, from the base the view shows (4): continued from the first one's 5.
        assert_eq!(base_after_own_landed_save(&store, "f", s, e, 4, 5, landed), 5);
        remember_queued(&store, "f", save(s, e, 5, "op-second"));
        // A read while the second one waits: the first one's 5 is still the view's own.
        assert!(own_versions(&store, "f", s, e, landed).contains_key(&5));
    }

    /// Delta check of #432 round 2, P1 (a): the file opened again while the earlier opening's save waited to land.
    /// The save then lands and notes its version under its own (earlier) opening: refused. The new opening has no own
    /// version, and a save from 4 is never continued from 5 (it answers Conflict).
    #[test]
    fn a_landing_noted_after_the_file_opened_again_is_not_the_new_openings() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let earlier = open_saves(&store, "f", s);
        let record = save(s, earlier, 4, "op-1");
        assert!(remember_queued(&store, "f", record.clone()));
        let now = open_saves(&store, "f", s);
        assert!(!note_landed(&store, "f", &record, 5), "refused: another opening's");
        assert!(own_versions(&store, "f", s, now, |_| Some(5)).is_empty());
        assert_eq!(base_after_own_landed_save(&store, "f", s, now, 4, 5, |_| Some(5)), 4);
    }

    /// Delta check of #432 round 2, P1 (b): the file opened again before the earlier opening's save remembered its
    /// queued write. The late remember is refused; its landing is nobody's own in the new opening.
    #[test]
    fn a_save_remembered_after_the_file_opened_again_is_not_the_new_openings() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let earlier = open_saves(&store, "f", s);
        let now = open_saves(&store, "f", s);
        assert!(
            !remember_queued(&store, "f", save(s, earlier, 4, "op-1")),
            "refused: another opening's"
        );
        let landed = |q: &QueuedSave| (q.op_id == "op-1").then_some(5);
        assert!(own_versions(&store, "f", s, now, landed).is_empty());
        assert_eq!(base_after_own_landed_save(&store, "f", s, now, 4, 5, landed), 4);
    }

    /// Delta check of #432 round 2, P1 (c): a read of the earlier opening copied its queued save out, the file opened
    /// again while it read the landing, and the read then notes the version: refused. Nothing of it is the new
    /// opening's.
    #[test]
    fn a_read_that_notes_after_the_file_opened_again_is_not_the_new_openings() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let earlier = open_saves(&store, "f", s);
        remember_queued(&store, "f", save(s, earlier, 4, "op-1"));
        let now = std::cell::Cell::new(0);
        // The landing is read with the record copied out; the new opening happens right then.
        let landed = |q: &QueuedSave| {
            now.set(open_saves(&store, "f", s));
            (q.op_id == "op-1").then_some(5)
        };
        assert!(
            own_versions(&store, "f", s, earlier, landed).is_empty(),
            "the earlier opening's record is gone"
        );
        assert!(own_versions(&store, "f", s, now.get(), |_| Some(5)).is_empty());
        assert_eq!(
            base_after_own_landed_save(&store, "f", s, now.get(), 4, 5, |_| Some(5)),
            4
        );
    }

    /// Delta check of #432 round 2: the record is read for its own opening only. A save of the later opening lands
    /// 4 → 5; a late read or save of the earlier opening finds none of it (no own version, no adoption: Conflict).
    #[test]
    fn an_earlier_opening_reads_nothing_of_a_later_ones_saves() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let earlier = open_saves(&store, "f", s);
        let now = open_saves(&store, "f", s);
        let record = save(s, now, 4, "op-now");
        assert!(remember_queued(&store, "f", record.clone()));
        assert!(note_landed(&store, "f", &record, 5));
        assert_eq!(
            own_versions(&store, "f", s, now, |_| None).get(&5),
            Some(&4),
            "the control"
        );
        assert!(own_versions(&store, "f", s, earlier, |_| Some(5)).is_empty());
        assert_eq!(
            base_after_own_landed_save(&store, "f", s, earlier, 4, 5, |_| Some(5)),
            4
        );
    }

    /// Delta check of #432 round 2, P3: a new opening drops the records of other sessions; a session transition of
    /// the account's runtime empties them all; a queued save keeps only its latest record.
    #[test]
    fn the_records_go_with_their_session_and_keep_the_latest_queued_save() {
        let (store, s) = (ViewerSavesStore::default(), session());
        let e = open_saves(&store, "f", s);
        remember_queued(&store, "f", save(s, e, 4, "op-1"));
        remember_queued(&store, "f", save(s, e, 4, "op-2"));
        assert_eq!(
            record_of(&store, "f", s, e, |saves| saves
                .queued
                .as_ref()
                .map(|q| q.op_id.clone()))
            .flatten(),
            Some("op-2".to_string()),
            "the latest only"
        );
        let other = SessionGeneration::for_test(1_000_004);
        open_saves(&store, "g", other);
        assert!(
            record_of(&store, "f", s, e, |_| ()).is_none(),
            "another session's record went"
        );
        let acct = crate::account::AccountRuntime::new(crate::account::AccountId("viewer-saves".into()));
        let turn = acct.session_generation();
        open_saves(&acct.viewer_saves, "f", turn);
        acct.advance_session_generation_holding_session_write_lock(crate::account::SessionTransition::Lock);
        assert!(
            acct.viewer_saves.0.lock().unwrap().is_empty(),
            "a session transition empties them"
        );
    }

    fn set_version(db: &StateDb, file_id: &str, version: i64) {
        let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
        contract.current_version = version;
        db.set_file_contract_state(&contract).unwrap();
    }

    /// The version a save became is read from its own landing, through the real accept, claim and landing
    /// transaction: the write the viewer's save is queued under (the same accept `queue_file_provider_modify_from`
    /// makes) is the one the landing records the produced version for. A version another device made after it is
    /// not that version, and stays a conflict.
    #[test]
    fn the_version_a_save_became_is_read_from_its_own_landing() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "f".into(),
            path: "f.txt".into(),
            status: FileStatus::Local,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 10,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        set_version(&db, "f", 4);
        db.track_staged_payload("/staged/viewer", None, false).unwrap();
        let accepted = db
            .accept_finder_write(
                &FinderAccept {
                    retire_parked: None,
                    op_id: "viewer-op",
                    file_id: "f",
                    kind: FinderAcceptKind::Modify {
                        incoming_base: Some("4"),
                    },
                    parent_id: None,
                    target_path: Some("f.txt"),
                    metadata_json: "{}",
                    payload_path: "/staged/viewer",
                    size_bytes: 3,
                    modified_at: 20,
                    backup_source_key: None,
                    now: 20,
                },
                &|_, _| Vec::new(),
            )
            .unwrap();
        let AcceptOutcome::Queued { token, .. } = accepted else {
            panic!("the save is queued")
        };
        let session = SessionGeneration::for_test(1);
        let store = ViewerSavesStore::default();
        let epoch = open_saves(&store, "f", session);
        let record = || QueuedSave {
            session,
            epoch,
            base: 4,
            op_id: "viewer-op".into(),
            token: Some(token.clone()),
        };
        assert_eq!(landed_version(&db, "f", &record()), None, "still queued");
        let ClaimOutcome::Claimed(claimed) = db.claim_operation("viewer-op", 21).unwrap() else {
            panic!("the save is claimed")
        };
        let write_id = claimed.write.as_ref().map(|write| write.write_id.clone());
        assert!(
            write_id.is_some(),
            "the save is write-keyed: its landing records the version it became"
        );
        let input = LandingInput {
            op_id: "viewer-op",
            claim_id: Some(&claimed.claim_id),
            write_id: write_id.as_deref(),
            local_file_id: "f",
            server_file_id: "f",
            target_path: Some("f.txt"),
            parent_id: None,
            landed_base: Some(4),
            produced_version: 5,
            produced_object_version_id: "object-5",
            size_bytes: 3,
            content_type: None,
            mime_type: None,
            completed_payload: Some("/staged/viewer"),
            completed_payload_hash: None,
            // The viewer's save never writes the sync folder's copy (task 2523, F2).
            place_written: false,
            finalization: None,
            now: 22,
        };
        let rekey = |op: &PendingOperation, _: &str| -> std::result::Result<Option<String>, StateDbError> {
            Ok(op.metadata_json.clone())
        };
        assert!(db.apply_landing(&input, &rekey).unwrap().is_some(), "it lands");
        assert_eq!(landed_version(&db, "f", &record()), Some(5));
        // Review of desktop#432: the viewer's read of the version learns that 5 is its own save, as often as it
        // asks, and forgets nothing: the next save from the viewer still continues from 5.
        remember_queued(&store, "f", record());
        let read = |q: &QueuedSave| landed_version(&db, "f", q);
        for _ in 0..2 {
            assert_eq!(
                own_versions(&store, "f", session, epoch, read).get(&5),
                Some(&4),
                "the own save's landed version, saved from 4"
            );
        }
        assert!(
            own_versions(&store, "f", SessionGeneration::for_test(1_000_009), epoch, read).is_empty(),
            "another session's record says nothing"
        );
        assert_eq!(
            base_after_own_landed_save(&store, "f", session, epoch, 4, 5, read),
            5,
            "still continued from: the reads forgot nothing"
        );
        // Delta check of #432, P1-1: the file opened again (open, Try again, Show it). The new opening holds none of
        // the earlier saves: the landing is not its own, and a save from the version it shows is a conflict.
        let reopened = open_saves(&store, "f", session);
        assert!(
            own_versions(&store, "f", session, reopened, read).is_empty(),
            "not the new opening's own"
        );
        assert_eq!(
            base_after_own_landed_save(&store, "f", session, reopened, 4, 5, read),
            4,
            "never continued from: the save from 4 answers Conflict"
        );
        // Another device saved after it, and the tick recorded its version.
        set_version(&db, "f", 6);
        let again = open_saves(&store, "f", session);
        remember_queued(
            &store,
            "f",
            QueuedSave {
                epoch: again,
                ..record()
            },
        );
        let read_again = |q: &QueuedSave| landed_version(&db, "f", q);
        assert_eq!(
            base_after_own_landed_save(&store, "f", session, again, 4, 6, read_again),
            4,
            "a version past the own one: a conflict"
        );
        assert_eq!(
            base_after_own_landed_save(&store, "f", session, again, 4, 5, read_again),
            5,
            "the own landed version: continued from"
        );
        // Another save's write took the file's held write: the first save's version is no longer known.
        let other = QueuedSave {
            token: Some("another-write".into()),
            ..record()
        };
        assert_eq!(landed_version(&db, "f", &other), None);
    }
}

#[cfg(test)]
#[path = "viewer_edit_staging_tests.rs"]
mod staging_tests;
