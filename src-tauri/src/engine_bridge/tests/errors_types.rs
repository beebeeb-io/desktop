//! One unit test per new type of D-P2-2: `EngineError`, `Op`, `LandRequest`
//! and `ProducedObject`. Each proves the type's contract without a
//! database or network: the Display text a refusal keeps, the boundary
//! conversion, and the enum's stored strings.

use super::*;

/// `EngineError` keeps the refusal's text and converts to `anyhow::Error` only
/// at the boundary. The text is what the File Provider extension and the
/// person read, so it must be the same as the existing struct's Display.
#[test]
fn engine_error_keeps_the_refusals_text() {
    let error = EngineError::UnknownItem(UnknownItem {
        requested_id: "11111111-1111-1111-1111-111111111111".to_string(),
    });
    assert_eq!(error.to_string(), "this item is not known to Beebeeb");
    let converted: anyhow::Error = error.into();
    assert_eq!(converted.to_string(), "this item is not known to Beebeeb");
}

/// `ProducedObject` keeps the version the server answered with and the object id
/// its content now has, and nothing else.
#[test]
fn produced_object_keeps_the_servers_version_and_object() {
    let produced = ProducedObject {
        version_id: 7,
        object_name: "object-123".to_string(),
    };
    assert_eq!(produced.version_id, 7);
    assert_eq!(produced.object_name, "object-123");
}

/// `LandRequest` carries the eleven landing arguments as one request: the op,
/// its claim (none for Keep Mine's inline run), the server row and the
/// completion's own numbers. The queue-moved signal still crosses the
/// `anyhow` boundary as its own type, so the lifecycle finds it by downcast.
#[test]
fn land_request_bundles_the_landing_inputs() {
    let op = PendingOperation {
        op_id: "op-1".to_string(),
        kind: OperationKind::UploadVersion,
        file_id: Some("file-1".to_string()),
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
    let mut post_complete_errors = vec!["thumbnail: skipped".to_string()];
    let request = LandRequest {
        local_file_id: "file-1",
        op: &op,
        claim: None,
        server_file_id: "server-file-1",
        produced: ProducedObject {
            version_id: 7,
            object_name: "object-123".to_string(),
        },
        size_bytes: 1024,
        content_type: Some("text/plain".to_string()),
        mime_type: None,
        post_complete_errors: &mut post_complete_errors,
        sync_root: Path::new("/sync"),
    };
    assert_eq!(request.local_file_id, "file-1");
    assert_eq!(request.op.op_id, "op-1");
    assert!(request.claim.is_none());
    assert_eq!(request.server_file_id, "server-file-1");
    assert_eq!(request.produced.version_id, 7);
    assert_eq!(request.produced.object_name, "object-123");
    assert_eq!(request.size_bytes, 1024);
    assert_eq!(request.content_type.as_deref(), Some("text/plain"));
    assert!(request.mime_type.is_none());
    assert_eq!(request.post_complete_errors.len(), 1);
    assert_eq!(request.sync_root, Path::new("/sync"));
}

/// `Op` names the `"operation"` tag a queued op's metadata carries. Every
/// variant spells the stored string the rows already hold, so the JSON stays
/// byte-identical, and the payload built from it keeps its exact shape.
#[test]
fn op_spells_the_stored_operation_tags() {
    let expected = [
        (Op::CreateFolder, "create_folder"),
        (Op::CreateFile, "create_file"),
        (Op::MetadataUpdate, "metadata_update"),
        (Op::UploadVersion, "upload_version"),
        (Op::Trash, "trash"),
        (Op::RestoreVersion, "restore_version"),
        (Op::PinTree, "pin_tree"),
        (Op::HydrateFile, "hydrate_file"),
    ];
    for (op, stored) in expected {
        assert_eq!(op.as_str(), stored);
        assert_eq!(
            serde_json::json!({ "operation": op.as_str() }).to_string(),
            serde_json::json!({ "operation": stored }).to_string()
        );
    }
}

/// `MediaError` keeps each step's text and reaches `EngineError` as its own cause.
#[test]
fn media_error_keeps_its_step_text_in_engine_error() {
    let error = EngineError::Media(MediaError::ZeroDimensions);
    assert_eq!(error.to_string(), "thumbnail source has zero dimensions");
    assert_eq!(
        EngineError::Media(MediaError::FfmpegFailed("boom".to_string())).to_string(),
        "ffmpeg video thumbnail failed: boom"
    );
    assert_eq!(
        EngineError::Media(MediaError::Base64 {
            label: "sender_public_key".to_string(),
            detail: "bad".to_string(),
        })
        .to_string(),
        "sender_public_key is not valid standard base64: bad"
    );
}

/// The boundary conversion keeps `QueueStateMoved` as its own type: the
/// lifecycle's "the op moved on" arms find it by downcast exactly as before.
#[test]
fn land_error_boundary_keeps_the_queue_moved_signal() {
    let error = land_error_to_anyhow(EngineError::QueueStateMoved(QueueStateMoved));
    assert!(error.downcast_ref::<QueueStateMoved>().is_some());
    assert_eq!(error.to_string(), "queue state moved");
}

/// Only the video mime family takes the ffmpeg frame path; everything else,
/// including a payload with no mime, decodes as an image.
#[test]
fn media_kind_reads_the_video_mime_family() {
    assert_eq!(MediaKind::from_mime(Some("video/mp4")), MediaKind::Video);
    assert_eq!(MediaKind::from_mime(Some("image/png")), MediaKind::Image);
    assert_eq!(MediaKind::from_mime(None), MediaKind::Image);
}

/// The kind, not the path, picks the syscall: a directory needs Dir and a file
/// needs File, so a caller that names the wrong one is refused the same way.
#[test]
fn engine_remove_takes_the_kind_it_was_told() {
    let root = std::env::temp_dir().join(format!("bb-removed-kind-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("file.txt");
    std::fs::write(&file, b"x").unwrap();
    let dir = root.join("dir");
    std::fs::create_dir(&dir).unwrap();

    assert!(engine_remove(&file, RemovedKind::Dir).is_err());
    assert!(file.exists());
    engine_remove(&file, RemovedKind::File).unwrap();
    assert!(!file.exists());

    assert!(engine_remove(&dir, RemovedKind::File).is_err());
    assert!(dir.exists());
    engine_remove(&dir, RemovedKind::Dir).unwrap();
    assert!(!dir.exists());
    let _ = std::fs::remove_dir(&root);
}

/// A binary side is sized and never shown; a text side shows its bytes. The
/// kind the window reads back is the same one the classifier was given.
#[test]
fn content_kind_keeps_a_binary_side_to_its_size() {
    let text = content_side_from_bytes(b"hello".to_vec(), ContentKind::Text);
    assert_eq!(text.text.as_deref(), Some("hello"));
    assert_eq!(text.size_bytes, Some(5));

    let binary = content_side_from_bytes(b"hello".to_vec(), ContentKind::Binary);
    assert_eq!(binary.text, None);
    assert_eq!(binary.unavailable_reason, None);
    assert_eq!(binary.size_bytes, Some(5));

    assert_eq!(ContentKind::from_flag(true), ContentKind::Text);
    assert!(!ContentKind::from_flag(false).is_text());
}

#[test]
fn pin_toggle_reads_the_command_flag() {
    assert!(PinToggle::from_flag(true).is_pin());
    assert!(!PinToggle::from_flag(false).is_pin());
}
