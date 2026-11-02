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








/// Whether a sealed name opens under this session's master key, by its own file id. It lives in the engine bridge since
/// task 1888, whose key check (spec D12) reads names the same way on Windows and Linux, where this module is not built.
pub use crate::engine_bridge::name_opens;












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

    /// The real `state.db` again, with a reset that is COUNTED rather than forbidden: on macOS and Linux a reset is
    /// allowed, so a test that expects the binding to keep the data must see the count stay 0.
    struct CountingReset {
        db: StateDb,
        resets: Cell<u32>,
    }

    impl CountingReset {
        fn open(dir: &std::path::Path) -> Self {
            Self {
                db: StateDb::open(dir.join("state.db")).unwrap(),
                resets: Cell::new(0),
            }
        }
    }

    impl LocalData for CountingReset {
        fn owner(&self) -> Result<Option<Identity>, String> {
            self.db.owner().map_err(|e| e.to_string())
        }
        fn has_account_data(&self) -> Result<bool, String> {
            self.db.has_account_data().map_err(|e| e.to_string())
        }
        fn reset(&self) -> Result<(), String> {
            self.resets.set(self.resets.get() + 1);
            self.db.clear_account_data(false).map_err(|e| e.to_string())
        }
        fn record_owner(&self, owner: &Identity) -> Result<(), String> {
            self.db.set_owner(owner).map_err(|e| e.to_string())
        }
        fn adopt_once(&self, candidate: Option<&Identity>) -> Result<bool, String> {
            self.db.adopt_owner_once(candidate).map_err(|e| e.to_string())
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

    mod stranding;
    mod listing;
    mod names;
    mod queue;
    mod verdicts;
}

mod types;
pub use types::{REQUIRED_PROOFS, Listing, Probe, Tally, Verdict, Outcome};

mod probe;
pub use probe::{eligible, probe, verdict, listing_from_status, listing_of};

mod judge;
pub use judge::{Judgement, judge};

mod adopt;
pub use adopt::{prove_and_adopt};
