//! Task 2523: another device's version reaches the copy on this PC (F1); a hydrate never writes over a held edit
//! (F3); a viewer save leaves no stale copy that an edit cannot be sent from (F2). The trace's two "wanted" tests
//! (lane tracestale, 2026-11-15) are here as real tests. The Linux half of F1 runs on Linux; the Windows half is the
//! native test in `windows_cf/ld0105_native_tests.rs` and the order's unit test in `engine_bridge/stale_copies.rs`.

use super::reg_1640::regression_1640_synced_file;
use super::*;

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
