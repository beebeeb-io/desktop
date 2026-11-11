//! `scan` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Every start of 0.8.17 on the maintainer's PC wrote one WARN line per held file of unknown content
/// (28,224 in ~100 s), burying every other warning. Each held edit is listed for review (task 2190), so
/// the log needs only the count: one line for a pass that holds new edits, the per-file line at DEBUG,
/// and nothing for a pass (or a start) whose held edits are the ones already listed. The line counts each
/// reason apart (review on #172): an edit with no known base needs a look; a file synced before 1640 does not.
#[tokio::test]
async fn task_2244_a_pass_that_holds_edits_logs_one_count_not_a_line_per_file() {
    let master_key = [194u8; 32];
    let (_dir, sync_root, _server, bridge) = rule3_setup(master_key);
    let names = ["a.txt", "b.txt", "c.txt"];
    for name in names.iter().chain(&["draft.txt"]) {
        std::fs::write(sync_root.join(name), name.as_bytes()).unwrap();
    }
    let mut memo = crate::watcher::ScanMemo::default();
    regression_2229_pass(&bridge, &sync_root, &mut memo);
    drain_upload_queue(&bridge, &sync_root).await;
    let draft = sync_root.join("draft.txt");
    let draft_row = bridge.db.get_file_by_path("draft.txt").unwrap().unwrap();
    std::fs::write(&draft, b"an edit made while the base was lost").unwrap();
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_base_version = 0, fetched_min_version = 0 WHERE file_id = '{}'",
            draft_row.file_id
        ))
        .unwrap();
    assert_eq!(
        sync_folder_content(&bridge.db, &draft_row.file_id, &draft, None).unwrap(),
        SyncFolderContent::Changed { base: None },
        "precondition: an edit with no known base"
    );
    for name in names {
        let row = bridge.db.get_file_by_path(name).unwrap().unwrap();
        bridge
            .db
            .test_execute(&format!(
                "UPDATE files SET local_hash = NULL, fetched_min_version = -1 WHERE file_id = '{}'",
                row.file_id
            ))
            .unwrap();
        assert!(
            bridge.db.is_unknown_content(&row.file_id).unwrap(),
            "precondition: content unknown"
        );
    }
    let warns = |logs: &str| {
        logs.lines()
            .filter(|l| l.contains(" WARN "))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };

    let logs = capture_logs_async(async {
        regression_2229_pass(&bridge, &sync_root, &mut memo);
    })
    .await;
    assert_eq!(
        bridge.db.list_held_edits().unwrap().len(),
        4,
        "precondition: the pass held all four"
    );
    let first = warns(&logs);
    assert_eq!(first.len(), 1, "one line for the pass, not one per file: {first:#?}");
    for count in ["held=4", "content_unknown=3", "base_unknown=1"] {
        assert!(first[0].contains(count), "the line carries {count}: {}", first[0]);
    }

    // A start: a fresh memo asks about every file again, and their held edits are the ones listed.
    let mut restarted = crate::watcher::ScanMemo::default();
    let logs = capture_logs_async(async {
        regression_2229_pass(&bridge, &sync_root, &mut restarted);
    })
    .await;
    assert_eq!(restarted.classified, 4, "precondition: the start asked about all four");
    assert_eq!(warns(&logs), Vec::<String>::new(), "nothing new held: no line");
}

// ── Task 2229: the scan skips a settled file until its bytes or its row change ──

/// (classified, skipped) of one pass of the scan with `memo`.
fn regression_2229_pass(
    bridge: &EngineBridge,
    sync_root: &Path,
    memo: &mut crate::watcher::ScanMemo,
) -> (usize, usize) {
    crate::watcher::run_one_scan_with(bridge, sync_root, memo);
    (memo.classified, memo.skipped)
}

/// A synced file whose bytes are the agreed content is classified once, then skipped while
/// its listing and its row stay the same. A write to its row, a save of new bytes, and the
/// periodic full pass each make the scan classify it again, and the save is still sent.
#[tokio::test]
async fn regression_2229_a_settled_file_is_classified_again_only_when_it_or_its_row_changes() {
    let master_key = [191u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let local = sync_root.join("kept.txt");
    std::fs::write(&local, b"made on this PC").unwrap();
    let mut memo = crate::watcher::ScanMemo::default();
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "a new file"
    );
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "kept.txt"),
        1,
        "precondition: queued"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file_by_path("kept.txt").unwrap().unwrap();
    assert_eq!(row.status, FileStatus::Local, "precondition: landed");

    // A queued file is not remembered, so the pass after its landing asks about it as for the first time.
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "landed: asked"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "settled, unchanged: skipped"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "and again"
    );

    bridge.db.set_size_bytes(&row.file_id, 4242).unwrap();
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "a write to its row: asked again"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "settled again"
    );

    memo.age_by(std::time::Duration::from_secs(11 * 60));
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "the full pass asks about every file"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "then trusts it again"
    );

    // Only a settled verdict is remembered: an original whose kept copy is pending is passed
    // over for now, so it is asked about on every pass, with its row and its bytes unchanged.
    let kept = bridge
        .db
        .record_kept_copy(&crate::state_db::KeptCopyInput {
            original_file_id: &row.file_id,
            original_path: "kept.txt",
            copy_path: "",
            copied_hash: "",
            original_removed: false,
            reason: crate::state_db::KeptReason::InUse,
            kept_at: 0,
        })
        .unwrap();
    bridge.db.set_size_bytes(&row.file_id, 4243).unwrap();
    for pass in 0..2 {
        assert_eq!(
            regression_2229_pass(&bridge, &sync_root, &mut memo),
            (1, 0),
            "passed over for now: asked again on pass {pass}"
        );
    }
    bridge.db.delete_kept_copy(kept).unwrap();
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "no longer held: settled"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "and skipped"
    );

    // An atomic save: the new bytes written elsewhere, then renamed over the file.
    let saved = sync_root.join("kept.txt.part");
    std::fs::write(&saved, b"saved by an editor").unwrap();
    std::fs::rename(&saved, &local).unwrap();
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "new bytes: asked again"
    );
    let ops = bridge.db.list_operations_for_file(&row.file_id).unwrap();
    assert_eq!(ops.len(), 1, "the save is queued, as an edit of the file");
    // Its row changed when the edit was queued; it is settled again until the row changes
    // once more, when the edit lands.
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "its row changed: asked again"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "settled while queued"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "landed: asked again"
    );
    let state = server.finish();
    assert_eq!(state.latest_plaintext(&row.file_id, master_key), b"saved by an editor");
}

/// Only a settled verdict is remembered: a file the scan passes over for now (here an
/// ignored temp name) is asked about on every pass.
#[tokio::test]
async fn regression_2229_a_file_passed_over_for_now_is_asked_about_every_pass() {
    let (_dir, sync_root, _server, bridge) = rule3_setup([192u8; 32]);
    std::fs::write(sync_root.join("~$draft.docx"), b"owner file").unwrap();
    let mut memo = crate::watcher::ScanMemo::default();
    for pass in 0..3 {
        assert_eq!(
            regression_2229_pass(&bridge, &sync_root, &mut memo),
            (1, 0),
            "pass {pass}"
        );
    }
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "~$draft.docx"),
        0,
        "never queued"
    );
}

/// A synced file of unknown content (synced before task 1640: task 2191's guarded rows) is held, not sent,
/// whatever its bytes are: only a write to its row can change that. The scan remembers it like a settled file,
/// so it no longer checks every such file on every pass (28,224 of them on the maintainer's PC, 0.8.15). When
/// the row learns the agreed content, the held edit is sent.
#[tokio::test]
async fn regression_2229_a_file_of_unknown_content_is_skipped_until_its_bytes_or_its_row_change() {
    let master_key = [193u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let local = sync_root.join("old.txt");
    std::fs::write(&local, b"synced before 0.8.13").unwrap();
    let mut memo = crate::watcher::ScanMemo::default();
    regression_2229_pass(&bridge, &sync_root, &mut memo);
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file_by_path("old.txt").unwrap().unwrap();
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = -1 WHERE file_id = '{}'",
            row.file_id
        ))
        .unwrap();
    assert!(
        bridge.db.is_unknown_content(&row.file_id).unwrap(),
        "precondition: content unknown"
    );

    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "asked once"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "held, unchanged: skipped"
    );

    std::fs::write(&local, b"edited after 0.8.13, kept on this PC").unwrap();
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "new bytes: asked again"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "still held: skipped"
    );
    assert!(
        bridge.db.list_operations_for_file(&row.file_id).unwrap().is_empty(),
        "an edit of unknown content is kept, not sent"
    );

    // The row learns the agreed content, as task 2191's check against the server will record it. The bytes
    // on disk are not it, so they are an edit on that version.
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_hash = '{}', local_base_version = 1, fetched_min_version = 0 WHERE file_id = '{}'",
            sha256_hex(b"synced before 0.8.13"),
            row.file_id
        ))
        .unwrap();
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "its row changed: asked again"
    );
    let ops = bridge.db.list_operations_for_file(&row.file_id).unwrap();
    assert_eq!(ops.len(), 1, "the held edit is queued");
    assert_eq!(ops[0].base_version, Some(1), "on the agreed version");
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext(&row.file_id, master_key),
        b"edited after 0.8.13, kept on this PC"
    );
}

/// An edit whose base version is not known is held, never sent without one, whatever its bytes are: the scan
/// remembers it until its bytes or its row change, and sends it once the row records a base.
#[tokio::test]
async fn regression_2229_an_edit_with_no_known_base_is_skipped_until_its_row_records_one() {
    let master_key = [195u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let local = sync_root.join("draft.txt");
    std::fs::write(&local, b"first version").unwrap();
    let mut memo = crate::watcher::ScanMemo::default();
    regression_2229_pass(&bridge, &sync_root, &mut memo);
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file_by_path("draft.txt").unwrap().unwrap();
    std::fs::write(&local, b"an edit made while the base was lost").unwrap();
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_base_version = 0, fetched_min_version = 0 WHERE file_id = '{}'",
            row.file_id
        ))
        .unwrap();
    assert_eq!(
        sync_folder_content(&bridge.db, &row.file_id, &local, None).unwrap(),
        SyncFolderContent::Changed { base: None },
        "precondition: an edit with no known base"
    );

    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "asked once"
    );
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "held, unchanged: skipped"
    );
    assert!(
        bridge.db.list_operations_for_file(&row.file_id).unwrap().is_empty(),
        "never sent without a base"
    );

    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_base_version = 1 WHERE file_id = '{}'",
            row.file_id
        ))
        .unwrap();
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "its row changed: asked again"
    );
    let ops = bridge.db.list_operations_for_file(&row.file_id).unwrap();
    assert_eq!(ops.len(), 1, "the held edit is queued");
    assert_eq!(ops[0].base_version, Some(1), "on the recorded base");
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext(&row.file_id, master_key),
        b"an edit made while the base was lost"
    );
}

/// Set the modification time of the file at `path`.
pub(super) fn regression_2229_set_modified(path: &Path, modified: std::time::SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

/// Task 2232 (review of desktop #167, P1): an edit that keeps the file's length and sets its modification
/// time back is still sent. The full pass, the scan's safety net for what the directory listing cannot show,
/// must not take the old content hash from a cache that compared size and time alone. Every fake server names
/// its first file `server-file-1`; the cache key holds the path too, so a test running beside this one, in
/// its own directory, cannot replace this test's entry. The cache's own proof is the `task_2232_` test below.
#[tokio::test]
async fn regression_2229_an_equal_length_edit_that_restores_the_mtime_is_sent() {
    let master_key = [194u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let local = sync_root.join("ledger.txt");
    std::fs::write(&local, b"balance: 100").unwrap();
    let mut memo = crate::watcher::ScanMemo::default();
    regression_2229_pass(&bridge, &sync_root, &mut memo);
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file_by_path("ledger.txt").unwrap().unwrap();
    // An hour old, so the hash the next pass takes is one the cache may hand back later.
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    regression_2229_set_modified(&local, hour_ago);
    assert_eq!(regression_2229_pass(&bridge, &sync_root, &mut memo), (1, 0), "asked");
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (0, 1),
        "settled: its bytes are the agreed content"
    );

    std::fs::write(&local, b"balance: 999").unwrap();
    regression_2229_set_modified(&local, hour_ago);
    memo.age_by(std::time::Duration::from_secs(11 * 60));
    assert_eq!(
        regression_2229_pass(&bridge, &sync_root, &mut memo),
        (1, 0),
        "the full pass asks about it"
    );
    let ops = bridge.db.list_operations_for_file(&row.file_id).unwrap();
    assert_eq!(ops.len(), 1, "the edit is queued");
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(state.latest_plaintext(&row.file_id, master_key), b"balance: 999");
}

/// Task 2232: a content hash is not reused for a file rewritten to the same length with its modification
/// time set back. The hash is taken in real time, past [`HASH_CACHE_SETTLED`], as the app takes it. Two
/// writes inside one timer tick can share a change time (on Windows, 27 of 40 runs of this test did when it
/// dated the hash an hour ahead and rewrote at once), and the settle window is what keeps a rewrite after a
/// reusable hash out of the hash's tick.
#[test]
fn task_2232_a_cached_hash_is_not_reused_for_an_equal_length_rewrite_that_restores_the_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("same-length.bin");
    let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::write(&path, b"AAAA").unwrap();
    regression_2229_set_modified(&path, hour_ago);
    std::thread::sleep(HASH_CACHE_SETTLED + std::time::Duration::from_millis(200));
    assert_eq!(cached_sha256_file_hex("task-2232", &path).unwrap(), sha256_hex(b"AAAA"));
    let reads = HASH_READS.with(std::cell::Cell::get);
    assert_eq!(cached_sha256_file_hex("task-2232", &path).unwrap(), sha256_hex(b"AAAA"));
    assert_eq!(
        HASH_READS.with(std::cell::Cell::get),
        reads,
        "precondition: the cached hash is reused"
    );
    std::fs::write(&path, b"BBBB").unwrap();
    regression_2229_set_modified(&path, hour_ago);
    assert_eq!(
        cached_sha256_file_hex("task-2232", &path).unwrap(),
        sha256_hex(b"BBBB"),
        "the hash of the bytes on disk, not the cached one"
    );
}

// ── Task 2204 S0: a Windows rename re-keys the file's row at once ──
//
// Review-154 P1-A, the case #154 widened: a rename left the row at the OLD path until the
// pull after the rename landed, and a create's landing wrote its queue-time path back. A new
// file written at the old name was then sent as a new version of the renamed file.

/// How the window between the rename and the new file is crossed.
#[derive(Clone, Copy, PartialEq)]
enum Task2204Window {
    /// The renamed file's create is still queued (review-154's scenario).
    NewFile,
    /// The renamed file landed long ago.
    SyncedFile,
    /// Nothing reaches the server until the new file is scanned too.
    Offline,
    /// The app restarts between the rename and the drain.
    Restart,
}

/// Create `a.txt`, rename it to `b.txt`, write a new `a.txt`, sync: the server holds two
/// files with the right bytes, and each row is at its own file's path.
async fn task_2204_rename_then_a_new_file_at_the_old_name(window: Task2204Window) {
    let master_key = [204u8; 32];
    let (dir, sync_root, server, mut bridge) = rule3_setup(master_key);
    let (a, b) = (sync_root.join("a.txt"), sync_root.join("b.txt"));
    std::fs::write(&a, b"old bytes").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "a.txt"),
        1,
        "precondition: queued as a new file"
    );
    if window == Task2204Window::SyncedFile {
        drain_upload_queue(&bridge, &sync_root).await;
        assert_eq!(
            bridge.db.get_file_by_path("a.txt").unwrap().unwrap().status,
            FileStatus::Local
        );
    }
    let renamed_id = bridge.db.get_file_by_path("a.txt").unwrap().unwrap().file_id;
    std::fs::rename(&a, &b).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &a, &b).await;
    assert_eq!(
        bridge.db.get_file_by_path("b.txt").unwrap().map(|row| row.file_id),
        Some(renamed_id.clone()),
        "the row moved with the file, before anything reached the server"
    );
    assert!(
        bridge.db.get_file_by_path("a.txt").unwrap().is_none(),
        "nothing is at the old name"
    );
    if window == Task2204Window::Restart {
        drop(bridge);
        bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        assert!(
            bridge.db.get_file_by_path("b.txt").unwrap().is_some(),
            "the re-key is durable"
        );
    }
    // Another length than the renamed file's, so the scan's size and modification-time
    // fingerprint can never hide a misrouted edit.
    std::fs::write(&a, b"the new file's bytes").unwrap();
    if window == Task2204Window::Offline {
        crate::watcher::run_one_scan(&bridge, &sync_root);
    }
    drain_upload_queue(&bridge, &sync_root).await;
    assert!(
        bridge
            .db
            .get_file_by_path("b.txt")
            .unwrap()
            .is_some_and(|row| row.status == FileStatus::Local),
        "the renamed file landed, and its row stayed at its new path"
    );
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;

    let b_row = bridge
        .db
        .get_file_by_path("b.txt")
        .unwrap()
        .expect("the renamed file has a row");
    let a_row = bridge
        .db
        .get_file_by_path("a.txt")
        .unwrap()
        .expect("the new file has a row");
    assert_ne!(a_row.file_id, b_row.file_id, "two files, not one");
    let state = server.finish();
    assert_eq!(state.files_with_content().len(), 2, "the server holds two files");
    assert_eq!(
        state.files[&b_row.file_id].versions.len(),
        1,
        "the renamed file got no version from the new file"
    );
    assert_eq!(
        state.files[&a_row.file_id].versions.len(),
        1,
        "the new file is new, not a later version of another"
    );
    assert_eq!(state.latest_plaintext(&b_row.file_id, master_key), b"old bytes");
    assert_eq!(
        state.latest_plaintext(&a_row.file_id, master_key),
        b"the new file's bytes"
    );
    assert!(
        state.patches.iter().any(|(id, _)| *id == b_row.file_id),
        "the rename was sent for the renamed file's server id"
    );
}

#[tokio::test]
async fn task_2204_a_new_file_at_a_renamed_new_file_s_old_name_is_a_new_file() {
    task_2204_rename_then_a_new_file_at_the_old_name(Task2204Window::NewFile).await;
}

#[tokio::test]
async fn task_2204_a_new_file_at_a_renamed_synced_file_s_old_name_is_a_new_file() {
    task_2204_rename_then_a_new_file_at_the_old_name(Task2204Window::SyncedFile).await;
}

#[tokio::test]
async fn task_2204_offline_rename_and_new_file_give_two_files() {
    task_2204_rename_then_a_new_file_at_the_old_name(Task2204Window::Offline).await;
}

#[tokio::test]
async fn task_2204_a_rename_survives_a_restart_mid_window() {
    task_2204_rename_then_a_new_file_at_the_old_name(Task2204Window::Restart).await;
}

/// S0.3: while the rename is queued (or parked), a pull that still names the old path
/// leaves the row where the file is; once nothing is queued, the pull applies as before.
#[tokio::test]
async fn task_2204_the_pull_holds_a_renamed_row() {
    let master_key = [205u8; 32];
    let (_dir, sync_root, _server, bridge) = rule3_setup(master_key);
    let (a, b) = (sync_root.join("a.txt"), sync_root.join("b.txt"));
    std::fs::write(&a, b"bytes").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let id = bridge.db.get_file_by_path("a.txt").unwrap().unwrap().file_id;
    std::fs::rename(&a, &b).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &a, &b).await;
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
    let snapshot_row = |name: &str| {
        serde_json::json!({
            "id": id,
            "name_encrypted": beebeeb_core::encrypt::encrypt_name(&mk, &id, name, None).unwrap(),
            "size_bytes": 5,
            "updated_at": now_secs(),
            "is_folder": false,
        })
    };
    let pull = |name: &str| {
        apply_metadata_file_row(
            &bridge.db,
            &snapshot_row(name),
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            now_secs(),
            &master_key,
            "",
        )
        .unwrap()
    };
    pull("a.txt");
    assert_eq!(
        bridge.db.get_file(&id).unwrap().unwrap().path,
        "b.txt",
        "held while the rename is queued"
    );
    let rename = bridge
        .db
        .list_operations_for_file(&id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    bridge
        .db
        .test_execute(&format!(
            "UPDATE operation_queue SET attempts = max_attempts WHERE op_id = '{}'",
            rename.op_id
        ))
        .unwrap();
    pull("a.txt");
    assert_eq!(
        bridge.db.get_file(&id).unwrap().unwrap().path,
        "b.txt",
        "held while it is parked"
    );
    bridge.db.remove_operation(&rename.op_id).unwrap();
    pull("c.txt");
    assert_eq!(
        bridge.db.get_file(&id).unwrap().unwrap().path,
        "c.txt",
        "nothing queued: the pull applies"
    );
}

/// S0.4: a safe save (Excel's: the original renamed to a hex name, the new bytes renamed onto
/// its name, the hex file deleted) is an edit of the original, never a trash plus a new file.
#[tokio::test]
async fn task_2204_an_excel_save_is_an_edit_of_the_file() {
    let master_key = [208u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let (original, hex, temp) = (
        sync_root.join("a.xlsx"),
        sync_root.join("3F2A9B1C"),
        sync_root.join("~tmp1.tmp"),
    );
    std::fs::write(&original, b"version one").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let id = bridge.db.get_file_by_path("a.xlsx").unwrap().unwrap().file_id;
    std::fs::rename(&original, &hex).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &original, &hex).await;
    std::fs::write(&temp, b"version two, longer").unwrap();
    std::fs::rename(&temp, &original).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &temp, &original).await;
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "a.xlsx"),
        0,
        "the new bytes wait while the rename is unsent"
    );
    std::fs::remove_file(&hex).unwrap();
    crate::watcher::handle_delete(&bridge, &sync_root, &hex).await;
    crate::watcher::run_one_scan(&bridge, &sync_root);
    let row = bridge
        .db
        .get_file_by_path("a.xlsx")
        .unwrap()
        .expect("the original's row is back at its name");
    assert_eq!(row.file_id, id);
    let ops = bridge.db.list_operations_for_file(&id).unwrap();
    assert_eq!(
        ops.iter().map(|op| op.kind.clone()).collect::<Vec<_>>(),
        vec![OperationKind::UploadVersion],
        "one edit of the original: no rename, no trash"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert!(state.trashes.is_empty(), "nothing trashed");
    assert_eq!(state.files_with_content(), vec![id.clone()], "one file on the server");
    assert_eq!(
        state.files[&id].versions.len(),
        2,
        "the save is the file's next version"
    );
    assert_eq!(state.latest_plaintext(&id, master_key), b"version two, longer");
}

/// S0.4's limits: a rename then a delete still trashes the file (nothing is at the vacated
/// path), and a manual swap (another tracked file now holds the vacated path, review-128 F3)
/// is a delete of the old one, not a fold.
#[tokio::test]
async fn task_2204_a_delete_after_a_rename_or_a_swap_is_a_delete() {
    let master_key = [209u8; 32];
    let (_dir, sync_root, _server, bridge) = rule3_setup(master_key);
    for name in ["gone.txt", "report.txt", "final.txt"] {
        std::fs::write(sync_root.join(name), name.as_bytes()).unwrap();
    }
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let trashes = |id: &str| {
        bridge
            .db
            .list_operations_for_file(id)
            .unwrap()
            .iter()
            .filter(|op| op.kind == OperationKind::TrashFile)
            .count()
    };
    let id = |path: &str| bridge.db.get_file_by_path(path).unwrap().unwrap().file_id;
    // Rename, then delete.
    let gone = id("gone.txt");
    let (g, g2) = (sync_root.join("gone.txt"), sync_root.join("gone2.txt"));
    std::fs::rename(&g, &g2).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &g, &g2).await;
    std::fs::remove_file(&g2).unwrap();
    crate::watcher::handle_delete(&bridge, &sync_root, &g2).await;
    assert_eq!(trashes(&gone), 1, "a rename then a delete is a delete");
    // A manual swap: report -> old, final -> report, delete old.
    let (report, fin) = (id("report.txt"), id("final.txt"));
    let (r, old, f) = (
        sync_root.join("report.txt"),
        sync_root.join("old.txt"),
        sync_root.join("final.txt"),
    );
    std::fs::rename(&r, &old).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &r, &old).await;
    std::fs::rename(&f, &r).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &f, &r).await;
    std::fs::remove_file(&old).unwrap();
    crate::watcher::handle_delete(&bridge, &sync_root, &old).await;
    assert_eq!(trashes(&report), 1, "the old report is deleted");
    assert_eq!(id("report.txt"), fin, "the final file holds the name");
}

/// A hydrate queued before the rename writes where the file is now, never over a new file
/// at the old name.
#[tokio::test]
async fn task_2204_a_queued_hydrate_follows_the_rename() {
    let master_key = [210u8; 32];
    let (_dir, sync_root, _server, bridge) = rule3_setup(master_key);
    let (a, b) = (sync_root.join("a.txt"), sync_root.join("b.txt"));
    std::fs::write(&a, b"bytes").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let id = bridge.db.get_file_by_path("a.txt").unwrap().unwrap().file_id;
    bridge
        .db
        .test_execute(&format!(
            "INSERT INTO operation_queue (op_id, kind, file_id, target_path, metadata_json, attempts, max_attempts)
                 VALUES ('hydrate-a', 'hydrate_file', '{id}', 'a.txt', '{{}}', 0, 25)"
        ))
        .unwrap();
    std::fs::rename(&a, &b).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &a, &b).await;
    assert_eq!(
        bridge
            .db
            .get_operation("hydrate-a")
            .unwrap()
            .unwrap()
            .target_path
            .as_deref(),
        Some("b.txt")
    );
}

/// Review-155 P1-B: an Excel save of a file synced before task 1640 (no agreed content) is not
/// folded: the new bytes could not be sent as an edit of it, so the delete stays a delete and
/// the new bytes upload as a new file once the rename is sent. Nothing is left unsent.
#[tokio::test]
async fn task_2204_a_safe_save_of_a_file_of_unknown_content_keeps_the_delete() {
    let master_key = [211u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let (original, hex, temp) = (
        sync_root.join("a.xlsx"),
        sync_root.join("3F2A9B1C"),
        sync_root.join("~tmp2.tmp"),
    );
    std::fs::write(&original, b"version one").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let id = bridge.db.get_file_by_path("a.xlsx").unwrap().unwrap().file_id;
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = -1 WHERE file_id = '{id}'"
        ))
        .unwrap();
    std::fs::rename(&original, &hex).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &original, &hex).await;
    std::fs::write(&temp, b"version two, longer").unwrap();
    std::fs::rename(&temp, &original).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &temp, &original).await;
    std::fs::remove_file(&hex).unwrap();
    crate::watcher::handle_delete(&bridge, &sync_root, &hex).await;
    let ops = bridge.db.list_operations_for_file(&id).unwrap();
    assert!(
        ops.iter().any(|op| op.kind == OperationKind::TrashFile),
        "not folded: the delete is queued"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge
        .db
        .get_file_by_path("a.xlsx")
        .unwrap()
        .expect("the saved workbook has a row");
    assert_ne!(row.file_id, id, "a new file");
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext(&row.file_id, master_key),
        b"version two, longer",
        "the save reached the server"
    );
}

/// Review-155 R1: an edit queued before a rename lands before it (its queue-time name first,
/// the rename last), even when the edit is backing off while the rename is due.
#[tokio::test]
async fn task_2204_an_edit_queued_before_a_rename_lands_first() {
    let master_key = [212u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let (a, b) = (sync_root.join("a.txt"), sync_root.join("b.txt"));
    std::fs::write(&a, b"first").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let id = bridge.db.get_file_by_path("a.txt").unwrap().unwrap().file_id;
    std::fs::write(&a, b"an edit of the file").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    server
        .state
        .lock()
        .unwrap()
        .fail_first_chunk_once
        .insert("session-2".into());
    let t0 = now_secs();
    bridge.process_due_operations(&sync_root, t0).await.unwrap();
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "a.txt"),
        1,
        "precondition: the edit backs off (30 s)"
    );
    std::fs::rename(&a, &b).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &a, &b).await;
    bridge
        .process_due_operations(&sync_root, t0 + RENAME_SETTLE_SECS + 1)
        .await
        .unwrap();
    assert!(
        bridge
            .db
            .list_operations_for_file(&id)
            .unwrap()
            .iter()
            .any(|op| op.kind == OperationKind::RenameFile),
        "the rename is due but waits for the edit"
    );
    std::fs::write(&a, b"a new file at the old name").unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let b_row = bridge
        .db
        .get_file_by_path("b.txt")
        .unwrap()
        .expect("the renamed file at its new path");
    let a_row = bridge.db.get_file_by_path("a.txt").unwrap().expect("the new file");
    assert_eq!(b_row.file_id, id);
    let state = server.finish();
    assert_eq!(state.latest_plaintext(&id, master_key), b"an edit of the file");
    assert_eq!(state.files[&id].versions.len(), 2);
    assert_eq!(
        state.latest_plaintext(&a_row.file_id, master_key),
        b"a new file at the old name"
    );
    assert_eq!(state.files[&a_row.file_id].versions.len(), 1);
}

/// Review-155 P2: a delete right after a rename is sent after the rename, never before it.
#[tokio::test]
async fn task_2204_a_trash_after_a_rename_waits_for_it() {
    let master_key = [213u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let (a, b) = (sync_root.join("a.txt"), sync_root.join("b.txt"));
    std::fs::write(&a, b"bytes").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let id = bridge.db.get_file_by_path("a.txt").unwrap().unwrap().file_id;
    std::fs::rename(&a, &b).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &a, &b).await;
    std::fs::remove_file(&b).unwrap();
    crate::watcher::handle_delete(&bridge, &sync_root, &b).await;
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    assert!(
        server.state.lock().unwrap().trashes.is_empty(),
        "the trash waits while the rename settles"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(state.trashes, vec![id.clone()], "then it is sent");
    let rename_at = state
        .requests
        .iter()
        .position(|(m, p)| m == "PATCH" && p.ends_with(&id));
    let trash_at = state
        .requests
        .iter()
        .position(|(m, p)| m == "DELETE" && p.ends_with(&id));
    assert!(
        rename_at.is_some() && rename_at < trash_at,
        "the rename first, then the trash"
    );
}

/// Review-155 P2-1: one malformed op row never stops every classify (the `json_valid` guard).
#[tokio::test]
async fn task_2204_a_malformed_op_does_not_stop_classify() {
    let (_dir, sync_root, _server, bridge) = rule3_setup([214u8; 32]);
    bridge
        .db
        .test_execute(
            "INSERT INTO operation_queue (op_id, kind, file_id, metadata_json, attempts, max_attempts)
                 VALUES ('bad', 'rename_file', 'other', 'not json {', 0, 25)",
        )
        .unwrap();
    let path = sync_root.join("new.txt");
    std::fs::write(&path, b"a new file").unwrap();
    assert!(
        bridge.classify_local_path(&sync_root, &path).is_some(),
        "a new file is still classified"
    );
}

/// S0 keeps today's behaviour for what it does not cover: a folder (its descendants would
/// need re-keying too), a rename onto another tracked file, and a move into a folder with no
/// row (the full design, review-128).
#[tokio::test]
async fn task_2204_folders_replaces_and_unresolved_parents_are_not_rekeyed() {
    let master_key = [207u8; 32];
    let (_dir, sync_root, _server, bridge) = rule3_setup(master_key);
    for name in ["a.txt", "t.txt", "m.txt"] {
        std::fs::write(sync_root.join(name), name.as_bytes()).unwrap();
    }
    crate::watcher::run_one_scan(&bridge, &sync_root);
    drain_upload_queue(&bridge, &sync_root).await;
    let row = |path: &str| bridge.db.get_file_by_path(path).unwrap();
    // A rename onto a tracked file.
    let (a, t) = (sync_root.join("a.txt"), sync_root.join("t.txt"));
    std::fs::remove_file(&t).unwrap();
    std::fs::rename(&a, &t).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &a, &t).await;
    assert!(row("a.txt").is_some(), "a replace keeps today's behaviour (no re-key)");
    // A move into a folder that has no row yet.
    let (m, moved) = (sync_root.join("m.txt"), sync_root.join("newdir").join("m.txt"));
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&m, &moved).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &m, &moved).await;
    assert!(row("m.txt").is_some(), "an unresolved parent keeps today's behaviour");
    // A folder.
    let (f, g) = (sync_root.join("f"), sync_root.join("g"));
    std::fs::create_dir_all(&f).unwrap();
    bridge.ensure_local_folder(&sync_root, &f).expect("a folder row");
    assert!(row("f").unwrap().is_dir(), "precondition: a folder row");
    std::fs::rename(&f, &g).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &f, &g).await;
    assert!(row("f").is_some(), "a folder keeps today's behaviour");
}

// ── Task 2190: a concurrent edit (this device and the web), as main behaves today ──
//
// The file is edited here and on the web before this device syncs, the tick runs, then the
// upload attempt and a hydrate. The tick's detector (`process_metadata_row`, the `Local`
// branch) still takes `local` and `base` from the same `content_hash`, so it never reports a
// conflict (its rewrite is not task 1640's). Since task 1640 the sync folder keeps both
// versions anyway: the edit is queued on its base and parks as stale with its bytes, and no
// hydrate writes over it. Each test says whether both versions survive.

/// One chunk of `plaintext` as the server stores it for `file_id` (the mock serves it back).
pub(super) fn task_2190_server_chunk(master_key: [u8; 32], file_id: &str, plaintext: &[u8]) -> Vec<u8> {
    beebeeb_core::encrypt::encrypt_chunk_raw(
        &beebeeb_core::kdf::derive_file_key(
            &beebeeb_core::kdf::MasterKey::from_bytes(master_key),
            file_id.as_bytes(),
        ),
        plaintext,
    )
    .unwrap()
}

/// The web saves `plaintext` as `file_id`'s next version, and the next snapshot lists it.
pub(super) fn task_2190_web_saves(server: &VersionedServerMock, master_key: [u8; 32], file_id: &str, plaintext: &[u8]) {
    let mut s = server.state.lock().unwrap();
    let chunk = task_2190_server_chunk(master_key, file_id, plaintext);
    let file = s.files.get_mut(file_id).unwrap();
    file.versions.push(vec![chunk]);
    let version = file.versions.len() as i64;
    let mut node = snap_node(&master_key, file_id, "notes.txt", None, false, 1_700_000_500);
    node["version_number"] = json!(version);
    node["size_bytes"] = json!(plaintext.len());
    s.authenticated_names
        .insert(file_id.into(), node["name_encrypted"].as_str().unwrap().into());
    s.snapshots
        .push_back(("200 OK".into(), json!({ "seq_id": 1, "nodes": [node] })));
}

/// Every file under `root` whose bytes are exactly `bytes`.
pub(super) fn task_2190_files_holding(root: &Path, bytes: &[u8]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if std::fs::read(&path).is_ok_and(|held| held == bytes) {
                found.push(path);
            }
        }
    }
    found
}

/// macOS (File Provider): the Mac's save is queued with base 1, the web saves version 2, the
/// tick runs, the upload is refused as stale and parks, and a fetch is served from the parked
/// copy. Both versions survive (the web's on the server, the Mac's in the queue and in
/// Finder). Since follow-up 2 the parked edit is the conflict: its review entry opens it
/// (`open_conflict`), where it had no action before (the pin's name until then:
/// `…_park_as_stale_base_and_never_become_a_conflict`). Still no `Conflict` status (plan E1).
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_a_finder_edit_and_a_web_edit_park_as_stale_base_and_become_a_decidable_conflict() {
    let master_key = [219u8; 32];
    let file = "21900000-0000-4000-8000-000000000001";
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);

    // 1. The Mac saves on version 1. The save is staged and not sent.
    let mac_save = fp_save(&bridge, dir.path(), file, "notes.txt", b"edited on this Mac", "1");
    let mac_token = mac_save.token.clone().expect("a Finder save names its bytes");
    let staged = bridge
        .db
        .list_operations_for_file(file)
        .unwrap()
        .remove(0)
        .payload_path
        .unwrap();

    // 2. The web saves version 2 before the Mac's upload starts.
    task_2190_web_saves(&server, master_key, file, b"edited on the web");

    // 3. The tick.
    let tick = sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert!(tick.conflicts.is_empty(), "the tick reports no conflict");
    let row = bridge.db.get_file(file).unwrap().unwrap();
    assert_eq!(
        row.status,
        FileStatus::Uploading,
        "a row with a queued save skips the detector (branch 3)"
    );
    assert_eq!(
        bridge
            .db
            .get_file_contract_state(file)
            .unwrap()
            .unwrap()
            .current_version,
        2,
        "the snapshot raised the row to the web's version"
    );
    assert_eq!(
        held_content_version(&bridge, file),
        mac_token,
        "Finder is still told the Mac's bytes, so it does not fetch the web's"
    );

    // 4. The upload attempt: base 1 against version 2.
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let parked = bridge.db.list_operations_for_file(file).unwrap().remove(0);
    assert_eq!(parked.attempts, parked.max_attempts, "parked after one attempt");
    assert_eq!(parked.last_error.as_deref(), Some("stale_base"));
    assert_eq!(
        bridge.db.get_file(file).unwrap().unwrap().status,
        FileStatus::Error,
        "the park settles the row to Error, not Conflict"
    );
    assert_eq!(
        held_content_version(&bridge, file),
        mac_token,
        "the park keeps the Mac's token"
    );

    // 5. Finder opens the file: the fetch is served from the parked copy.
    server.state.lock().unwrap().serve_downloads = true;
    let dest = dir.path().join("fetch").join("notes.txt");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    assert_eq!(
        bridge.serve_hydrate(file, &dest, &[dir.path()], None).await.unwrap(),
        HydrateSource::Queue
    );
    assert_eq!(std::fs::read(&dest).unwrap(), b"edited on this Mac");

    // What the person can see: one `stale_base` review entry, no conflict anywhere.
    let feed = bridge.version_conflict_feed().unwrap();
    let kinds: Vec<&str> = feed.iter().map(|entry| entry.kind.as_str()).collect();
    assert_eq!(kinds, vec!["stale_base"]);
    assert_eq!(
        feed[0].action, "open_conflict",
        "the parked edit is a decidable conflict (follow-up 2)"
    );
    assert!(bridge.db.list_by_status(FileStatus::Conflict).unwrap().is_empty());

    // Both versions survive.
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        b"edited on this Mac",
        "the Mac's bytes are kept in the parked op's staged copy"
    );
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!(file), json!(1), 409)],
        "one init, refused as stale"
    );
    assert_eq!(
        state.files[file].versions.len(),
        2,
        "the server holds the web's version as current"
    );
    assert_eq!(state.latest_plaintext(file, master_key), b"edited on the web");
}

/// A hydrated sync-folder file at version 1, then `edit` (when given) written on disk without
/// the upload driver seeing it, then the web's version 2 and the tick (follow-up 1, plan §7.2).
pub(super) async fn task_2190_fu1_tick_after(
    master_key: [u8; 32],
    file: &str,
    edit: Option<&[u8]>,
) -> (
    tempfile::TempDir,
    PathBuf,
    VersionedServerMock,
    EngineBridge,
    FileEntry,
    SyncTickOutcome,
    usize,
) {
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![task_2190_server_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let local = sync_root.join("notes.txt");
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    let before = bridge.db.get_file(file).unwrap().unwrap();
    if let Some(edit) = edit {
        std::fs::write(&local, edit).unwrap();
    }
    task_2190_web_saves(&server, master_key, file, b"edited on the web");
    let reads = HASH_READS.with(std::cell::Cell::get);
    let tick = sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let reads_in_the_pull = HASH_READS.with(std::cell::Cell::get) - reads;
    (dir, sync_root, server, bridge, before, tick, reads_in_the_pull)
}

/// Row 2: an edit the scan has not queued yet, and a newer server version. The tick records
/// the server's version and leaves the edit's base alone; the scan then queues it on base 1,
/// it parks as stale, and both versions survive.
#[tokio::test]
async fn task_2190_fu1_a_concurrent_edit_the_scan_has_not_queued_keeps_its_base_and_parks() {
    let master_key = [221u8; 32];
    let file = "21900000-0000-4000-8000-000000000011";
    let (_dir, sync_root, server, bridge, before, tick, reads_in_the_pull) =
        task_2190_fu1_tick_after(master_key, file, Some(b"edited before the scan")).await;
    assert_eq!(reads_in_the_pull, 0, "the pull reads no file (review-169 #7)");
    assert!(tick.conflicts.is_empty(), "no ConflictDetected (E1)");
    let row = bridge.db.get_file(file).unwrap().unwrap();
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(row.status, FileStatus::Local, "no Conflict status (E1)");
    assert_eq!(contract.current_version, 2, "the server's version is recorded");
    let _ = before;

    let local = sync_root.join("notes.txt");
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "scan"
    ));
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap()[0].base_version,
        Some(1)
    );
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let parked = bridge.db.list_operations_for_file(file).unwrap().remove(0);
    assert_eq!(
        parked.last_error.as_deref(),
        Some("stale_base"),
        "refused as stale and parked"
    );
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"edited before the scan",
        "the edit is on disk"
    );
    drop(bridge);
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext(file, master_key),
        b"edited on the web",
        "the web's version is current"
    );
}
