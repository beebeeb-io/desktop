//! Task 2267 (P1): a Windows PC whose local data had no recorded owner, or an owner recorded only by an email the
//! account no longer uses, refused every sync start with `OTHER_ACCOUNT_ON_WINDOWS`, although the data was the
//! signed-in account's own. Signing out did not help: the Windows sign-out keeps unsent changes, and those cannot send
//! while sync is refused.
//!
//! The one-shot startup adoption (`account_binding::adopt_unbound`) names its candidate by the stored session's email,
//! and a session stored before the email was kept has none, so the window closed with no owner recorded. From then on
//! every start saw unowned data and decided `Reset`, which Windows refuses (R10).
//!
//! This module settles such a refusal with evidence instead of a guess (fix round 2, the lead's review of the PR):
//! - ONE listing of this account's own tree (`GET /api/v1/sync/snapshot`, the account's non-trashed files only).
//! - EVERY local My Files row the server named must be in that tree with its name opening under THIS session's master
//!   key, or be gone from a folder that is (no sampling). Another account's My Files rows are not in this account's
//!   tree, and a name sealed under another key never opens (AES-GCM authenticates it), so another account's data is
//!   never adopted (R10 holds).
//! - Every queued change must act on this account's proven data. One that cannot be tied to it (a new item at the top
//!   level, or in a folder that is not proven this account's own, such as a shared one) does not block the adoption:
//!   it is held for the person's review (`found_while_paused`), recorded in the same transaction as the owner, and is
//!   never sent on its own. The person uploads it or keeps it on this PC only (`found_while_paused.rs`).
//! - Nothing is deleted here in any case.

use crate::account_binding::{Identity, same_account};
use crate::state_db::{ProofOp, ProofRow, StateDb};
use std::collections::{HashMap, HashSet};
use std::future::Future;

/// How many of the server-named rows must prove to be this account's own (all of them when fewer were checked).
pub const REQUIRED_PROOFS: usize = 2;

/// What the server answered for this account's tree.
#[derive(Debug, Clone)]
pub enum Listing {
    /// The nodes of `GET /api/v1/sync/snapshot`: this account's own, non-trashed files and folders.
    Nodes(Vec<serde_json::Value>),
    /// The server refused this session's token (401): nothing about ownership, and not a connection problem either.
    SignedOut,
    /// No answer that says anything about ownership: offline, a timeout, 429, a server error.
    Unavailable,
}

/// One server-named row, judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// In this account's tree, and its name decrypts under this session's master key: this account's own.
    Own,
    /// In this account's tree, but its name does not decrypt under this account's key.
    Unproven,
    /// Proves nothing either way: a plaintext (legacy) name, no name, or a name sealed under someone else's key by
    /// design (a file received through a file request, or one a collaborator uploaded into this account's folder).
    /// Set aside, never counted for or against.
    Neutral,
    /// Not in this account's tree, but its nearest ancestor that is, is this account's own: deleted since.
    Gone,
    /// Not in this account's tree, and nothing of this account's explains it (another account's file looks exactly
    /// like that).
    Missing,
}

/// The counts of one proof. Logged and reported; never a name or a path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    /// Server-named My Files rows judged: all of them.
    pub checked: usize,
    pub own: usize,
    pub neutral: usize,
    /// Rows deleted since from a folder proven this account's own.
    pub gone: usize,
    pub unproven: usize,
    pub missing: usize,
    /// Queued changes that act on proven data: they upload after the adoption.
    pub proven_changes: usize,
    /// Queued changes held for the person's review (`found_while_paused`).
    pub held_for_review: usize,
    /// 1 when the server refused this session's token for the listing.
    pub signed_out: usize,
    /// 1 when the listing did not answer, twice.
    pub unavailable: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Adopt,
    NotProven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing a proof can settle: the data is owned by this session already, there is none, or the owner record
    /// names an account by its user id that this session cannot be compared with (R10's strongest signal).
    NotEligible,
    /// Proven, and this session is now the recorded owner: the next start proceeds and keeps everything. The changes
    /// that could not be tied to this account's data are held for review.
    Adopted(Tally),
    NotProven(Tally),
    /// No usable listing; nothing recorded. A later start asks again.
    Unavailable(Tally),
    /// The session or the owner record changed while the proof ran; nothing recorded.
    Changed(Tally),
}

/// Whether a refused start is one a proof may settle: this session is known, local data exists, and the data has no
/// recorded owner, or one recorded by email only that differs from this session's.
pub fn eligible(owner: Option<&Identity>, session: &Identity, has_local_data: bool) -> bool {
    if !session.is_known() || !has_local_data {
        return false;
    }
    match owner.filter(|owner| owner.is_known()) {
        None => true,
        // An owner recorded WITH a user id is compared by that id only (review P3-2): another id is another account,
        // R10's strongest signal, never questioned, and a session whose id is not known yet cannot be compared with it.
        Some(owner) => owner.user_id.is_none() && same_account(owner, session) == Some(false),
    }
}

/// Whether `sealed` (a `name_encrypted` envelope) opens under a file key derived from `master_key` and `file_id`, in
/// every format the engine reads: the string id (as every client seals today) and the 16-byte binary id (the old CLI),
/// each in both envelopes (`engine_bridge::decrypt_shared_name_with_key`). The key is derived from this id, so a name
/// sealed for another file, or under another account's key, never opens. Task 1888's key check reads names the same
/// way.
pub fn name_opens(master_key: &[u8; 32], file_id: &str, sealed: &str) -> bool {
    let master = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
    let by_string = beebeeb_core::kdf::derive_file_key(&master, file_id.as_bytes());
    let by_binary = uuid::Uuid::parse_str(file_id)
        .ok()
        .map(|id| beebeeb_core::kdf::derive_file_key(&master, id.as_bytes()));
    std::iter::once(by_string).chain(by_binary).any(|file_key| {
        crate::engine_bridge::decrypt_shared_name_with_key(&file_key, sealed).is_some_and(|name| !name.trim().is_empty())
    })
}

/// Judge one node of this account's tree against this session's master key.
pub fn probe(master_key: &[u8; 32], node: &serde_json::Value) -> Probe {
    let set = |field: &str| node.get(field).is_some_and(|value| !value.is_null() && value.as_str() != Some(""));
    // Sealed under someone else's key by design (review P2-3): a file received through a file request, or one a
    // collaborator uploaded into this account's folder (the server names `uploaded_by` only when it is not the owner,
    // and wraps a file key for the owner only on such uploads).
    if set("file_request_id") || set("uploaded_by") || set("owner_wrapped_file_key") {
        return Probe::Neutral;
    }
    let Some(file_id) = node.get("id").and_then(serde_json::Value::as_str) else {
        return Probe::Unproven;
    };
    // Lead review of round 2, P3-6: a node that does not say who sealed its name is no evidence of a file request, so a
    // name that does not open is unproven there too (the production API says it, since 2026-10-27).
    match node
        .get("name_encrypted")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|sealed| sealed.starts_with('{'))
    {
        // A plaintext (legacy) name, or none: proves nothing either way.
        None => Probe::Neutral,
        Some(sealed) if name_opens(master_key, file_id, sealed) => Probe::Own,
        Some(_) => Probe::Unproven,
    }
}

/// The decision over the whole tree (see the module docs and the PR's Decisions).
pub fn verdict(tally: &Tally) -> Verdict {
    // Anything this account cannot explain blocks: a name that does not open under its key, or a row the server named
    // that is not in its tree and not gone from one of its folders.
    if tally.unproven > 0 || tally.missing > 0 {
        return Verdict::NotProven;
    }
    // Review P3-1: rows that prove nothing never outnumber the rows that prove ownership, and at least
    // `REQUIRED_PROOFS` prove it (every row, when fewer were checked).
    if tally.neutral > tally.own {
        return Verdict::NotProven;
    }
    if tally.own > 0 && tally.own >= REQUIRED_PROOFS.min(tally.checked) {
        Verdict::Adopt
    } else {
        Verdict::NotProven
    }
}

/// An HTTP failure of the listing as the proof reads it (`None`: no status at all, e.g. offline).
pub fn listing_from_status(status: Option<u16>) -> Listing {
    match status {
        Some(401) => Listing::SignedOut,
        _ => Listing::Unavailable,
    }
}

/// This account's tree as the proof reads it (self-review F2): the snapshot, which lists only what is not in the trash,
/// and the trash listing, so a row the person trashed on the server while sync was refused is still found in the tree.
/// Either one unread is a listing that did not answer.
#[cfg(any(target_os = "windows", test))]
pub fn listing_of(
    snapshot: anyhow::Result<crate::api_client::SyncSnapshot>,
    trashed: anyhow::Result<Vec<serde_json::Value>>,
) -> Listing {
    match (snapshot, trashed) {
        (Ok(snapshot), Ok(trashed)) => Listing::Nodes(snapshot.nodes.into_iter().chain(trashed).collect()),
        (Err(error), _) | (_, Err(error)) => listing_from_status(
            error
                .downcast_ref::<reqwest::Error>()
                .and_then(reqwest::Error::status)
                .map(|status| status.as_u16()),
        ),
    }
}

/// What the proof concluded about the rows and the queue (pure: no database, no network).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Judgement {
    pub tally: Tally,
    /// The queued changes that act on proven data.
    pub proven: HashSet<String>,
}

/// Judge every server-named My Files row and every queued change against this account's tree.
pub fn judge(master_key: &[u8; 32], rows: &[ProofRow], ops: &[ProofOp], nodes: &[serde_json::Value]) -> Judgement {
    let tree: HashMap<&str, &serde_json::Value> = nodes
        .iter()
        .filter_map(|node| node.get("id").and_then(serde_json::Value::as_str).map(|id| (id, node)))
        .collect();
    let local: HashMap<&str, &ProofRow> = rows.iter().map(|row| (row.file_id.as_str(), row)).collect();
    // Where two rows hold one path (a folder made here and its server twin, task 1888), the server's row wins.
    let by_path: HashMap<&str, &ProofRow> = rows
        .iter()
        .filter(|row| !row.server_named)
        .chain(rows.iter().filter(|row| row.server_named))
        .map(|row| (row.path.trim_matches('/'), row))
        .collect();
    let judge = Judge {
        master_key,
        tree,
        local,
        by_path,
        probes: std::cell::RefCell::new(HashMap::new()),
    };

    let mut tally = Tally::default();
    for row in rows.iter().filter(|row| row.my_files && row.server_named) {
        tally.checked += 1;
        match judge.row(row) {
            Probe::Own => tally.own += 1,
            Probe::Neutral => tally.neutral += 1,
            Probe::Gone => tally.gone += 1,
            Probe::Unproven => tally.unproven += 1,
            Probe::Missing => tally.missing += 1,
        }
    }
    // Lead review of round 2, P1: a path proves a folder made here whose own path is an own folder's (its twin: its
    // create sends no content, and task 1888 folds it into that folder), never anything else (self-review of round 3,
    // F2: not a new folder inside it). Every other change needs one anchor tied by real parent links.
    let proven: HashSet<String> = ops
        .iter()
        .filter(|op| {
            let own = op.file_id.as_deref().map(|anchor| judge.anchor(anchor));
            let ties: Vec<Tie> = own.into_iter().chain(op.parent_id.as_deref().map(|anchor| judge.anchor(anchor))).collect();
            !ties.is_empty()
                && !ties.contains(&Tie::Not)
                && (ties.contains(&Tie::Linked) || (op.kind == "create_folder" && own == Some(Tie::Twin)))
        })
        .map(|op| op.op_id.clone())
        .collect();
    tally.proven_changes = proven.len();
    Judgement { tally, proven }
}

/// How a queued change's anchor is tied to this account's proven data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tie {
    /// Up real parent links to an own row (or one gone from an own folder).
    Linked,
    /// The anchor is a row made here at the path of an own row of the server's (its twin), and nothing else ties it.
    Twin,
    /// Only through a path otherwise: a twin further up, or the folder at a path (a row with no parent link, its own or
    /// one up the walk).
    ByPath,
    Not,
}

/// The tree and the local rows, indexed, with each present node's probe computed once.
struct Judge<'a> {
    master_key: &'a [u8; 32],
    tree: HashMap<&'a str, &'a serde_json::Value>,
    local: HashMap<&'a str, &'a ProofRow>,
    by_path: HashMap<&'a str, &'a ProofRow>,
    probes: std::cell::RefCell<HashMap<&'a str, Probe>>,
}

impl<'a> Judge<'a> {
    /// The probe of the node `id` of this account's tree, or `None` when the tree has no such node.
    fn present(&self, id: &'a str) -> Option<Probe> {
        let node = self.tree.get(id)?;
        Some(*self.probes.borrow_mut().entry(id).or_insert_with(|| probe(self.master_key, node)))
    }

    /// The folder `row` is in: its parent link, or, for a row without one, the row at its path's folder; and whether
    /// it was found by the path.
    fn parent_link(&self, row: &'a ProofRow) -> Option<(&'a str, bool)> {
        if let Some(parent) = row.parent_id.as_deref() {
            return Some((parent, false));
        }
        let (folder, _) = row.path.trim_matches('/').rsplit_once('/')?;
        self.by_path.get(folder).map(|parent| (parent.file_id.as_str(), true))
    }

    /// A server-named row: its node's probe, or, when this account's tree does not hold it, `Gone` if its nearest
    /// ancestor the tree holds is this account's own (deleted since), else `Missing`. The walk follows the local
    /// parents and ends as `Missing` at the top, at a parent with no row, or on a cycle.
    fn row(&self, row: &'a ProofRow) -> Probe {
        self.row_tie(row).0
    }

    /// [`Self::row`], and whether the walk up to the ancestor that explains a row gone from the tree went through a
    /// folder found at a path (a row with no parent link): such a row is tied to that folder by its path only
    /// (self-review of round 3, F1).
    fn row_tie(&self, row: &'a ProofRow) -> (Probe, bool) {
        if let Some(probe) = self.present(&row.file_id) {
            return (probe, false);
        }
        let mut seen: HashSet<&str> = HashSet::from([row.file_id.as_str()]);
        let mut current = row;
        let mut via_path = false;
        while let Some((id, found_by_path)) = self.parent_link(current) {
            if !seen.insert(id) {
                break;
            }
            via_path |= found_by_path;
            if let Some(probe) = self.present(id) {
                return (if probe == Probe::Own { Probe::Gone } else { Probe::Missing }, via_path);
            }
            match self.local.get(id) {
                Some(&parent) => current = parent,
                None => break,
            }
        }
        (Probe::Missing, via_path)
    }

    /// How a queued change's anchor (the item it acts on, or the folder it acts in) is tied to this account's proven
    /// data: the first judged row up its local parents (or at a row made here's own path, its twin) is in My Files and
    /// proven its own (or gone from its own folder). An item made on this PC at the top level, or in a folder shared
    /// with this account, or in one whose name proves nothing, is not. The tie is `Twin` when the anchor's own row
    /// is a twin's, and `ByPath` when the walk went through a twin further up or through the folder at a path (a row
    /// with no parent link), the walk up from a row gone from the tree included.
    fn anchor(&self, anchor: &'a str) -> Tie {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut id = anchor;
        let mut by_path = false;
        let tied = |proven: bool, by_path: bool| match (proven, by_path) {
            (false, _) => Tie::Not,
            (true, false) => Tie::Linked,
            (true, true) => Tie::ByPath,
        };
        let proves = |row: &'a ProofRow| {
            let (probe, via_path) = self.row_tie(row);
            (row.my_files && matches!(probe, Probe::Own | Probe::Gone), via_path)
        };
        loop {
            if !seen.insert(id) {
                return Tie::Not;
            }
            let Some(&row) = self.local.get(id) else {
                // No local row: a node of this account's tree this PC has not written yet.
                return tied(self.present(id) == Some(Probe::Own), by_path);
            };
            if row.server_named {
                let (proven, via_path) = proves(row);
                return tied(proven, by_path || via_path);
            }
            // A folder made here whose create never reached the server shares its path with the server's folder
            // (its twin, task 1888): it is that folder.
            if let Some(&twin) = self.by_path.get(row.path.trim_matches('/')).filter(|twin| twin.server_named) {
                let (proven, _) = proves(twin);
                return match tied(proven, true) {
                    Tie::ByPath if id == anchor => Tie::Twin,
                    tie => tie,
                };
            }
            match self.parent_link(row) {
                Some((parent, found_by_path)) => {
                    by_path |= found_by_path;
                    id = parent;
                }
                None => return Tie::Not,
            }
        }
    }
}

/// Run the proof for `session` over `db` and, when it holds, record `session` as the owner and hold for review what
/// could not be tied to its data, in one transaction. `list` asks the server for this account's tree; a listing that
/// does not answer is asked once more. `still_current` is checked before every question and right before the record
/// is written, so a session that changed meanwhile (a Lock, a sign-out, another sign-in) stops the asking and records
/// nothing. The owner is replaced only if it is still the one the proof began with. Nothing is ever deleted.
pub async fn prove_and_adopt<L, Fut>(
    db: &StateDb,
    session: &Identity,
    master_key: &[u8; 32],
    list: L,
    still_current: impl Fn() -> bool,
) -> Result<Outcome, String>
where
    L: Fn() -> Fut,
    Fut: Future<Output = Listing>,
{
    let owner = db.owner().map_err(|e| format!("read the local data owner: {e}"))?;
    let has_local_data = db
        .has_account_data()
        .map_err(|e| format!("inspect the local data: {e}"))?;
    if !eligible(owner.as_ref(), session, has_local_data) {
        return Ok(Outcome::NotEligible);
    }
    let mut tally = Tally::default();
    let mut nodes = None;
    for _ in 0..2 {
        if !still_current() {
            return Ok(Outcome::Changed(tally));
        }
        match list().await {
            Listing::Nodes(listed) => {
                nodes = Some(listed);
                break;
            }
            Listing::SignedOut => {
                tally.signed_out = 1;
                return Ok(Outcome::Unavailable(tally));
            }
            Listing::Unavailable => {}
        }
    }
    let Some(nodes) = nodes else {
        tally.unavailable = 1;
        return Ok(Outcome::Unavailable(tally));
    };
    // Read after the listing: whatever was written while it ran is judged too, or held (it is not in `proven`).
    let rows = db.own_data_proof_rows().map_err(|e| format!("read the local rows: {e}"))?;
    let ops = db.own_data_proof_ops().map_err(|e| format!("read the queue: {e}"))?;
    let Judgement { tally, proven } = judge(master_key, &rows, &ops, &nodes);
    if verdict(&tally) != Verdict::Adopt {
        return Ok(Outcome::NotProven(tally));
    }
    if !still_current() {
        return Ok(Outcome::Changed(tally));
    }
    match db
        .adopt_owner_holding(owner.as_ref(), session, &proven, now_secs())
        .map_err(|e| format!("record the local data owner: {e}"))?
    {
        Some(held) => Ok(Outcome::Adopted(Tally { held_for_review: held, ..tally })),
        None => Ok(Outcome::Changed(tally)),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_binding::{
        Bound, IDENTITY_UNKNOWN, LocalData, OTHER_ACCOUNT_ON_WINDOWS, adopt_unbound, bind_before_engine_start,
    };
    use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, OperationKind, PendingOperation};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    const WINDOWS: bool = false; // `reset_allowed` on Windows

    const MINE: [u8; 32] = [7u8; 32];
    const THEIRS: [u8; 32] = [9u8; 32];

    fn session() -> Identity {
        Identity::new(Some("user-owner"), Some("owner@example.eu"))
    }

    /// The real `state.db`, through the binding's own trait, as the engine start reads it.
    struct Db(StateDb);

    impl LocalData for Db {
        fn owner(&self) -> Result<Option<Identity>, String> {
            self.0.owner().map_err(|e| e.to_string())
        }
        fn has_account_data(&self) -> Result<bool, String> {
            self.0.has_account_data().map_err(|e| e.to_string())
        }
        fn reset(&self) -> Result<(), String> {
            panic!("Windows never resets in these tests")
        }
        fn record_owner(&self, owner: &Identity) -> Result<(), String> {
            self.0.set_owner(owner).map_err(|e| e.to_string())
        }
        fn adopt_once(&self, candidate: Option<&Identity>) -> Result<bool, String> {
            self.0.adopt_owner_once(candidate).map_err(|e| e.to_string())
        }
    }

    struct Pc {
        db: Db,
        _dir: tempfile::TempDir,
    }

    /// How a row reached this PC, which decides the columns that say the server named it.
    #[derive(Clone, Copy)]
    enum Shape {
        /// Written by the snapshot: the server's version, `remote_updated_at` 0 (the My Files ingest cannot read the
        /// server's RFC 3339 `updated_at`). 39,626 of the 40,473 server rows on the maintainer's PC look like this.
        Snapshot,
        /// Written by a delta: stamped `now`, no version.
        Delta,
        /// Made on this PC and never uploaded.
        Local,
    }

    fn empty_pc() -> Pc {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        Pc { db: Db(db), _dir: dir }
    }

    /// A PC that synced `rows` My Files rows from the server, at the top level, and holds one change waiting to upload
    /// on the first of them (a held write), the shape of the reported PCs.
    fn pc_with(rows: &[&str]) -> Pc {
        let pc = empty_pc();
        for id in rows {
            add_row(&pc, id, None, Shape::Snapshot);
        }
        if let Some(id) = rows.first() {
            pc.db.0.enqueue_operation(&edit("held-1", id)).unwrap();
        }
        pc
    }

    fn add_row(pc: &Pc, id: &str, parent: Option<&str>, shape: Shape) {
        add_row_in(pc, id, parent, shape, Namespace::MyFiles, ItemKind::File);
    }

    fn add_folder(pc: &Pc, id: &str, parent: Option<&str>, shape: Shape) {
        add_row_in(pc, id, parent, shape, Namespace::MyFiles, ItemKind::Folder);
    }

    /// A row written through its contract, as the pull and the watcher write every row.
    fn add_row_in(pc: &Pc, id: &str, parent: Option<&str>, shape: Shape, namespace: Namespace, kind: ItemKind) {
        add_row_at(pc, id, &format!("dir/{id}"), parent, shape, namespace, kind);
    }

    fn add_row_at(
        pc: &Pc,
        id: &str,
        path: &str,
        parent: Option<&str>,
        shape: Shape,
        namespace: Namespace,
        kind: ItemKind,
    ) {
        let (version, remote_updated_at) = match shape {
            Shape::Snapshot => (3, 0),
            Shape::Delta => (0, 1_700_000_000),
            Shape::Local => (0, 0),
        };
        pc.db
            .0
            .upsert_file_with_contract(
                &FileEntry {
                    file_id: id.to_string(),
                    path: path.to_string(),
                    status: FileStatus::Local,
                    size_bytes: 10,
                    modified_at: 1_700_000_000,
                    content_hash: None,
                    remote_updated_at,
                    parent_id: parent.map(str::to_string),
                    item_kind: kind.clone(),
                },
                |contract| {
                    contract.parent_id = parent.map(str::to_string);
                    contract.namespace = namespace;
                    contract.item_kind = kind;
                    contract.current_version = version;
                },
            )
            .unwrap();
    }

    fn ids(prefix: &str, count: usize) -> Vec<String> {
        (0..count).map(|n| format!("{prefix}-0000-4000-8000-{n:012}")).collect()
    }

    fn op(op_id: &str, kind: OperationKind, file_id: Option<&str>, parent_id: Option<&str>) -> PendingOperation {
        PendingOperation {
            op_id: op_id.to_string(),
            kind,
            file_id: file_id.map(str::to_string),
            parent_id: parent_id.map(str::to_string),
            target_path: Some("dir/x".to_string()),
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    /// An edit of a file the server knows.
    fn edit(op_id: &str, file_id: &str) -> PendingOperation {
        PendingOperation {
            base_version: Some(3),
            ..op(op_id, OperationKind::UploadVersion, Some(file_id), None)
        }
    }

    /// A new item this PC made (its row is `Shape::Local`) in `parent`. A new file's upload says it creates the file
    /// (`metadata_json.operation`), as the engine queues it.
    fn create(op_id: &str, file_id: &str, parent: Option<&str>, folder: bool) -> PendingOperation {
        if folder {
            return op(op_id, OperationKind::CreateFolder, Some(file_id), parent);
        }
        PendingOperation {
            metadata_json: Some(r#"{"operation":"create_file"}"#.to_string()),
            ..op(op_id, OperationKind::UploadVersion, Some(file_id), parent)
        }
    }

    fn sealed_name(owner_key: [u8; 32], file_id: &str) -> String {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_key);
        beebeeb_core::encrypt::encrypt_name(&mk, file_id, "notes.txt", Some("text/plain")).unwrap()
    }

    /// This account's tree's node for `file_id`, its name sealed under `owner_key` exactly as every client seals it.
    fn node(owner_key: [u8; 32], file_id: &str, parent: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "id": file_id,
            "parent_id": parent,
            "name_encrypted": sealed_name(owner_key, file_id),
            "file_request_id": null,
            "uploaded_by": null,
            "owner_wrapped_file_key": null,
            "size_bytes": 10,
            "is_folder": false,
            "version_number": 3,
        })
    }

    fn mine(file_id: &str) -> serde_json::Value {
        node(MINE, file_id, None)
    }

    fn with(mut node: serde_json::Value, field: &str, value: serde_json::Value) -> serde_json::Value {
        node[field] = value;
        node
    }

    /// A name sealed under the LEGACY key derivation (the 16-byte binary UUID, as the old CLI sealed names).
    fn sealed_binary_node(owner_key: [u8; 32], file_id: &str) -> serde_json::Value {
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_key);
        let uuid = uuid::Uuid::parse_str(file_id).unwrap();
        let key = beebeeb_core::kdf::derive_file_key(&mk, uuid.as_bytes());
        let plaintext = serde_json::json!({ "name": "old.txt", "mime_type": null }).to_string();
        let blob = beebeeb_core::encrypt::encrypt_metadata(&key, &plaintext).unwrap();
        with(mine(file_id), "name_encrypted", serde_json::json!(serde_json::to_string(&blob).unwrap()))
    }

    /// A server that answers the listing from a fixed tree, after any scripted failures, and counts the listings.
    struct Server {
        nodes: Vec<serde_json::Value>,
        first: RefCell<VecDeque<Listing>>,
        listed: Cell<usize>,
    }

    impl Server {
        fn new(nodes: Vec<serde_json::Value>) -> Self {
            Self { nodes, first: RefCell::new(VecDeque::new()), listed: Cell::new(0) }
        }
        fn failing_first(self, answers: Vec<Listing>) -> Self {
            *self.first.borrow_mut() = answers.into();
            self
        }
        fn list(&self) -> std::future::Ready<Listing> {
            self.listed.set(self.listed.get() + 1);
            let scripted = self.first.borrow_mut().pop_front();
            std::future::ready(scripted.unwrap_or_else(|| Listing::Nodes(self.nodes.clone())))
        }
    }

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn prove(pc: &Pc, session: &Identity, key: [u8; 32], server: &Server) -> Outcome {
        block_on(prove_and_adopt(&pc.db.0, session, &key, || server.list(), || true)).expect("the proof runs")
    }

    fn due(pc: &Pc) -> Vec<String> {
        pc.db
            .0
            .list_due_operations(i64::MAX)
            .unwrap()
            .into_iter()
            .map(|op| op.op_id)
            .collect()
    }

    /// The op ids held for the person's review, sorted; the hold summary counts the same.
    fn held_for_review(pc: &Pc) -> Vec<String> {
        let mut held: Vec<String> = pc.db.0.found_while_paused_ops().unwrap().into_iter().map(|op| op.op_id).collect();
        held.sort();
        let summary = pc.db.0.queue_hold_summary().unwrap();
        let counted = summary
            .held
            .iter()
            .find(|group| group.reason == "found_while_paused")
            .map_or(0, |group| group.count);
        assert_eq!(counted, held.len() as i64, "the hold summary shows them under their own reason");
        held
    }

    const IDS: [&str; 3] = [
        "0a000000-0000-4000-8000-000000000001",
        "0a000000-0000-4000-8000-000000000002",
        "0a000000-0000-4000-8000-000000000003",
    ];

    fn own_server() -> Server {
        Server::new(IDS.iter().map(|id| mine(id)).collect())
    }

    // ---- the stranding paths, reproduced against the real state.db ----

    /// Path 1 (the report): a session stored before its email was kept names no adoption candidate at the first
    /// startup, so the one-shot window closes with no owner, and every later start refuses with the
    /// "another account" sentence although the data is this account's own. The proof adopts it; the held write is
    /// kept and the next start proceeds.
    #[test]
    fn a_closed_window_with_no_candidate_no_longer_strands_the_owner() {
        let pc = pc_with(&IDS);
        assert_eq!(adopt_unbound(&pc.db, None), Ok(false), "the startup had no candidate");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string()),
            "stranded before the proof"
        );

        let server = own_server();
        let outcome = prove(&pc, &session(), MINE, &server);

        assert!(
            matches!(outcome, Outcome::Adopted(t) if t.own == 3 && t.checked == 3 && t.proven_changes == 1),
            "{outcome:?}"
        );
        assert_eq!(server.listed.get(), 1, "one listing, no question per file");
        assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(due(&pc), vec!["held-1".to_string()], "the held write is kept, to upload");
    }

    /// Path 2: the startup had a candidate but the window was already shut (an engine start or a sign-in opened the
    /// database first: `StateDbLocalData::for_engine_start` closes it). The late candidate is refused, as designed;
    /// the proof still adopts.
    #[test]
    fn a_window_shut_before_the_candidate_arrived_no_longer_strands_the_owner() {
        let pc = pc_with(&IDS);
        pc.db.0.close_adoption_window().unwrap();
        assert_eq!(adopt_unbound(&pc.db, Some(&session())), Ok(false), "the window was already shut");
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );

        assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(_)));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(due(&pc).len(), 1);
    }

    /// Path 3: signing out cannot break the loop. The Windows sign-out refuses while a change waits to upload, and
    /// the change cannot upload while sync is refused, so sign-out and sign-in go round. After the proof the held
    /// write is still there, and the start proceeds, so it can upload.
    #[test]
    fn the_sign_out_loop_is_broken_by_the_proof_not_by_deleting_the_held_write() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        assert!(pc.db.0.windows_signout_preflight().is_err(), "sign-out refuses: a change waits to upload");
        assert!(pc.db.0.finish_windows_signout().is_err(), "and the purge refuses too");

        assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(_)));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(due(&pc).len(), 1, "nothing was deleted to get here");
    }

    /// Path 4: the owner was recorded by email only, and the account's email changed since. The emails differ, so the
    /// binding decided "another account". The proof adopts and records the full identity.
    #[test]
    fn an_email_only_owner_whose_email_changed_is_proven_and_rerecorded() {
        let pc = pc_with(&IDS);
        pc.db.0.set_owner(&Identity::new(None, Some("old-address@example.eu"))).unwrap();
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );

        assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(_)));
        assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
    }

    /// A session whose user id is not known yet (its profile was not fetched) over unowned data: the same proof
    /// applies, and the email-only record it writes is completed by the next start.
    #[test]
    fn a_session_without_its_user_id_yet_is_proven_by_its_email() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let by_email = Identity::new(None, Some("owner@example.eu"));
        assert!(matches!(prove(&pc, &by_email, MINE, &own_server()), Outcome::Adopted(_)));
        assert_eq!(bind_before_engine_start(&pc.db, &session(), WINDOWS), Ok(Bound::Kept));
        assert_eq!(pc.db.0.owner().unwrap(), Some(session()), "the id completes the record");
    }

    // ---- R10 holds: another account's data is never adopted, and nothing is deleted ----

    /// Another account's files are not in this account's tree. Nothing is adopted, the start still refuses, and every
    /// row and the held write are still there, not held for review either.
    #[test]
    fn another_accounts_data_is_never_adopted() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();

        let outcome = prove(&pc, &session(), MINE, &Server::new(Vec::new()));

        assert!(matches!(outcome, Outcome::NotProven(t) if t.missing == 3 && t.own == 0), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(
            bind_before_engine_start(&pc.db, &session(), WINDOWS),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );
        assert_eq!(due(&pc).len(), 1, "the held write is untouched: not deleted, not held for review");
        assert_eq!(pc.db.0.list_files().unwrap().len(), 3);
    }

    /// A node of this account's tree whose name does not open under this account's key blocks the adoption.
    #[test]
    fn a_name_sealed_under_another_key_blocks_adoption() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![mine(IDS[0]), mine(IDS[1]), node(THEIRS, IDS[2], None)]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::NotProven(t) if t.unproven == 1 && t.own == 2), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// The lead's case: a database that is 95% this account's own and 5% another account's (rows at the top level
    /// that are not in this account's tree) is refused EVERY time, because every row is checked, none sampled.
    #[test]
    fn a_five_percent_mixed_database_is_refused_every_time() {
        let all = ids("0c000000", 100);
        let theirs: HashSet<usize> = [3, 27, 51, 74, 99].into_iter().collect();
        for _ in 0..25 {
            let pc = empty_pc();
            for id in &all {
                add_row(&pc, id, None, Shape::Snapshot);
            }
            pc.db.0.enqueue_operation(&edit("held-1", &all[0])).unwrap();
            adopt_unbound(&pc.db, None).unwrap();
            let tree = all
                .iter()
                .enumerate()
                .filter(|(n, _)| !theirs.contains(n))
                .map(|(_, id)| mine(id))
                .collect();

            let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

            assert!(
                matches!(outcome, Outcome::NotProven(t) if t.checked == 100 && t.own == 95 && t.missing == 5),
                "{outcome:?}"
            );
            assert_eq!(pc.db.0.owner().unwrap(), None);
            assert_eq!(due(&pc), vec!["held-1".to_string()], "nothing held, nothing deleted");
        }
    }

    /// Every server-named row is judged, wherever it sits: one row of another account among 300 of this account's is
    /// found, at any position.
    #[test]
    fn every_row_the_server_named_is_judged_not_a_sample() {
        let all = ids("0c000001", 300);
        for theirs in [0, 150, 299] {
            let pc = empty_pc();
            for id in &all {
                add_row(&pc, id, None, Shape::Snapshot);
            }
            adopt_unbound(&pc.db, None).unwrap();
            let tree = all.iter().enumerate().filter(|(n, _)| *n != theirs).map(|(_, id)| mine(id)).collect();
            let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
            assert!(matches!(outcome, Outcome::NotProven(t) if t.checked == 300 && t.missing == 1), "{outcome:?}");
        }
    }

    /// Round 1 sampled only rows with `remote_updated_at > 0`, which the snapshot never writes. A row written by the
    /// snapshot, and one written by a delta, are both server-named and both judged.
    #[test]
    fn rows_written_by_the_snapshot_or_a_delta_are_both_judged() {
        let pc = empty_pc();
        add_row(&pc, IDS[0], None, Shape::Snapshot);
        add_row(&pc, IDS[1], None, Shape::Delta);
        add_row(&pc, IDS[2], None, Shape::Snapshot); // another account's: not in this tree
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &Server::new(vec![mine(IDS[0]), mine(IDS[1])]));
        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.checked == 3 && t.own == 2 && t.missing == 1),
            "{outcome:?}"
        );
        let named: Vec<(String, bool)> = pc
            .db
            .0
            .own_data_proof_rows()
            .unwrap()
            .into_iter()
            .map(|row| (row.file_id, row.server_named))
            .collect();
        assert!(named.iter().all(|(_, named)| *named), "{named:?}");
    }

    /// An owner recorded WITH a user id that differs from this session's is another account by R10's strongest
    /// signal. No proof is attempted: the server is not even asked.
    #[test]
    fn an_owner_recorded_by_another_user_id_is_never_questioned() {
        let pc = pc_with(&IDS);
        let alice = Identity::new(Some("user-alice"), Some("alice@example.eu"));
        pc.db.0.set_owner(&alice).unwrap();
        let server = own_server();

        assert_eq!(prove(&pc, &session(), MINE, &server), Outcome::NotEligible);
        assert_eq!(server.listed.get(), 0, "nothing asked");
        assert_eq!(pc.db.0.owner().unwrap(), Some(alice));
    }

    // ---- the listing ----

    /// Offline, or the server failing: the listing is asked twice, then nothing is decided and nothing recorded; a
    /// later start asks again.
    #[test]
    fn no_answer_decides_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server().failing_first(vec![Listing::Unavailable, Listing::Unavailable]);

        let outcome = prove(&pc, &session(), MINE, &server);

        assert!(matches!(outcome, Outcome::Unavailable(t) if t.unavailable == 1 && t.signed_out == 0), "{outcome:?}");
        assert_eq!(server.listed.get(), 2);
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(due(&pc).len(), 1);
    }

    /// A listing that does not answer the first time is asked once more.
    #[test]
    fn a_listing_that_does_not_answer_is_asked_once_more() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server().failing_first(vec![Listing::Unavailable]);
        assert!(matches!(prove(&pc, &session(), MINE, &server), Outcome::Adopted(t) if t.own == 3));
        assert_eq!(server.listed.get(), 2);
    }

    /// A token the server refuses (401) is not "offline": it is told apart, so the refusal does not tell the person to
    /// connect to the internet, and it is not asked again.
    #[test]
    fn a_refused_token_is_told_apart_from_no_connection() {
        assert!(matches!(listing_from_status(Some(401)), Listing::SignedOut));
        for status in [None, Some(404), Some(429), Some(500), Some(503)] {
            assert!(matches!(listing_from_status(status), Listing::Unavailable), "{status:?}");
        }
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server().failing_first(vec![Listing::SignedOut]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::Unavailable(t) if t.signed_out == 1 && t.unavailable == 0), "{outcome:?}");
        assert_eq!(server.listed.get(), 1);
    }

    /// A session that changed before the listing (a Lock, a sign-out) asks nothing: the key and token are not used.
    #[test]
    fn a_session_that_changed_before_the_listing_asks_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(&pc.db.0, &session(), &MINE, || server.list(), || false)).unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(server.listed.get(), 0);
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// A session that changed after the listing records nothing, and holds nothing.
    #[test]
    fn a_session_that_changed_during_the_proof_records_nothing() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            || server.list(),
            || server.listed.get() == 0,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(due(&pc).len(), 1, "nothing held");
    }

    /// An owner recorded by someone else while the proof ran is never replaced, and nothing is held.
    #[test]
    fn an_owner_recorded_during_the_proof_is_never_replaced() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        add_folder(&pc, "new-folder", None, Shape::Local);
        pc.db.0.enqueue_operation(&create("op-new", "new-folder", None, true)).unwrap();
        let server = own_server();
        let racer = Identity::new(Some("user-racer"), None);
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            || {
                pc.db.0.set_owner(&racer).unwrap();
                server.list()
            },
            || true,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), Some(racer));
        assert_eq!(due(&pc).len(), 2, "nothing held when nothing was recorded");
    }

    /// A Windows sign-out that emptied the database while the proof ran (its purge also removes the owner record):
    /// nothing is recorded, so the next account to sign in on this PC finds no owner and no data.
    #[test]
    fn an_owner_is_never_recorded_on_a_database_emptied_during_the_proof() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            || {
                if pc.db.0.has_account_data().unwrap() {
                    pc.db.0.remove_operation("held-1").unwrap();
                    pc.db.0.finish_windows_signout().unwrap();
                }
                server.list()
            },
            || true,
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Changed(_) | Outcome::NotProven(_)), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert!(!pc.db.0.has_account_data().unwrap());
    }

    /// Nothing the server named on this PC: nothing can be proven, so nothing is adopted.
    #[test]
    fn data_without_any_server_row_is_never_adopted() {
        let pc = empty_pc();
        add_row(&pc, "new-file", None, Shape::Local);
        pc.db.0.enqueue_operation(&create("op-new", "new-file", None, false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &own_server());
        assert!(matches!(outcome, Outcome::NotProven(t) if t.checked == 0), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
        assert_eq!(due(&pc).len(), 1);
    }

    // ---- rows gone since ----

    /// A file deleted since from one of this account's own folders does not block adoption: it is gone.
    #[test]
    fn a_file_deleted_from_one_of_this_accounts_folders_does_not_block_adoption() {
        let folder = "0a000000-0000-4000-8000-0000000000f0";
        let pc = pc_with(&IDS);
        add_folder(&pc, folder, None, Shape::Snapshot);
        add_row(&pc, "0a000000-0000-4000-8000-0000000000f1", Some(folder), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine(folder));
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(matches!(outcome, Outcome::Adopted(t) if t.gone == 1 && t.missing == 0), "{outcome:?}");
    }

    /// The nearest ancestor that IS in the tree decides: a file under a folder deleted since, inside a folder of this
    /// account's own, is gone.
    #[test]
    fn a_file_under_a_deleted_folder_inside_an_own_folder_is_gone() {
        let (top, deleted, file) = ("folder-top", "folder-deleted", "file-deleted");
        let pc = pc_with(&IDS);
        add_folder(&pc, top, None, Shape::Snapshot);
        add_folder(&pc, deleted, Some(top), Shape::Snapshot);
        add_row(&pc, file, Some(deleted), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine(top));
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(matches!(outcome, Outcome::Adopted(t) if t.gone == 2 && t.missing == 0), "{outcome:?}");
    }

    /// A file not in this account's tree whose folders are not in it either (up to the top) is not explained: it
    /// blocks. So does one whose nearest ancestor in the tree proves nothing (a plaintext name).
    #[test]
    fn a_missing_file_that_no_own_folder_explains_blocks_adoption() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "their-folder", None, Shape::Snapshot);
        add_row(&pc, "their-file", Some("their-folder"), Shape::Snapshot);
        add_folder(&pc, "plain-folder", None, Shape::Snapshot);
        add_row(&pc, "file-in-plain", Some("plain-folder"), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(with(mine("plain-folder"), "name_encrypted", serde_json::json!("Photos")));
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.missing == 3 && t.gone == 0 && t.neutral == 1),
            "{outcome:?}"
        );
    }

    /// Local parents that point at each other end the walk as missing; the proof terminates.
    #[test]
    fn a_cycle_in_local_parents_ends_as_missing() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "loop-a", Some("loop-b"), Shape::Snapshot);
        add_folder(&pc, "loop-b", Some("loop-a"), Shape::Snapshot);
        adopt_unbound(&pc.db, None).unwrap();
        let outcome = prove(&pc, &session(), MINE, &own_server());
        assert!(matches!(outcome, Outcome::NotProven(t) if t.missing == 2), "{outcome:?}");
    }

    // ---- names that prove nothing ----

    /// A name that proves nothing either way is set aside: a plaintext legacy name, no name, a file received through a
    /// file request, and (review P2-3) a file a collaborator uploaded into this account's folder (`uploaded_by`, or a
    /// file key wrapped for the owner).
    #[test]
    fn a_name_that_proves_nothing_is_set_aside_not_a_veto() {
        let id = IDS[0];
        let other = sealed_name(THEIRS, id);
        let cases = [
            (with(mine(id), "name_encrypted", serde_json::json!("Photos")), "plaintext"),
            (with(mine(id), "name_encrypted", serde_json::Value::Null), "no name"),
            (
                with(with(mine(id), "name_encrypted", serde_json::json!(other)), "file_request_id", serde_json::json!("0f000000-0000-4000-8000-000000000001")),
                "a file request upload",
            ),
            (
                with(with(mine(id), "name_encrypted", serde_json::json!(other)), "uploaded_by", serde_json::json!("user-colleague")),
                "uploaded by a collaborator",
            ),
            (
                with(with(mine(id), "name_encrypted", serde_json::json!(other)), "owner_wrapped_file_key", serde_json::json!("wrapped")),
                "a file key wrapped for the owner",
            ),
        ];
        for (node, why) in &cases {
            assert_eq!(probe(&MINE, node), Probe::Neutral, "{why}");
        }

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![cases[0].0.clone(), mine(IDS[1]), mine(IDS[2])]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::Adopted(t) if t.own == 2 && t.neutral == 1), "{outcome:?}");
    }

    /// Lead review of round 2, P3-6 (replaces self-review F1): a node that does not say who sealed its name (no
    /// `file_request_id`, `uploaded_by` or `owner_wrapped_file_key` key at all) and whose name does not open under this
    /// account's key is unproven, as one that says nobody else sealed it: a missing key is no evidence of a file
    /// request. An own name still proves it.
    #[test]
    fn a_name_that_does_not_open_is_unproven_when_the_api_does_not_say_who_sealed_it() {
        let without = |mut node: serde_json::Value, fields: &[&str]| {
            for field in fields {
                node.as_object_mut().unwrap().remove(*field);
            }
            node
        };
        let attribution = ["file_request_id", "uploaded_by", "owner_wrapped_file_key"];
        let received = |id: &str| with(mine(id), "name_encrypted", serde_json::json!(sealed_name(THEIRS, id)));
        assert_eq!(probe(&MINE, &without(received(IDS[0]), &attribution)), Probe::Unproven, "the API does not say");
        assert_eq!(
            probe(&MINE, &without(received(IDS[0]), &["uploaded_by", "owner_wrapped_file_key"])),
            Probe::Unproven,
            "the API says only that no file request sealed it"
        );
        assert_eq!(probe(&MINE, &received(IDS[0])), Probe::Unproven, "the API says nobody else sealed it");
        assert_eq!(probe(&MINE, &without(mine(IDS[0]), &attribution)), Probe::Own, "an own name still proves it");

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![
            without(received(IDS[0]), &attribution),
            without(mine(IDS[1]), &attribution),
            without(mine(IDS[2]), &attribution),
        ]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::NotProven(t) if t.own == 2 && t.unproven == 1), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// Self-review F2: a row the person put in the trash on the server while sync was refused is not in the snapshot,
    /// which lists only what is not in the trash. The proof reads the trash too (`listing_of`), so a top-level folder
    /// this account trashed (with a file in it that the trash listing does not name) is its own, and the data is
    /// adopted. Read from the snapshot alone, the same PC is refused. Either listing unread: no verdict.
    #[test]
    fn a_folder_this_account_trashed_on_the_server_is_its_own() {
        let trashed = "0b000000-0000-4000-8000-000000000009";
        let inside = "0b000000-0000-4000-8000-00000000000a";
        let snapshot = || crate::api_client::SyncSnapshot { seq_id: 7, nodes: vec![mine(IDS[0]), mine(IDS[1]), mine(IDS[2])] };
        let pc = || {
            let pc = pc_with(&IDS);
            add_folder(&pc, trashed, None, Shape::Snapshot);
            add_row(&pc, inside, Some(trashed), Shape::Snapshot);
            adopt_unbound(&pc.db, None).unwrap();
            pc
        };

        let outcome = prove(&pc(), &session(), MINE, &Server::new(snapshot().nodes));
        assert!(matches!(outcome, Outcome::NotProven(t) if t.missing == 2), "precondition, the snapshot alone: {outcome:?}");

        let in_trash = vec![with(mine(trashed), "is_trashed", serde_json::json!(true))];
        let Listing::Nodes(nodes) = listing_of(Ok(snapshot()), Ok(in_trash)) else {
            panic!("both listings read: the nodes of both");
        };
        let outcome = prove(&pc(), &session(), MINE, &Server::new(nodes));
        assert!(matches!(outcome, Outcome::Adopted(t) if t.own == 4 && t.missing == 0), "{outcome:?}");

        assert!(matches!(listing_of(Ok(snapshot()), Err(anyhow::anyhow!("offline"))), Listing::Unavailable));
        assert!(matches!(listing_of(Err(anyhow::anyhow!("offline")), Ok(Vec::new())), Listing::Unavailable));
    }

    /// Review P3-1: more rows that prove nothing than rows that prove ownership is not enough.
    #[test]
    fn more_names_that_prove_nothing_than_own_names_is_not_adopted() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let plain = |id: &str| with(mine(id), "name_encrypted", serde_json::json!("Photos"));
        let server = Server::new(vec![mine(IDS[0]), plain(IDS[1]), plain(IDS[2])]);
        let outcome = prove(&pc, &session(), MINE, &server);
        assert!(matches!(outcome, Outcome::NotProven(t) if t.own == 1 && t.neutral == 2), "{outcome:?}");
        assert_eq!(pc.db.0.owner().unwrap(), None);
    }

    /// A name this account sealed under the legacy key derivation (the old CLI's binary UUID) is this account's own,
    /// as the engine reads it.
    #[test]
    fn a_name_sealed_under_the_legacy_key_derivation_proves_ownership() {
        let id = IDS[0];
        assert_eq!(probe(&MINE, &sealed_binary_node(MINE, id)), Probe::Own);
        assert_eq!(probe(&MINE, &sealed_binary_node(THEIRS, id)), Probe::Unproven, "another account's, legacy");

        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = Server::new(vec![sealed_binary_node(MINE, IDS[0]), mine(IDS[1]), mine(IDS[2])]);
        assert!(matches!(prove(&pc, &session(), MINE, &server), Outcome::Adopted(t) if t.own == 3));
    }

    // ---- the queue: proven changes upload, the rest wait for the person ----

    /// The lead's case: all of this account's own data, plus a new folder at the top level (with a file in it) made
    /// while sync was refused. The data is adopted; the new folder's two changes are held for review in the same
    /// transaction as the owner record, and NOTHING of them is due; the edit of an own file uploads.
    #[test]
    fn an_all_own_database_with_a_new_top_level_folder_adopts_and_holds_the_folder() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "new-folder", None, Shape::Local);
        add_row(&pc, "new-file", Some("new-folder"), Shape::Local);
        pc.db.0.enqueue_operation(&create("op-folder", "new-folder", None, true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-file", "new-file", Some("new-folder"), false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();

        let outcome = prove(&pc, &session(), MINE, &own_server());

        assert!(
            matches!(outcome, Outcome::Adopted(t) if t.own == 3 && t.proven_changes == 1 && t.held_for_review == 2),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), Some(session()));
        assert_eq!(due(&pc), vec!["held-1".to_string()], "nothing of the new folder is due");
        assert_eq!(held_for_review(&pc), vec!["op-file".to_string(), "op-folder".to_string()]);
        assert_eq!(pc.db.0.list_files().unwrap().len(), 5, "nothing deleted");
    }

    /// A new file in one of this account's own folders is tied to its data: it uploads.
    #[test]
    fn a_new_file_in_an_own_folder_uploads() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "own-folder", None, Shape::Snapshot);
        add_folder(&pc, "new-sub", Some("own-folder"), Shape::Local);
        add_row(&pc, "new-file", Some("new-sub"), Shape::Local);
        pc.db.0.enqueue_operation(&create("op-sub", "new-sub", Some("own-folder"), true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-file", "new-file", Some("new-sub"), false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("own-folder"));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 3 && t.held_for_review == 0), "{outcome:?}");
        assert_eq!(due(&pc).len(), 3);
    }

    /// 128 of the 2,979 nested rows made on the maintainer's PC have no parent link, only their path. A new file whose
    /// row has none is tied to its folder by its path: in one of this account's own folders it uploads, at the top
    /// level it is held.
    #[test]
    fn a_new_item_whose_row_has_no_parent_link_is_judged_by_its_path() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "own-folder", "Projects", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "new-file", "Projects/new.txt", None, Shape::Local, Namespace::MyFiles, ItemKind::File);
        add_row_at(&pc, "new-top", "new-top.txt", None, Shape::Local, Namespace::MyFiles, ItemKind::File);
        pc.db.0.enqueue_operation(&create("op-file", "new-file", Some("own-folder"), false)).unwrap();
        pc.db.0.enqueue_operation(&create("op-top", "new-top", None, false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("own-folder"));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 2 && t.held_for_review == 1), "{outcome:?}");
        assert_eq!(held_for_review(&pc), vec!["op-top".to_string()]);
    }

    /// The maintainer's PC (task 1888): 2,832 folders made here whose create never reached the server share their
    /// path with the server's own folder (their twin). A folder create there is tied to this account's data through
    /// its own twin: it is not held. Without this rule all 2,984 queued changes on that PC were held for review. The
    /// new file's own row here is linked to the server's folder (a real parent link), so its upload is proven by more
    /// than a path (lead review of round 2, P1: content needs one such link). On that PC the uploads name the folder
    /// made here instead, and wait for the click: `on_the_maintainers_pc_an_upload_into_a_folder_made_here_waits`.
    #[test]
    fn a_change_in_a_folder_made_here_whose_server_twin_is_own_is_not_held() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "twin", "Backups", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "twin-sub", "Backups/PC", Some("twin"), Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "ghost", "Backups", None, Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "ghost-sub", "Backups/PC", Some("ghost"), Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "new-file", "Backups/a.txt", Some("twin"), Shape::Local, Namespace::MyFiles, ItemKind::File);
        pc.db.0.enqueue_operation(&create("op-ghost", "ghost", None, true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-ghost-sub", "ghost-sub", Some("ghost"), true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-new-file", "new-file", Some("ghost"), false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("twin"));
        tree.push(mine("twin-sub"));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(
            matches!(outcome, Outcome::Adopted(t) if t.own == 5 && t.proven_changes == 4 && t.held_for_review == 0),
            "{outcome:?}"
        );
        assert!(held_for_review(&pc).is_empty());
    }

    /// The maintainer's PC as it is (measured read-only: 145 of its 2,984 queued changes): an upload whose row names
    /// a folder made here as its parent is tied to the account by that folder's twin, a path only. It is new content,
    /// so it waits for the person's click (lead review of round 2, P1), while the folder's own create is proven.
    #[test]
    fn on_the_maintainers_pc_an_upload_into_a_folder_made_here_waits() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "twin", "Backups", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "ghost", "Backups", None, Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "new-file", "Backups/a.txt", Some("ghost"), Shape::Local, Namespace::MyFiles, ItemKind::File);
        pc.db.0.enqueue_operation(&create("op-ghost", "ghost", None, true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-new-file", "new-file", Some("ghost"), false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("twin"));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 2 && t.held_for_review == 1), "{outcome:?}");
        assert_eq!(held_for_review(&pc), vec!["op-new-file".to_string()]);
    }

    /// Lead review of round 2, P1: account A's rows made on this PC sit at the paths of account B's own rows (a
    /// legacy database where A's synced rows are gone and A's unsent changes are not). A's new "Documents" folder is
    /// tied to B's own "Documents" by its path, and a folder create sends no content, so it is proven (task 1888 folds
    /// it into B's folder). A's files are tied to B's data by a path only: through the folder (its twin), as a file at
    /// the path of one of B's files, or by the folder at their path when their row has no parent link. None of them
    /// is sent without the person's click.
    #[test]
    fn a_path_twin_proves_a_folder_made_here_but_never_the_content_in_it() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "b-documents", "Documents", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "b-notes", "Documents/notes.txt", Some("b-documents"), Shape::Snapshot, Namespace::MyFiles, ItemKind::File);
        add_row_at(&pc, "a-documents", "Documents", None, Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "a-report", "Documents/report.pdf", Some("a-documents"), Shape::Local, Namespace::MyFiles, ItemKind::File);
        add_row_at(&pc, "a-notes", "Documents/notes.txt", Some("a-documents"), Shape::Local, Namespace::MyFiles, ItemKind::File);
        add_row_at(&pc, "a-loose", "Documents/loose.txt", None, Shape::Local, Namespace::MyFiles, ItemKind::File);
        pc.db.0.enqueue_operation(&create("op-a-documents", "a-documents", None, true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-a-report", "a-report", Some("a-documents"), false)).unwrap();
        pc.db.0.enqueue_operation(&create("op-a-notes", "a-notes", Some("a-documents"), false)).unwrap();
        pc.db.0.enqueue_operation(&create("op-a-loose", "a-loose", None, false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("b-documents"));
        tree.push(mine("b-notes"));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(
            matches!(outcome, Outcome::Adopted(t) if t.own == 5 && t.proven_changes == 2 && t.held_for_review == 3),
            "{outcome:?}"
        );
        let mut now_due = due(&pc);
        now_due.sort();
        assert_eq!(now_due, vec!["held-1".to_string(), "op-a-documents".to_string()], "only the folder create");
        assert_eq!(
            held_for_review(&pc),
            vec!["op-a-loose".to_string(), "op-a-notes".to_string(), "op-a-report".to_string()],
            "no content is sent on a path's word"
        );
    }

    /// Self-review of round 3 (F1): a server row gone from this account's tree, with no parent link, is explained by
    /// the own folder at its path, so it is tied to that folder by a path only. New content for it is held. The same
    /// row with a real parent link to that folder is gone from it, tied by the link, and its edit uploads.
    #[test]
    fn content_for_a_gone_row_explained_only_by_the_folder_at_its_path_is_held() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "own-folder", "Projects", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "gone", "Projects/old.txt", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::File);
        add_row_at(&pc, "gone-linked", "Projects/linked.txt", Some("own-folder"), Shape::Snapshot, Namespace::MyFiles, ItemKind::File);
        pc.db.0.enqueue_operation(&edit("op-gone", "gone")).unwrap();
        pc.db.0.enqueue_operation(&edit("op-gone-linked", "gone-linked")).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("own-folder"));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(matches!(outcome, Outcome::Adopted(t) if t.gone == 2 && t.held_for_review == 1), "{outcome:?}");
        assert_eq!(held_for_review(&pc), vec!["op-gone".to_string()]);
    }

    /// Self-review of round 3 (F2): a path proves a folder create only for the folder at that path, its own twin. A
    /// new folder inside a folder made here has no twin: its create would give the account a folder name it never
    /// had, on a path's word, so it is held. The folders made here that do have an own twin are proven.
    #[test]
    fn a_new_folder_inside_a_folder_made_here_is_held_when_it_has_no_twin_of_its_own() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "twin", "Backups", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "twin-sub", "Backups/PC", Some("twin"), Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "ghost", "Backups", None, Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "ghost-sub", "Backups/PC", Some("ghost"), Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "ghost-new", "Backups/New", Some("ghost"), Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        pc.db.0.enqueue_operation(&create("op-ghost", "ghost", None, true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-ghost-sub", "ghost-sub", Some("ghost"), true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-ghost-new", "ghost-new", Some("ghost"), true)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("twin"));
        tree.push(mine("twin-sub"));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(matches!(outcome, Outcome::Adopted(t) if t.proven_changes == 3 && t.held_for_review == 1), "{outcome:?}");
        assert_eq!(held_for_review(&pc), vec!["op-ghost-new".to_string()]);
    }

    /// Lead review of round 2, P2-1: a row counts as made on this PC only when a queued create names it. Every other
    /// row is judged against the tree, even with both version columns at 0: one that is not in it is missing, and
    /// blocks. Before, such a row was taken for this PC's own new item and skipped.
    #[test]
    fn a_row_is_made_here_only_when_a_queued_create_names_it() {
        let pc = pc_with(&IDS);
        add_row(&pc, "never-queued", None, Shape::Local);
        add_row(&pc, "only-edited", None, Shape::Local);
        add_row(&pc, "new-here", None, Shape::Local);
        pc.db.0.enqueue_operation(&edit("op-edit", "only-edited")).unwrap();
        pc.db.0.enqueue_operation(&create("op-new", "new-here", None, false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();

        let outcome = prove(&pc, &session(), MINE, &own_server());

        assert!(
            matches!(outcome, Outcome::NotProven(t) if t.checked == 5 && t.own == 3 && t.missing == 2),
            "{outcome:?}"
        );
        assert_eq!(pc.db.0.owner().unwrap(), None);
        let made_here: Vec<String> = pc
            .db
            .0
            .own_data_proof_rows()
            .unwrap()
            .into_iter()
            .filter(|row| !row.server_named)
            .map(|row| row.file_id)
            .collect();
        assert_eq!(made_here, vec!["new-here".to_string()]);
    }

    /// Lead review of round 2, P3-2: an unproven change paused for any reason (quota, permission, a locked file) is held
    /// for review too: queued again, it would lose that pause and be sent unjudged. A proven one keeps its reason.
    #[test]
    fn an_unproven_change_paused_for_any_reason_is_held() {
        use crate::state_db::OperationPauseReason::{Locked, Permission, Quota};
        let pc = pc_with(&IDS);
        for (n, reason) in [Quota, Permission, Locked].into_iter().enumerate() {
            let id = format!("new-{n}");
            add_folder(&pc, &id, None, Shape::Local);
            pc.db.0.enqueue_operation(&create(&format!("op-{n}"), &id, None, true)).unwrap();
            pc.db.0.record_operation_pause(&format!("op-{n}"), reason, None, 5).unwrap();
        }
        pc.db.0.enqueue_operation(&edit("op-own-edit", IDS[1])).unwrap();
        pc.db.0.record_operation_pause("op-own-edit", Quota, None, 5).unwrap();
        adopt_unbound(&pc.db, None).unwrap();

        assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(t) if t.held_for_review == 3));
        assert_eq!(held_for_review(&pc), vec!["op-0".to_string(), "op-1".to_string(), "op-2".to_string()]);
        let quota = pc.db.0.queue_hold_summary().unwrap().held.into_iter().find(|group| group.reason == "quota");
        assert_eq!(quota.map(|group| group.count), Some(1), "the proven edit keeps its own pause");
    }

    /// The twin rule proves nothing by itself: a folder made here whose server twin's name proves nothing (or is not
    /// in this account's tree) keeps its changes held.
    #[test]
    fn a_change_in_a_folder_made_here_whose_server_twin_proves_nothing_is_held() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "twin", "Backups", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "ghost", "Backups", None, Shape::Local, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "new-file", "Backups/a.txt", Some("ghost"), Shape::Local, Namespace::MyFiles, ItemKind::File);
        pc.db.0.enqueue_operation(&create("op-ghost", "ghost", None, true)).unwrap();
        pc.db.0.enqueue_operation(&create("op-new-file", "new-file", Some("ghost"), false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(with(mine("twin"), "name_encrypted", serde_json::json!("Backups")));

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(matches!(outcome, Outcome::Adopted(t) if t.neutral == 1 && t.held_for_review == 2), "{outcome:?}");
        assert_eq!(held_for_review(&pc), vec!["op-ghost".to_string(), "op-new-file".to_string()]);
    }

    /// A server row with no parent link that is gone from this account's tree is explained by the folder at its path.
    #[test]
    fn a_deleted_row_without_a_parent_link_is_explained_by_the_folder_at_its_path() {
        let pc = pc_with(&IDS);
        add_row_at(&pc, "own-folder", "Projects", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::Folder);
        add_row_at(&pc, "deleted", "Projects/old.txt", None, Shape::Snapshot, Namespace::MyFiles, ItemKind::File);
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(mine("own-folder"));
        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));
        assert!(matches!(outcome, Outcome::Adopted(t) if t.gone == 1 && t.missing == 0), "{outcome:?}");
    }

    /// Changes in a folder shared with this account, and a new file in a folder whose name proves nothing, are not
    /// tied to this account's own data: held for review, never sent on their own. They do not block the adoption.
    #[test]
    fn changes_in_a_shared_folder_or_an_unproven_folder_are_held() {
        let pc = pc_with(&IDS);
        add_row_in(&pc, "shared-folder", None, Shape::Delta, Namespace::SharedWithMe, ItemKind::Folder);
        add_row_in(&pc, "shared-file", Some("shared-folder"), Shape::Delta, Namespace::SharedWithMe, ItemKind::File);
        add_row(&pc, "new-in-shared", Some("shared-folder"), Shape::Local);
        add_folder(&pc, "plain-folder", None, Shape::Snapshot);
        add_row(&pc, "new-in-plain", Some("plain-folder"), Shape::Local);
        pc.db.0.enqueue_operation(&edit("op-shared-edit", "shared-file")).unwrap();
        pc.db.0.enqueue_operation(&create("op-shared-new", "new-in-shared", Some("shared-folder"), false)).unwrap();
        pc.db.0.enqueue_operation(&create("op-plain-new", "new-in-plain", Some("plain-folder"), false)).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        let mut tree: Vec<_> = IDS.iter().map(|id| mine(id)).collect();
        tree.push(with(mine("plain-folder"), "name_encrypted", serde_json::json!("Photos")));
        tree.push(mine("own-extra")); // the tree may hold more than this PC

        let outcome = prove(&pc, &session(), MINE, &Server::new(tree));

        assert!(matches!(outcome, Outcome::Adopted(t) if t.held_for_review == 3 && t.proven_changes == 1), "{outcome:?}");
        assert_eq!(due(&pc), vec!["held-1".to_string()]);
        assert_eq!(
            held_for_review(&pc),
            vec!["op-plain-new".to_string(), "op-shared-edit".to_string(), "op-shared-new".to_string()]
        );
    }

    /// A change queued after the proof read the queue was never judged: it is held for review, not sent.
    #[test]
    fn a_change_queued_after_the_proof_read_the_queue_is_held() {
        let pc = pc_with(&IDS);
        adopt_unbound(&pc.db, None).unwrap();
        let server = own_server();
        let checks = Cell::new(0);
        let outcome = block_on(prove_and_adopt(
            &pc.db.0,
            &session(),
            &MINE,
            || server.list(),
            || {
                checks.set(checks.get() + 1);
                if server.listed.get() == 1 && checks.get() == 2 {
                    // Right before the record: the watcher queues an edit of an own file.
                    pc.db.0.enqueue_operation(&edit("late", IDS[1])).unwrap();
                }
                true
            },
        ))
        .unwrap();
        assert!(matches!(outcome, Outcome::Adopted(t) if t.held_for_review == 1), "{outcome:?}");
        assert_eq!(due(&pc), vec!["held-1".to_string()]);
        assert_eq!(held_for_review(&pc), vec!["late".to_string()]);
    }

    /// A change paused because the server refused the old session (`auth`) that the proof cannot tie to this
    /// account's data is held for review too, so the account's resume does not send it; a proven one stays `auth`.
    #[test]
    fn an_unproven_change_paused_for_auth_is_held_and_a_proven_one_is_not() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "new-folder", None, Shape::Local);
        pc.db.0.enqueue_operation(&create("op-folder", "new-folder", None, true)).unwrap();
        pc.db.0.enqueue_operation(&edit("op-own-edit", IDS[1])).unwrap();
        for op_id in ["op-folder", "op-own-edit"] {
            pc.db.0.record_operation_pause(op_id, crate::state_db::OperationPauseReason::Auth, None, 5).unwrap();
        }
        adopt_unbound(&pc.db, None).unwrap();

        assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(t) if t.held_for_review == 1));
        assert_eq!(pc.db.0.resume_operations_paused_for_auth(0).unwrap(), 1, "only the proven one resumes");
        assert_eq!(held_for_review(&pc), vec!["op-folder".to_string()]);
    }

    /// "Upload to my Beebeeb": every change held for review is due again; nothing else changes.
    #[test]
    fn upload_releases_only_the_changes_held_for_review() {
        let pc = pc_with(&IDS);
        add_folder(&pc, "new-folder", None, Shape::Local);
        pc.db.0.enqueue_operation(&create("op-folder", "new-folder", None, true)).unwrap();
        pc.db.0.enqueue_operation(&edit("op-auth", IDS[1])).unwrap();
        adopt_unbound(&pc.db, None).unwrap();
        assert!(matches!(prove(&pc, &session(), MINE, &own_server()), Outcome::Adopted(_)));
        pc.db.0.record_operation_pause("op-auth", crate::state_db::OperationPauseReason::Auth, None, 5).unwrap();

        assert_eq!(pc.db.0.release_found_while_paused(&["op-folder".to_string(), "op-auth".to_string()], 5).unwrap(), 1);

        let mut now_due = due(&pc);
        now_due.sort();
        assert_eq!(now_due, vec!["held-1".to_string(), "op-folder".to_string()], "op-auth stays paused for auth");
    }

    // ---- the pure parts ----

    #[test]
    fn the_verdict_table() {
        let t = |own, neutral, gone, unproven, missing| Tally {
            checked: own + neutral + gone + unproven + missing,
            own,
            neutral,
            gone,
            unproven,
            missing,
            ..Tally::default()
        };
        let rows = [
            (t(0, 0, 0, 0, 0), Verdict::NotProven, "nothing checked"),
            (t(1, 0, 0, 0, 0), Verdict::Adopt, "one row, its own"),
            (t(2, 0, 0, 0, 0), Verdict::Adopt, "two own"),
            (t(1, 0, 1, 0, 0), Verdict::NotProven, "one own of two checked is not enough (P3-1)"),
            (t(3, 0, 1, 0, 0), Verdict::Adopt, "a file gone from an own folder does not block"),
            (t(0, 0, 2, 0, 0), Verdict::NotProven, "only gone files prove nothing"),
            (t(7, 0, 0, 0, 1), Verdict::NotProven, "a file this account cannot see blocks"),
            (t(7, 0, 0, 1, 0), Verdict::NotProven, "a name sealed under another key blocks"),
            (t(2, 1, 0, 0, 0), Verdict::Adopt, "fewer neutral than own"),
            (t(2, 2, 0, 0, 0), Verdict::Adopt, "as many neutral as own"),
            (t(1, 2, 0, 0, 0), Verdict::NotProven, "more neutral than own (P3-1)"),
            (t(0, 3, 0, 0, 0), Verdict::NotProven, "only neutral"),
            (t(1, 1, 0, 0, 0), Verdict::NotProven, "one own of two checked (P3-1: min(2, checked))"),
            (t(40_000, 0, 12, 0, 0), Verdict::Adopt, "a large own tree"),
        ];
        for (tally, expected, why) in rows {
            assert_eq!(verdict(&tally), expected, "{why}: {tally:?}");
        }
    }

    #[test]
    fn a_probe_is_own_only_when_the_name_decrypts_under_this_key_for_this_id() {
        let id = IDS[0];
        assert_eq!(probe(&MINE, &mine(id)), Probe::Own);
        assert_eq!(probe(&MINE, &node(THEIRS, id, None)), Probe::Unproven, "another key");
        assert_eq!(
            probe(&MINE, &with(mine(id), "name_encrypted", serde_json::json!(sealed_name(MINE, IDS[1])))),
            Probe::Unproven,
            "another file's name"
        );
        assert!(name_opens(&MINE, id, &sealed_name(MINE, id)));
        assert!(!name_opens(&THEIRS, id, &sealed_name(MINE, id)));
    }

    #[test]
    fn eligibility() {
        let me = session();
        let me_by_email = Identity::new(None, Some("owner@example.eu"));
        let old_email = Identity::new(None, Some("old@example.eu"));
        let alice = Identity::new(Some("user-alice"), Some("alice@example.eu"));
        let alice_old_email = Identity::new(Some("user-alice"), Some("old@example.eu"));
        assert!(eligible(None, &me, true), "unowned data");
        assert!(!eligible(None, &me, false), "no data");
        assert!(!eligible(Some(&me), &me, true), "already this session's");
        assert!(!eligible(Some(&me_by_email), &me, true), "the same account by email");
        assert!(eligible(Some(&old_email), &me, true), "an email-only owner that differs");
        assert!(eligible(Some(&old_email), &me_by_email, true), "an email-only owner, a session without its id yet");
        // Review P3-2: an owner recorded WITH a user id is compared by user id only. A session whose id is not known
        // yet cannot be compared: not eligible until it is.
        assert!(!eligible(Some(&alice_old_email), &me_by_email, true), "an owner with an id, a session without one");
        assert!(!eligible(Some(&alice), &me, true), "another user id: never");
        assert!(!eligible(None, &Identity::default(), true), "an unknown session: never");
    }

    /// The copy a refused Windows start shows never tells a person they have another account.
    #[test]
    fn the_refusal_never_claims_another_account() {
        for sentence in [OTHER_ACCOUNT_ON_WINDOWS, IDENTITY_UNKNOWN] {
            let lower = sentence.to_lowercase();
            assert!(!lower.contains("another"), "{sentence}");
            assert!(!lower.contains("other account"), "{sentence}");
        }
        assert!(OTHER_ACCOUNT_ON_WINDOWS.contains("Nothing was uploaded or deleted"));
    }
}
