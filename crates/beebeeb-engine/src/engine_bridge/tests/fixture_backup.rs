const ALICE: (&str, &str) = ("u-a", "a@beebeeb.io");
const BOB: (&str, &str) = ("u-b", "b@beebeeb.io");

fn identity(who: (&str, &str)) -> crate::account_binding::Identity {
    crate::account_binding::Identity::new(Some(who.0), Some(who.1))
}

/// A bridge for `who`, with its sync root, outside the backed-up folder.
fn backup_bridge(dir: &Path, base_url: String, who: (&str, &str)) -> (EngineBridge, PathBuf) {
    let bridge = test_bridge_with_api(&dir.join("state.db"), base_url, [31u8; 32]);
    bridge.db.set_owner(&identity(who)).unwrap();
    let sync_root = dir.join("Beebeeb");
    std::fs::create_dir_all(&sync_root).unwrap();
    (bridge, sync_root)
}

/// The person's own file in their `Pictures`.
fn person_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let pictures = dir.join("Pictures");
    std::fs::create_dir_all(&pictures).unwrap();
    let path = pictures.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// What must not change: the bytes (sha-256), the length and the modification time.
#[derive(Debug, PartialEq, Eq)]
struct SourceState {
    sha256: String,
    len: u64,
    modified: std::time::SystemTime,
}

fn source_state(path: &Path) -> SourceState {
    let meta = std::fs::metadata(path).unwrap_or_else(|e| panic!("the source is gone: {e}"));
    SourceState {
        sha256: sha256_file_hex(path).unwrap(),
        len: meta.len(),
        modified: meta.modified().unwrap(),
    }
}

fn pictures_request(source: &Path) -> beebeeb_backup::UploadRequest {
    beebeeb_backup::UploadRequest {
        category: beebeeb_types::backup::Category::Pictures,
        key: source.file_name().unwrap().to_string_lossy().into_owned(),
        source_path: source.to_path_buf(),
        parent_id: uuid::Uuid::parse_str("8d3f1b2a-4c5e-4f60-8a7b-9c0d1e2f3a4b").unwrap(),
        name: source.file_name().unwrap().to_string_lossy().into_owned(),
        fingerprint: beebeeb_backup::fingerprint(source).unwrap(),
        reason: beebeeb_core::backup::UploadReason::New,
        record_key: None,
        remote_id: None,
    }
}

fn enqueue(bridge: &EngineBridge, source: &Path) -> String {
    match bridge.enqueue_backup_upload(&pictures_request(source)).unwrap() {
        BackupEnqueue::Queued(op_id) => op_id,
        other => panic!("not queued: {other:?}"),
    }
}

fn backup_ops(bridge: &EngineBridge) -> i64 {
    bridge
        .db
        .hold_lock_for_test()
        .query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE origin = 'backup'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn count_rows(bridge: &EngineBridge, table: &str) -> i64 {
    bridge
        .db
        .hold_lock_for_test()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
        .unwrap()
}
