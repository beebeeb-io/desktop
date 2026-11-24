//! Task 2538 (engine part): a version restored on another device reaches this PC with its version number, and a
//! versionless update never stamps a row's server time from this PC's clock. Driven through the functions the tick
//! runs (`apply_sync_op`, `apply_snapshot`), with no server. Ledger 2468 L409: Keep there into the file's own place
//! replaces an edited file whose agreed content is recorded (the hydrate's in-place guard), its copy kept first; a
//! hydrate that keeps no copy (a plain one, the Tauri-era Keep there) is still refused. The hydrate-level tests reach no
//! server either: the address answers nothing, so a hydrate let past the guard fails at its first request.

use super::*;

const MASTER_KEY: [u8; 32] = [38u8; 32];
/// The server times the rows carry: far below this PC's clock, as a restore's time is below the second a versionless op
/// stamped from this PC's clock before task 2538.
const SEEDED_AT: i64 = 1_000;
const RESTORED_AT: i64 = 2_000;
const VERSION_TWO: &[u8] = b"version two";

/// A bridge on a fresh state database in `dir`. Its server address is never reached: nothing here makes a request.
fn bridge_in(dir: &Path) -> EngineBridge {
    let db = Arc::new(StateDb::open(dir.join("state.db")).unwrap());
    let api = Arc::new(ApiClient::new("http://127.0.0.1:9".into(), "token".into(), MASTER_KEY));
    EngineBridge::new(&crate::identity::TEST, db, api)
}

/// The name as a server row carries it, sealed as `tests/sync.rs`'s `enc_name` seals it (a fixture, never a write).
#[allow(deprecated)]
fn sealed_name(file_id: &str, name: &str) -> String {
    let master = beebeeb_core::kdf::MasterKey::from_bytes(MASTER_KEY);
    beebeeb_core::encrypt::encrypt_name(&master, file_id, name, None).unwrap()
}

/// A file at the top level, as the snapshot lists it.
fn node(file_id: &str, name: &str, version: i64, size: i64, updated_at: i64) -> serde_json::Value {
    serde_json::json!({
        "id": file_id,
        "parent_id": serde_json::Value::Null,
        "name_encrypted": sealed_name(file_id, name),
        "size_bytes": size,
        "is_folder": false,
        "content_hash": serde_json::Value::Null,
        "version_number": version,
        "is_trashed": false,
        "is_starred": false,
        "updated_at": updated_at,
    })
}

fn snapshot_of(nodes: Vec<serde_json::Value>) -> crate::api_client::SyncSnapshot {
    crate::api_client::SyncSnapshot { seq_id: 90, nodes }
}

fn op(op_type: &str, payload: serde_json::Value) -> crate::api_client::SyncOp {
    crate::api_client::SyncOp {
        seq_id: 70,
        op_type: op_type.to_string(),
        payload,
    }
}

fn rename_op(file_id: &str, new_name: &str) -> crate::api_client::SyncOp {
    op(
        "file_rename",
        serde_json::json!({ "id": file_id, "new_name_encrypted": sealed_name(file_id, new_name) }),
    )
}

/// A copy on this PC in sync at `version`: a `Local` row at the server time `updated_at`, its agreed content (`bytes`)
/// recorded at that version.
fn seed_synced_copy(bridge: &EngineBridge, file_id: &str, name: &str, version: i64, updated_at: i64, bytes: &[u8]) {
    let row = node(file_id, name, version, bytes.len() as i64, updated_at);
    apply_metadata_file_row(
        bridge.db(),
        &row,
        Namespace::MyFiles,
        None,
        None,
        PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
        updated_at,
        &MASTER_KEY,
        "",
    )
    .unwrap()
    .expect("the row is written");
    bridge.db().set_status(file_id, FileStatus::Local).unwrap();
    bridge
        .db()
        .record_local_baseline(file_id, version, &sha256_hex(bytes))
        .unwrap();
}

/// The files #447's refresh pass would bring to their current version (the rule's one query).
fn refresh_due(bridge: &EngineBridge) -> Vec<String> {
    bridge
        .db()
        .list_stale_agreed_copies(10)
        .unwrap()
        .into_iter()
        .map(|copy| copy.file_id)
        .collect()
}

/// (a) A version is restored on another device (version 1 comes back as version 3, at server time `RESTORED_AT`).
/// The next update this PC reads names no version (a rename), then the snapshot lists the restored version. After
/// the pass the row holds the restored version, the copy on this PC is due for #447's refresh, and #450's preview run
/// takes no source from the old bytes. Mutant M1 (`synthesize_op_row` stamps `now_secs()` again): red at the rename.
#[test]
fn a_restore_elsewhere_then_a_rename_refreshes_the_copy_and_sends_no_preview() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-00000000000a";
    seed_synced_copy(&bridge, file, "beach.jpg", 2, SEEDED_AT, VERSION_TWO);
    let mut conflicts = Vec::new();
    let now = now_secs();

    apply_sync_op(&bridge, &sync_root, &rename_op(file, "sea.jpg"), now, &mut conflicts).unwrap();
    let renamed = bridge.db().get_file(file).unwrap().unwrap();
    println!(
        "restore_elsewhere (a) after the rename: path={} remote_updated_at={}",
        renamed.path, renamed.remote_updated_at
    );
    assert_eq!(renamed.path, "sea.jpg", "the rename landed");
    assert_eq!(
        renamed.remote_updated_at, SEEDED_AT,
        "a versionless update keeps the row's server time, never this PC's clock"
    );

    // The restored version has the same size as version two: only its version number tells it apart.
    let restored = node(file, "sea.jpg", 3, VERSION_TWO.len() as i64, RESTORED_AT);
    apply_snapshot(
        &bridge,
        &sync_root,
        &snapshot_of(vec![restored]),
        now,
        now,
        &mut conflicts,
    )
    .unwrap();

    let contract = bridge.db().get_file_contract_state(file).unwrap().unwrap();
    let due = refresh_due(&bridge);
    let photos = crate::photo_previews::plan::local_photos(bridge.db()).expect("the local state reads");
    let photo = photos
        .iter()
        .find(|photo| photo.id == file)
        .expect("the photo is listed");
    println!(
        "restore_elsewhere (a) after the snapshot: current={} local_base={} due={} behind={}",
        contract.current_version,
        contract.local_base_version,
        due.len(),
        photo.behind
    );
    assert_eq!(
        (contract.current_version, contract.local_base_version),
        (3, 2),
        "the row holds the restored version; the copy is still version two's"
    );
    assert_eq!(due, vec![file.to_string()], "#447's refresh is due for the copy");
    assert!(photo.behind, "#450's preview run takes no source from the old bytes");
    assert!(conflicts.is_empty());
}

/// (b) A row whose server time an earlier build stamped from this PC's clock (a versionless update before task 2538):
/// the snapshot naming a later version re-anchors it, though its time is before that stamp. Mutant M2 (the version
/// clause of the `Local` branch deleted, the time rule alone): red.
#[test]
fn a_snapshot_with_a_newer_version_re_anchors_a_row_a_versionless_op_stamped() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-00000000000b";
    let stamped = now_secs();
    seed_synced_copy(&bridge, file, "notes.txt", 2, stamped, VERSION_TWO);
    let mut conflicts = Vec::new();

    let restored = node(file, "notes.txt", 3, 14, RESTORED_AT);
    apply_snapshot(
        &bridge,
        &sync_root,
        &snapshot_of(vec![restored]),
        stamped,
        stamped,
        &mut conflicts,
    )
    .unwrap();

    let contract = bridge.db().get_file_contract_state(file).unwrap().unwrap();
    let row = bridge.db().get_file(file).unwrap().unwrap();
    println!(
        "restore_elsewhere (b): current={} remote_updated_at={} size={}",
        contract.current_version, row.remote_updated_at, row.size_bytes
    );
    assert_eq!(contract.current_version, 3, "the snapshot's later version is the row's");
    assert_eq!(refresh_due(&bridge), vec![file.to_string()], "#447's refresh is due");
    // macOS raises only the version and the size from a snapshot (§6.3.2); the rest of the row keeps its time there.
    #[cfg(not(target_os = "macos"))]
    assert_eq!(
        (row.remote_updated_at, row.size_bytes),
        (RESTORED_AT, 14),
        "re-anchored to the server's time and size"
    );
}

/// Step 1: an update that names no version (a rename, a thumbnail flag) keeps the row's server time; one that names
/// its version stamps the time as before. Mutant M1: red at the rename.
#[test]
fn a_versionless_op_keeps_the_rows_server_time() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-00000000000c";
    seed_synced_copy(&bridge, file, "notes.txt", 2, SEEDED_AT, VERSION_TWO);
    let mut conflicts = Vec::new();
    let now = now_secs();

    apply_sync_op(&bridge, &sync_root, &rename_op(file, "plans.txt"), now, &mut conflicts).unwrap();
    let renamed = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(renamed.path, "plans.txt");
    assert_eq!(
        (renamed.remote_updated_at, renamed.modified_at),
        (SEEDED_AT, SEEDED_AT),
        "the rename keeps the row's server time"
    );

    let flag = op("file_update", serde_json::json!({ "id": file, "has_thumbnail": true }));
    apply_sync_op(&bridge, &sync_root, &flag, now, &mut conflicts).unwrap();
    let flagged = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(flagged.remote_updated_at, SEEDED_AT, "a thumbnail flag keeps it too");
    assert_eq!(
        bridge
            .db()
            .get_file_contract_state(file)
            .unwrap()
            .unwrap()
            .current_version,
        2,
        "neither changes the version"
    );

    let before = now_secs();
    let versioned = op(
        "file_update",
        serde_json::json!({ "id": file, "version_number": 3, "size_bytes": 20 }),
    );
    apply_sync_op(&bridge, &sync_root, &versioned, now, &mut conflicts).unwrap();
    let updated = bridge.db().get_file(file).unwrap().unwrap();
    println!(
        "restore_elsewhere versionless: renamed={} flagged={} versioned={} (clock before {before})",
        renamed.remote_updated_at, flagged.remote_updated_at, updated.remote_updated_at
    );
    assert!(
        updated.remote_updated_at >= before,
        "an update naming its version stamps the time as before"
    );
    assert_eq!(
        bridge
            .db()
            .get_file_contract_state(file)
            .unwrap()
            .unwrap()
            .current_version,
        3
    );
}

/// The time rule still holds where the versions agree: a snapshot naming the row's own version at an earlier server
/// time is skipped (its other name and size are not taken).
#[test]
fn a_snapshot_with_the_same_version_and_older_time_is_still_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-00000000000d";
    seed_synced_copy(&bridge, file, "notes.txt", 2, 5_000, VERSION_TWO);
    let mut conflicts = Vec::new();

    let same = node(file, "renamed.txt", 2, 99, 4_000);
    apply_snapshot(
        &bridge,
        &sync_root,
        &snapshot_of(vec![same]),
        6_000,
        6_000,
        &mut conflicts,
    )
    .unwrap();

    let row = bridge.db().get_file(file).unwrap().unwrap();
    let contract = bridge.db().get_file_contract_state(file).unwrap().unwrap();
    println!(
        "restore_elsewhere same version: path={} remote_updated_at={} size={} current={}",
        row.path, row.remote_updated_at, row.size_bytes, contract.current_version
    );
    assert_eq!(
        (
            row.path.as_str(),
            row.remote_updated_at,
            row.size_bytes,
            contract.current_version
        ),
        ("notes.txt", 5_000, VERSION_TWO.len() as i64, 2),
        "skipped: nothing of the snapshot's row is taken"
    );
    assert!(refresh_due(&bridge).is_empty());
}

/// A snapshot naming an earlier version than the row holds never re-anchors it, even at a later server time: the write
/// would lower the row's version. Mutant M4 (the `Less` arm deleted): red.
#[test]
fn a_snapshot_with_a_lower_version_never_re_anchors() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-00000000000e";
    seed_synced_copy(&bridge, file, "notes.txt", 3, SEEDED_AT, VERSION_TWO);
    let mut conflicts = Vec::new();

    let lagging = node(file, "older.txt", 2, 5, 9_000);
    apply_snapshot(
        &bridge,
        &sync_root,
        &snapshot_of(vec![lagging]),
        9_500,
        9_500,
        &mut conflicts,
    )
    .unwrap();

    let row = bridge.db().get_file(file).unwrap().unwrap();
    let contract = bridge.db().get_file_contract_state(file).unwrap().unwrap();
    println!(
        "restore_elsewhere lower version: path={} remote_updated_at={} current={}",
        row.path, row.remote_updated_at, contract.current_version
    );
    assert_eq!(contract.current_version, 3, "the row's version is never lowered");
    assert_eq!(
        (row.path.as_str(), row.remote_updated_at),
        ("notes.txt", SEEDED_AT),
        "nothing of the snapshot's row is taken"
    );
}

/// Task 2445's clause holds: a `Local` row with an empty path is placed again from the snapshot, whatever its version
/// and time say.
#[test]
fn a_snapshot_still_places_a_local_row_with_an_empty_path() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-00000000000f";
    seed_synced_copy(&bridge, file, "notes.txt", 2, 5_000, VERSION_TWO);
    bridge
        .db()
        .test_execute(&format!("UPDATE files SET path = '' WHERE file_id = '{file}'"))
        .unwrap();
    let mut conflicts = Vec::new();

    let same = node(file, "notes.txt", 2, VERSION_TWO.len() as i64, 4_000);
    apply_snapshot(
        &bridge,
        &sync_root,
        &snapshot_of(vec![same]),
        6_000,
        6_000,
        &mut conflicts,
    )
    .unwrap();

    assert_eq!(bridge.db().get_file(file).unwrap().unwrap().path, "notes.txt");
}

/// A conflict at `place`: the row is `Conflict`, agreed content `b"agreed version"` recorded at version 2, and the place
/// holds this PC's edit.
fn conflict_with_an_edit(bridge: &EngineBridge, sync_root: &Path, file_id: &str) -> PathBuf {
    seed_synced_copy(bridge, file_id, "report.docx", 2, SEEDED_AT, b"agreed version");
    bridge.db().set_status(file_id, FileStatus::Conflict).unwrap();
    let place = sync_root.join("report.docx");
    std::fs::write(&place, b"edited on this PC").unwrap();
    place
}

/// L409: Keep there into the file's own place (the caller's start look, a copy of the place kept first) lets the
/// person's resolution replace a file with agreed content recorded and edited bytes. Any other hydrate of that place
/// still refuses: a plain one on the same conflict row, and the queue. Mutant M3 (`Changed | Queued → Ok(true)` again,
/// whatever Keep there says): red.
#[test]
fn keep_there_replaces_an_edited_file_with_an_agreed_hash() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-000000000010";
    let place = conflict_with_an_edit(&bridge, &sync_root, file);
    assert_eq!(
        sync_folder_content(bridge.db(), file, &place, None).unwrap(),
        SyncFolderContent::Changed { base: Some(2) },
        "the case: agreed content recorded, edited bytes"
    );

    let keep_there = bridge.place_holds_an_unsent_change(file, &place, true, true).unwrap();
    let plain = bridge.place_holds_an_unsent_change(file, &place, true, false).unwrap();
    let queued = bridge.place_holds_an_unsent_change(file, &place, false, false).unwrap();
    println!("keep_there agreed edit: refused keep_there={keep_there} plain={plain} queued={queued}");
    assert!(!keep_there, "Keep there replaces it (its copy was kept first)");
    assert!(plain, "a hydrate that is not Keep there still refuses");
    assert!(queued, "the queue still refuses");
    assert_eq!(
        std::fs::read(&place).unwrap(),
        b"edited on this PC",
        "the guard writes nothing"
    );
}

/// The guard's other refusals hold. Unknown content (nothing agreed, nothing fetched) at the place still refuses
/// every hydrate that is not the person's resolution (the queue). Keep there's relief stops at agreed content: an
/// edited file whose content is known only from fetches (no agreed content recorded) still refuses under Keep there.
/// Unknown content under Keep there itself is not refused, today as before (S6, task 2191: the person's to resolve, its
/// copy kept first), so the test covers these two cases.
/// Mutant M3b (the `has_agreed_content` condition dropped): red.
#[test]
fn keep_there_relief_stops_at_agreed_content() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-000000000011";
    let place = conflict_with_an_edit(&bridge, &sync_root, file);

    bridge
        .db()
        .test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = -1, fetched_hash = NULL WHERE file_id = '{file}'"
        ))
        .unwrap();
    assert_eq!(
        sync_folder_content(bridge.db(), file, &place, None).unwrap(),
        SyncFolderContent::Unknown
    );
    let unknown_queued = bridge.place_holds_an_unsent_change(file, &place, false, false).unwrap();

    bridge
        .db()
        .test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = 2, fetched_hash = '{}' WHERE file_id = '{file}'",
            sha256_hex(b"the fetched version")
        ))
        .unwrap();
    assert_eq!(
        sync_folder_content(bridge.db(), file, &place, None).unwrap(),
        SyncFolderContent::Changed { base: Some(2) },
        "the case: content known only from fetches, edited bytes"
    );
    let fetched_keep_there = bridge.place_holds_an_unsent_change(file, &place, true, true).unwrap();

    println!("keep_there not agreed: refused unknown_queued={unknown_queued} fetched_keep_there={fetched_keep_there}");
    assert!(unknown_queued, "unknown content still refuses the queue");
    assert!(fetched_keep_there, "Keep there's relief is for agreed content only");
    assert_eq!(std::fs::read(&place).unwrap(), b"edited on this PC");
}

/// The start look Keep there takes before its safety copy, moved past the settle boundary (the place was written a
/// moment ago), as `tests/start_look_s6.rs` takes it.
fn seen_settled(place: &Path) -> PlaceSeen {
    match PlaceSeen::take_at(place, SystemTime::now() + Duration::from_secs(3)) {
        Settle::Seen(seen) => seen,
        other => panic!("the look: {other:?}"),
    }
}

/// Whether `result` is the in-place guard's refusal (an unsent change at the place).
fn refused_as_unsent(result: &anyhow::Result<()>) -> bool {
    result
        .as_ref()
        .err()
        .is_some_and(|error| engine_cause::<UnsentLocalChange>(error).is_some())
}

/// L409 through Keep there's own entry (`resolve_theirs_seen`, the one the native Keep there calls after its look and
/// its safety copy): an edited file with agreed content recorded is let past the in-place guard, and the hydrate goes
/// on to the download. No server answers here, so the download fails, the row reads `error` (the hydrate's own failure
/// status, written only past the guard) and the place keeps its bytes. Mutants M3 (the relief deleted) and M5
/// (`keep_there = false` in `hydrate_from`): red, refused by the guard with the row still a conflict.
#[tokio::test]
async fn keep_there_lets_the_resolution_past_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-000000000012";
    let place = conflict_with_an_edit(&bridge, &sync_root, file);
    let seen = seen_settled(&place);

    let result = bridge.resolve_theirs_seen(file, &sync_root, seen).await;
    let status = bridge.db().get_file(file).unwrap().unwrap().status;
    println!(
        "keep_there past the guard: refused_as_unsent={} status={status:?} result={result:?}",
        refused_as_unsent(&result)
    );
    assert!(result.is_err(), "no server answers: the download fails");
    assert!(!refused_as_unsent(&result), "not refused by the in-place guard");
    assert_eq!(
        status,
        FileStatus::Error,
        "the hydrate went past the guard to the download, which failed"
    );
    assert_eq!(std::fs::read(&place).unwrap(), b"edited on this PC");
}

/// L409's relief is Keep there's alone: a plain hydrate of the same conflict into its place (`hydrate_file`, which
/// keeps no copy) is still refused by the guard before anything is asked, and the row stays a conflict. Mutant M6
/// (`keep_there = persons_resolution`, the Keep there condition dropped): red.
#[tokio::test]
async fn keep_there_a_plain_hydrate_of_the_conflict_still_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-000000000013";
    let place = conflict_with_an_edit(&bridge, &sync_root, file);

    let result = bridge.hydrate_file(file, &place, &[sync_root.as_path()]).await;
    let status = bridge.db().get_file(file).unwrap().unwrap().status;
    println!(
        "keep_there plain hydrate: refused_as_unsent={} status={status:?}",
        refused_as_unsent(&result)
    );
    assert!(refused_as_unsent(&result), "refused by the in-place guard: {result:?}");
    assert_eq!(status, FileStatus::Conflict);
    assert_eq!(std::fs::read(&place).unwrap(), b"edited on this PC");
}

/// The Tauri-era Keep there (`resolve_keep_theirs`) keeps no copy of the place, so L409's relief is not its own: an
/// edited file with agreed content recorded is refused before anything is asked, as before L409, and keeps its bytes.
/// Mutant M7 (its refusal in `resolve_keep_theirs` deleted): red (the hydrate goes on to the download).
#[tokio::test]
async fn keep_there_without_a_kept_copy_still_refuses_an_edited_file() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-000000000014";
    let place = conflict_with_an_edit(&bridge, &sync_root, file);

    let result = bridge.resolve_keep_theirs(file, &sync_root).await;
    let status = bridge.db().get_file(file).unwrap().unwrap().status;
    println!(
        "keep_there without a kept copy: refused_as_unsent={} status={status:?}",
        refused_as_unsent(&result)
    );
    assert!(refused_as_unsent(&result), "refused before the hydrate: {result:?}");
    assert_eq!(status, FileStatus::Conflict);
    assert_eq!(std::fs::read(&place).unwrap(), b"edited on this PC");
}

/// A row in conflict whose agreed content (`agreed`, at version 1) is recorded, as the hydrate's guard reads it. The
/// name is the place's: the row is written directly, never decrypted.
fn put_conflict_row(bridge: &EngineBridge, file_id: &str, name: &str, agreed: &[u8]) {
    let entry = FileEntry {
        file_id: file_id.to_string(),
        path: name.to_string(),
        status: FileStatus::Conflict,
        size_bytes: agreed.len() as i64,
        modified_at: SEEDED_AT,
        content_hash: None,
        remote_updated_at: SEEDED_AT,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    bridge
        .db()
        .upsert_file_with_contract(&entry, |contract| contract.item_kind = ItemKind::File)
        .unwrap();
    bridge
        .db()
        .record_local_baseline(file_id, 1, &sha256_hex(agreed))
        .unwrap();
}

/// The agreed content at the place when Keep there looks, and a save of the same length made during its download.
const AGREED_AT_THE_LOOK: &[u8] = b"the agreed content.";
const SAVED_DURING_IT: &[u8] = b"saved during it!!!!";

/// the relief lets the pre-write hash check pass for an edit against agreed content on
/// Keep there's path, so a save made DURING the download, over a file that was agreed at the look, is caught only by
/// the start-look compare (S6 R8). It must still be refused, and the save kept. Driven on the native Keep there's entry
/// (`resolve_theirs_seen`) against the fake account, which serves the file's real chunks, so the download completes
/// and the hydrate reaches its checks before the write. Two saves: in place (the same file; its times move, 20 ms after
/// the write, as `tests/start_look_s6.rs` saves), and another file renamed over the place (another inode). Mutant M8
/// (the start-look compare in `hydrate_from`'s `place_unchanged` always `true`): red, the save is replaced.
#[tokio::test]
async fn keep_there_refuses_a_save_made_during_the_download() {
    assert_eq!(
        AGREED_AT_THE_LOOK.len(),
        SAVED_DURING_IT.len(),
        "a save of the same size"
    );
    let (account, _phrase) = crate::test_api::fake_account::FakeAccount::register(
        "restore-elsewhere@example.eu",
        "an invented password",
        crate::test_api::fake_account::TEST_PHRASE,
    );
    let account = Arc::new(account);
    let fake = crate::test_api::fake_account::start_fake(account.clone());
    for (n, inode_change) in [(1u8, false), (2, true)] {
        let file = format!("25380000-0000-4000-8000-00000000002{n}");
        account.add_file_with_content(&file, None, "report.docx", b"the server's version");
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("Beebeeb");
        std::fs::create_dir_all(&sync_root).unwrap();
        let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
        let api = Arc::new(ApiClient::new(
            fake.base.clone(),
            account.session_token.clone(),
            account.master_key,
        ));
        let bridge = EngineBridge::new(&crate::identity::TEST, db, api);
        let place = sync_root.join("report.docx");
        std::fs::write(&place, AGREED_AT_THE_LOOK).unwrap();
        put_conflict_row(&bridge, &file, "report.docx", AGREED_AT_THE_LOOK);
        assert_eq!(
            sync_folder_content(bridge.db(), &file, &place, None).unwrap(),
            SyncFolderContent::Agreed,
            "the case: agreed content at the look"
        );
        let seen = seen_settled(&place);
        let at = place.clone();
        bridge.seams.arm("hydrate:before_write", move || {
            if inode_change {
                let staged = at.with_extension("save-tmp");
                std::fs::write(&staged, SAVED_DURING_IT).unwrap();
                std::fs::rename(&staged, &at).unwrap();
            } else {
                std::thread::sleep(Duration::from_millis(20));
                std::fs::write(&at, SAVED_DURING_IT).unwrap();
            }
        });
        // Which check caught it: the compare runs only once the hash check before the write let the hydrate through.
        let reached_compare = Arc::new(AtomicBool::new(false));
        let reached = reached_compare.clone();
        bridge
            .seams
            .arm("hydrate:before_replace", move || reached.store(true, Ordering::SeqCst));

        let result = bridge.resolve_theirs_seen(&file, &sync_root, seen).await;
        let status = bridge.db().get_file(&file).unwrap().unwrap().status;
        println!(
            "keep_there save during the download (inode change {inode_change}): refused_as_unsent={} \
             caught_by_the_start_look_compare={} status={status:?} result={result:?}",
            refused_as_unsent(&result),
            reached_compare.load(Ordering::SeqCst)
        );
        assert!(refused_as_unsent(&result), "the save is never replaced");
        assert_eq!(std::fs::read(&place).unwrap(), SAVED_DURING_IT, "the save is kept");
        assert_eq!(status, FileStatus::Conflict, "the row is back where it was");
    }
    drop(fake);
}

/// a snapshot node whose upload is still running and that names a
/// later version is left to the time rule (as macOS's §6.3.2 snapshot fill skips such a node): at an older server time
/// it is skipped, at a later one it is applied as before. Mutant M9 (the `is_uploading` condition of the `Greater` arm
/// dropped): red, the older node re-anchors the row.
#[test]
fn an_uploading_snapshot_node_keeps_the_time_rule() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bridge_in(dir.path());
    let file = "25380000-0000-4000-8000-000000000015";
    seed_synced_copy(&bridge, file, "notes.txt", 2, 5_000, VERSION_TWO);
    let mut conflicts = Vec::new();

    let mut older = node(file, "notes.txt", 3, 14, 4_000);
    older["is_uploading"] = serde_json::json!(true);
    apply_snapshot(
        &bridge,
        &sync_root,
        &snapshot_of(vec![older]),
        6_000,
        6_000,
        &mut conflicts,
    )
    .unwrap();
    let after_older = (
        bridge
            .db()
            .get_file_contract_state(file)
            .unwrap()
            .unwrap()
            .current_version,
        bridge.db().get_file(file).unwrap().unwrap().remote_updated_at,
    );

    let mut later = node(file, "notes.txt", 3, 14, 7_000);
    later["is_uploading"] = serde_json::json!(true);
    apply_snapshot(
        &bridge,
        &sync_root,
        &snapshot_of(vec![later]),
        8_000,
        8_000,
        &mut conflicts,
    )
    .unwrap();
    let after_later = bridge.db().get_file(file).unwrap().unwrap().remote_updated_at;
    println!(
        "restore_elsewhere uploading node: after the older (version, time)={after_older:?}, after the later time={after_later}"
    );
    assert_eq!(after_older, (2, 5_000), "an uploading node at an older time is skipped");
    assert_eq!(
        after_later, 7_000,
        "at a later time the time rule applies it, as before"
    );
}
