//! One unit test per converted area of D-P2-2b: the landing request's ids and
//! the upload path's ids now carry `state_db`'s newtypes, so which row an id
//! addresses is named at the hand-over instead of living in a bare string.

use super::*;

/// Area A — the landing request: the local row id, the server row id and the
/// produced object's version id are the typed ids, and the version *number* the
/// row lands on is not one of them.
#[test]
fn land_request_carries_the_landing_ids_as_ids() {
    let op = PendingOperation {
        op_id: "op-typed".to_string(),
        kind: OperationKind::UploadVersion,
        file_id: Some("local-typed".to_string()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut post_complete_errors = Vec::new();
    let request = LandRequest {
        local_file_id: LocalFileId::new("local-typed"),
        op: &op,
        claim: None,
        server_file_id: ServerFileId::new("server-typed"),
        produced: ProducedObject {
            version_number: 7,
            object_version_id: VersionId::new("object-typed"),
        },
        size_bytes: 1,
        content_type: None,
        mime_type: None,
        post_complete_errors: &mut post_complete_errors,
        sync_root: Path::new("/sync"),
    };

    assert_eq!(request.local_file_id.as_str(), "local-typed");
    assert_eq!(request.server_file_id.as_str(), "server-typed");
    assert_eq!(request.produced.object_version_id.as_str(), "object-typed");
    assert_eq!(request.produced.version_number, 7);
}

/// Area B — the upload path: the session's id check takes the local id as an
/// id, and a replace session that holds another row's id is still refused.
#[test]
fn the_upload_session_check_takes_the_local_id() {
    let local = LocalFileId::new("5e6f7a8b-9c0d-4e1f-2a3b-4c5d6e7f8091");
    assert!(
        ensure_session_item_id(
            &super::uploads_3::l13_resume(false, true, local.as_str()),
            local
        )
        .is_ok()
    );

    let err = ensure_session_item_id(&super::uploads_3::l13_resume(false, true, "another-row"), local)
        .unwrap_err()
        .to_string();
    assert!(err.contains("server_changed_item_id"), "got {err:?}");
}
