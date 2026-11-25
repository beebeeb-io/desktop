//! dc-re-f: a row's date is the server's, never the clock of the PC that echoed an op.

use super::*;

const KEY: [u8; 32] = [36u8; 32];
const FILE: &str = "0e000000-0000-4000-8000-0000000000f1";
const JUNE: i64 = 1_750_000_000;
const NOW: i64 = 1_790_000_000;

fn bridge_in(dir: &Path) -> (Arc<StateDb>, EngineBridge) {
    let db = Arc::new(StateDb::open(dir.join("state.db")).unwrap());
    let api = Arc::new(ApiClient::new("http://127.0.0.1:9".into(), "token".into(), KEY));
    let bridge = EngineBridge::new(&crate::identity::TEST, db.clone(), api);
    (db, bridge)
}

#[allow(deprecated)]
fn node(updated_at: i64) -> serde_json::Value {
    let name = beebeeb_core::encrypt::encrypt_name(
        &beebeeb_core::kdf::MasterKey::from_bytes(KEY),
        FILE,
        "a.txt",
        Some("text/plain"),
    )
    .unwrap();
    serde_json::json!({
        "id": FILE, "parent_id": serde_json::Value::Null, "name_encrypted": name, "size_bytes": 3,
        "is_folder": false, "version_number": 1, "is_trashed": false, "updated_at": updated_at,
    })
}

fn list(bridge: &EngineBridge, updated_at: i64) {
    process_metadata_row(bridge, &node(updated_at), "", NOW, RowSource::Snapshot, &mut Vec::new()).unwrap();
}

#[test]
fn modified_at_of_a_local_row_stamped_now_takes_the_server_date() {
    let dir = tempfile::tempdir().unwrap();
    let (db, bridge) = bridge_in(dir.path());
    list(&bridge, JUNE);
    // The op echo stamped this PC's clock on the row, which then went Local.
    let mut row = db.get_file(FILE).unwrap().unwrap();
    row.status = FileStatus::Local;
    row.modified_at = NOW;
    row.remote_updated_at = NOW;
    db.upsert_file(&row).unwrap();
    list(&bridge, JUNE);
    let row = db.get_file(FILE).unwrap().unwrap();
    assert_eq!(row.modified_at, JUNE);
    assert_eq!(row.remote_updated_at, JUNE);
}

#[test]
fn modified_at_of_a_locally_edited_row_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (db, bridge) = bridge_in(dir.path());
    list(&bridge, JUNE);
    let mut row = db.get_file(FILE).unwrap().unwrap();
    row.status = FileStatus::Local;
    row.modified_at = NOW;
    row.remote_updated_at = JUNE;
    db.upsert_file(&row).unwrap();
    list(&bridge, JUNE);
    assert_eq!(db.get_file(FILE).unwrap().unwrap().modified_at, NOW);
}
