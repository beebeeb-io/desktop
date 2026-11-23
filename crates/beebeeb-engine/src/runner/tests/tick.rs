//! tick tests split out of runner.rs.

use super::*;

fn status_db(dir: &Path) -> Arc<StateDb> {
    Arc::new(StateDb::open(dir.join("state.db")).unwrap())
}

fn upload_op(op_id: &str, file_id: &str) -> crate::state_db::PendingOperation {
    crate::state_db::PendingOperation {
        op_id: op_id.into(),
        kind: crate::state_db::OperationKind::UploadFile,
        file_id: Some(file_id.into()),
        parent_id: None,
        target_path: None,
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

fn seed_file(db: &StateDb, id: &str, size: i64) {
    db.upsert_file(&crate::state_db::FileEntry {
        file_id: id.into(),
        path: format!("Work/{id}.bin"),
        status: FileStatus::Local,
        size_bytes: size,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: crate::state_db::ItemKind::File,
    })
    .unwrap();
}

#[test]
fn a_good_tick_with_nothing_left_is_idle_and_ends_the_batch() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    let board = crate::transfer_progress::TransferBoard::new();
    let g = board.begin("a", crate::transfer_progress::Direction::Up, 500);
    g.update(500);
    g.finish();
    let link = crate::link_health::LinkMonitor::new();
    let tracker = StatusTracker::new(None);
    let payload = finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board).to_json();
    assert_eq!(payload["state"], "idle");
    assert_eq!(payload["files_remaining"], 0);
    assert_eq!(payload["bytes_total"], 500, "the final frame of the batch: 500 of 500");
    assert_eq!(payload["bytes_done"], 500);
    assert!(
        payload["last_tick_ok_at"].as_i64().unwrap() > 0,
        "a good check stamps its time"
    );
    assert_eq!(
        board.finished_bytes(),
        0,
        "nothing left: the next batch starts its bar at 0"
    );
}

#[test]
fn a_good_tick_with_work_left_keeps_the_batch_going() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    seed_file(&db, "b", 300);
    db.enqueue_operation(&upload_op("op-b", "b")).unwrap();
    let board = crate::transfer_progress::TransferBoard::new();
    let g = board.begin("a", crate::transfer_progress::Direction::Up, 500);
    g.update(500);
    g.finish();
    let link = crate::link_health::LinkMonitor::new();
    let tracker = StatusTracker::new(None);
    let payload = finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board).to_json();
    assert_eq!(payload["files_remaining"], 1);
    assert_eq!(payload["bytes_total"], 800, "500 finished + 300 still queued");
    assert_eq!(payload["bytes_done"], 500);
    assert_eq!(board.finished_bytes(), 500, "work is left, so the batch is not over");
}

#[test]
fn a_tick_whose_request_failed_at_the_link_is_offline_and_not_a_good_check() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    let board = crate::transfer_progress::TransferBoard::new();
    let link = crate::link_health::LinkMonitor::new();
    link.note_failure(
        crate::link_health::LinkFailure {
            kind: crate::link_health::LinkFailureKind::Offline,
            reason: crate::link_health::LinkReason::Connect,
        },
        now_secs(),
    );
    let tracker = StatusTracker::new(None);
    let payload = finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board).to_json();
    assert_eq!(payload["state"], "offline");
    assert_eq!(
        payload["legacy_state"], "idle",
        "an Ok tick still reads idle to old consumers"
    );
    assert_eq!(payload["last_tick_ok_at"], serde_json::Value::Null);
}

#[test]
fn a_failed_tick_is_an_error_with_the_error_text() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    let board = crate::transfer_progress::TransferBoard::new();
    let link = crate::link_health::LinkMonitor::new();
    let tracker = StatusTracker::new(None);
    let err = anyhow::anyhow!("HTTP 503 Service Unavailable: down");
    let payload = finish_tick_payload(&tracker, Err(&err), &link, 0, &db, &board).to_json();
    assert_eq!(payload["state"], "error");
    assert_eq!(payload["legacy_state"], "error");
    assert_eq!(payload["error"], "HTTP 503 Service Unavailable: down");
    assert_eq!(payload["reason"]["code"], "http_5xx");
}

#[test]
fn the_tick_loop_reports_through_the_tracker() {
    // `run` needs a real window system, so its wiring is asserted on the source (CRLF
    // normalised: a Windows checkout has CRLF, slice 5's lesson).
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    // The signature is wrapped over lines by rustfmt. (A single-line needle would also find this very line.)
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    for needle in [
        "emit_payload(&app, tracker.running());",
        "emit_payload(&app, paused_payload(&tracker, hold));",
        "emit_payload(&app, tracker.stopped());",
        "finish_tick_payload(&tracker, Ok(()), &link, tick_started_at, &db, &transfers)",
        "finish_tick_payload(&tracker, Err(&e), &link, tick_started_at, &db, &transfers)",
        "run_status_pulse(",
        ".with_transfers(transfers.clone())",
        "let link = api.link();",
    ] {
        assert_eq!(
            body.matches(needle).count(),
            1,
            "`run` must contain exactly one `{needle}`"
        );
    }
    // The old bare emitters are gone from the loop: each `emit_status` left in `run` is a
    // start-up failure (`"error"`), never a state the tracker owns.
    for old in [
        "emit_status(&app, \"idle\"",
        "emit_status(&app, \"paused\"",
        "emit_status(&app, \"running\"",
        "emit_status(&app, \"stopped\"",
    ] {
        assert_eq!(
            body.matches(old).count(),
            0,
            "`run` still emits `{old}` around the tracker"
        );
    }
}

/// Spec §10.2: engine start marks the uploads an earlier build queued,
/// once, on macOS only (on Windows it would hand write ids to Cloud Files uploads, which have
/// no base), after the start repair and before the stale in-flight rows are reconciled (that
/// reconcile is keyed on the write id). `run` needs a real window system, so the wiring is
/// asserted on the source, on every platform.
#[test]
fn engine_start_marks_earlier_build_uploads_on_macos_only_after_the_repair() {
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let call = "db.mark_earlier_build_uploads()";
    assert_eq!(body.matches(call).count(), 1, "`run` marks once");
    assert!(
        body.contains(&format!("#[cfg(target_os = \"macos\")]\n    match {call}")),
        "the marking is gated to macOS"
    );
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("`run` calls {needle}"));
    assert!(
        at("db.engine_start_repair()") < at(call) && at(call) < at("db.reconcile_stale_in_flight_on_startup()"),
        "after the start repair, before the in-flight reconcile"
    );
}

/// Spec §8.7 S6: engine start removes the staged copies nothing will upload, once, on macOS
/// only, after the start repair cleared the claims and before the IPC server serves a write.
/// `run` needs a real window system, so the wiring is asserted on the source, on every platform.
#[test]
fn engine_start_sweeps_unreferenced_staged_copies_on_macos_after_the_repair_before_ipc() {
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let call =
        "crate::staged_payload::sweep_unreferenced(&db, &crate::engine_bridge::finder_staging_candidates(identity))";
    assert_eq!(body.matches(call).count(), 1, "`run` sweeps once");
    assert!(
        body.contains(&format!("#[cfg(target_os = \"macos\")]\n    match {call}")),
        "the sweep is gated to macOS"
    );
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("`run` calls {needle}"));
    assert!(
        at("db.engine_start_repair()") < at(call) && at(call) < at("crate::ipc_socket::serve_ipc("),
        "after the start repair, before the IPC server"
    );
}

/// Spec §11: the engine-start lines from the start
/// repair to the in-flight reconcile log counts and, when a step fails, the error's category;
/// never the error's text, which can carry a path. The reconcile's count is the rows it
/// reset, and its line says so. `run` needs a real window system, so this is asserted on the
/// source, on every platform.
#[test]
fn engine_start_lines_log_counts_and_error_categories_only() {
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let from = body.find("match db.engine_start_repair()").expect("the start repair");
    let to = from
        + body[from..]
            .find("// The token's buffer MOVES into the client")
            .expect("the end of the start steps");
    let steps = &body[from..to];
    assert_eq!(steps.matches("Err(e)").count(), 4, "four steps can fail:\n{steps}");
    assert_eq!(
        steps
            .matches("reason = crate::engine_bridge::error_category(&e)")
            .count(),
        4,
        "each failure logs its category:\n{steps}"
    );
    for raw in ["error = %e", "%e,", "error = ?e", "{e}"] {
        assert!(
            !steps.contains(raw),
            "an engine-start line logs the error's text (`{raw}`):\n{steps}"
        );
    }
    assert!(
        steps.contains("reset,\n") && steps.contains("in-flight rows from a previous engine session reset"),
        "the reconcile line names what it counts:\n{steps}"
    );
}

/// A host that keeps every `engine-status` payload the engine sends it, as the JSON the UI receives.
#[cfg(not(target_os = "windows"))]
struct StatusRecorder(Arc<Mutex<Vec<serde_json::Value>>>);

#[cfg(not(target_os = "windows"))]
impl crate::host::EngineHost for StatusRecorder {
    fn event(&self, event: EngineEvent) {
        if event.name() == crate::engine_status::EVENT {
            self.0.lock().unwrap().push(event.payload());
        }
    }
    fn notify(&self, _notice: Notice) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn status_recorder() -> (Host, Arc<Mutex<Vec<serde_json::Value>>>) {
    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    (Arc::new(StatusRecorder(events.clone())), events)
}

#[cfg(not(target_os = "windows"))]
async fn wait_for_event(
    events: &Arc<Mutex<Vec<serde_json::Value>>>,
    within: Duration,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> Option<serde_json::Value> {
    let started = std::time::Instant::now();
    while started.elapsed() < within {
        if let Some(found) = events.lock().unwrap().iter().find(|e| pred(e)).cloned() {
            return Some(found);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    None
}

#[test]
fn the_pulses_activity_counts_an_upload_that_is_due_and_not_one_in_retry_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    seed_file(&db, "f1", 100);
    seed_file(&db, "f2", 5_000);
    let mut due = upload_op("op-f1", "f1");
    due.next_retry_at = 1; // long past: due now
    let mut backing_off = upload_op("op-f2", "f2");
    backing_off.attempts = 1;
    backing_off.next_retry_at = i64::MAX; // waiting out a backoff
    db.enqueue_operation(&due).unwrap();
    db.enqueue_operation(&backing_off).unwrap();

    let board = crate::transfer_progress::TransferBoard::new();
    let activity = current_activity(&db, &board);
    assert_eq!(activity.files_remaining, 1, "one file left, not two");
    assert_eq!(
        activity.bytes_total, 100,
        "the backed-off file's 5000 bytes are not in the total"
    );
}

/// RC4-1 (VM 401: 18 files copied into the folder root, "Up to date" at 129 of 134 polls). The status line says
/// "Up to date" for an engine that reports `idle`. The scan queues the copied files at once; their uploads start at
/// the next tick, up to 30 s later. All that time the engine must report `syncing` with the count, and `idle` only
/// when the last one has left the queue. Sampled here as the pulse does, once a second, before any tick or upload.
#[test]
fn status_is_syncing_from_the_scan_that_queued_copied_files_until_the_last_is_sent() {
    let temp = tempfile::tempdir().unwrap();
    let sync_root = temp.path().join("sync-root");
    let state_dir = temp.path().join("state");
    std::fs::create_dir_all(&sync_root).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();
    let db = status_db(&state_dir);
    let api = Arc::new(crate::api_client::ApiClient::new(
        "https://api.beebeeb.io".into(),
        "token".into(),
        [17u8; 32],
    ));
    let bridge = EngineBridge::new(&crate::identity::TEST, db.clone(), api);
    let board = crate::transfer_progress::TransferBoard::new();
    let link = crate::link_health::LinkMonitor::new();
    let tracker = StatusTracker::new(None);
    // A finished check with nothing to do: the engine says `idle` ("Up to date"), and the pulse is quiet.
    assert_eq!(
        finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board).to_json()["state"],
        "idle"
    );
    assert_eq!(status_pulse_once(&tracker, &db, &board, false), None);

    for n in 0..18 {
        std::fs::write(sync_root.join(format!("copied-{n}.txt")), format!("file {n}")).unwrap();
    }
    crate::watcher::run_one_scan(&bridge, &sync_root);
    assert_eq!(
        db.list_due_operations(i64::MAX).unwrap().len(),
        18,
        "the scan queued all 18"
    );

    // No tick has run and no upload has started: nothing is on the board.
    assert!(board.active().is_empty());
    let queued = status_pulse_once(&tracker, &db, &board, false)
        .expect("the pulse after the scan reports the queued files")
        .to_json();
    assert_eq!(queued["state"], "syncing", "18 files wait to be sent: not `idle`");
    assert_eq!(queued["files_remaining"], 18);
    // It keeps saying so on every look while they wait.
    let still = status_pulse_once(&tracker, &db, &board, false)
        .expect("still queued")
        .to_json();
    assert_eq!(still["state"], "syncing");

    // A check that finishes while files still wait does not say `idle` either.
    let after_tick = finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board).to_json();
    assert_eq!(
        after_tick["state"], "syncing",
        "a good check with 18 files left is not `idle`"
    );
    assert_eq!(after_tick["files_remaining"], 18);
    assert!(
        after_tick["last_tick_ok_at"].as_i64().unwrap() > 0,
        "the check still stamps its time"
    );
    assert_eq!(
        status_pulse_once(&tracker, &db, &board, false)
            .expect("still queued")
            .to_json()["state"],
        "syncing"
    );

    // All but one sent: still `syncing`. The last one leaves the queue: `idle` once, then quiet.
    let ops = db.list_due_operations(i64::MAX).unwrap();
    for op in &ops[1..] {
        db.remove_operation(&op.op_id).unwrap();
    }
    let one_left = status_pulse_once(&tracker, &db, &board, false)
        .expect("one left")
        .to_json();
    assert_eq!(one_left["state"], "syncing");
    assert_eq!(one_left["files_remaining"], 1);
    db.remove_operation(&ops[0].op_id).unwrap();
    let done = status_pulse_once(&tracker, &db, &board, false)
        .expect("the last file sent is reported")
        .to_json();
    assert_eq!(done["state"], "idle");
    assert_eq!(done["files_remaining"], 0);
    assert_eq!(status_pulse_once(&tracker, &db, &board, false), None);
}

/// RC4-1: a queued upload that waits out a backoff, and a paused engine, do not read as `syncing`.
#[test]
fn status_is_not_syncing_for_an_upload_that_waits_out_a_backoff_or_while_paused() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    seed_file(&db, "f1", 100);
    let mut backing_off = upload_op("op-f1", "f1");
    backing_off.attempts = 1;
    backing_off.next_retry_at = i64::MAX;
    db.enqueue_operation(&backing_off).unwrap();
    let board = crate::transfer_progress::TransferBoard::new();
    let tracker = StatusTracker::new(None);
    tracker.finish_tick(&crate::engine_status::Outcome::Synced, 1, Activity::default());
    assert_eq!(status_pulse_once(&tracker, &db, &board, false), None);

    seed_file(&db, "f2", 100);
    db.enqueue_operation(&upload_op("op-f2", "f2")).unwrap();
    let paused = status_pulse_once(&tracker, &db, &board, true).expect("the pause is reported");
    assert_eq!(paused.state, crate::engine_status::State::Paused);
    assert_eq!(status_pulse_once(&tracker, &db, &board, true), None);
    // Resumed with one file due: `syncing` at the next look.
    let resumed = status_pulse_once(&tracker, &db, &board, false).expect("one file is due");
    assert_eq!(resumed.state, crate::engine_status::State::Syncing);
}

#[cfg(not(target_os = "windows"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_pulse_emits_syncing_with_progress_then_idle_and_resets_the_batch() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    seed_file(&db, "f1", 100);
    db.enqueue_operation(&upload_op("op-f1", "f1")).unwrap();
    let board = crate::transfer_progress::TransferBoard::new();
    let paused = Arc::new(AtomicBool::new(false));
    let (app, events) = status_recorder();
    let tracker = Arc::new(StatusTracker::new(None));
    tracker.finish_tick(&crate::engine_status::Outcome::Synced, 1, Activity::default());
    let pulse = tokio::spawn(run_status_pulse(
        app.clone(),
        tracker,
        db.clone(),
        board.clone(),
        paused,
    ));

    // Quiet: nothing is moving, so nothing is emitted (and no DB work is done).
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(events.lock().unwrap().is_empty(), "an idle engine emits nothing");

    // A file starts moving: within a couple of pulses the UI hears `syncing`.
    let transfer = board.begin("f1", crate::transfer_progress::Direction::Up, 100);
    transfer.update(40);
    let syncing = wait_for_event(&events, Duration::from_millis(3000), |e| e["state"] == "syncing")
        .await
        .expect("a `syncing` event while a file moves");
    assert_eq!(syncing["files_remaining"], 1);
    assert_eq!(syncing["bytes_total"], 100);
    assert_eq!(syncing["bytes_done"], 40);
    assert_eq!(syncing["legacy_state"], "idle");

    // It finishes and leaves the queue: the pulse says `idle` and ends the batch.
    transfer.update(100);
    transfer.finish();
    db.remove_operation("op-f1").unwrap();
    let idle = wait_for_event(&events, Duration::from_millis(3000), |e| e["state"] == "idle")
        .await
        .expect("an `idle` event when the work is done");
    assert_eq!(idle["files_remaining"], 0);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(board.finished_bytes(), 0, "the pulse ends the batch when it goes quiet");
    pulse.abort();
}

#[cfg(not(target_os = "windows"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_pulse_reports_a_pause_the_moment_it_happens() {
    let dir = tempfile::tempdir().unwrap();
    let db = status_db(dir.path());
    let board = crate::transfer_progress::TransferBoard::new();
    let paused = Arc::new(AtomicBool::new(false));
    let (app, events) = status_recorder();
    let tracker = Arc::new(StatusTracker::new(None));
    tracker.finish_tick(&crate::engine_status::Outcome::Synced, 1, Activity::default());
    let pulse = tokio::spawn(run_status_pulse(app.clone(), tracker, db, board, paused.clone()));
    paused.store(true, Ordering::Relaxed);
    let paused_event = wait_for_event(&events, Duration::from_millis(3000), |e| e["state"] == "paused").await;
    assert!(
        paused_event.is_some(),
        "the pulse must say `paused` within two seconds of the flag"
    );
    pulse.abort();
}

/// Task 2521, a tick or a beat that ran long (a large upload, a slow answer, a PC that slept) is not
/// followed by a burst of the periods it missed. With tokio's default (`Burst`) those fire back to back, and an ended
/// session's refusals would then arrive milliseconds apart: one moment counted as the whole run. `Delay` starts the
/// next period when the late one ends. Both feeds of the run are pinned: the runner's tick and the heartbeat's beat.
#[test]
fn a_late_tick_or_beat_delays_the_next_one_instead_of_a_burst() {
    for (file, source, interval, set) in [
        (
            "runner.rs",
            include_str!("../../runner.rs"),
            "let mut tick = tokio::time::interval(TICK_INTERVAL);\n",
            "tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);",
        ),
        (
            "runner/telemetry.rs",
            include_str!("../telemetry.rs"),
            "let mut ticker = tokio::time::interval(interval);\n",
            "ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);",
        ),
    ] {
        let source = source.replace("\r\n", "\n");
        let source = &source[..source.find("#[cfg(test)]\nmod tests").unwrap_or(source.len())];
        assert_eq!(
            source.matches("tokio::time::interval(").count(),
            1,
            "{file}: one interval"
        );
        let at = source.find(interval).unwrap_or_else(|| panic!("{file}: the interval"));
        let after: String = source[at + interval.len()..]
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("//"))
            .take(1)
            .collect();
        assert_eq!(after, set, "{file}: the interval delays a missed period");
    }
    // The paused tick's check of the session is bounded well under the stop's graceful wait.
    assert!(PAUSED_SESSION_CHECK_TIMEOUT < GRACEFUL_ABORT_TIMEOUT);
}

// ── AbortOutcome per-stage classification (Bug A) ───────────────────────
//
// The old `abort() -> bool` conflated "the engine task is gone" with "the
// Windows Cloud Files callback revocation succeeded", so a revocation
// failure (Explorer-held handles, 3s drain timeout) was reported as
// "could not stop the sync engine". The classification must attribute the
// failure to the stage that actually failed.

/// Task 2523 (F1): the stale-copy pass runs once per tick, only at a tick whose pull reached the server, after the
/// reconcile pass and before the queue step (on Linux that step sends the re-downloads it queued), off the async
/// workers; on Windows only once the start-up guard allows freeing work. `run` needs a real window system, so the
/// wiring is asserted on the source, on every platform.
#[test]
fn the_tick_runs_the_stale_copy_pass_after_the_pull_and_before_the_queue_step() {
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let call = "pass_bridge.refresh_stale_copies(&pass_root)";
    assert_eq!(body.matches(call).count(), 1, "`run` runs the pass once per tick");
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("`run` has {needle}"));
    assert!(
        at(".reconcile_then_resume(") < at(call) && at(call) < at("let completed_sync_work = queue_step("),
        "after the reconcile pass, before the queue step"
    );
    assert!(
        at("if tick.reached_server && stale_copies_allowed {") < at(call),
        "only after a pull that came back"
    );
    assert!(body.contains("let stale_copies_allowed = crate::unknown_check::STARTUP_GUARD.allows_freeing();"));
    assert!(body.contains("tokio::task::spawn_blocking(move || pass_bridge.refresh_stale_copies(&pass_root))"));
}

/// S6: the kept files' step runs at a tick whose pull reached the server, after the stale pass and before the queue
/// step, never while a sign-out's send phase is armed (a paused tick returned before it); #447's per-file fetch loop
/// is gone (the stale pass's kept files join the one batch). Asserted on the source, on every platform.
#[test]
fn s6_the_tick_runs_the_kept_step_after_the_stale_pass_before_the_queue_step() {
    let source = include_str!("../../runner.rs").replace("\r\n", "\n");
    let start = source
        .find("async fn run(\n    identity: &'static crate::identity::EngineIdentity,\n    app: Host")
        .expect("run exists");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("`run` has {needle}"));
    let pass = at("pass_bridge.refresh_stale_copies(&pass_root)");
    let queue = at("let completed_sync_work = queue_step(");
    for call in [
        "crate::engine_bridge::spawn_kept_batch(",
        "crate::engine_bridge::queue_kept_step(",
    ] {
        assert_eq!(body.matches(call).count(), 1, "`run` calls {call} once");
        let call_at = at(call);
        assert!(
            pass < call_at && call_at < queue,
            "{call}: after the stale pass, before the queue step"
        );
        let guard_at = body[..call_at].rfind("if ").unwrap();
        let guard = &body[guard_at..guard_at + body[guard_at..].find('{').unwrap()];
        assert_eq!(
            guard.trim_end(),
            "if tick.reached_server && !send_phase.is_armed()",
            "{call}: only at a tick whose pull came back, never in a sign-out's send phase"
        );
    }
    assert_eq!(
        body.matches("hydrate_placeholder(&place)").count(),
        0,
        "the per-file fetch loop is gone"
    );
}
