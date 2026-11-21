//! Slice S2a's tests (plan wave 3: task 2267 and R10 ruling 8 in the engine), at the engine: the upgrade's candidate
//! fixed at the first startup and adopted inside the start, the ownership proof a refused start owes, the Lock and
//! the sign-out stopping it, an interrupted keep settled before an engine exists, and the card's decisions.
//!
//! Each test has a state folder of its own (`STATE_DIR_OVERRIDE` around `Engine::open`), a session put in memory as a
//! sign-in leaves it, a listing in place of the server's (`TestSeams::listing`), and Windows' answers in place of the
//! OS (`Port`: nobody's folder, and a registration that refuses with [`CODE`]). A start therefore goes past the
//! binding to the registration and never makes an engine here. Every name is invented.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::mpsc;
use std::time::Instant;

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, OperationKind, PendingOperation};
use crate::sync_root_owner::Registration;
use crate::sync_root_port::SyncRootPort;

/// Row S4 race: a stop that comes right after the proof was spawned, or before it read the keys.
#[path = "own_data_start_race_tests.rs"]
mod race;

/// Row fx-stop-timeout: a stopped proof that does not let go of its client within the stop's wait.
#[path = "own_data_start_stop_tests.rs"]
mod stop;

/// w26-account-switch: A signed out, B starts (v5 bug 6).
#[path = "own_data_start_switch_tests.rs"]
mod switch;

/// dc-111: "Discard and switch" over another account's unsent files.
#[path = "account_switch_discard_tests.rs"]
mod account_switch_discard;

type TestEngine = Engine<MemoryCredentials>;
type Answer = std::pin::Pin<Box<dyn std::future::Future<Output = Listing> + Send>>;

/// This session's master key, and another account's.
const MINE: [u8; 32] = [7; 32];
const THEIRS: [u8; 32] = [9; 32];
/// `HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_INVALID_REQUEST)`, an invented refusal of the registration.
const CODE: i32 = 0x8007_017C_u32 as i32;
const TODAY: &str = "2026-11-15";
const IDS: [&str; 3] = [
    "0a000000-0000-4000-8000-000000000001",
    "0a000000-0000-4000-8000-000000000002",
    "0a000000-0000-4000-8000-000000000003",
];
/// How long a test waits for the proof service.
const WAIT: Duration = Duration::from_secs(20);

fn owner() -> Identity {
    Identity::new(Some("user-owner"), Some("owner@example.eu"))
}

fn other() -> Identity {
    Identity::new(Some("user-other"), Some("other@example.eu"))
}

/// Windows' answers as a test makes them: nobody's folder, no live engine lock, a registration that refuses.
struct Port {
    registers: AtomicU32,
}

impl Port {
    fn registers(&self) -> u32 {
        self.registers.load(Ordering::SeqCst)
    }
}

impl SyncRootPort for Port {
    fn registration(&self, _folder: &Path) -> Registration {
        Registration::Nobody
    }
    fn register(&self, _folder: &Path) -> Result<(), Option<i32>> {
        self.registers.fetch_add(1, Ordering::SeqCst);
        Err(Some(CODE))
    }
    fn engine_lock_held(&self, _state_dir: &Path) -> bool {
        false
    }
}

/// One PC: a temp folder holding `state/` (this handle's state folder) and `Beebeeb/` (the sync folder), and the
/// handle on it. The start after an adoption asks the same `port`.
struct Pc {
    engine: Arc<TestEngine>,
    temp: tempfile::TempDir,
    port: Arc<Port>,
}

impl Pc {
    fn new() -> Pc {
        let _ = state_paths::init_for_test();
        let temp = tempfile::tempdir().unwrap();
        let state_dir = temp.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir_all(temp.path().join("Beebeeb")).unwrap();
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: "an-invented-slot".into(),
            state_dir: state_dir.clone(),
            api_base: "http://127.0.0.1:9".into(),
        };
        let opened = tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(
            state_paths::STATE_DIR_OVERRIDE.scope(state_dir, async move {
                Engine::open(&TEST, config, host, MemoryCredentials::default())
            }),
        );
        let engine = Arc::new(opened.expect("the handle opens on a state folder of its own"));
        let port = Arc::new(Port {
            registers: AtomicU32::new(0),
        });
        let pc = Pc { engine, temp, port };
        let restart: Arc<dyn SyncRootPort> = pc.port.clone();
        pc.seams(|seams| seams.restart_port = Some(restart));
        pc
    }

    fn root(&self) -> PathBuf {
        self.temp.path().join("Beebeeb")
    }

    fn state_db(&self) -> PathBuf {
        state_paths::state_db_path_from_state_dir(&self.engine.config.state_dir)
    }

    fn db(&self) -> StateDb {
        StateDb::open(self.state_db()).unwrap()
    }

    /// The folder a keep moves items into today, next to the sync folder.
    fn kept_dir(&self) -> PathBuf {
        self.temp.path().join(found_while_paused::KEPT_FOLDER_NAME).join(TODAY)
    }

    fn sign_in(&self, who: &Identity) {
        install_session(&self.engine, who);
    }

    fn seams<T>(&self, change: impl FnOnce(&mut TestSeams) -> T) -> T {
        let seat = self.engine.own_data_seat();
        let mut state = lock(&seat.state);
        change(&mut state.test)
    }

    fn outcomes(&self) -> Vec<&'static str> {
        self.seams(|seams| seams.outcomes.clone())
    }

    fn wait_for_outcomes(&self, count: usize) {
        wait_until("the proof's outcome", || self.outcomes().len() >= count);
    }

    fn startup_candidate(&self) -> Option<Identity> {
        lock(&self.engine.own_data_seat().state).startup.candidate.clone()
    }

    /// The start a person makes, with Windows' answers from `port`.
    fn start(&self) -> Result<crate::app::SyncStart, SyncStartRefused> {
        self.engine.start_sync_with(self.root(), &*self.port)
    }

    /// A new folder made on this PC at the top level of the sync folder: on disk, its row, its create queued.
    fn new_folder(&self, db: &StateDb, id: &str, name: &str) {
        std::fs::create_dir_all(self.root().join(name)).unwrap();
        row(db, id, name, true, 0);
        db.enqueue_operation(&queued(&format!("op-{id}"), OperationKind::CreateFolder, id, None))
            .unwrap();
    }

    /// `user_version`, read on a connection of its own.
    fn user_version(&self) -> i64 {
        rusqlite::Connection::open(self.state_db())
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    /// Every row of `sync_state`, `files` and `operation_queue`, read on a connection of its own.
    fn contents(&self) -> Vec<String> {
        let conn = rusqlite::Connection::open(self.state_db()).unwrap();
        let mut contents = Vec::new();
        for query in [
            "SELECT key || '=' || value FROM sync_state ORDER BY key",
            "SELECT file_id || ' ' || path FROM files ORDER BY file_id",
            "SELECT op_id || ' ' || IFNULL(paused_reason, '') FROM operation_queue ORDER BY op_id",
        ] {
            let mut statement = conn.prepare(query).unwrap();
            let rows = statement.query_map([], |row| row.get::<_, String>(0)).unwrap();
            contents.extend(rows.map(Result::unwrap));
        }
        contents
    }
}

/// A session in memory, as a sign-in and an unlock leave it (this account's key, the email, the user id recorded for
/// it). The session's turn moves on.
fn install_session(engine: &TestEngine, who: &Identity) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-token".into(),
        master_key: MINE,
        email: who.email.clone(),
    });
    engine.remember_session_user_id(who.user_id.clone());
    engine.set_auth_present(true);
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A My Files row at the top level, written through its contract. `version` 3: the server named it; 0: made here.
fn row(db: &StateDb, id: &str, path: &str, folder: bool, version: i64) {
    let kind = if folder { ItemKind::Folder } else { ItemKind::File };
    db.upsert_file_with_contract(
        &FileEntry {
            file_id: id.to_string(),
            path: path.to_string(),
            status: FileStatus::Local,
            size_bytes: 10,
            modified_at: 1_700_000_000,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: kind.clone(),
        },
        |contract| {
            contract.parent_id = None;
            contract.namespace = Namespace::MyFiles;
            contract.item_kind = kind;
            contract.current_version = version;
        },
    )
    .unwrap();
}

/// The server's rows a PC synced (`IDS`), and one edit of the first waiting to upload.
fn synced_rows(db: &StateDb) {
    for id in IDS {
        row(db, id, &format!("dir/{id}"), false, 3);
    }
    let edit = PendingOperation {
        base_version: Some(3),
        ..queued("op-edit", OperationKind::UploadVersion, IDS[0], None)
    };
    db.enqueue_operation(&edit).unwrap();
}

fn queued(op_id: &str, kind: OperationKind, file_id: &str, parent: Option<&str>) -> PendingOperation {
    PendingOperation {
        op_id: op_id.to_string(),
        kind,
        file_id: Some(file_id.to_string()),
        parent_id: parent.map(str::to_string),
        target_path: Some("x".to_string()),
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

fn held(db: &StateDb) -> Vec<String> {
    let mut held: Vec<String> = db
        .found_while_paused_ops()
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    held.sort();
    held
}

fn due(db: &StateDb) -> Vec<String> {
    let mut due: Vec<String> = db
        .list_due_operations(i64::MAX)
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    due.sort();
    due
}

/// A node of this account's tree, its name sealed under `key` as every client seals it.
fn node(key: [u8; 32], file_id: &str) -> serde_json::Value {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(key);
    serde_json::json!({
        "id": file_id,
        "parent_id": null,
        "name_encrypted": beebeeb_core::encrypt::encrypt_name(&master_key, file_id, "notes.txt", Some("text/plain")).unwrap(),
        "file_request_id": null,
        "uploaded_by": null,
        "owner_wrapped_file_key": null,
        "size_bytes": 10,
        "is_folder": false,
        "version_number": 3,
    })
}

fn tree(key: [u8; 32]) -> Vec<serde_json::Value> {
    IDS.iter().map(|id| node(key, id)).collect()
}

/// A listing that answers `listing` at once.
fn answering(listing: Listing) -> TestListing {
    Arc::new(move || -> Answer {
        let listing = listing.clone();
        Box::pin(async move { listing })
    })
}

/// A listing that says it was asked, then never answers.
fn hanging(asked: mpsc::Sender<()>) -> TestListing {
    let asked = Mutex::new(asked);
    Arc::new(move || -> Answer {
        let _ = asked.lock().unwrap().send(());
        Box::pin(std::future::pending())
    })
}

/// A listing that says it was asked, then answers `nodes` once `gate` is opened.
fn gated(asked: mpsc::Sender<()>, gate: Arc<tokio::sync::Notify>, nodes: Vec<serde_json::Value>) -> TestListing {
    let asked = Mutex::new(asked);
    Arc::new(move || -> Answer {
        let _ = asked.lock().unwrap().send(());
        let gate = gate.clone();
        let nodes = nodes.clone();
        Box::pin(async move {
            gate.notified().await;
            Listing::Nodes(nodes)
        })
    })
}

/// The files and folders under `root`, by path relative to it.
fn listing(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            found.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    found.sort();
    found
}

// ---- the proof a refused start owes (task 2267) ----

/// Rule 1 (mutant M1: skip the start after `Adopted`), and rule 3 (mutant M3: the adoption releases the held change).
/// A start refused with `OtherAccountsData` over data that IS this account's own (every server-named row opens under
/// this session's key, and the listing names it) is adopted, and sync starts again without another press: the start
/// goes past the binding to the registration. The owner record is the session's identity; the change to proven data
/// is due, the new folder at the top level is held, and nothing is deleted.
#[test]
fn a_windows_refusal_over_unowned_data_asks_for_the_proof_and_only_an_adoption_starts_sync() {
    let pc = Pc::new();
    let db = pc.db();
    synced_rows(&db);
    pc.new_folder(&db, "new-folder", "Trip");
    let rows = db.list_files().unwrap().len();
    drop(db);
    pc.sign_in(&owner());
    // The listing is held until the refused start has been checked: an answer at once lets the proof adopt and start
    // sync again by itself before the check below runs (it did on a fast Windows PC: "asked Windows nothing: 1 vs 0").
    let (asked, listing_asked) = mpsc::channel();
    let gate = Arc::new(tokio::sync::Notify::new());
    pc.seams(|seams| seams.listing = Some(gated(asked, gate.clone(), tree(MINE))));
    pc.engine.serve_own_data_proofs();

    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    assert_eq!(pc.port.registers(), 0, "the refused start asked Windows nothing");
    listing_asked.recv_timeout(WAIT).expect("the refusal asked for the proof");
    assert_eq!(pc.port.registers(), 0, "and nothing starts while the proof waits for its answer");
    gate.notify_one();
    pc.wait_for_outcomes(1);

    assert_eq!(pc.outcomes(), vec!["adopted"]);
    assert_eq!(pc.port.registers(), 1, "sync started again by itself, past the binding");
    assert_eq!(
        pc.engine.view().start,
        Start::Refused(SyncStartRefused::RegistrationFailed { code: Some(CODE) }),
        "the start after the proof is recorded as any start is"
    );
    let db = pc.db();
    assert_eq!(
        db.owner().unwrap(),
        Some(owner()),
        "the session's identity is the owner"
    );
    assert_eq!(
        due(&db),
        vec!["op-edit".to_string()],
        "the change to proven data uploads"
    );
    assert_eq!(
        held(&db),
        vec!["op-new-folder".to_string()],
        "rule 3: held, never sent on its own"
    );
    assert_eq!(db.list_files().unwrap().len(), rows, "nothing deleted");
}

/// Rule 2 (mutant M2: `adopt_owner_holding` without the verdict check). Over ANOTHER account's data (names sealed under
/// another key) nothing is adopted: the refusal stays, the owner record is unchanged, nothing is deleted or released,
/// and Windows is asked nothing.
#[test]
fn another_accounts_data_is_never_adopted_and_nothing_is_deleted() {
    let pc = Pc::new();
    let db = pc.db();
    synced_rows(&db);
    drop(db);
    std::fs::create_dir_all(pc.root().join("dir")).unwrap();
    std::fs::write(pc.root().join("dir").join(IDS[1]), b"ten bytes!").unwrap();
    let files = listing(&pc.root());
    let contents = pc.contents();
    pc.sign_in(&owner());
    pc.seams(|seams| seams.listing = Some(answering(Listing::Nodes(tree(THEIRS)))));
    pc.engine.serve_own_data_proofs();

    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    pc.wait_for_outcomes(1);

    assert_eq!(pc.outcomes(), vec!["not_proven"]);
    assert_eq!(
        pc.engine.view().start,
        Start::Refused(SyncStartRefused::OtherAccountsData)
    );
    assert_eq!(pc.db().owner().unwrap(), None, "the owner record is unchanged");
    assert_eq!(pc.contents(), contents, "every row and queued change as it was");
    assert_eq!(listing(&pc.root()), files, "every file as it was");
    assert_eq!(pc.port.registers(), 0, "no start");
}

/// Rule 4 (mutant M2b: `still_current` is `|| true`). A session that changes while the listing runs: nothing is
/// recorded and nothing starts.
#[test]
fn a_session_that_moves_during_the_listing_records_nothing_and_starts_nothing() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let engine = Arc::downgrade(&pc.engine);
    let nodes = tree(MINE);
    let moving: TestListing = Arc::new(move || -> Answer {
        if let Some(engine) = engine.upgrade() {
            install_session(&engine, &owner());
        }
        let nodes = nodes.clone();
        Box::pin(async move { Listing::Nodes(nodes) })
    });
    pc.seams(|seams| seams.listing = Some(moving));
    pc.engine.serve_own_data_proofs();

    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    pc.wait_for_outcomes(1);

    assert_eq!(pc.outcomes(), vec!["changed"]);
    assert_eq!(pc.db().owner().unwrap(), None, "nothing recorded");
    assert_eq!(pc.port.registers(), 0, "no start");
}

/// Rule 5 (mutant M4: no `stop_proofs` in the Lock step). The Lock stops a proof whose listing hangs, and the proof's
/// client (the session's token and key) is gone before the Lock returns.
#[test]
fn a_lock_stops_a_running_proof_and_its_client_is_released() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    pc.seams(|seams| seams.listing = Some(hanging(asked)));
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    listing_asked.recv_timeout(WAIT).expect("the proof is listing");
    let clients = pc.seams(|seams| seams.clients.clone());
    assert_eq!(clients.len(), 1);
    assert!(clients[0].strong_count() > 0, "the premise: the proof holds its client");

    let _ = pc.engine.lock();

    assert_eq!(
        clients[0].strong_count(),
        0,
        "the Lock released the proof's client before it returned"
    );
    pc.wait_for_outcomes(1);
    assert_eq!(pc.outcomes(), vec!["stopped"]);
    assert_eq!(pc.db().owner().unwrap(), None);
}

/// Rule 5 (mutant M4b: no `stop_proofs` in the sign-out step), as the test above for the sign-out that keeps the local
/// data.
#[test]
fn a_sign_out_stops_a_running_proof_and_its_client_is_released() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    pc.seams(|seams| seams.listing = Some(hanging(asked)));
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    listing_asked.recv_timeout(WAIT).expect("the proof is listing");
    let clients = pc.seams(|seams| seams.clients.clone());
    assert!(clients[0].strong_count() > 0, "the premise: the proof holds its client");

    let _ = pc.engine.sign_out_keeping_local_data();

    assert_eq!(
        clients[0].strong_count(),
        0,
        "the sign-out released the proof's client before it returned"
    );
    pc.wait_for_outcomes(1);
    assert_eq!(pc.outcomes(), vec!["stopped"]);
}

/// Rule 6. A listing that never answers (its time-out, here 50 ms) or answers nothing usable: the refusal becomes
/// `IdentityUnknown` ("connect and open Beebeeb again"), and nothing is recorded.
#[test]
fn a_listing_that_times_out_or_answers_nothing_says_identity_unknown() {
    let (asked, _listing_asked) = mpsc::channel();
    for listing in [hanging(asked), answering(Listing::Unavailable)] {
        let pc = Pc::new();
        synced_rows(&pc.db());
        pc.sign_in(&owner());
        pc.seams(|seams| {
            seams.listing = Some(listing);
            seams.list_timeout = Some(Duration::from_millis(50));
        });
        pc.engine.serve_own_data_proofs();

        assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
        pc.wait_for_outcomes(1);

        assert_eq!(pc.outcomes(), vec!["unavailable"]);
        assert_eq!(
            pc.engine.view().start,
            Start::Refused(SyncStartRefused::IdentityUnknown)
        );
        assert_eq!(pc.db().owner().unwrap(), None);
    }
}

/// Rule 6 (mutant M5: `Unavailable` with `signed_out == 1` mapped to `IdentityUnknown`). A listing the server refused
/// with 401 is not "offline": the refusal stays as it was.
#[test]
fn a_listing_refused_as_signed_out_is_not_turned_into_offline() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    pc.seams(|seams| seams.listing = Some(answering(Listing::SignedOut)));
    pc.engine.serve_own_data_proofs();

    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    pc.wait_for_outcomes(1);

    assert_eq!(pc.outcomes(), vec!["unavailable"]);
    assert_eq!(
        pc.engine.view().start,
        Start::Refused(SyncStartRefused::OtherAccountsData)
    );
}

/// Rule 7 (mutant M6: no one-proof-per-turn guard). A second refusal for the same turn, while the first proof lists,
/// starts no second proof.
#[test]
fn a_second_refusal_for_the_same_turn_starts_no_second_proof() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    let gate = Arc::new(tokio::sync::Notify::new());
    pc.seams(|seams| seams.listing = Some(gated(asked, gate.clone(), tree(MINE))));
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    listing_asked.recv_timeout(WAIT).expect("the first proof is listing");

    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::OtherAccountsData),
        "refused again, same turn"
    );
    gate.notify_one();
    wait_until("the second refusal's turn at the service", || {
        pc.seams(|seams| seams.owed_seen) >= 2
    });

    assert_eq!(pc.seams(|seams| seams.proofs_run), 1, "one proof for the turn");
    assert_eq!(pc.outcomes(), vec!["adopted"]);
    assert_eq!(pc.port.registers(), 1);
}

// ---- an interrupted keep, settled before an engine exists (task 2267, ruling (a) 4) ----

/// The data of `owner()`, with a new folder `Trip` at the top level held for review (as an adoption leaves it).
fn owned_with_a_held_folder(pc: &Pc) -> StateDb {
    let db = pc.db();
    pc.new_folder(&db, "new-folder", "Trip");
    assert!(
        db.adopt_owner_holding(None, &owner(), &HashSet::new(), 5)
            .unwrap()
            .is_some_and(|held| held > 0),
        "the premise: owned, the folder's change held"
    );
    db
}

/// A keep of everything held that ends in the middle: `move_first` says whether the item moved before the end.
fn interrupted_keep(pc: &Pc, db: &StateDb, move_first: bool) {
    let crash = |from: &Path, to: &Path| -> std::io::Result<()> {
        if move_first {
            found_while_paused::rename_no_replace(from, to).unwrap();
        }
        panic!("the process ends here");
    };
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        found_while_paused::keep(db, &pc.root(), TODAY, &crash, &held(db))
    }));
    assert!(crashed.is_err(), "the premise: the keep was interrupted");
    assert!(db.keep_journal().unwrap().is_some(), "the premise: a move in progress");
}

/// Rule 8 (mutant M7: the settle after `EngineRunner::spawn`). A keep a crash interrupted after the move is finished by
/// the next start, before an engine is made: here the start goes on to the registration (which refuses, so no engine
/// exists) and the item's rows and changes are already forgotten; the item is in the kept folder.
#[test]
fn a_keep_a_crash_interrupted_is_finished_before_the_next_engine_is_created() {
    let pc = Pc::new();
    let db = owned_with_a_held_folder(&pc);
    interrupted_keep(&pc, &db, true);
    assert!(
        pc.kept_dir().join("Trip").is_dir() && !pc.root().join("Trip").exists(),
        "the premise: moved"
    );
    drop(db);
    pc.sign_in(&owner());

    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) })
    );

    assert_eq!(pc.port.registers(), 1, "the start went on after the settle");
    let db = pc.db();
    assert_eq!(db.keep_journal().unwrap(), None, "settled");
    assert_eq!(held(&db), Vec::<String>::new(), "its change is forgotten");
    assert!(db.list_files().unwrap().is_empty(), "its row is forgotten");
    assert!(pc.kept_dir().join("Trip").is_dir(), "nothing lost");
}

/// Rule 8, the other half, and Part B wired. A keep interrupted BEFORE the move is undone by the next start (the item
/// stays where it is, still held). Then the card's decisions answer on this account's own data: the review shows the
/// held change, Keep moves the item, and "Show in Explorer" finds the dated folder.
#[test]
fn the_found_while_paused_decision_is_wired_and_an_interrupted_keep_is_settled_before_the_engine() {
    let pc = Pc::new();
    let db = owned_with_a_held_folder(&pc);
    interrupted_keep(&pc, &db, false);
    drop(db);
    pc.sign_in(&owner());

    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) })
    );

    let db = pc.db();
    assert_eq!(db.keep_journal().unwrap(), None, "settled");
    assert!(pc.root().join("Trip").is_dir(), "undone: the item stays where it is");
    assert_eq!(held(&db), vec!["op-new-folder".to_string()], "still held");

    let turn = pc.engine.session_turn();
    let review = pc
        .engine
        .found_while_paused_review_in(turn, &pc.root())
        .expect("the review");
    assert_eq!(review.op_ids, vec!["op-new-folder".to_string()]);
    let kept = pc
        .engine
        .found_while_paused_keep_in(turn, &review.op_ids, &pc.root(), TODAY)
        .expect("the keep");
    assert_eq!(kept.moved_items, 1);
    assert!(pc.kept_dir().join("Trip").is_dir() && !pc.root().join("Trip").exists());
    assert_eq!(pc.engine.kept_folder_to_show_in(turn, &pc.root()), Ok(pc.kept_dir()));
}

// ---- the card's decisions (Part B) ----

/// Rule 9 (mutant M8: the release before the first ask). Upload releases exactly the ids the review returned; a change
/// held after the review stays held; a reconcile pass is asked for before the release and again after it.
#[test]
fn upload_releases_only_the_reviewed_ids_between_two_reconcile_asks() {
    let pc = Pc::new();
    let db = pc.db();
    pc.new_folder(&db, "new-a", "A");
    pc.new_folder(&db, "new-b", "B");
    db.adopt_owner_holding(None, &owner(), &HashSet::new(), 5).unwrap();
    pc.sign_in(&owner());
    let turn = pc.engine.session_turn();
    let review = pc
        .engine
        .found_while_paused_review_in(turn, &pc.root())
        .expect("the review");
    assert_eq!(review.op_ids, vec!["op-new-a".to_string(), "op-new-b".to_string()]);
    pc.new_folder(&db, "new-c", "C");
    db.adopt_owner_holding(Some(&owner()), &owner(), &HashSet::new(), 6)
        .unwrap();
    assert_eq!(held(&db).len(), 3, "the premise: a third change held after the review");
    let asks = pc.engine.acct.reconcile_pass_wanted.load(Ordering::SeqCst);

    let released = pc.engine.found_while_paused_upload_in(turn, &review.op_ids, 100);

    assert_eq!(released, Ok(2));
    assert_eq!(
        held(&db),
        vec!["op-new-c".to_string()],
        "held after the review: still held"
    );
    assert_eq!(due(&db), vec!["op-new-a".to_string(), "op-new-b".to_string()]);
    assert_eq!(pc.engine.acct.reconcile_pass_wanted.load(Ordering::SeqCst), asks + 2);
    assert_eq!(
        pc.seams(|seams| seams.asks_at_release.clone()),
        vec![asks + 1],
        "asked once before the release"
    );
}

/// Rule 9. Keep moves by rename only and never over a name: an item of the same name already in the dated folder
/// stays as it was, and the kept item gets the next free name.
#[test]
fn keep_moves_by_rename_and_never_over_a_name() {
    let pc = Pc::new();
    let db = owned_with_a_held_folder(&pc);
    std::fs::create_dir_all(pc.kept_dir().join("Trip")).unwrap();
    std::fs::write(pc.kept_dir().join("Trip").join("there.txt"), b"before").unwrap();
    pc.sign_in(&owner());
    let turn = pc.engine.session_turn();

    let kept = pc
        .engine
        .found_while_paused_keep_in(turn, &held(&db), &pc.root(), TODAY)
        .expect("the keep");

    assert_eq!(kept.moved_items, 1);
    assert!(!pc.root().join("Trip").exists(), "moved out of the sync folder");
    assert!(pc.kept_dir().join("Trip (2)").is_dir(), "under the next free name");
    assert_eq!(
        std::fs::read(pc.kept_dir().join("Trip").join("there.txt")).unwrap(),
        b"before",
        "nothing replaced"
    );
}

/// Rule 9 (mutant M9: a non-dated name accepted). "Show in Explorer" opens only a `YYYY-MM-DD` folder directly inside
/// the kept folder next to the sync folder, that is there.
#[test]
fn the_kept_folder_is_shown_only_when_dated_inside_the_kept_folder_and_there() {
    let pc = Pc::new();
    let db = pc.db();
    row(&db, IDS[0], "dir/a", false, 3);
    db.set_owner(&owner()).unwrap();
    pc.sign_in(&owner());
    let turn = pc.engine.session_turn();
    let gone = Err(KEPT_FOLDER_GONE.to_string());
    assert_eq!(
        pc.engine.kept_folder_to_show_in(turn, &pc.root()),
        gone,
        "nothing recorded"
    );
    let kept_folder = pc.temp.path().join(found_while_paused::KEPT_FOLDER_NAME);
    let elsewhere = pc.temp.path().join("elsewhere").join(TODAY);
    for (recorded, made, shown) in [
        (pc.kept_dir(), true, true),
        (kept_folder.join("not-a-date"), true, false),
        (elsewhere, true, false),
        (kept_folder.join("2026-01-01"), false, false),
    ] {
        if made {
            std::fs::create_dir_all(&recorded).unwrap();
        }
        db.finish_keep(&[], &recorded.to_string_lossy()).unwrap();
        let expected = if shown { Ok(recorded.clone()) } else { gone.clone() };
        assert_eq!(
            pc.engine.kept_folder_to_show_in(turn, &pc.root()),
            expected,
            "{recorded:?}"
        );
    }
}

/// Rule 9. Every decision refuses another account's local data, a turn that moved since the card was read, and a
/// handle with nobody signed in, with the one sentence, and changes nothing.
#[test]
fn every_decision_refuses_another_accounts_data_and_a_moved_turn() {
    let pc = Pc::new();
    let db = pc.db();
    pc.new_folder(&db, "new-folder", "Trip");
    db.adopt_owner_holding(None, &other(), &HashSet::new(), 5).unwrap();
    let ids = held(&db);
    let not_yours = FOUND_WHILE_PAUSED_NOT_YOURS.to_string();
    let every_decision = |turn: SessionGeneration| {
        [
            pc.engine.found_while_paused_review_in(turn, &pc.root()).err(),
            pc.engine.found_while_paused_upload_in(turn, &ids, 100).err(),
            pc.engine
                .found_while_paused_keep_in(turn, &ids, &pc.root(), TODAY)
                .err(),
            pc.engine.kept_folder_to_show_in(turn, &pc.root()).err(),
        ]
    };
    let refused = [
        Some(not_yours.clone()),
        Some(not_yours.clone()),
        Some(not_yours.clone()),
        Some(not_yours.clone()),
    ];

    assert_eq!(every_decision(pc.engine.session_turn()), refused, "nobody signed in");
    pc.sign_in(&owner());
    assert_eq!(
        every_decision(pc.engine.session_turn()),
        refused,
        "another account's data"
    );
    db.set_owner(&owner()).unwrap();
    let read_at = pc.engine.session_turn();
    pc.sign_in(&owner());
    assert_eq!(
        every_decision(read_at),
        refused,
        "the turn moved since the card was read"
    );
    assert_eq!(held(&db), ids, "nothing released");
    assert!(pc.root().join("Trip").is_dir(), "nothing moved");
}

// ---- the upgrade from a release before the binding (R10 ruling 8; lead rulings 2026-11-15 17:35) ----

/// Ruling (1) (mutant MU1: the startup hook returns at once; MU2: the window's mark not written). The first startup,
/// with this account's key loaded, fixes the candidate and shuts the window on disk, and writes nothing else: no owner,
/// no row, no change.
#[test]
fn the_first_startup_shuts_the_window_writing_only_its_mark_and_fixes_the_candidate() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    assert_eq!(
        pc.user_version(),
        0,
        "the premise: the window is open, as a release before the binding left it"
    );
    let contents = pc.contents();
    pc.sign_in(&owner());

    pc.engine.fix_upgrade_candidate_at_startup();

    assert_eq!(pc.user_version(), 1, "shut on disk");
    assert_eq!(pc.startup_candidate(), Some(owner()));
    assert_eq!(pc.contents(), contents, "nothing written but the mark");
}

/// Ruling (1). A first startup with no key adopts nobody later, and shuts the window all the same.
#[test]
fn a_first_startup_without_a_key_fixes_no_candidate_and_shuts_the_window() {
    let pc = Pc::new();
    synced_rows(&pc.db());

    pc.engine.fix_upgrade_candidate_at_startup();

    assert_eq!(pc.user_version(), 1);
    assert_eq!(pc.startup_candidate(), None);
}

/// Ruling (1). The startup never makes a `state.db` (nor its folder's contents).
#[test]
fn the_startup_makes_no_state_db() {
    let pc = Pc::new();
    pc.sign_in(&owner());
    let before = listing(&pc.engine.config.state_dir);

    pc.engine.fix_upgrade_candidate_at_startup();

    assert!(!pc.state_db().exists(), "no state.db");
    assert_eq!(
        listing(&pc.engine.config.state_dir),
        before,
        "nothing made in the state folder"
    );
    assert_eq!(
        pc.startup_candidate(),
        Some(owner()),
        "fixed for the start that brings a state.db"
    );
}

/// Ruling (1). Only the first startup of a handle fixes the candidate: a later one, whatever is signed in then,
/// changes nothing.
#[test]
fn only_the_first_startup_fixes_the_candidate() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    pc.engine.fix_upgrade_candidate_at_startup();
    pc.sign_in(&other());

    pc.engine.fix_upgrade_candidate_at_startup();

    assert_eq!(pc.startup_candidate(), Some(owner()));
}

/// Ruling (2a) (mutant MU4: no adoption in `local_data_for_start`). The candidate's own first start records it as the
/// owner INSIDE the start, before the binding and any engine, and keeps everything: the queued change is due, nothing
/// is held, and the start goes on to the registration. The window is shut after it.
#[test]
fn the_candidates_first_start_adopts_in_the_start_and_keeps_everything() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    pc.engine.fix_upgrade_candidate_at_startup();

    assert_eq!(
        pc.start(),
        Err(SyncStartRefused::RegistrationFailed { code: Some(CODE) })
    );

    let db = pc.db();
    assert_eq!(db.owner().unwrap(), Some(owner()));
    assert_eq!(due(&db), vec!["op-edit".to_string()], "the queue is kept");
    assert_eq!(held(&db), Vec::<String>::new(), "nothing held");
    assert_eq!(pc.port.registers(), 1, "recorded before the registration");
    assert_eq!(pc.startup_candidate(), None, "adopted once");
    assert!(!db.adopt_owner_once(Some(&other())).unwrap(), "the window is shut");
}

/// Ruling (2a) (mutant MU3: the candidate rule without the comparison). Another account than the candidate adopts
/// nothing at its start: refused, nobody recorded; the candidate is still the candidate.
#[test]
fn another_account_than_the_candidate_adopts_nothing_at_its_start() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    pc.engine.fix_upgrade_candidate_at_startup();
    pc.sign_in(&other());

    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(pc.db().owner().unwrap(), None);
    assert_eq!(pc.startup_candidate(), Some(owner()));
}

/// Ruling (1) and (2). A key that came after the first startup adopts nothing by the startup's route (the proof is the
/// way for such a PC).
#[test]
fn a_key_that_came_after_the_first_startup_adopts_nothing() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.engine.fix_upgrade_candidate_at_startup();
    pc.sign_in(&owner());

    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(pc.db().owner().unwrap(), None);
}

/// Ruling (2a) (mutant MU5: the adoption without its "no owner yet" check). The candidate never replaces a recorded
/// owner: the start is refused and the owner stays.
#[test]
fn the_upgrade_candidate_never_replaces_a_recorded_owner() {
    let pc = Pc::new();
    let db = pc.db();
    synced_rows(&db);
    db.set_owner(&other()).unwrap();
    drop(db);
    pc.sign_in(&owner());
    pc.engine.fix_upgrade_candidate_at_startup();

    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));

    assert_eq!(pc.db().owner().unwrap(), Some(other()));
}

/// Ruling (2a) and (3). The candidate rule: the same account by the binding's rule, and nothing for an unknown session
/// or candidate.
#[test]
fn the_upgrade_candidate_rule() {
    let id_only = |id: &str| Identity::new(Some(id), None);
    let email_only = |email: &str| Identity::new(None, Some(email));
    let cases: [(Option<Identity>, Identity, bool); 7] = [
        (Some(owner()), owner(), true),
        (Some(id_only("user-owner")), owner(), true),
        (Some(id_only("user-owner")), other(), false),
        (Some(owner()), Identity::default(), false),
        (Some(Identity::default()), owner(), false),
        (None, owner(), false),
        (Some(id_only("user-owner")), email_only("owner@example.eu"), false),
    ];
    for (candidate, session, adopts) in cases {
        assert_eq!(
            account_binding::is_the_upgrade_candidate(candidate.as_ref(), &session),
            adopts,
            "{candidate:?} / {session:?}"
        );
    }
}

/// For the lead's the Windows test machine rung R-UP-owner: the statements that read, remove and shut a 0.8.25 `state.db`'s owner record
/// and adoption window, run on a copy of the release's own image (`tests/fixtures/state-db-0.8.25/closed`).
#[test]
fn the_r_up_owner_statements_work_on_a_0_8_25_state_db() {
    const READ: &str = "SELECT key, value FROM sync_state WHERE key IN ('owner_user_id', 'owner_email') ORDER BY key";
    const REMOVE: &str = "DELETE FROM sync_state WHERE key IN ('owner_user_id', 'owner_email')";
    const SHUT: &str = "PRAGMA user_version = 1";
    let image = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state-db-0.8.25/closed/state.db");
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.db");
    std::fs::copy(image, &path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "INSERT INTO sync_state (key, value) VALUES ('owner_user_id', 'user-owner'), ('owner_email', 'owner@example.eu')",
    )
    .unwrap();
    let read = |conn: &rusqlite::Connection| -> Vec<(String, String)> {
        let mut statement = conn.prepare(READ).unwrap();
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert_eq!(read(&conn).len(), 2, "the premise: an owner recorded");

    assert_eq!(conn.execute(REMOVE, []).unwrap(), 2);
    conn.execute_batch(SHUT).unwrap();

    assert_eq!(read(&conn), Vec::<(String, String)>::new());
    drop(conn);
    let db = StateDb::open(&path).unwrap();
    assert_eq!(db.owner().unwrap(), None);
    assert!(
        db.has_account_data().unwrap(),
        "the release's rows are account data: a start is refused over them"
    );
    assert!(!db.adopt_owner_once(Some(&owner())).unwrap(), "the window is shut");
}

/// The lock as a restored session with its key leaves it (core's start rule, app lock off): Open, so the close of a
/// session the server ended has something to close.
fn open_lock(engine: &TestEngine) {
    *engine.lock_flow.lock().unwrap() = beebeeb_api::flows::LockFlow::at_start(beebeeb_api::flows::StartFacts {
        session_stored: true,
        ask_password_at_start: Some(false),
        vault_closed: Some(false),
        app_lock: Some(false),
    });
}

/// Batch 3 row S4 (mutant S4-M1: no `stop_proofs` in `close_session_the_server_refused`): a session the server ended
/// (a 401 a second question confirmed) stops a running proof, and its client is released before the close returns,
/// as the Lock's is.
#[test]
fn a_session_the_server_ended_stops_a_running_proof_and_its_client_is_released() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    pc.seams(|seams| seams.listing = Some(hanging(asked)));
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    listing_asked.recv_timeout(WAIT).expect("the proof is listing");
    let clients = pc.seams(|seams| seams.clients.clone());
    assert_eq!(clients.len(), 1);
    assert!(clients[0].strong_count() > 0, "the premise: the proof holds its client");
    open_lock(&pc.engine);
    let turn = pc.engine.acct.session_generation();

    let closed = pc.engine.close_session_the_server_refused(Some(turn));

    assert!(closed.is_some(), "the premise: the server's word closed the vault");
    assert_eq!(
        clients[0].strong_count(),
        0,
        "the close of the ended session released the proof's client before it returned"
    );
    pc.wait_for_outcomes(1);
    assert_eq!(pc.outcomes(), vec!["stopped"]);
    assert_eq!(pc.db().owner().unwrap(), None);
}

/// Batch 3 row S4 (mutant S4-M2: no `stop_proofs` in `prepare_replacement`): the strong unlock that replaces this
/// account's session while its key is in memory stops a running proof, and its client is released before the old
/// session leaves memory.
#[test]
fn a_replaced_session_stops_a_running_proof_and_its_client_is_released() {
    let pc = Pc::new();
    synced_rows(&pc.db());
    pc.sign_in(&owner());
    let (asked, listing_asked) = mpsc::channel();
    pc.seams(|seams| seams.listing = Some(hanging(asked)));
    pc.engine.serve_own_data_proofs();
    assert_eq!(pc.start(), Err(SyncStartRefused::OtherAccountsData));
    listing_asked.recv_timeout(WAIT).expect("the proof is listing");
    let clients = pc.seams(|seams| seams.clients.clone());
    assert_eq!(clients.len(), 1);
    assert!(clients[0].strong_count() > 0, "the premise: the proof holds its client");
    // The stored session the strong unlock replaces: this account's (its owner tag), as a sign-in stored it.
    {
        let vault = pc.engine.vault();
        vault
            .install_session(crate::keychain::SessionToken::new("an-invented-stored-token").unwrap())
            .unwrap();
        vault.bind_session_owner("user-owner").unwrap();
    }
    let profile = crate::account_dto::AccountProfile {
        user_id: "user-owner".into(),
        email: "owner@example.eu".into(),
        email_verified: true,
        created_at: "2026-11-15T00:00:00Z".into(),
        frozen_at: None,
        role: None,
        totp_enabled: None,
        is_impersonation: None,
        admin_user_id: None,
        password_counter: None,
    };
    let turn = pc.engine.acct.session_generation();

    let prepared = pc
        .engine
        .runtime
        .block_on(pc.engine.prepare_replacement(turn, &profile));

    let (next, replaced) = prepared.expect("the premise: this account's stored session is replaced");
    assert!(
        replaced.is_some(),
        "the premise: the old token is handed back to be ended"
    );
    assert_ne!(
        next, turn,
        "the premise: the unlocked path (the key left memory, the turn moved)"
    );
    assert_eq!(
        clients[0].strong_count(),
        0,
        "the replacement released the proof's client before the old session left memory"
    );
    pc.wait_for_outcomes(1);
    assert_eq!(pc.outcomes(), vec!["stopped"]);
}
