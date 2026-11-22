//! Task 2272: an op that fails to apply holds the cursor before it, is tried again after a back-off,
//! and one that keeps failing is parked with words — never stepped over silently.

use super::*;
use crate::engine_bridge::syncflow::{
    SYNC_OP_MAX_ATTEMPTS, expire_sync_op_backoff_for_test, fail_sync_op_apply_for_test, parked_sync_ops,
    sync_op_failed_attempts,
};

const ID6: &str = "aaaa0000-0000-4000-8000-000000000006";
const ID7: &str = "aaaa0000-0000-4000-8000-000000000007";
const ID8: &str = "aaaa0000-0000-4000-8000-000000000008";

fn seed(bridge: &EngineBridge, id: &str, name: &str) {
    bridge
        .db()
        .upsert_file(&FileEntry {
            file_id: id.into(),
            path: name.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 8,
            modified_at: 100,
            content_hash: None,
            remote_updated_at: 100,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
}

fn delete_op(seq: i64, id: &str) -> serde_json::Value {
    serde_json::json!({ "seq_id": seq, "op_type": "file_delete", "payload": { "id": id } })
}

/// The page `since=5` (ops 6, 7, 8) and the page `since=6` (ops 7, 8).
fn page(since: i64) -> (String, serde_json::Value) {
    let ops: Vec<_> = [(6, ID6), (7, ID7), (8, ID8)]
        .into_iter()
        .filter(|(seq, _)| *seq > since)
        .map(|(seq, id)| delete_op(seq, id))
        .collect();
    (
        "200 OK".into(),
        serde_json::json!({ "since": since, "head_seq": 8, "ops": ops }),
    )
}

fn present(bridge: &EngineBridge, id: &str) -> bool {
    bridge.db().get_file(id).unwrap().is_some()
}

#[tokio::test]
async fn cursor_2272_failed_op_mid_page_holds_cursor_and_retries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let server = SyncMockServer::start(vec![page(5), page(6), page(6)]);
    let bridge = test_bridge_with_api(&root.join("state.db"), server.base_url.clone(), [44u8; 32]);
    bridge.db().set_sync_cursor(5).unwrap();
    for (id, name) in [(ID6, "six.txt"), (ID7, "seven.txt"), (ID8, "eight.txt")] {
        seed(&bridge, id, name);
    }

    // Tick 1: op 7, in the middle of the page, fails. Op 6 landed; the cursor stops at 6 and
    // op 8 waits behind op 7 so the order holds.
    fail_sync_op_apply_for_test(&[7]);
    sync_tick(&bridge, root).await.unwrap();
    assert_eq!(
        bridge.db().get_sync_cursor().unwrap(),
        Some(6),
        "cursor stays before the failed op"
    );
    assert!(!present(&bridge, ID6), "op 6 before it applied");
    assert!(
        present(&bridge, ID7) && present(&bridge, ID8),
        "op 7 failed, op 8 waits"
    );
    assert_eq!(sync_op_failed_attempts(root, 7), 1);

    // Tick 2: inside the back-off op 7 is not tried again, and nothing moves.
    fail_sync_op_apply_for_test(&[]);
    sync_tick(&bridge, root).await.unwrap();
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(6));
    assert!(present(&bridge, ID7) && present(&bridge, ID8));
    assert_eq!(sync_op_failed_attempts(root, 7), 1, "a waiting op costs no attempt");

    // Tick 3: the back-off passed and the op applies now: the change was not lost.
    expire_sync_op_backoff_for_test(root);
    sync_tick(&bridge, root).await.unwrap();
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(8));
    assert!(!present(&bridge, ID7) && !present(&bridge, ID8));
    assert_eq!(sync_op_failed_attempts(root, 7), 0);
    assert!(parked_sync_ops(root).is_empty());

    let requests = server.finish();
    let paths: Vec<_> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/api/v1/sync/ops?since=5",
            "/api/v1/sync/ops?since=6",
            "/api/v1/sync/ops?since=6"
        ]
    );
}

#[tokio::test]
async fn cursor_2272_op_that_keeps_failing_is_parked_with_words() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut responses = vec![page(5)];
    responses.extend((1..SYNC_OP_MAX_ATTEMPTS).map(|_| page(6)));
    let server = SyncMockServer::start(responses);
    let bridge = test_bridge_with_api(&root.join("state.db"), server.base_url.clone(), [45u8; 32]);
    bridge.db().set_sync_cursor(5).unwrap();
    for (id, name) in [(ID6, "six.txt"), (ID7, "seven.txt"), (ID8, "eight.txt")] {
        seed(&bridge, id, name);
    }
    fail_sync_op_apply_for_test(&[7]);

    for attempt in 1..SYNC_OP_MAX_ATTEMPTS {
        sync_tick(&bridge, root).await.unwrap();
        assert_eq!(
            bridge.db().get_sync_cursor().unwrap(),
            Some(6),
            "attempt {attempt}: held"
        );
        assert_eq!(sync_op_failed_attempts(root, 7), attempt);
        assert!(parked_sync_ops(root).is_empty(), "attempt {attempt}: not parked yet");
        assert!(bridge.db().peek_resnapshot_request().unwrap().is_none());
        expire_sync_op_backoff_for_test(root);
    }

    // The last attempt parks it: on the review list with words, a re-snapshot asked for so the
    // item still converges, and the ops after it go on.
    sync_tick(&bridge, root).await.unwrap();
    fail_sync_op_apply_for_test(&[]);
    let parked = parked_sync_ops(root);
    assert_eq!(parked.len(), 1);
    let p = &parked[0];
    assert_eq!(
        (p.seq_id, p.op_type.as_str(), p.item_id.as_str()),
        (7, "file_delete", ID7)
    );
    assert_eq!(p.attempts, SYNC_OP_MAX_ATTEMPTS);
    assert!(
        p.words.contains("could not be applied") && p.words.contains(ID7),
        "{}",
        p.words
    );
    assert!(p.error.contains("op 7 fails"), "{}", p.error);
    assert!(
        bridge.db().peek_resnapshot_request().unwrap().is_some(),
        "re-snapshot requested"
    );
    assert_eq!(bridge.db().get_sync_cursor().unwrap(), Some(8));
    assert!(present(&bridge, ID7), "the parked op itself applied nothing");
    assert!(!present(&bridge, ID8), "op 8 after it applied");
    assert_eq!(sync_op_failed_attempts(root, 7), 0);
    server.finish();
}

#[test]
fn cursor_2272_backoff_doubles_and_is_capped() {
    use crate::engine_bridge::syncflow::sync_op_backoff_secs;
    let waits: Vec<_> = (1..=8).map(sync_op_backoff_secs).collect();
    assert_eq!(waits, [30, 60, 120, 240, 480, 600, 600, 600]);
}
