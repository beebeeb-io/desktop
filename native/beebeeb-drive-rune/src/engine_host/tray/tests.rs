//! The flyout on the real engine (`EngineTraySource`): the host's fold of the engine's reports, what the
//! state database says, and real engine handles from signed out to locked, on credential stores in memory
//! and the engine's fake API on loopback. An engine handle is on the process's state folder (the engine
//! refuses another), which `engine_host::tests` writes: nothing here writes in it. The database tests use
//! a database of their own. Every name is invented.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::engine_status::{Activity, Reason, plain_status};
use beebeeb_engine::host::{ChannelHost, EngineEvent, EngineStatus, HostMessage, PauseChanged, PauseState};
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileEntry, ItemKind, LocalActivityEventInput, TransferActivityInput};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, start_fake};
use beebeeb_engine::unknown_check::StartupGuardProgress;
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::open::{self, Opened};
use crate::frames::tray::source::{BoardFixture, Changes, FRAMES, Hidden, Status, What};
use crate::shell::actions::EngineActions;

const EMAIL: &str = "tray-lane@example.eu";
const PASSWORD: &str = "an invented password for the tray";

/// An engine handle of its own, signed out: credential stores of its own in memory, the API at `api_base`.
fn engine_at(api_base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "tray-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api_base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

/// A signed-out engine whose API is the test build's closed loopback port: it reaches nothing.
fn offline_engine() -> Arc<Engine<MemoryCredentials>> {
    engine_at(&open::api_base(None, true))
}

/// The host's side of `engine` (`LiveEngine`), with a queue the test sends the engine's reports into.
fn live_on(engine: &Arc<Engine<MemoryCredentials>>) -> (Arc<LiveEngine<MemoryCredentials>>, mpsc::Sender<HostMessage>) {
    let (reports, messages) = mpsc::channel();
    let opened = Opened {
        engine: engine.clone(),
        identity: &TEST,
        gate: engine.gate(),
    };
    (Arc::new(LiveEngine::new(&opened, messages)), reports)
}

fn report(status: beebeeb_engine::engine_status::StatusPayload) -> HostMessage {
    HostMessage::Event(EngineEvent::Status(EngineStatus {
        status,
        startup_guard: StartupGuardProgress::default(),
    }))
}

fn plain(state: &str) -> HostMessage {
    report(plain_status(state, None, None))
}

fn syncing(files: u32, total: u64, done: u64) -> HostMessage {
    let mut status = plain_status("syncing", None, None);
    status.activity = Some(Activity {
        files_remaining: files,
        bytes_total: total,
        bytes_done: done,
    });
    report(status)
}

fn error(code: &'static str) -> HostMessage {
    let mut status = plain_status("error", None, None);
    status.reason = Some(Reason { code, detail: None });
    report(status)
}

fn pause(state: PauseState) -> HostMessage {
    HostMessage::Event(EngineEvent::PauseChanged(PauseChanged { state }))
}

/// Every report the engine sends moves the one view the flyout and the b read; the resume switch is no
/// report (never "idle"), and every other message leaves the view as it was.
#[test]
fn each_report_of_the_engine_moves_the_view_the_flyout_and_the_b_read() {
    let engine = offline_engine();
    let (live, reports) = live_on(&engine);
    assert_eq!(
        report_of(&live.status_view()),
        Report::Unknown,
        "before the first report"
    );
    let steps = [
        (plain("running"), "running", Report::Starting, EngineState::Idle),
        (
            syncing(3, 4_000, 1_000),
            "syncing",
            Report::Syncing,
            EngineState::Syncing,
        ),
        (plain("idle"), "idle", Report::Idle, EngineState::Idle),
        (pause(PauseState::Paused), "paused", Report::Quiet, EngineState::Idle),
        (pause(PauseState::Idle), "", Report::Unknown, EngineState::Idle),
        (plain("offline"), "offline", Report::Offline, EngineState::Offline),
        (error("timeout"), "error", Report::ServerError, EngineState::Offline),
        (
            error(beebeeb_engine::engine_status::REASON_ENGINE_ERROR),
            "error",
            Report::Failed,
            EngineState::Error,
        ),
        (
            error(beebeeb_engine::engine_status::REASON_AUTH),
            "error",
            Report::Unknown,
            EngineState::Idle,
        ),
    ];
    for (message, state, expected, b) in steps {
        reports.send(message).unwrap();
        assert_eq!(live.drain(), 1, "one message taken");
        let view = live.status_view();
        assert_eq!(
            (view.state.as_str(), report_of(&view), engine_state(&view)),
            (state, expected, b),
            "after {state:?}"
        );
    }
    // A sync's numbers, and none of them after the next report.
    reports.send(syncing(3, 4_000, 1_000)).unwrap();
    live.drain();
    assert_eq!(
        progress_of(&live.status_view()),
        Progress {
            files_remaining: Some(3),
            bytes_total: Some(4_000),
            bytes_done: Some(1_000),
        }
    );
    reports.send(plain("idle")).unwrap();
    live.drain();
    assert_eq!(progress_of(&live.status_view()), Progress::default());
    // Any other message is dropped.
    reports
        .send(HostMessage::Event(EngineEvent::KeptFolderChanged))
        .unwrap();
    assert_eq!(live.drain(), 1);
    assert_eq!(live.status_view().state, "idle");
    assert_eq!(live.drain(), 0, "the queue is empty");
}

/// The lead's rule for the fold: a drain from the flyout moves what the main window reads, and the main
/// window's drain (its session, read every frame) moves what the flyout reads.
#[test]
fn a_drain_from_the_flyout_moves_what_the_main_window_reads_and_back() {
    let engine = offline_engine();
    let (live, reports) = live_on(&engine);
    let tray = EngineTraySource::new(live.clone());

    reports.send(plain("offline")).unwrap();
    assert_eq!(tray.facts().report, Report::Offline, "the flyout read it");
    assert_eq!(live.drain(), 0, "the flyout's read took it from the queue");
    assert_eq!(
        live.status_view().state,
        "offline",
        "the main window reads the same report"
    );

    reports.send(plain("idle")).unwrap();
    let _ = live.session();
    assert_eq!(live.drain(), 0, "the main window's read took it");
    assert_eq!(
        tray.facts().report,
        Report::Idle,
        "the flyout reads what the main window took"
    );
}

/// What a facts read reports, with the flyout's words: every text a person sees.
fn texts(snapshot: &Snapshot) -> Vec<String> {
    let name = AppIdentity::CURRENT.product_name();
    let mut out = vec![snapshot.status.title(name)];
    out.extend(snapshot.status.line(name));
    out.extend(snapshot.status.code().map(str::to_string));
    out.extend(snapshot.button.map(|(action, _)| action.label().to_string()));
    match &snapshot.changes {
        Changes::Hidden(why) => out.extend([why.title().to_string(), why.sentence(name)]),
        Changes::Empty { text } => out.push(text.clone()),
        Changes::Rows { rows, .. } => {
            for row in rows {
                out.push(row.name.clone());
                out.push(match &row.what {
                    What::Done { text, .. } | What::Moving { text, .. } | What::Failed { text } => text.clone(),
                });
            }
        }
    }
    out
}

/// The board's sample data: every row of every flyout frame, and every status line that carries a
/// number (counts, sizes, a version, a code). The copy around them is the running app's too.
fn sample_data() -> Vec<String> {
    let name = AppIdentity::CURRENT.product_name();
    let mut data = Vec::new();
    for frame in FRAMES {
        let snapshot = BoardFixture::frame(frame).expect("a flyout frame").snapshot();
        let code = snapshot.status.code().map(str::to_string);
        let speed = match &snapshot.status {
            Status::Syncing { speed, .. } => speed.clone(),
            _ => None,
        };
        for text in [
            Some(snapshot.status.title(name)),
            snapshot.status.line(name),
            code,
            speed,
        ]
        .into_iter()
        .flatten()
        .filter(|text| text.chars().any(|c| c.is_ascii_digit()))
        {
            data.push(text);
        }
        if let Changes::Rows { rows, .. } = &snapshot.changes {
            for row in rows {
                data.push(row.name.clone());
                if let What::Done { text, .. } | What::Moving { text, .. } | What::Failed { text } = &row.what
                    && text != "Moved to Trash on this PC"
                {
                    data.push(text.clone());
                }
            }
        }
    }
    data.sort();
    data.dedup();
    data
}

fn assert_no_sample_data(snapshot: &Snapshot, state: &str) {
    let data = sample_data();
    assert!(
        data.len() >= 30 && data.iter().any(|sample| sample == "Moodboard v3.pdf"),
        "the instrument reads the board ({} strings)",
        data.len()
    );
    for text in texts(snapshot) {
        for sample in &data {
            assert!(
                !text.contains(sample.as_str()),
                "{state}: the board's {sample:?} reached the running app's flyout ({text:?})"
            );
        }
    }
}

/// Signed out on a real engine: the board's sign-in state, no file names, nothing read from the board.
#[test]
fn a_signed_out_engine_shows_signed_out_and_no_file_names() {
    let engine = offline_engine();
    assert_eq!(engine.gate(), EngineGate::SignedOut);
    let (live, _reports) = live_on(&engine);
    let snapshot = EngineTraySource::new(live.clone()).snapshot();
    assert_eq!(snapshot.status, Status::SignedOut);
    assert_eq!(snapshot.changes, Changes::Hidden(Hidden::SignedOut));
    assert_eq!(snapshot.button, Some((Action::SignIn, true)));
    assert!(
        !snapshot.explorer_ready,
        "Cloud Files is not wired: never in File Explorer"
    );
    assert_no_sample_data(&snapshot, "signed out");
    let b = live.tray_inputs();
    assert!(!b.signed_in && !b.session_expired && b.engine == EngineState::Idle);
}

fn entry(id: &str, path: &str, status: FileStatus, kind: ItemKind, at: i64) -> FileEntry {
    FileEntry {
        file_id: id.to_string(),
        path: path.to_string(),
        status,
        size_bytes: if kind == ItemKind::Folder { 0 } else { 2_400_000 },
        modified_at: at,
        content_hash: None,
        remote_updated_at: at,
        parent_id: None,
        item_kind: kind,
    }
}

fn put(db: &StateDb, entry: FileEntry) {
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

fn transfer(db: &StateDb, direction: &'static str, id: &str, path: &str, at: i64) {
    db.record_transfer_activity(TransferActivityInput {
        direction,
        file_id: Some(id.to_string()),
        file_name: path.rsplit('/').next().unwrap_or(path).to_string(),
        rel_path: Some(path.to_string()),
        bytes: 2_400_000,
        occurred_at: at,
    })
    .unwrap();
}

/// A queue row for the file `id` at `path`: `attempts` of 5, paused for `reason` when there is one.
fn queued(db_path: &std::path::Path, id: &str, path: &str, attempts: i64, reason: Option<&str>, at: i64) {
    let sql = rusqlite::Connection::open(db_path).unwrap();
    sql.execute(
        "INSERT INTO files (file_id, path) VALUES (?1, ?2)",
        rusqlite::params![id, path],
    )
    .unwrap();
    sql.execute(
        "INSERT INTO operation_queue (op_id, kind, file_id, target_path, attempts, max_attempts, next_retry_at,
                                      paused_reason, created_at)
         VALUES (?1, 'upload_file', ?2, ?3, ?4, 5, 0, ?5, ?6)",
        rusqlite::params![format!("op-{id}"), id, path, attempts, reason, at],
    )
    .unwrap();
}

/// The state database a synced PC holds, written with the engine's own writers (and, for the queue and
/// the Backups flag the server sets, the columns themselves): a file on its way up, one given up, one held
/// for the quota, one waiting, a conflict, three things that happened, and a device's manifest in Backups.
fn synced_database(db_path: &std::path::Path, now: i64) -> StateDb {
    let db = StateDb::open(db_path).unwrap();
    put(
        &db,
        entry("d-plans", "Plans", FileStatus::Local, ItemKind::Folder, now - 9_000),
    );
    put(
        &db,
        entry("d-backups", "Backups", FileStatus::Local, ItemKind::Folder, now - 9_000),
    );
    put(
        &db,
        entry(
            "f-draft",
            "Plans/draft-a3.odt",
            FileStatus::Uploading,
            ItemKind::File,
            now - 30,
        ),
    );
    put(
        &db,
        entry(
            "f-both",
            "Plans/both-a3.ods",
            FileStatus::Conflict,
            ItemKind::File,
            now - 600,
        ),
    );
    transfer(&db, "up", "f-ledger", "Plans/ledger-a3.ods", now - 120);
    transfer(&db, "down", "f-song", "Plans/song-a3.m4a", now - 3 * 3_600);
    transfer(&db, "up", "f-manifest", "Backups/A3-PC/.device.json", now - 60);
    db.record_local_activity(LocalActivityEventInput {
        event_type: LocalActivityKind::MovedToTrash,
        file_id: Some("f-old".into()),
        file_name: "old-a3.md".into(),
        rel_path: Some("Plans/old-a3.md".into()),
        occurred_at: now - 26 * 3_600,
    })
    .unwrap();
    queued(db_path, "f-stuck", "Plans/stuck-a3.mov", 5, None, now - 900);
    queued(db_path, "f-held", "Plans/held-a3.mov", 0, Some("quota"), now - 800);
    queued(db_path, "f-wait", "Plans/wait-a3.txt", 0, None, now - 700);
    let sql = rusqlite::Connection::open(db_path).unwrap();
    sql.execute(
        "UPDATE files SET system_role = 'backups_root' WHERE file_id = 'd-backups'",
        [],
    )
    .unwrap();
    db
}

fn names(local: &Local) -> Vec<(&str, RowKind)> {
    local.rows.iter().map(|row| (row.path.as_str(), row.kind)).collect()
}

/// What the database says, read for the flyout: the counts the status takes, the moving files only
/// while sync runs, what was given up, what happened newest first, and never a device's manifest
/// (LD-0113).
#[test]
fn the_database_on_this_pc_gives_the_flyout_its_counts_and_its_history() {
    let dir = tempfile::tempdir().unwrap();
    let now = 1_794_000_000;
    let db = synced_database(&dir.path().join("state.db"), now);

    let local = local_of(&db, now, true);
    assert_eq!(
        (local.conflicts, local.gave_up, local.unsent, local.held_for_quota),
        (1, 1, 1, 1),
        "one conflict, one given up, one waiting, one held for the quota"
    );
    assert!(local.moving >= 1, "the upload counts as moving");
    assert_eq!(local.files, None, "the file count is not wired");
    assert_eq!(
        names(&local),
        [
            ("Plans/draft-a3.odt", RowKind::Uploading { bytes: 2_400_000 }),
            ("Plans/stuck-a3.mov", RowKind::Failed),
            ("Plans/ledger-a3.ods", RowKind::Uploaded),
            ("Plans/song-a3.m4a", RowKind::Downloaded),
            ("Plans/old-a3.md", RowKind::MovedToTrash),
        ]
    );
    assert_eq!(local.rows[0].at, now, "on its way now");
    assert_eq!(
        local.rows[2].id.as_deref(),
        Some("f-ledger"),
        "a click shows the engine's file"
    );

    // Sync does not run: an upload in the database is not on its way.
    let still = local_of(&db, now, false);
    assert!(
        still
            .rows
            .iter()
            .all(|row| !matches!(row.kind, RowKind::Uploading { .. })),
        "{:?}",
        names(&still)
    );

    // The flyout's handle on the database is read-only (`EngineTraySource::database`): it reads the same.
    let read_only = StateDb::open_read_only(dir.path().join("state.db")).expect("a read-only handle");
    assert_eq!(local_of(&read_only, now, true), local);
}

/// A real account on the fake API, from signed out to locked: each state the engine is in, the storage
/// read from Beebeeb (once, on a thread of its own), the database not even read while locked, and none of
/// the board's sample data in any of them. (What the database says is the test above's, on a database of
/// its own: this handle's is the process's.)
#[test]
fn a_real_account_from_signed_out_to_locked_shows_what_the_engine_knows() {
    let (mut account, phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    account.used_bytes = 960_000_000_000;
    account.quota_bytes = 1_000_000_000_000;
    let account = Arc::new(account);
    let fake = start_fake(account.clone());
    let engine = engine_at(&fake.base);
    let (live, _reports) = live_on(&engine);
    let mut tray = EngineTraySource::new(live.clone());

    let signed_out = tray.snapshot();
    assert_eq!(signed_out.status, Status::SignedOut);
    assert_no_sample_data(&signed_out, "signed out");

    // Signed in, no key of the account on this PC.
    engine
        .sign_in_password(EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    assert_eq!(engine.gate(), EngineGate::NeedsRecoveryPhrase);
    let phrase_needed = tray.snapshot();
    assert_eq!(
        (&phrase_needed.status, &phrase_needed.changes, phrase_needed.button),
        (&Status::NeedsRecoveryPhrase, &Changes::Hidden(Hidden::NoKeys), None)
    );
    assert_no_sample_data(&phrase_needed, "needs the recovery phrase");

    // Unlocked, sync not running: the window's state, never "up to date"; names may show.
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    assert_eq!(engine.gate(), EngineGate::Ready);
    tray.follow(&TrayInputs::default());
    let ready = tray.snapshot();
    assert_eq!((&ready.status, ready.button), (&Status::NotSyncing, None));
    assert!(!matches!(ready.changes, Changes::Hidden(_)), "{:?}", ready.changes);
    assert_no_sample_data(&ready, "ready, sync not running");
    let b = live.tray_inputs();
    assert!(b.signed_in && !b.locked && !b.paused && b.engine == EngineState::Idle);

    // The storage, read once from Beebeeb on its own thread; the next reads within a minute ask nothing.
    let waited = Instant::now();
    let storage = loop {
        if let Some(storage) = tray.facts().storage {
            break storage;
        }
        assert!(
            waited.elapsed() < Duration::from_secs(20),
            "the storage read did not come back"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(storage, (960_000_000_000, 1_000_000_000_000));
    let reads = |fake: &beebeeb_engine::test_api::FakeApi| {
        fake.routes()
            .iter()
            .filter(|route| route.as_str() == "GET /api/v1/billing/usage")
            .count()
    };
    let _ = tray.snapshot();
    let settled = Instant::now();
    while tray.storage.lock().unwrap().reading {
        assert!(
            settled.elapsed() < Duration::from_secs(20),
            "a storage read did not come back"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(reads(&fake), 1, "one read of the storage, none again within the minute");

    // Locked: the keys leave memory, and nothing names a file.
    let _ = engine.lock();
    assert_eq!(engine.gate(), EngineGate::Locked);
    let locked = tray.snapshot();
    assert_eq!(
        (&locked.status, &locked.changes, locked.button),
        (&Status::LockedSyncOff, &Changes::Hidden(Hidden::Locked), None)
    );
    let facts = tray.facts();
    assert!(
        facts.local.is_none() && facts.storage.is_none(),
        "while locked the database is not read, nor the storage"
    );
    assert_no_sample_data(&locked, "locked");
    assert!(live.tray_inputs().locked);
}
