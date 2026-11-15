//! S6, Linux: a kept file this PC does not hold is fetched again through the queue (`HydrateFile`, reason
//! `kept_refetch`), only into an empty place, and a queued fetch never replaces a file the person made meanwhile. The
//! plan, the batch and the backoff are unit-tested in `engine_bridge/kept_refetch.rs` (every platform); Windows'
//! production fetch is the native test in `windows_cf/stale_2523_native_tests.rs`.
#![cfg(all(not(target_os = "windows"), not(target_os = "macos")))]

use super::reg_1640::regression_1640_chunk;
use super::*;

/// A kept file this PC does not hold: on the server at version 1 (`b"version one"`), its row `cloud_only` and pinned,
/// of unknown content (never fetched on this PC), nothing at its place. Returns the place.
fn kept_cloud_only(
    master_key: [u8; 32],
    file: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    VersionedServerMock,
    Arc<EngineBridge>,
    PathBuf,
) {
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![regression_1640_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    bridge.db.set_status(file, FileStatus::CloudOnly).unwrap();
    bridge.db.set_recursive_pin(file, true, 1).unwrap();
    let place = sync_root.join("notes.txt");
    (dir, sync_root, server, Arc::new(bridge), place)
}

fn requests(server: &VersionedServerMock) -> usize {
    server.state.lock().unwrap().requests.len()
}

/// R4, R5: the step queues ONE fetch into the file's place and downloads nothing itself; a second step queues nothing
/// more; the queue brings the file.
#[tokio::test(flavor = "multi_thread")]
async fn s6_kept_linux_queues_one_fetch_and_the_queue_brings_it() {
    let master_key = [161u8; 32];
    let file = "56100000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, place) = kept_cloud_only(master_key, file);
    let state = Arc::new(KeptRefetch::default());
    let before = requests(&server);
    crate::engine_bridge::queue_kept_step(&bridge, &sync_root, &state).await;
    let during = requests(&server) - before;
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    println!(
        "s6 kept linux: requests during the step={during} ops={:?}",
        ops.iter()
            .map(|op| (op.kind.clone(), op.target_path.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(during, 0, "the step itself downloads nothing");
    assert_eq!(ops.len(), 1, "one fetch queued");
    assert_eq!(ops[0].kind, OperationKind::HydrateFile);
    assert_eq!(
        ops[0].target_path.as_deref(),
        Some("notes.txt"),
        "into the file's place"
    );
    assert_eq!(operation_metadata(&ops[0]).unwrap()["reason"], "kept_refetch");
    crate::engine_bridge::queue_kept_step(&bridge, &sync_root, &state).await;
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap().len(),
        1,
        "a second step queues nothing more"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file(file).unwrap().unwrap();
    drop(server.finish());
    assert_eq!(std::fs::read(&place).unwrap(), b"version one", "the queue brought it");
    assert_eq!(row.status, FileStatus::Local);
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
}

/// R7 (a): a place that holds anything, an empty file included, is the person's: nothing is queued into it.
#[tokio::test]
async fn s6_kept_linux_never_queues_into_a_place_that_holds_a_file() {
    let master_key = [162u8; 32];
    let file = "56200000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, place) = kept_cloud_only(master_key, file);
    for bytes in [&b"the person's own file"[..], b""] {
        std::fs::write(&place, bytes).unwrap();
        let queued = bridge.queue_kept_fetches(&sync_root, &KeptRefetch::default()).unwrap();
        let ops = bridge.db.list_operations_for_file(file).unwrap();
        println!(
            "s6 kept linux, a file of {} bytes there: queued={queued} ops={}",
            bytes.len(),
            ops.len()
        );
        assert_eq!(queued, 0);
        assert!(ops.is_empty(), "nothing queued into a place that holds a file");
        assert_eq!(std::fs::read(&place).unwrap(), bytes, "the file untouched");
    }
    drop(server.finish());
}

/// R7 (b), (c): the first fetch fails (the server refuses), so the row reads `error` and the op waits for its retry.
/// The person makes a file at the place. The retry is refused (a queued hydrate never takes the person's-resolution
/// exemption), the file keeps its bytes, and the fetch is dropped, never retried over it.
#[tokio::test]
async fn s6_kept_linux_a_retry_never_replaces_a_file_made_after_a_failed_fetch() {
    let master_key = [163u8; 32];
    let file = "56300000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, place) = kept_cloud_only(master_key, file);
    server.state.lock().unwrap().serve_downloads = false;
    assert_eq!(
        bridge.queue_kept_fetches(&sync_root, &KeptRefetch::default()).unwrap(),
        1
    );
    let now = now_secs();
    bridge.process_due_operations(&sync_root, now).await.unwrap();
    let row = bridge.db.get_file(file).unwrap().unwrap();
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    println!(
        "s6 kept linux, attempt 1: status={:?} ops={} attempts={:?}",
        row.status,
        ops.len(),
        ops.first().map(|op| op.attempts)
    );
    assert_eq!(row.status, FileStatus::Error, "the failed attempt leaves the row error");
    assert_eq!(ops.len(), 1, "the op waits for its retry");
    server.state.lock().unwrap().serve_downloads = true;
    std::fs::write(&place, b"made by the person after the failed fetch").unwrap();
    bridge.process_due_operations(&sync_root, now + 10_000).await.unwrap();
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    drop(server.finish());
    println!("s6 kept linux, attempt 2: ops={}", ops.len());
    assert_eq!(
        std::fs::read(&place).unwrap(),
        b"made by the person after the failed fetch",
        "the file keeps its bytes"
    );
    assert!(ops.is_empty(), "the fetch is dropped, never retried over it");
}

/// R7 (d): a kept fetch whose row moved (by a path write that does not move queued ops, made here directly) is
/// dropped; the file at the old name keeps its bytes and nothing is written at the new one.
#[tokio::test]
async fn s6_kept_linux_a_fetch_whose_row_moved_is_dropped() {
    let master_key = [164u8; 32];
    let file = "56400000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, place) = kept_cloud_only(master_key, file);
    assert_eq!(
        bridge.queue_kept_fetches(&sync_root, &KeptRefetch::default()).unwrap(),
        1
    );
    bridge
        .db
        .test_execute(&format!("UPDATE files SET path = 'moved.txt' WHERE file_id = '{file}'"))
        .unwrap();
    std::fs::write(&place, b"the person's file at the old name").unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    drop(server.finish());
    println!(
        "s6 kept linux, moved: ops={} new={}",
        ops.len(),
        sync_root.join("moved.txt").exists()
    );
    assert_eq!(
        std::fs::read(&place).unwrap(),
        b"the person's file at the old name",
        "the file at the old name keeps its bytes"
    );
    assert!(ops.is_empty(), "the fetch is dropped, not retried");
    assert!(!sync_root.join("moved.txt").exists(), "nothing written");
}
