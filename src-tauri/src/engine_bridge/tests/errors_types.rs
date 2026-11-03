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

/// `ProducedObject` keeps the version number the server answered with and the
/// object version id its content now has, and nothing else.
#[test]
fn produced_object_keeps_the_servers_version_and_object() {
    let produced = ProducedObject {
        version_number: 7,
        object_version_id: VersionId::new("object-123"),
    };
    assert_eq!(produced.version_number, 7);
    assert_eq!(produced.object_version_id.as_str(), "object-123");
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
        local_file_id: LocalFileId::new("file-1"),
        op: &op,
        claim: None,
        server_file_id: ServerFileId::new("server-file-1"),
        produced: ProducedObject {
            version_number: 7,
            object_version_id: VersionId::new("object-123"),
        },
        size_bytes: 1024,
        content_type: Some("text/plain".to_string()),
        mime_type: None,
        post_complete_errors: &mut post_complete_errors,
        sync_root: Path::new("/sync"),
    };
    assert_eq!(request.local_file_id.as_str(), "file-1");
    assert_eq!(request.op.op_id, "op-1");
    assert!(request.claim.is_none());
    assert_eq!(request.server_file_id.as_str(), "server-file-1");
    assert_eq!(request.produced.version_number, 7);
    assert_eq!(request.produced.object_version_id.as_str(), "object-123");
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
    assert!(engine_cause::<QueueStateMoved>(&error).is_some());
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

/// MalformedRecord is a stored record that did not parse: its text is the
/// parser's, unchanged, and it stays the error's cause, so the chain a
/// classification walks reads as it did before the split.
#[test]
fn malformed_record_keeps_the_parsers_text() {
    let base64_error = base64::engine::general_purpose::STANDARD.decode("!!").unwrap_err();
    let base64_text = base64_error.to_string();
    let error: EngineError = base64_error.into();
    assert_eq!(error.to_string(), base64_text);
    assert_eq!(
        std::error::Error::source(&error).map(ToString::to_string),
        Some(base64_text)
    );

    let json_error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let json_text = json_error.to_string();
    let error: EngineError = json_error.into();
    assert_eq!(error.to_string(), json_text);
}

/// KeyCustody names which wrap opens a folder's children: the owner's custody
/// envelope, or the invite's own wrap.
#[test]
fn key_custody_names_the_unwrap_path() {
    assert!(KeyCustody::Custody.is_custody());
    assert!(!KeyCustody::Invite.is_custody());
}

/// OriginalRemoval names what happened to a kept copy's original: it left the
/// disk, or it stayed because it was in use.
#[test]
fn original_removal_names_the_prunes_outcome() {
    assert!(OriginalRemoval::Removed.is_removed());
    assert!(!OriginalRemoval::InPlace.is_removed());
}

/// The init guard answers by the kind: only a File Provider replace needs a
/// base, and a missing one parks the write as base_unknown.
#[test]
fn the_init_guard_reads_the_kind() {
    assert!(guard_init_base(None, InitKind::Create).is_ok());
    assert!(guard_init_base(None, InitKind::Replace).is_ok());
    assert!(guard_init_base(Some(3), InitKind::FinderReplace).is_ok());
    let error = guard_init_base(None, InitKind::FinderReplace).unwrap_err();
    assert!(error.is_caused_by::<ParkNow>());
}

/// The failed-attempt rule answers by the stage: a recorded completion never
/// uses up its last attempt, a File Provider refusal that looks transient backs
/// off to the cap, and a permission refusal parks at once.
#[test]
fn the_failed_attempt_rule_reads_the_stage() {
    assert_eq!(
        failed_attempt_rule(OperationFailureClass::Retryable, AttemptState::Completed),
        FailedAttemptRule::Retry(AttemptCount::ShortOfTheCap)
    );
    assert_eq!(
        failed_attempt_rule(OperationFailureClass::Auth, AttemptState::Upload),
        FailedAttemptRule::Pause(OperationPauseReason::Auth)
    );
    assert_eq!(
        failed_attempt_rule(OperationFailureClass::Permission, AttemptState::FinderWriteFinal),
        FailedAttemptRule::Park(ParkReason::PermissionDenied)
    );
    assert_eq!(
        failed_attempt_rule(OperationFailureClass::Retryable, AttemptState::FinderWriteTransient),
        FailedAttemptRule::Retry(AttemptCount::UpToTheBackoffCap)
    );
    assert_eq!(
        failed_attempt_rule(OperationFailureClass::Retryable, AttemptState::FinderWriteFinal),
        FailedAttemptRule::Retry(AttemptCount::UsesUp)
    );
}

/// ThumbnailWait names the two states of a landed write's thumbnail work.
#[test]
fn thumbnail_wait_names_the_two_states() {
    assert!(ThumbnailWait::Waiting.is_waiting());
    assert!(!ThumbnailWait::Settled.is_waiting());
}

/// A marker a lower layer wrapped stays findable on the bridge's own error, the
/// reach downcast_ref had on the boundary's anyhow::Error.
#[test]
fn an_engine_error_finds_a_marker_wrapped_in_anyhow() {
    let error: EngineError = anyhow::Error::new(crate::download_pin::VersionGone).into();
    assert!(error.is_caused_by::<crate::download_pin::VersionGone>());
    assert!(!error.is_caused_by::<crate::download_pin::PinLost>());
}
