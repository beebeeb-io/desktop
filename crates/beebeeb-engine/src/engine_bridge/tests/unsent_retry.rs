//! Task 2409 (owner, 13:39: "Add in the try again directly if it failed"): Try again, end to end through the queue step.
//! A change that stopped after repeated errors is sent by the next queue step after Try again; one waiting out a
//! backoff is sent by the step right after it, not at its retry time.

use super::*;

fn folder_create(op_id: &str, attempts: i64, next_retry_at: i64) -> PendingOperation {
    PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::CreateFolder,
        file_id: Some(format!("folder-{op_id}")),
        parent_id: None,
        target_path: None,
        metadata_json: Some(serde_json::json!({ "name_encrypted": "sealed" }).to_string()),
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts,
        max_attempts: 25,
        next_retry_at,
        last_error: Some("HTTP status server error (500 Internal Server Error)".into()),
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    }
}

/// Before Try again the step sends neither; after it, one step sends both, in one request each.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn after_try_again_the_next_queue_step_sends_a_given_up_and_a_backing_off_change() {
    let dir = tempfile::tempdir().unwrap();
    let server = SyncMockServer::start(vec![
        ("200 OK".into(), serde_json::json!({ "id": "folder-stopped" })),
        ("200 OK".into(), serde_json::json!({ "id": "folder-backing-off" })),
    ]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);
    bridge.db.enqueue_operation(&folder_create("stopped", 25, 0)).unwrap();
    bridge
        .db
        .enqueue_operation(&folder_create("backing-off", 3, 10_000))
        .unwrap();

    let before = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert!(
        before.completed_op_ids.is_empty() && before.retried_op_ids.is_empty(),
        "nothing is due: {before:?}"
    );

    for op in ["stopped", "backing-off"] {
        assert_eq!(
            bridge.db.retry_unsent_change(op, 200).unwrap(),
            crate::state_db::RetryOutcome::Retried
        );
    }
    let after = bridge.process_due_operations(dir.path(), 200).await.unwrap();

    let mut completed = after.completed_op_ids.clone();
    completed.sort();
    assert_eq!(
        completed,
        ["backing-off".to_string(), "stopped".to_string()],
        "both sent at the step after Try again"
    );
    assert_eq!(server.finish().len(), 2, "one request each");
    assert_eq!(
        bridge.db.unsent_changes(0, 100, 200).unwrap().total,
        0,
        "sent: no longer counted"
    );
}

/// w36: a nudge's step beside a running tick sends the sync changes and leaves the backup's uploads to the tick's own
/// step: a long backup queue never stands before a save.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn w36_the_changes_only_step_leaves_the_backups_uploads_to_the_whole_step() {
    let dir = tempfile::tempdir().unwrap();
    let server = SyncMockServer::start(vec![
        ("200 OK".into(), serde_json::json!({ "id": "folder-change" })),
        ("200 OK".into(), serde_json::json!({ "id": "folder-backup" })),
    ]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);
    let mut backup = folder_create("backup", 0, 0);
    backup.backup_source_key = Some("music".into());
    backup.created_at = 50;
    bridge.db.enqueue_operation(&backup).unwrap();
    bridge.db.enqueue_operation(&folder_create("change", 0, 0)).unwrap();

    let changes = bridge.process_due_changes(dir.path(), 200).await.unwrap();
    assert_eq!(
        changes.completed_op_ids,
        ["change".to_string()],
        "the older backup op is not in its way"
    );
    let whole = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(whole.completed_op_ids, ["backup".to_string()]);
    assert_eq!(server.finish().len(), 2, "one request each");
}

/// w36: two queue steps can run at once (the tick's, and a nudge's beside it). An op one of them is sending is skipped
/// by the other, so a change is never sent twice; once the first is done with it, a step takes it again.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn w36_an_op_another_step_is_sending_is_not_sent_twice() {
    let dir = tempfile::tempdir().unwrap();
    let server = SyncMockServer::start(vec![("200 OK".into(), serde_json::json!({ "id": "folder-one" }))]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);
    bridge.db.enqueue_operation(&folder_create("one", 0, 0)).unwrap();

    let other_step = bridge.begin_op("one").expect("nobody sends it yet");
    assert!(bridge.begin_op("one").is_none(), "one sender at a time");
    for outcome in [
        bridge.process_due_operations(dir.path(), 200).await.unwrap(),
        bridge.process_due_changes(dir.path(), 200).await.unwrap(),
    ] {
        assert!(
            outcome.completed_op_ids.is_empty() && outcome.retried_op_ids.is_empty(),
            "skipped, not attempted: {outcome:?}"
        );
    }
    assert_eq!(bridge.db.get_operation("one").unwrap().unwrap().attempts, 0);

    drop(other_step);
    let after = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(after.completed_op_ids, ["one".to_string()]);
    assert_eq!(server.finish().len(), 1, "sent once");
}
