//! A15: a remote move into a shared folder publishes nothing.
//!
//! Owns: the sync-op enrichment path for `file_move` (the desktop's only
//! remote-move entry).
//! Must not: seal a name, mint a wrap, or reach the network.
//! Tests: this whole file.

use super::*;

#[tokio::test]
async fn a15_remote_move_into_a_shared_folder_publishes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    // No server: any publication attempt would fail this call.
    let bridge = test_bridge(&dir.path().join("state.db"));
    let item = "40000000-0000-4000-8000-0000000000f1";
    let root = "40000000-0000-4000-8000-0000000000f2";
    seed_folder_row(&bridge, item, "Notes");
    seed_folder_row(&bridge, root, "Shared");
    // A15: "Shared" is a real shared folder, and the moved owner item already
    // carries a persisted owner envelope — the old `owner_wrap.is_some()`
    // heuristic would read that as a foreign row and refetch it.
    bridge.db().remember_folder_custody_at_revision(root, 0).unwrap();
    assert!(bridge.db().has_seen_folder_custody(root).unwrap());
    let other = beebeeb_core::kdf::FileKey::from_bytes([0x42; 32]);
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &other,
        "Notes",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&crate::owner_keys::OwnedKeyRecord {
            file_id: item.into(),
            name_encrypted: name,
            // Core refuses to mint an L8 wrap for an owner id; the old heuristic
            // only read `is_some`, so any persisted bytes reproduce it.
            owner_wrap: Some(vec![0x11; 60]),
            ..Default::default()
        })
        .unwrap();
    let op = crate::api_client::SyncOp {
        seq_id: 7,
        op_type: "file_move".into(),
        payload: serde_json::json!({"id":item,"parent_id":root}),
    };
    let prepared = crate::engine_bridge::syncflow::prepare_owned_sync_op(&bridge, &op)
        .await
        .expect("A15 must not abort a remote move");
    let prepared = prepared.expect("a move op is carried through");
    assert_eq!(prepared.payload["parent_id"], serde_json::json!(root));
    assert_eq!(prepared.payload["new_parent_id"], serde_json::json!(root));
    assert!(
        prepared.payload.get("owner_wrapped_file_key").is_none(),
        "an owner move into R publishes no owner key: {}",
        prepared.payload
    );
    let record = bridge.db().get_owned_key_record(item).unwrap();
    assert!(
        record.is_none_or(|record| record.pending_owner_wrap.is_none()),
        "no adoption was minted for the move"
    );
}
