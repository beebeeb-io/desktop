//! Task 2274, plan Task 7: the host, every phase on a scratch PC and the loopback server. T13: no name, path or id
//! reaches the screen, an error or a log line, while the run file and the list on this PC hold them.
use super::Command;
use super::host::{DO_NOT_RESTORE, Env, FOLDER_SHARES, LIST_FILE, RUN_FILE, execute, list};
use super::list::Observed;
use super::model::{Applied, Outcome, RunFile};
use super::server::RepairApi;
use super::server_tests::{RepairMockServer, file, get, invites, links};
use crate::api_client::ApiClient;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
use crate::unknown_check::{OnDisk, PlatformView};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const A: &str = "aaaa1111-2222-4333-8444-555566667777";
const B: &str = "bbbb1111-2222-4333-8444-555566667777";
const PARENT: &str = "pppp1111-2222-4333-8444-555566667777";
const FOLDER: &str = "Belastingaangifte 2026";
const NAME: &str = "Geheim plan.docx";
const ROOT_NAME: &str = "Sync root Q7-marker";
const TOKEN: &str = "tok-s3cret-0001";
const MASTER: [u8; 32] = [7; 32];
const CONTENT: &[u8] = b"the same bytes";
const CURSOR: i64 = 7;
const SEQ: i64 = 12;

type Route = (String, u16, Vec<u8>);

/// A scratch PC: the app's state dir with A and B at one path, a sync root, an out dir.
struct Pc {
    _dir: tempfile::TempDir,
    db_path: PathBuf,
    root: PathBuf,
    out: PathBuf,
}

impl Pc {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join(ROOT_NAME);
        let out = dir.path().join("out");
        for path in [&state, &root, &out] {
            std::fs::create_dir_all(path).unwrap();
        }
        let db_path = state.join("state.db");
        let db = StateDb::open(&db_path).unwrap();
        row(&db, A, Some(&crate::engine_bridge::sha256_hex(CONTENT)));
        row(&db, B, None);
        db.set_sync_cursor(CURSOR).unwrap();
        Self { _dir: dir, db_path, root, out }
    }

    fn path() -> String {
        format!("{FOLDER}/{NAME}")
    }

    /// Run one phase with the app stopped (the lock is granted) against `base_url`.
    async fn run(&self, command: Command, base_url: &str) -> anyhow::Result<(i32, String)> {
        let base_url = base_url.to_string();
        let api = move || Ok(RepairApi::new(ApiClient::new(base_url.clone(), TOKEN.into(), MASTER)));
        let lock = || Ok(Box::new(()) as Box<dyn std::any::Any>);
        self.run_with(command, &api, &lock).await
    }

    async fn run_with(
        &self,
        command: Command,
        api: &dyn Fn() -> anyhow::Result<RepairApi>,
        lock: &dyn Fn() -> anyhow::Result<Box<dyn std::any::Any>>,
    ) -> anyhow::Result<(i32, String)> {
        let observe = |_: &Path| at_a();
        let env = Env {
            db_path: &self.db_path,
            root: &self.root,
            out_dir: &self.out,
            observe: &observe,
            lock,
            api,
            echo_wait: Duration::from_secs(5),
            echo_poll: Duration::from_millis(5),
        };
        execute(command, &env).await
    }

    fn out_file(&self, name: &str) -> String {
        std::fs::read_to_string(self.out.join(name)).unwrap()
    }
}

fn row(db: &StateDb, id: &str, local_hash: Option<&str>) {
    db.upsert_file(&FileEntry {
        file_id: id.into(),
        path: Pc::path(),
        status: if local_hash.is_some() { FileStatus::Local } else { FileStatus::CloudOnly },
        size_bytes: CONTENT.len() as i64,
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

fn at_a() -> Observed {
    Observed {
        view: PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some(A.into()) },
        size: Some(CONTENT.len() as u64),
        modified: Some(7),
        read_only: Some(true),
    }
}

fn found(id: &str, trashed: bool) -> Route {
    get(&format!("/api/v1/files/{id}"), 200, file(PARENT, CONTENT.len() as i64, trashed, 1))
}

/// Every answer the whole run needs, in the order each request is repeated: the server read, the compare (B again and
/// its one chunk), `apply-one`'s re-check, the trash, A and B read back, and the op log.
fn server() -> RepairMockServer {
    let key = beebeeb_core::kdf::derive_file_key(&beebeeb_core::kdf::MasterKey::from_bytes(MASTER), B.as_bytes());
    let chunk = beebeeb_core::encrypt::encrypt_chunk_raw(&key, CONTENT).unwrap();
    let ops = serde_json::json!({ "ops": [{ "seq_id": SEQ, "op_type": "file_trash", "payload": { "id": B } }], "since": CURSOR });
    RepairMockServer::start(vec![
        found(A, false),
        found(A, false),
        found(A, false),
        found(B, false),
        found(B, false),
        found(B, false),
        found(B, true),
        get(&format!("/api/v1/shares/by-file/{B}"), 200, links(&[])),
        get("/api/v1/shares/invites/sent", 200, invites(&[(B, "revoked", None)])),
        (format!("GET /api/v1/files/{B}/chunks/0"), 200, chunk),
        (format!("DELETE /api/v1/files/{B}"), 200, b"{}".to_vec()),
        get(&format!("/api/v1/sync/ops?since={CURSOR}"), 200, ops.to_string()),
    ])
}

const PROBE: &str = "t13 probe event";

/// One TRACE event from one callsite, as any code the tool runs could log.
fn probe() {
    tracing::trace!("{PROBE}");
}

/// A writer the test's tracing subscriber appends to.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// T13 (spec A3, I5): every phase, end to end, and two refusals. The screen, the errors and every log line at any
/// level carry none of the names, paths, ids or the token; the run file and the list on this PC carry them.
#[test]
fn t13_no_name_path_or_id_reaches_the_screen_an_error_or_a_log() {
    let pc = Pc::new();
    let mock = server();
    let logs = Captured::default();
    let writer = logs.clone();
    let capture = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish(),
    );
    // tracing caches each callsite's interest for every thread. While one dispatcher is registered, a callsite first
    // hit on another thread (a test running alongside, with no subscriber there) is cached as never, and this test's
    // capture would miss its events: so a second dispatcher stays registered for the whole test, and the probe, first
    // hit on a thread without a subscriber, must still reach the capture.
    let _second = tracing::Dispatch::new(tracing_subscriber::registry());
    std::thread::spawn(probe).join().unwrap();
    let mut seen: Vec<String> = Vec::new();
    tracing::dispatcher::with_default(&capture, || {
        probe();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            for command in [
                Command::List,
                Command::Retention { days: 30 },
                Command::ServerRead,
                Command::Compare { download_ok: 1 },
            ] {
                let (code, screen) = pc.run(command, &mock.base_url).await.unwrap();
                assert_eq!(code, 0, "{command:?}: {screen}");
                seen.push(screen);
            }
            let mut run = RunFile::load(&pc.out.join(RUN_FILE)).unwrap();
            assert_eq!(super::model::kebab(&run.pairs[0].class), "identical", "the compare found the same bytes");
            run.pairs[0].action = super::model::Action::TrashB;
            run.save(&pc.out.join(RUN_FILE)).unwrap();
            let (code, screen) = pc.run(Command::ApplyOne { pair: 1 }, &mock.base_url).await.unwrap();
            assert_eq!(code, 0, "{screen}");
            seen.push(screen);
            // The app starts and applies B's trash.
            StateDb::open(&pc.db_path).unwrap().set_sync_cursor(SEQ).unwrap();
            let (code, screen) = pc.run(Command::CheckEcho { pair: 1 }, &mock.base_url).await.unwrap();
            assert_eq!(code, 0, "{screen}");
            seen.push(screen);
            for refused in [Command::ApplyOne { pair: 5 }, Command::List] {
                seen.push(pc.run(refused, &mock.base_url).await.unwrap_err().to_string());
            }
        });
    });
    let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains(PROBE), "the capture saw the probe's TRACE event:\n{logs}");
    let screen = seen.join("\n");
    let secrets = [A, B, PARENT, FOLDER, NAME, ROOT_NAME, TOKEN, &pc.root.display().to_string()];
    for secret in secrets {
        assert!(!screen.contains(secret), "the screen or an error names {secret:?}:\n{screen}");
        assert!(!logs.contains(secret), "a log line names {secret:?}:\n{logs}");
    }
    assert!(screen.contains("Listed 1 pairs"), "{screen}");
    assert!(screen.contains("pair 1: B's row forgotten on this PC, B moved to Beebeeb's trash (its trash op is 12)"), "{screen}");
    assert!(screen.contains("pair 1: the app applied the echo"), "{screen}");
    // On this PC, the list and the run file hold what the person needs: the path, both ids, B's trash time.
    let (list, run) = (pc.out_file(LIST_FILE), pc.out_file(RUN_FILE));
    for wanted in [A, B, NAME, FOLDER] {
        assert!(list.contains(wanted) && run.contains(wanted), "the out dir holds {wanted:?}");
    }
    assert!(list.contains("B in Beebeeb's trash since 2026-10-29T10:00:00+00:00 (trash op 12); purged after 2026-11-28"), "{list}");
    assert!(!run.contains(TOKEN) && !list.contains(TOKEN), "the token is never written");
    assert_eq!(mock.requests().iter().filter(|request| request.starts_with("DELETE")).count(), 1, "one trash");
}

/// T15: every phase but `check-echo` claims the app's lock first; refused, it reads nothing, calls nothing and writes
/// nothing.
#[tokio::test]
async fn every_phase_but_check_echo_needs_the_app_stopped() {
    let pc = Pc::new();
    let mock = RepairMockServer::start(vec![]);
    let base_url = mock.base_url.clone();
    let api = move || Ok(RepairApi::new(ApiClient::new(base_url.clone(), TOKEN.into(), MASTER)));
    let refused = || Err(anyhow::anyhow!(super::APP_RUNNING));
    for command in
        [Command::List, Command::ServerRead, Command::Compare { download_ok: 1 }, Command::ApplyOne { pair: 1 }]
    {
        let error = pc.run_with(command, &api, &refused).await.unwrap_err().to_string();
        assert_eq!(error, super::APP_RUNNING, "{command:?}");
    }
    assert!(mock.requests().is_empty(), "no call: {:?}", mock.requests());
    assert_eq!(std::fs::read_dir(&pc.out).unwrap().count(), 0, "nothing written to the out dir");
}

/// T15's other side: `check-echo` runs while the app runs (it needs the app to apply the echo), so it claims no lock.
#[tokio::test]
async fn check_echo_runs_while_the_app_runs() {
    let pc = Pc::new();
    pc.run(Command::List, "http://127.0.0.1:9").await.unwrap();
    let mut run = RunFile::load(&pc.out.join(RUN_FILE)).unwrap();
    run.pairs[0].applied = Some(Applied { outcome: Outcome::Forgotten, cursor: CURSOR, backup: "b".into(), at: 0 });
    run.save(&pc.out.join(RUN_FILE)).unwrap();
    StateDb::open(&pc.db_path).unwrap().delete_file(B).unwrap();
    let api = || -> anyhow::Result<RepairApi> { anyhow::bail!("check-echo calls no server") };
    let refused = || Err(anyhow::anyhow!(super::APP_RUNNING));
    let (code, screen) = pc.run_with(Command::CheckEcho { pair: 1 }, &api, &refused).await.unwrap();
    assert_eq!((code, screen.contains("the app applied the echo")), (0, true), "{screen}");
}

/// `list` never replaces a run file: it holds what was done and B's ids for a restore (L2).
#[tokio::test]
async fn list_never_replaces_a_run_file() {
    let pc = Pc::new();
    pc.run(Command::List, "http://127.0.0.1:9").await.unwrap();
    let before = pc.out_file(RUN_FILE);
    let error = pc.run(Command::List, "http://127.0.0.1:9").await.unwrap_err().to_string();
    assert!(error.contains("A run file is in the out dir already"), "{error}");
    assert_eq!(pc.out_file(RUN_FILE), before);
}

/// OP3: `apply-one` refuses until the trash retention is recorded, before it reads the server or opens the DB.
#[tokio::test]
async fn apply_one_needs_the_retention_first() {
    let pc = Pc::new();
    let mock = RepairMockServer::start(vec![]);
    pc.run(Command::List, &mock.base_url).await.unwrap();
    let error = pc.run(Command::ApplyOne { pair: 1 }, &mock.base_url).await.unwrap_err().to_string();
    assert!(error.contains("record it with `retention <days>` first"), "{error}");
    assert!(mock.requests().is_empty());
}

/// L1: a build whose own state DB code changes the schema of a copy of the app's DB stops before the app's DB is opened.
/// Here the app's DB lacks an index this build creates, as a DB from an older app would.
#[tokio::test]
async fn a_build_that_changes_the_schema_is_refused() {
    let pc = Pc::new();
    let mock = RepairMockServer::start(vec![]);
    let conn = rusqlite::Connection::open(&pc.db_path).unwrap();
    conn.execute_batch("DROP INDEX idx_status").unwrap();
    drop(conn);
    let before = std::fs::read(&pc.db_path).unwrap();
    let error = pc.run(Command::List, &mock.base_url).await.unwrap_err().to_string();
    assert!(error.contains("does not match the app's state database"), "{error}");
    assert_eq!(std::fs::read(&pc.db_path).unwrap(), before, "the app's DB is untouched");
    assert!(!pc.out.join(RUN_FILE).exists());
}

/// L1 again at `apply-one`, the phase that writes the app's DB: the guard runs before the DB is opened or the server
/// is called, whatever was listed before.
#[tokio::test]
async fn apply_one_refuses_a_build_that_changes_the_schema() {
    let pc = Pc::new();
    let mock = RepairMockServer::start(vec![]);
    pc.run(Command::List, &mock.base_url).await.unwrap();
    pc.run(Command::Retention { days: 30 }, &mock.base_url).await.unwrap();
    let conn = rusqlite::Connection::open(&pc.db_path).unwrap();
    conn.execute_batch("DROP INDEX idx_status").unwrap();
    drop(conn);
    let before = std::fs::read(&pc.db_path).unwrap();
    let error = pc.run(Command::ApplyOne { pair: 1 }, &mock.base_url).await.unwrap_err().to_string();
    assert!(error.contains("does not match the app's state database"), "{error}");
    assert_eq!(std::fs::read(&pc.db_path).unwrap(), before, "the app's DB is untouched");
    assert!(mock.requests().is_empty(), "no call: {:?}", mock.requests());
}

/// The list's header carries the L8 sentence, the OP5 warning and the retention; a trashed B carries its purge date.
#[test]
fn the_list_says_what_was_not_checked_and_when_b_is_purged() {
    let pc = Pc::new();
    let db = StateDb::open(&pc.db_path).unwrap();
    let listing = super::list::list_pairs(&db, &pc.root, &|_: &Path| at_a()).unwrap();
    let mut run = RunFile { pairs: listing.pairs, more_than_two: 2, retention_days: None };
    run.pairs[0].applied = Some(Applied {
        outcome: Outcome::Trashed { seq: SEQ, b_trashed_at: Some("2026-10-29T10:00:00+00:00".into()) },
        cursor: CURSOR,
        backup: "state-1-0.db".into(),
        at: 0,
    });
    let text = list(&run);
    for line in [FOLDER_SHARES, DO_NOT_RESTORE, "Trash retention: not recorded yet", "(counted, not listed): 2"] {
        assert!(text.contains(line), "{line}:\n{text}");
    }
    assert!(text.contains("purged after the retention (not recorded)"), "{text}");
    run.retention_days = Some(30);
    let text = list(&run);
    assert!(text.contains("Trash retention: 30 days"), "{text}");
    assert!(text.contains("purged after 2026-11-28"), "{text}");
}

/// T13's other half (spec I5): the tool's one print is the screen `execute` returns, and the example prints only the
/// error's own text. The module and the app code the tool runs (state DB, API client, lock, settings, state paths,
/// placeholders) print nothing themselves; their tracing goes nowhere, since the host installs no subscriber.
#[test]
fn the_tool_prints_only_its_screen() {
    // Every macro call whose whole name is a print (`eprintln!(` is one call, not also a `println!(`).
    let prints = |source: &str| {
        let name = |at: usize| {
            let start = source[..at].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
            &source[start..at]
        };
        let names = ["println", "eprintln", "print", "eprint", "dbg"];
        source.match_indices("!(").filter(|(at, _)| names.contains(&name(*at))).count()
    };
    let module = [
        ("apply.rs", include_str!("apply.rs")),
        ("compare.rs", include_str!("compare.rs")),
        ("echo.rs", include_str!("echo.rs")),
        ("host.rs", include_str!("host.rs")),
        ("list.rs", include_str!("list.rs")),
        ("model.rs", include_str!("model.rs")),
        ("server.rs", include_str!("server.rs")),
        ("state_db.rs", include_str!("../state_db.rs")),
        ("api_client.rs", include_str!("../api_client.rs")),
        ("lockfile.rs", include_str!("../lockfile.rs")),
        ("config.rs", include_str!("../config.rs")),
        ("state_paths.rs", include_str!("../state_paths.rs")),
        ("placeholders.rs", include_str!("../windows_cf/placeholders.rs")),
    ];
    for (name, source) in module {
        assert_eq!(prints(source), 0, "{name} prints");
    }
    let host = include_str!("mod.rs");
    assert_eq!(prints(host), 1, "mod.rs prints once");
    assert_eq!(host.matches("println!(\"{screen}\")").count(), 1, "and that print is the screen");
    let example = include_str!("../../examples/duplicate_repair.rs");
    assert_eq!(prints(example), 1, "the example prints once");
    assert_eq!(example.matches("eprintln!(\"{error}\")").count(), 1, "and that print is the error's text");
    for source in [host, example] {
        assert!(!source.contains("tracing_subscriber"), "no subscriber is installed");
    }
}
