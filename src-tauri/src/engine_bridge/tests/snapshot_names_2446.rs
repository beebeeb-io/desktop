//! Task 2446: the server's snapshot sends `file_request_id`, `sender_ephemeral_pubkey` and `wrapped_content_key` as
//! JSON null on every ordinary row (since 2026-11-02) and `updated_at` as an RFC 3339 string. A null field is not a
//! carried request envelope: the row's own key is tried, its name is read, and a child sits under its folder.
//! Rows have the production shape; they go through `prepare_owned_rows` and `apply_snapshot` as a tick sends them.

use super::*;

const MK: [u8; 32] = [7u8; 32];

/// One snapshot row as production sends it: the request fields and the owner fields null, a text `updated_at`.
pub(super) fn prod_row(id: &str, parent_id: Option<&str>, name: &str, is_folder: bool) -> serde_json::Value {
    let master = beebeeb_core::kdf::MasterKey::from_bytes(MK);
    let name_encrypted = beebeeb_core::encrypt::encrypt_name(&master, id, name, None).unwrap();
    serde_json::json!({
        "id": id,
        "parent_id": parent_id,
        "name_encrypted": name_encrypted,
        "file_request_id": null,
        "sender_ephemeral_pubkey": null,
        "wrapped_content_key": null,
        "acted_via_share_id": null,
        "uploaded_by": null,
        "owner_wrapped_file_key": null,
        "chunk_count": 1,
        "size_bytes": if is_folder { 0 } else { 2000 },
        "is_folder": is_folder,
        "is_uploading": false,
        "content_hash": null,
        "has_thumbnail": false,
        "version_number": 1,
        "storage_pool_id": null,
        "is_trashed": false,
        "updated_at": "2026-11-10T12:00:00Z"
    })
}

/// A bridge whose API answers 404 to everything: an ordinary owner row needs no request to be read.
pub(super) fn offline_bridge(dir: &Path) -> EngineBridge {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        use std::io::Read as _;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        }
    });
    test_bridge_with_api(&dir.join("state.db"), base, MK)
}

const FOLDER: &str = "f0000000-0000-4000-8000-000000002446";
const FILE: &str = "a0000000-0000-4000-8000-000000002446";

#[tokio::test]
async fn a_production_shaped_snapshot_row_is_read_named_and_nested() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = offline_bridge(dir.path());
    let rows = [
        prod_row(FOLDER, None, "Docs", true),
        prod_row(FILE, Some(FOLDER), "Report.pdf", false),
    ];
    let prepared = bridge.prepare_owned_rows(&rows).await.unwrap();
    assert_eq!(prepared.len(), 2, "both rows are reported");
    for row in &prepared {
        let id = row.row["id"].as_str().unwrap();
        assert!(
            row.readable,
            "{id}: a null request field is not a carried envelope; the row's key reads it"
        );
        assert!(row.record.blocked_fingerprint.is_none(), "{id}: not blocked");
    }
    let snapshot = crate::api_client::SyncSnapshot {
        seq_id: 1,
        nodes: prepared.into_iter().map(|p| p.row).collect(),
    };
    apply_snapshot(&bridge, dir.path(), &snapshot, 100, 100, &mut Vec::new()).unwrap();
    let folder = bridge.db().get_file(FOLDER).unwrap().expect("the folder is stored");
    assert_eq!(folder.path, "Docs");
    let file = bridge.db().get_file(FILE).unwrap().expect("the file is stored");
    assert_eq!(
        file.path, "Docs/Report.pdf",
        "the name is read and the file sits under its folder"
    );
    assert_eq!(file.parent_id.as_deref(), Some(FOLDER));
}
