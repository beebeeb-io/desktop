//! Task 2274, plan Task 5: apply one pair (spec phase 3, I1, I2b, A5, T1, T4, T7/T8, T10-T12, T14) against a real
//! state DB and the loopback server. The platform is injected, so these run on every OS; `engine_tests` checks the
//! order's premises on the app's own apply paths (T2, T3, the snapshot prune).
use super::apply::{Local, apply_one};
use super::list::{Observed, list_pairs};
use super::model::{Action, Class, Outcome, Reason, RunFile};
use super::server::{RepairApi, server_read};
use super::server_tests::{RepairMockServer, file, get, invites, links};
use crate::api_client::ApiClient;
use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};
use crate::unknown_check::{OnDisk, PlatformView};
use std::path::{Path, PathBuf};

const A: &str = "a1";
const B: &str = "b1";
const PATH: &str = "dup.txt";
/// The app's op cursor when the tool runs.
const CURSOR: i64 = 7;
const TRASHED_AT: &str = "2026-10-29T10:00:00+00:00";

type Route = (String, u16, Vec<u8>);

/// This PC: a state DB with A and B at one path, and the out dir.
struct Pc {
    dir: tempfile::TempDir,
    db: StateDb,
    out: PathBuf,
}

impl Pc {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        row(&db, A, PATH, Some("aa"));
        row(&db, B, PATH, None);
        db.set_sync_cursor(CURSOR).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        Self { dir, db, out }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.path().join("state.db")
    }

    /// The DB backups in the out dir (each with its WAL beside it, when the DB had one).
    fn backups(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.out)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".db"))
            .collect();
        names.sort();
        names
    }

    /// Every column of a row, as SQLite holds it.
    fn columns(&self, file_id: &str) -> Vec<String> {
        let conn = rusqlite::Connection::open(self.db_path()).unwrap();
        let mut statement = conn.prepare("SELECT * FROM files WHERE file_id = ?1").unwrap();
        let count = statement.column_count();
        statement
            .query_row([file_id], |row| {
                (0..count).map(|index| row.get::<_, rusqlite::types::Value>(index).map(|value| format!("{value:?}"))).collect()
            })
            .unwrap()
    }

    async fn apply(&self, mock: &RepairMockServer, run: &mut RunFile, observed: Observed) -> anyhow::Result<Outcome> {
        let db_path = self.db_path();
        let observe = move |_: &Path| observed.clone();
        let local = Local { db: &self.db, db_path: &db_path, root: Path::new("C:/root"), out_dir: &self.out, observe: &observe };
        apply_one(&local, &api(mock), run, 1).await
    }
}

fn row(db: &StateDb, id: &str, path: &str, local_hash: Option<&str>) {
    db.upsert_file(&FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: if local_hash.is_some() { FileStatus::Local } else { FileStatus::CloudOnly },
        size_bytes: 10,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    if let Some(hash) = local_hash {
        db.record_local_baseline(id, 1, hash).unwrap();
    }
}

/// A's placeholder, in sync and wholly on disk, as the list saw it.
fn at_a() -> Observed {
    Observed {
        view: PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some(A.into()) },
        size: Some(10),
        modified: Some(7),
        read_only: Some(true),
    }
}

fn api(mock: &RepairMockServer) -> RepairApi {
    RepairApi::new(ApiClient::new(mock.base_url.clone(), "test-token".into(), [0; 32]))
}

fn found(id: &str, size: i64, trashed: bool, version: i64) -> Route {
    get(&format!("/api/v1/files/{id}"), 200, file("p", size, trashed, version))
}

/// The server read of the pair (phase 1b, and the re-check): A live with 10 bytes, B live, neither shared.
fn read(b_size: i64, b_version: i64) -> Vec<Route> {
    vec![
        found(A, 10, false, 1),
        found(B, b_size, false, b_version),
        get(&format!("/api/v1/shares/by-file/{B}"), 200, links(&[])),
        get("/api/v1/shares/invites/sent", 200, invites(&[])),
    ]
}

/// The op log after the app's cursor.
fn ops(list: &[(i64, &str, &str)]) -> Route {
    let ops: Vec<_> = list
        .iter()
        .map(|(seq, kind, id)| serde_json::json!({ "seq_id": seq, "op_type": kind, "payload": { "id": id } }))
        .collect();
    get(&format!("/api/v1/sync/ops?since={CURSOR}"), 200, serde_json::json!({ "ops": ops, "since": CURSOR }).to_string())
}

/// The trash and what the server answers after it: A live, B in the trash, B's op in the log.
fn trashed(b_size: i64, b_version: i64, log: Route) -> Vec<Route> {
    vec![
        (format!("DELETE /api/v1/files/{B}"), 200, b"{}".to_vec()),
        found(A, 10, false, 1),
        found(B, b_size, true, b_version),
        log,
    ]
}

/// The run file after list and server read, with B's size and version as the server holds them.
async fn listed(pc: &Pc, b_size: i64, b_version: i64) -> RunFile {
    let listing = list_pairs(&pc.db, Path::new("C:/root"), &|_: &Path| at_a()).unwrap();
    let mut run = RunFile { pairs: listing.pairs, more_than_two: listing.more_than_two, retention_days: None };
    let mock = RepairMockServer::start(read(b_size, b_version));
    server_read(&api(&mock), &mut run.pairs).await.unwrap();
    run
}

/// An identical pair (L12), as the compare leaves it, with the proposed action.
async fn identical(pc: &Pc) -> RunFile {
    let mut run = listed(pc, 10, 1).await;
    assert_eq!(run.pairs[0].class, Class::Unknown, "precondition: waiting for the compare");
    run.pairs[0].class = Class::Identical;
    run.pairs[0].action = Class::Identical.proposed();
    run
}

fn rows_in(db: &Path) -> i64 {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.query_row("SELECT COUNT(*) FROM files WHERE path = ?1", [PATH], |row| row.get(0)).unwrap()
}

/// T1, I1, I2b: B's row is forgotten, then B is trashed once, then A and B are read back and B's op is found. A's row
/// is unchanged in every column, and the backup taken before the forget holds both rows.
#[tokio::test]
async fn t1_an_identical_pair_forgets_b_then_trashes_it_once() {
    let pc = Pc::new();
    let mut run = identical(&pc).await;
    assert_eq!(run.pairs[0].action, Action::TrashB);
    let a_before = pc.columns(A);
    let mut routes = read(10, 1);
    routes.extend(trashed(10, 1, ops(&[(9, "file_update", "other"), (12, "file_trash", B)])));
    let mock = RepairMockServer::start(routes);

    let outcome = pc.apply(&mock, &mut run, at_a()).await.unwrap();

    assert_eq!(outcome, Outcome::Trashed { seq: 12, b_trashed_at: Some(TRASHED_AT.into()) });
    assert!(pc.db.get_file(B).unwrap().is_none(), "B's row is forgotten");
    assert_eq!(pc.columns(A), a_before, "A's row is unchanged in every column (I2b)");
    let requests = mock.requests();
    let writes: Vec<&String> = requests.iter().filter(|line| !line.starts_with("GET ")).collect();
    assert_eq!(writes, [&format!("DELETE /api/v1/files/{B}")], "one soft trash and nothing else: {requests:?}");
    let delete = requests.iter().position(|line| line.starts_with("DELETE ")).unwrap();
    assert!(requests[..delete].contains(&format!("GET /api/v1/files/{B}")), "B re-read first (A5): {requests:?}");
    assert_eq!(
        requests[delete + 1..],
        [format!("GET /api/v1/files/{A}"), format!("GET /api/v1/files/{B}"), format!("GET /api/v1/sync/ops?since={CURSOR}")],
    );
    let applied = run.pairs[0].applied.clone().expect("recorded in the run file");
    assert_eq!((applied.outcome, applied.cursor), (outcome, CURSOR));
    assert!(applied.backup.starts_with("state-1-") && applied.backup.ends_with(".db"), "{}", applied.backup);
    assert!(pc.backups().contains(&applied.backup));
    assert_eq!(rows_in(&pc.out.join(&applied.backup)), 2, "the backup was taken before the forget");

    // A pair applied once is never applied again.
    let again = RepairMockServer::start(Vec::new());
    assert_eq!(pc.apply(&again, &mut run, at_a()).await.unwrap(), Outcome::Skipped { reason: Reason::AlreadyDone });
    assert_eq!(again.requests(), Vec::<String>::new());
}

/// The trash went through but the server does not confirm it, or B's op is not in the log: recorded, and the run
/// stops for the lane to report.
#[tokio::test]
async fn a_trash_the_server_does_not_confirm_is_unexpected() {
    for (b_after, log, expected) in [
        (false, ops(&[(12, "file_trash", B)]), Outcome::Unexpected { a_live: true, b_trashed: false, seq: Some(12) }),
        (true, ops(&[]), Outcome::Unexpected { a_live: true, b_trashed: true, seq: None }),
    ] {
        let pc = Pc::new();
        let mut run = identical(&pc).await;
        let mut routes = read(10, 1);
        routes.extend(trashed(10, 1, log));
        if !b_after {
            routes[6] = found(B, 10, false, 1);
        }
        let mock = RepairMockServer::start(routes);
        let outcome = pc.apply(&mock, &mut run, at_a()).await.unwrap();
        assert_eq!(outcome, expected);
        assert_eq!(run.pairs[0].applied.as_ref().map(|applied| applied.outcome.clone()), Some(expected));
    }
}

/// T12, I1: the trash fails after B's row was forgotten. The pair stops there, recorded; `apply-one` again re-checks
/// it with B's row gone and sends the trash once more.
#[tokio::test]
async fn t12_a_failed_trash_stops_at_forgotten_and_is_retried_by_a_second_run() {
    let pc = Pc::new();
    let mut run = identical(&pc).await;
    let mut routes = read(10, 1);
    routes.push((format!("DELETE /api/v1/files/{B}"), 500, b"{}".to_vec()));
    let mock = RepairMockServer::start(routes);

    let outcome = pc.apply(&mock, &mut run, at_a()).await.unwrap();

    assert_eq!(outcome, Outcome::TrashFailed { status: Some(500) });
    assert!(pc.db.get_file(B).unwrap().is_none(), "B's row was forgotten before the trash (I1)");
    assert!(pc.db.get_file(A).unwrap().is_some());
    assert_eq!(mock.requests().last(), Some(&format!("DELETE /api/v1/files/{B}")), "nothing after the failed trash");
    assert_eq!(run.pairs[0].applied.as_ref().map(|applied| applied.outcome.clone()), Some(outcome));

    let mut routes = read(10, 1);
    routes.extend(trashed(10, 1, ops(&[(12, "file_trash", B)])));
    let mock = RepairMockServer::start(routes);
    let again = pc.apply(&mock, &mut run, at_a()).await.unwrap();
    assert_eq!(again, Outcome::Trashed { seq: 12, b_trashed_at: Some(TRASHED_AT.into()) });
    assert_eq!(mock.requests().iter().filter(|line| line.starts_with("DELETE ")).count(), 1);
    assert_eq!(pc.backups().len(), 2, "each step backs up first");
}

/// T4, A5: B changed on the server since the server read (its time or its size): skipped, nothing written.
#[tokio::test]
async fn t4_a_b_changed_on_the_server_since_the_read_is_skipped_without_a_write() {
    for change in ["updated_at", "size_bytes"] {
        let pc = Pc::new();
        let mut run = identical(&pc).await;
        let mut b: serde_json::Value = serde_json::from_str(&file("p", 10, false, 1)).unwrap();
        b[change] = if change == "updated_at" { serde_json::json!("2026-10-29T11:00:00+00:00") } else { serde_json::json!(11) };
        let mut routes = read(10, 1);
        routes[1] = get(&format!("/api/v1/files/{B}"), 200, b.to_string());
        let mock = RepairMockServer::start(routes);

        let outcome = pc.apply(&mock, &mut run, at_a()).await.unwrap();

        assert_eq!(outcome, Outcome::Skipped { reason: Reason::ChangedOnServer }, "{change}");
        assert!(pc.db.get_file(B).unwrap().is_some(), "{change}: B's row kept");
        assert!(mock.requests().iter().all(|line| line.starts_with("GET ")), "{change}: {:?}", mock.requests());
        assert_eq!(pc.backups(), Vec::<String>::new(), "{change}: nothing was about to change");
        assert_eq!(run.pairs[0].applied, None);
    }
}

/// A5: the file at the path, or the rows, differ from the list: skipped before any call.
#[tokio::test]
async fn a_pair_that_changed_on_this_pc_is_skipped_before_any_call() {
    let mut touched = at_a();
    touched.modified = Some(8);
    let mut freed = at_a();
    freed.view = PlatformView::Placeholder { on_disk: OnDisk::Nothing, in_sync: true, identity: Some(A.into()) };
    for (case, observed) in [("modified", touched), ("freed", freed)] {
        let pc = Pc::new();
        let mut run = identical(&pc).await;
        let mock = RepairMockServer::start(Vec::new());
        let outcome = pc.apply(&mock, &mut run, observed).await.unwrap();
        assert_eq!(outcome, Outcome::Skipped { reason: Reason::ChangedOnThisPc }, "{case}");
        assert_eq!(mock.requests(), Vec::<String>::new(), "{case}");
        assert!(pc.db.get_file(B).unwrap().is_some(), "{case}");
    }

    // Work queued for B since the list (L3b), and a third row at the path (L0).
    let pc = Pc::new();
    let mut run = identical(&pc).await;
    pc.db
        .enqueue_operation(&PendingOperation {
            op_id: "op".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some(B.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();
    let mock = RepairMockServer::start(Vec::new());
    assert_eq!(pc.apply(&mock, &mut run, at_a()).await.unwrap(), Outcome::Skipped { reason: Reason::ChangedOnThisPc });
    let pc = Pc::new();
    let mut run = identical(&pc).await;
    row(&pc.db, "c1", PATH, None);
    assert_eq!(pc.apply(&mock, &mut run, at_a()).await.unwrap(), Outcome::Skipped { reason: Reason::ChangedOnThisPc });
    assert_eq!(mock.requests(), Vec::<String>::new());
    assert_eq!(pc.backups(), Vec::<String>::new());
}

/// T7/T8: a pair left to the person (L8, L9) is not touched while it is marked "none", and is trashed as T1 once the
/// person marks it.
#[tokio::test]
async fn t7_t8_a_pair_left_to_the_person_is_trashed_only_when_marked() {
    for (class, b_size, b_version) in [(Class::DiffersBySize, 12, 1), (Class::YourCall, 10, 2)] {
        let pc = Pc::new();
        let mut run = listed(&pc, b_size, b_version).await;
        assert_eq!(run.pairs[0].class, class, "precondition");
        assert_eq!(run.pairs[0].action, Action::None, "{class:?}: the tool proposes nothing");
        let idle = RepairMockServer::start(Vec::new());
        assert_eq!(pc.apply(&idle, &mut run, at_a()).await.unwrap(), Outcome::Skipped { reason: Reason::NotMarked });
        assert_eq!(idle.requests(), Vec::<String>::new());
        assert!(pc.db.get_file(B).unwrap().is_some());

        run.pairs[0].action = Action::TrashB;
        let mut routes = read(b_size, b_version);
        routes.extend(trashed(b_size, b_version, ops(&[(12, "file_trash", B)])));
        let mock = RepairMockServer::start(routes);
        let outcome = pc.apply(&mock, &mut run, at_a()).await.unwrap();
        assert_eq!(outcome, Outcome::Trashed { seq: 12, b_trashed_at: Some(TRASHED_AT.into()) }, "{class:?}");
        assert!(pc.db.get_file(B).unwrap().is_none());
    }
}

/// T10 (L5, L6): B already in the trash, or gone: B's row is forgotten, and nothing but reads reaches the server.
#[tokio::test]
async fn t10_a_b_in_the_trash_or_gone_is_forgotten_without_a_trash() {
    for (class, b_route) in [(Class::BInTrash, Some(found(B, 10, true, 1))), (Class::BGone, None)] {
        let pc = Pc::new();
        let routes: Vec<Route> = std::iter::once(found(A, 10, false, 1)).chain(b_route).collect();
        let listing = list_pairs(&pc.db, Path::new("C:/root"), &|_: &Path| at_a()).unwrap();
        let mut run = RunFile { pairs: listing.pairs, more_than_two: 0, retention_days: None };
        server_read(&api(&RepairMockServer::start(routes.clone())), &mut run.pairs).await.unwrap();
        assert_eq!(run.pairs[0].class, class, "precondition");
        run.pairs[0].action = class.proposed();
        assert_eq!(run.pairs[0].action, Action::ForgetB);
        let a_before = pc.columns(A);
        let mock = RepairMockServer::start(routes);

        let outcome = pc.apply(&mock, &mut run, at_a()).await.unwrap();

        assert_eq!(outcome, Outcome::Forgotten, "{class:?}");
        assert!(pc.db.get_file(B).unwrap().is_none(), "{class:?}");
        assert_eq!(pc.columns(A), a_before, "{class:?}");
        assert!(mock.requests().iter().all(|line| line.starts_with("GET ")), "{class:?}: {:?}", mock.requests());
        assert_eq!(pc.backups().len(), 1, "{class:?}: backed up first");
    }
}

/// T11: a run file with a pair number twice, missing or out of order, a mark its class does not allow, or a pair it
/// does not hold is refused whole, before any read or write. An unknown action does not even load, and the error does
/// not repeat what the file holds (I5).
#[tokio::test]
async fn t11_a_run_file_marked_wrong_is_refused_before_any_write() {
    let pc = Pc::new();
    let run = identical(&pc).await;
    let mut twice = run.clone();
    twice.pairs.push(run.pairs[0].clone());
    let mut skipped = run.clone();
    skipped.pairs[0].n = 2;
    let mut not_allowed = run.clone();
    not_allowed.pairs[0].class = Class::AGone;
    let mut forget_live = run.clone();
    forget_live.pairs[0].action = Action::ForgetB;
    for (case, mut bad) in [("twice", twice), ("skipped", skipped), ("not allowed", not_allowed), ("forget a live B", forget_live)] {
        let mock = RepairMockServer::start(Vec::new());
        assert!(pc.apply(&mock, &mut bad, at_a()).await.is_err(), "{case}");
        assert_eq!(mock.requests(), Vec::<String>::new(), "{case}");
        assert_eq!(bad.pairs[0].applied, None, "{case}");
    }
    let mock = RepairMockServer::start(Vec::new());
    let db_path = pc.db_path();
    let observe = |_: &Path| at_a();
    let local = Local { db: &pc.db, db_path: &db_path, root: Path::new("C:/root"), out_dir: &pc.out, observe: &observe };
    assert!(apply_one(&local, &api(&mock), &mut run.clone(), 2).await.is_err(), "a pair the file does not hold");
    assert_eq!(mock.requests(), Vec::<String>::new());
    assert_eq!(pc.backups(), Vec::<String>::new());
    assert!(pc.db.get_file(B).unwrap().is_some());

    let path = pc.out.join("run.json");
    run.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap().replace("\"trash-b\"", "\"trash-everything\"");
    std::fs::write(&path, text).unwrap();
    let error = RunFile::load(&path).unwrap_err().to_string();
    assert!(error.contains("line") && error.contains("trash-b"), "{error}");
    assert!(!error.contains("trash-everything") && !error.contains(PATH) && !error.contains(B), "{error}");
}

/// The code lines of a source file: comments say what the server is, the code must not call it.
fn code(source: &str) -> String {
    source.lines().filter(|line| !line.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

/// The identifiers that follow `prefix` in `text`.
fn after(text: &str, prefix: &str) -> Vec<String> {
    text.match_indices(prefix)
        .map(|(at, _)| {
            text[at + prefix.len()..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>()
        })
        .collect()
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

/// T14 (I3, amended twice): the tool reaches the server only through `RepairApi`, and `RepairApi` only through six
/// `ApiClient` calls: GET file, GET chunk, GET `/sync/ops?since=`, GET `/shares/by-file/{id}`, GET
/// `/shares/invites/sent` and the soft DELETE. Any other call, another sync-ops or shares variant, a POST or
/// `/permanent` fails this test.
#[test]
fn t14_the_tool_reaches_the_server_through_exactly_six_calls() {
    let client = include_str!("../api_client.rs");
    assert_eq!(client.matches("#[cfg(feature = \"duplicate-repair\")]").count(), 1, "one block of the tool's calls");
    let start = client.find("#[cfg(feature = \"duplicate-repair\")]\nimpl ApiClient {").expect("the tool's calls");
    let block = code(&client[start..start + client[start..].find("\n}\n").unwrap()]);
    assert_eq!(
        sorted(after(&block, "fn ")),
        ["repair_chunk", "repair_file", "repair_invites", "repair_links", "repair_ops", "repair_trash"],
    );
    assert_eq!(
        sorted(after(&block, "\"/api/v1/")),
        ["files", "shares", "sync"],
        "the paths: files/{{id}}, shares/by-file/{{id}}, shares/invites/sent, sync/ops?since="
    );
    for path in ["\"/api/v1/files/{}\"", "\"/api/v1/shares/by-file/{}\"", "\"/api/v1/shares/invites/sent\"", "\"/api/v1/sync/ops?since={since}\""] {
        assert!(block.contains(path), "{path}");
    }
    assert_eq!(block.matches("\"/api/v1/").count(), 5, "file, links, invites, ops and the trash: {block}");
    assert_eq!(block.matches("self.get_typed(").count(), 4, "four JSON reads");
    assert_eq!(sorted(after(&block, "self.client.")), ["delete", "get"], "GET a chunk, DELETE the trash");
    assert_eq!(block.matches("self.client.").count(), 2);
    assert_eq!(block.matches("self.chunk_url(").count(), 1);
    for forbidden in ["permanent", "restore", ".post(", ".put(", ".patch(", "self.request("] {
        assert!(!block.contains(forbidden), "{forbidden}");
    }

    let server = code(include_str!("server.rs"));
    assert_eq!(
        sorted(after(&server, "self.api.")),
        ["master_key", "repair_chunk", "repair_file", "repair_invites", "repair_links", "repair_ops", "repair_trash"],
        "`master_key` derives a file key: no call"
    );
    let start = server.find("impl RepairApi {").unwrap();
    let methods = &server[start..start + server[start..].find("\n}\n").unwrap()];
    assert_eq!(
        sorted(after(methods, "fn ")),
        ["chunk", "file", "file_key", "live_invites", "live_links", "new", "trash", "trash_seq"],
    );

    // Every other file of the module, and the host, reaches the server only through `RepairApi`.
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("model.rs", include_str!("model.rs")),
        ("list.rs", include_str!("list.rs")),
        ("compare.rs", include_str!("compare.rs")),
        ("apply.rs", include_str!("apply.rs")),
        ("echo.rs", include_str!("echo.rs")),
        ("host.rs", include_str!("host.rs")),
        ("examples/duplicate_repair.rs", include_str!("../../examples/duplicate_repair.rs")),
    ];
    for (name, source) in sources {
        let source = code(source);
        let built = source.matches("RepairApi::new(crate::api_client::ApiClient::new(").count();
        assert_eq!(source.matches("ApiClient").count(), built, "{name}: an ApiClient is only built into a RepairApi");
        for forbidden in ["api_client::", "EngineBridge", "reqwest", "\"/api/"] {
            let allowed = if forbidden == "api_client::" { built } else { 0 };
            assert_eq!(source.matches(forbidden).count(), allowed, "{name}: {forbidden}");
        }
    }
    // Every non-test source file of the module is in the list above.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/duplicate_repair");
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.ends_with("tests.rs"))
        .collect();
    files.sort();
    assert_eq!(files, ["apply.rs", "compare.rs", "echo.rs", "host.rs", "list.rs", "mod.rs", "model.rs", "server.rs"]);
}
