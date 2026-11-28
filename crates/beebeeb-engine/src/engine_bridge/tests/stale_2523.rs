//! Task 2523: another device's version reaches the copy on this PC (F1); a hydrate never writes over a held edit
//! (F3); a viewer save leaves no stale copy that an edit cannot be sent from (F2). The trace's two "wanted" tests
//! are here as real tests. The Linux half of F1 runs on Linux; the Windows half is the
//! native test in `windows_cf/ld0105_native_tests.rs` and the order's unit test in `engine_bridge/stale_copies.rs`.

use super::reg_1640::regression_1640_synced_file;
#[cfg(not(target_os = "windows"))]
use super::reg_1640::regression_1640_web_saves;
use super::*;

/// The kinds of the review feed's entries.
fn feed_kinds(bridge: &EngineBridge) -> Vec<String> {
    bridge
        .version_conflict_feed()
        .unwrap()
        .into_iter()
        .map(|entry| entry.kind)
        .collect()
}

/// Q1, wanted (the trace's red): another device's newer version of a file on this PC reaches the copy in the sync
/// folder. Today the pull raised the row only and the copy kept version 1, marked agreed, with no time limit. The
/// trace's probe drove the pull and a queue drain; the pass is the runner's step between them (one added line).
/// Linux: the pass queues ONE re-download into the file's place, and the refreshed row leaves the rule's set.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_q1_wanted_a_pulled_version_reaches_the_copy_on_this_pc() {
    let master_key = [233u8; 32];
    let file = "23300000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let pass = bridge.refresh_stale_copies(&sync_root).unwrap();
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    println!(
        "stale-2523 q1: queued={} changed={} later={} ops={:?}",
        pass.queued.len(),
        pass.changed,
        pass.later,
        ops.iter().map(|op| op.kind.clone()).collect::<Vec<_>>()
    );
    assert_eq!(pass.queued, vec![file.to_string()], "one re-download queued");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].kind, OperationKind::HydrateFile);
    assert_eq!(
        ops[0].target_path.as_deref(),
        Some("notes.txt"),
        "into the file's own place"
    );
    assert_eq!(
        bridge.refresh_stale_copies(&sync_root).unwrap(),
        StaleCopyPass::default(),
        "a second pass before the queue ran queues nothing more"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    let row = bridge.db.get_file(file).unwrap().unwrap();
    let on_disk = std::fs::read(&local).unwrap();
    println!(
        "stale-2523 q1 after the drain: current={} local_base={} status={:?} on_disk={:?}",
        contract.current_version,
        contract.local_base_version,
        row.status,
        String::from_utf8_lossy(&on_disk)
    );
    drop(server.finish());
    assert!(
        on_disk == b"edited on the web" || row.status != FileStatus::Local,
        "the copy on this PC still holds version 1 while the row says version {}",
        contract.current_version
    );
    assert_eq!(
        on_disk, b"edited on the web",
        "Linux: the copy holds the current version"
    );
    assert_eq!(
        (contract.current_version, contract.local_base_version),
        (2, 2),
        "agreed at the current version"
    );
    assert_eq!(
        contract.local_hash.as_deref(),
        Some(sha256_hex(b"edited on the web").as_str())
    );
    assert_eq!(row.status, FileStatus::Local);
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
    assert_eq!(
        bridge.refresh_stale_copies(&sync_root).unwrap(),
        StaleCopyPass::default(),
        "the refreshed copy leaves the rule's set: no re-download every tick"
    );
}

/// A copy with a change is never replaced: the pass leaves its bytes, queues nothing, and the change, once seen, is
/// sent on version 1 and parked as the two-versions question (as before), with both versions kept.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_a_copy_with_a_change_is_left_alone_and_parks_as_before() {
    let master_key = [234u8; 32];
    let file = "23400000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    std::fs::write(&local, b"edited on this PC, not seen yet").unwrap();
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let pass = bridge.refresh_stale_copies(&sync_root).unwrap();
    println!(
        "stale-2523 changed: queued={} changed={} later={}",
        pass.queued.len(),
        pass.changed,
        pass.later
    );
    assert_eq!(
        (pass.queued.len(), pass.changed),
        (0, 1),
        "seen as a change, nothing queued"
    );
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"edited on this PC, not seen yet",
        "its bytes are kept"
    );
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "scan"
    ));
    drain_upload_queue(&bridge, &sync_root).await;
    let kinds = feed_kinds(&bridge);
    let state = server.finish();
    println!("stale-2523 changed: kinds={kinds:?} inits={:?}", state.init_summary());
    assert_eq!(kinds, vec!["stale_base".to_string()], "the two-versions question");
    assert_eq!(state.latest_plaintext(file, master_key), b"edited on the web");
    assert_eq!(std::fs::read(&local).unwrap(), b"edited on this PC, not seen yet");
}

/// A copy with a change is reported once: the next tick's pass neither counts it again nor reads its bytes again
/// (the log said `changed=2` every 30 s for ever), until the copy itself changes.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_copies_a_change_seen_once_is_not_counted_or_read_again_on_the_next_tick() {
    let master_key = [235u8; 32];
    let file = "23500000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    std::fs::write(&local, b"edited on this PC, not seen yet").unwrap();
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let first = bridge.refresh_stale_copies(&sync_root).unwrap();
    assert_eq!(first.changed, 1, "seen once");
    let second = bridge.refresh_stale_copies(&sync_root).unwrap();
    assert_eq!(
        (second.changed, second.later),
        (0, 0),
        "a settled change is not counted again"
    );
    std::fs::write(&local, b"edited again, longer than before").unwrap();
    let third = bridge.refresh_stale_copies(&sync_root).unwrap();
    assert_eq!(third.changed, 1, "a copy that changed again is looked at again");
    drop(server);
}

/// The hydrate the pass queued meets a change made after the pass checked the bytes, before the download started:
/// refused, the change keeps its bytes, and the re-download is dropped (never retried over it).
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_an_edit_after_the_pass_before_the_redownload_is_kept() {
    let master_key = [235u8; 32];
    let file = "23500000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 1);
    std::fs::write(&local, b"edited after the pass").unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    let row = bridge.db.get_file(file).unwrap().unwrap();
    drop(server.finish());
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"edited after the pass",
        "the change is kept"
    );
    assert!(
        bridge.db.list_operations_for_file(file).unwrap().is_empty(),
        "the re-download is dropped, not retried"
    );
    assert_eq!(
        contract.local_base_version, 1,
        "still agreed at version 1: the change is sent on it"
    );
    assert_eq!(row.status, FileStatus::Local, "not an error");
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 0);
}

/// The same, with the change made DURING the download (a seam right before the place is replaced): the hydrate asks
/// again there, so the downloaded version never replaces it. Before, the guard ran only before the download.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_an_edit_during_the_redownload_is_kept() {
    let master_key = [236u8; 32];
    let file = "23600000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 1);
    let place = local.clone();
    bridge.seams.arm("hydrate:before_write", move || {
        std::fs::write(&place, b"edited during the download").unwrap();
    });
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file(file).unwrap().unwrap();
    drop(server.finish());
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"edited during the download",
        "the change made during the download is kept"
    );
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
    assert_eq!(row.status, FileStatus::Local, "the row is back to what it was");
}

/// F3: an edit the engine holds as "content unknown" (a file with no agreed content and nothing fetched, outside the
/// population hydrated before 0.8.13) is never written over by a hydrate into its place. Before, only that
/// population was guarded, and F1's re-download into a `Local` file's place could have reached such an edit.
#[tokio::test]
async fn stale_2523_f3_a_held_edit_is_never_hydrated_over() {
    let master_key = [237u8; 32];
    let file = "23700000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = 0, fetched_hash = NULL WHERE file_id = '{file}'"
        ))
        .unwrap();
    std::fs::write(&local, b"held: content unknown").unwrap();
    assert_eq!(
        sync_folder_content(&bridge.db, file, &local, None).unwrap(),
        SyncFolderContent::Unknown,
        "precondition: content unknown"
    );
    assert!(
        !bridge.db.is_unknown_content(file).unwrap(),
        "precondition: not the pre-0.8.13 population"
    );
    let hydrated = bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await;
    drop(server.finish());
    println!("stale-2523 f3: hydrate={hydrated:?}");
    assert!(
        matches!(&hydrated, Err(e) if engine_cause::<UnsentLocalChange>(e).is_some()),
        "refused: {hydrated:?}"
    );
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"held: content unknown",
        "the held edit keeps its bytes"
    );
}

/// Q2, wanted (the trace's red): after a save from outside the sync folder (the in-app viewer's write-keyed save on
/// Windows and Linux; saved here through the same write-keyed path), an edit of the sync folder's older copy becomes
/// the two-versions question (sent on version 1, the version its bytes are, refused as stale, parked), never a held
/// "from before 0.8.13" row that nothing ever sends. macOS: a File Provider save IS the system's copy.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn stale_2523_q2_wanted_an_edit_of_the_copy_left_by_a_viewer_save_is_a_decidable_conflict() {
    let master_key = [232u8; 32];
    let file = "23200000-0000-4000-8000-000000000001";
    let (dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    fp_save(&bridge, dir.path(), file, "notes.txt", b"saved in the viewer", "1");
    drain_upload_queue(&bridge, &sync_root).await;
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    println!(
        "stale-2523 q2 after the landing: current={} local_base={} local_hash_is_v1={}",
        contract.current_version,
        contract.local_base_version,
        contract.local_hash.as_deref() == Some(sha256_hex(b"version one").as_str())
    );
    assert_eq!(
        (contract.current_version, contract.local_base_version),
        (2, 1),
        "the folder's copy stays agreed at version 1, behind version 2"
    );
    std::fs::write(&local, b"version one, edited in another app").unwrap();
    crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "notify");
    drain_upload_queue(&bridge, &sync_root).await;
    let kinds = feed_kinds(&bridge);
    let state = server.finish();
    println!("stale-2523 q2-wanted kinds={kinds:?} inits={:?}", state.init_summary());
    assert_eq!(state.latest_plaintext(file, master_key), b"saved in the viewer");
    assert_eq!(
        kinds,
        vec!["stale_base".to_string()],
        "the person gets the two-versions question"
    );
    assert_eq!(std::fs::read(&local).unwrap(), b"version one, edited in another app");
}

/// F2 with F1 (Linux): after the viewer's save, the next pass brings the folder's copy to the saved version, so
/// Explorer and every other app read what the person saved in Beebeeb.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_after_a_viewer_save_the_folder_copy_is_brought_to_the_saved_version() {
    let master_key = [238u8; 32];
    let file = "23800000-0000-4000-8000-000000000001";
    let (dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    fp_save(&bridge, dir.path(), file, "notes.txt", b"saved in the viewer", "1");
    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 1);
    drain_upload_queue(&bridge, &sync_root).await;
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    drop(server.finish());
    assert_eq!(std::fs::read(&local).unwrap(), b"saved in the viewer");
    assert_eq!((contract.current_version, contract.local_base_version), (2, 2));
}

/// Another device renames the file to `name` (still version 2), seen by the next pull.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn web_renames(server: &VersionedServerMock, master_key: [u8; 32], file_id: &str, name: &str) {
    let mut s = server.state.lock().unwrap();
    let mut node = snap_node(&master_key, file_id, name, None, false, 1_700_000_600);
    node["version_number"] = json!(2);
    node["size_bytes"] = json!(b"edited on the web".len());
    s.authenticated_names
        .insert(file_id.into(), node["name_encrypted"].as_str().unwrap().into());
    s.snapshots
        .push_back(("200 OK".into(), json!({ "seq_id": 2, "nodes": [node] })));
}

/// Review P1-1 (Linux): a re-download queued before another device renamed the file never writes at the old name.
/// The engine stops during the download (a seam before the write cancels it; the engine's own start-up repair runs),
/// the next pull moves the row to `notes-final.txt` (the copy here stays `notes.txt`), and the person edits
/// `notes.txt`. Before, the queued op still named `notes.txt`, which was no longer the row's place, so neither hydrate
/// guard looked, and version 2 replaced the edit. Now the pull moves the op with its row (fix a), and a re-download
/// whose destination is not the row's place is dropped (fix c, its own test below).
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_a_redownload_queued_before_a_remote_rename_never_writes_at_the_old_name() {
    use futures_util::FutureExt as _;
    let master_key = [239u8; 32];
    let file = "23900000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 1);
    bridge.seams.arm("hydrate:before_write", || {
        panic!("stale-2523: the engine stops during the re-download")
    });
    let stopped = std::panic::AssertUnwindSafe(bridge.process_due_operations(&sync_root, now_secs()))
        .catch_unwind()
        .await;
    assert!(stopped.is_err(), "the re-download was cut off before the write");
    assert_eq!(std::fs::read(&local).unwrap(), b"version one", "nothing written yet");
    bridge.db.engine_start_repair().unwrap();
    bridge.db.reconcile_stale_in_flight_on_startup().unwrap();
    web_renames(&server, master_key, file, "notes-final.txt");
    // The mock answers `/sync/ops` with no ops: the rename reaches this pull as a snapshot (as after a restart's
    // re-snapshot), through the same row write a `file_rename` op takes.
    bridge.db.request_resnapshot().unwrap();
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let row = bridge.db.get_file(file).unwrap().unwrap();
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    println!(
        "stale-2523 p1 before the drain: path={} status={:?} ops={:?}",
        row.path,
        row.status,
        ops.iter().map(|op| op.target_path.clone()).collect::<Vec<_>>()
    );
    assert_eq!(row.path, "notes-final.txt", "precondition: the pull moved the row");
    assert_eq!(ops.len(), 1, "precondition: the re-download is still queued");
    std::fs::write(&local, b"edited at the old name").unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    let renamed = sync_root.join("notes-final.txt");
    let at_new_name = std::fs::read(&renamed).ok();
    drop(server.finish());
    println!(
        "stale-2523 p1 after the drain: old={:?} new={:?}",
        String::from_utf8_lossy(&std::fs::read(&local).unwrap()),
        at_new_name.as_deref().map(String::from_utf8_lossy)
    );
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"edited at the old name",
        "the edit at the old name is kept"
    );
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
    assert_eq!(
        at_new_name.as_deref(),
        Some(&b"edited on the web"[..]),
        "fix (a): the re-download moved with its row and reached the file's new place"
    );
}

/// Review P1-1, fix (c) on its own: a re-download whose destination the row has left by a path write that does not
/// move queued ops (made here directly, so fix (a) cannot repair it) is dropped, and the file at the old name keeps
/// its bytes.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_a_redownload_whose_file_moved_is_dropped() {
    let master_key = [240u8; 32];
    let file = "24000000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 1);
    bridge
        .db
        .test_execute(&format!("UPDATE files SET path = 'moved.txt' WHERE file_id = '{file}'"))
        .unwrap();
    std::fs::write(&local, b"edited at the old name").unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    drop(server.finish());
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"edited at the old name",
        "the file at the old name keeps its bytes"
    );
    assert!(
        bridge.db.list_operations_for_file(file).unwrap().is_empty(),
        "the re-download is dropped, not retried"
    );
    assert!(!sync_root.join("moved.txt").exists(), "nothing written");
}

/// Review P2-1 (Linux): a save made while the downloaded copy is being written (a seam after the copy is written,
/// right before it replaces the place) is never replaced. Before, the last look came before the copy was written.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[tokio::test]
async fn stale_2523_a_save_while_the_redownload_is_written_is_kept() {
    let master_key = [241u8; 32];
    let file = "24100000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(bridge.refresh_stale_copies(&sync_root).unwrap().queued.len(), 1);
    let place = local.clone();
    bridge.seams.arm("hydrate:before_replace", move || {
        std::fs::write(&place, b"saved while the download was written").unwrap();
    });
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file(file).unwrap().unwrap();
    drop(server.finish());
    let left: Vec<String> = std::fs::read_dir(&sync_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    println!("stale-2523 p2-1: folder={left:?} status={:?}", row.status);
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"saved while the download was written",
        "the save is kept"
    );
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
    assert_eq!(row.status, FileStatus::Local, "the row is back to what it was");
    assert_eq!(left, vec!["notes.txt".to_string()], "no written copy is left behind");
}

/// Dm-72, the Mac's arm (run on every OS by the Mac rule): another device's version of a kept file reaches a Mac
/// without a fetch of the engine's own. The pass signals the File Provider once and queues nothing; nothing comes
/// down and nothing is written into the state folder; the system's fetch of Finder's copy is the ONE download, a
/// second Finder fetch started beside it copies it, and the row then leaves the rule's set (no signal every tick).
#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn dm72_on_a_mac_a_kept_files_new_version_signals_finder_and_comes_down_once() {
    use crate::file_provider_port::{FinderCopiesPort, with_finder_copies};
    let master_key = [72u8; 32];
    let file = "72000000-0000-4000-8000-000000000001";
    let (dir, sync_root, server, bridge, hidden) = regression_1640_synced_file(master_key, file).await;
    bridge.db.set_recursive_pin(file, true, 1).unwrap();
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let chunk_downloads = |server: &VersionedServerMock| {
        let state = server.state.lock().unwrap();
        let chunks = state
            .requests
            .iter()
            .filter(|(method, path)| method == "GET" && path.contains("/chunks/"));
        chunks.count()
    };
    let before = chunk_downloads(&server);
    let finder = Arc::new(FinderCopiesPort::default());
    let mac_pass = || {
        system_fetch_for_test(true);
        let pass = with_finder_copies(finder.clone(), || bridge.refresh_stale_copies(&sync_root));
        system_fetch_for_test(false);
        pass.unwrap()
    };

    let pass = mac_pass();
    println!(
        "dm-72 pass: signalled={:?} queued={:?} signals={}",
        pass.signalled,
        pass.queued,
        finder.signals()
    );
    assert_eq!(
        pass.signalled,
        vec![file.to_string()],
        "the File Provider is signalled for the item"
    );
    assert!(pass.queued.is_empty() && pass.emptied.is_empty() && pass.refetch.is_empty());
    assert_eq!(finder.signals(), 1, "one working-set signal");
    assert!(
        bridge.db.list_operations_for_file(file).unwrap().is_empty(),
        "no engine fetch is queued"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(chunk_downloads(&server), before, "the engine downloads nothing itself");
    assert_eq!(
        std::fs::read(&hidden).unwrap(),
        b"version one",
        "nothing is written into the state folder"
    );
    assert_eq!(mac_pass(), StaleCopyPass::default(), "signalled once for this version");
    assert_eq!(finder.signals(), 1);

    // The system's fetch of Finder's copy, and Finder opening the file while it runs: one download.
    let staging = dir.path().join("hydrate-cache");
    std::fs::create_dir_all(&staging).unwrap();
    let (system, opened) = (staging.join("system.txt"), staging.join("opened.txt"));
    let roots = [staging.as_path()];
    system_fetch_for_test(true);
    let (first, second) = tokio::join!(
        bridge.serve_hydrate(file, &system, &roots, None),
        bridge.serve_hydrate(file, &opened, &roots, None),
    );
    system_fetch_for_test(false);
    assert_eq!(
        (first.unwrap(), second.unwrap()),
        (HydrateSource::Server, HydrateSource::Copy)
    );
    assert_eq!(chunk_downloads(&server) - before, 1, "one download of the new version");
    assert_eq!(std::fs::read(&system).unwrap(), b"edited on the web");
    assert_eq!(std::fs::read(&opened).unwrap(), b"edited on the web");
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        (contract.current_version, contract.local_base_version),
        (2, 2),
        "agreed at the current version"
    );
    assert_eq!(
        contract.local_hash.as_deref(),
        Some(sha256_hex(b"edited on the web").as_str()),
        "Finder's copy is recorded as the current version's content"
    );
    assert_eq!(bridge.db.get_file(file).unwrap().unwrap().status, FileStatus::Local);
    assert_eq!(
        mac_pass(),
        StaleCopyPass::default(),
        "the fetched copy leaves the rule's set"
    );
    assert_eq!(finder.signals(), 1, "no signal every tick");
    drop(server.finish());
}

/// Dm-72: a busy bridge gate signals nothing and remembers nothing, so the next tick's pass asks again.
#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn dm72_on_a_mac_a_busy_gate_leaves_the_signal_to_the_next_pass() {
    use crate::file_provider_port::{BusyGatePort, FinderCopiesPort, with_finder_copies, with_port};
    let master_key = [73u8; 32];
    let file = "72000000-0000-4000-8000-000000000002";
    let (_dir, sync_root, server, bridge, _hidden) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    system_fetch_for_test(true);
    let busy = Arc::new(BusyGatePort::default());
    let met_busy = with_port(busy.clone(), || bridge.refresh_stale_copies(&sync_root));
    let finder = Arc::new(FinderCopiesPort::default());
    let next = with_finder_copies(finder.clone(), || bridge.refresh_stale_copies(&sync_root));
    system_fetch_for_test(false);
    let (met_busy, next) = (met_busy.unwrap(), next.unwrap());
    assert_eq!((busy.asked(), met_busy.signalled.len(), met_busy.later), (1, 0, 1));
    assert_eq!(next.signalled, vec![file.to_string()], "the next pass signals");
    assert_eq!(finder.signals(), 1);
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
    drop(server.finish());
}
