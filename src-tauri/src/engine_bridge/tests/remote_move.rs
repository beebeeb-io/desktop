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
