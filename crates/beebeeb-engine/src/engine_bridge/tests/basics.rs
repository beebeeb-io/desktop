//! `basics` engine tests, split out of `engine_bridge.rs`.

use super::*;
#[test]
fn test_file_state_machine_transitions() {
    // cloud_only -> downloading -> local
    let mut sm = FileSM::new(FileStatus::CloudOnly);
    sm.transition(FileEvent::DownloadStart).unwrap();
    assert_eq!(sm.state(), FileStatus::Downloading);
    sm.transition(FileEvent::DownloadComplete).unwrap();
    assert_eq!(sm.state(), FileStatus::Local);
}

#[test]
fn test_invalid_transition_rejected() {
    let mut sm = FileSM::new(FileStatus::CloudOnly);
    assert!(sm.transition(FileEvent::UploadStart).is_err());
}

// ── Shared upload-trigger helpers (task 0780) ────────────────────────────
// Relocated from the retired `watcher`-local copies so the one shared
// engine-internal + relative-path logic stays covered.

#[test]
fn engine_internal_filters_state_dir_and_lock() {
    let root = std::path::PathBuf::from("/sync");
    assert!(path_is_engine_internal(&root, &root.join(".beebeeb").join("state.db")));
    assert!(path_is_engine_internal(&root, &root.join(".beebeeb-sync.lock")));
    assert!(path_is_engine_internal(
        &root,
        &root.join("sub").join(".beebeeb").join("x")
    ));
    assert!(!path_is_engine_internal(&root, &root.join("photo.jpg")));
    assert!(!path_is_engine_internal(&root, &root.join("docs").join("a.txt")));
}

#[test]
fn relative_db_path_is_slash_joined_without_leading_slash() {
    let root = std::path::PathBuf::from("/sync");
    assert_eq!(relative_db_path(&root, &root.join("a.txt")).as_deref(), Some("a.txt"));
    assert_eq!(
        relative_db_path(&root, &root.join("docs").join("b.md")).as_deref(),
        Some("docs/b.md")
    );
    assert_eq!(relative_db_path(&root, &std::path::PathBuf::from("/other/c.txt")), None);
}

#[test]
fn test_temp_file_filter_covers_editor_and_system_names() {
    for name in [
        ".DS_Store",
        "._report.pdf",
        "~$budget.xlsx",
        "draft.txt~",
        ".report.swp",
        "upload.tmp",
        "video.crdownload",
    ] {
        assert!(is_ignored_finder_name(name), "{name} should be ignored");
    }
    assert!(!is_ignored_finder_name("report.pdf"));
    assert!(!is_ignored_finder_name(".env.sample"));
}

#[test]
fn test_content_side_from_bytes_covers_text_binary_and_size_cap() {
    // Task 1546 finding 2: the conflict window's diff body was a
    // hardcoded placeholder for EVERY conflict, text or binary,
    // regardless of actual file content. `content_side_from_bytes` is
    // the pure classifier `EngineBridge::conflict_content_preview` uses
    // to turn real bytes into what the window can safely show.
    let small_text = content_side_from_bytes(b"hello world".to_vec(), ContentKind::Text);
    assert_eq!(small_text.text.as_deref(), Some("hello world"));
    assert_eq!(small_text.size_bytes, Some(11));
    assert!(small_text.unavailable_reason.is_none());

    // Binary side never carries text, even for tiny content — only size.
    let binary = content_side_from_bytes(b"hello world".to_vec(), ContentKind::Binary);
    assert!(binary.text.is_none());
    assert_eq!(binary.size_bytes, Some(11));
    assert!(binary.unavailable_reason.is_none());

    // Invalid UTF-8 for a file the caller marked as text: honest
    // "isn't valid UTF-8" reason, never fabricated text.
    let invalid_utf8 = content_side_from_bytes(vec![0xFF, 0xFE, 0xFD], ContentKind::Text);
    assert!(invalid_utf8.text.is_none());
    assert_eq!(invalid_utf8.size_bytes, Some(3));
    assert!(invalid_utf8.unavailable_reason.unwrap().contains("UTF-8"));

    // Oversized text: the whole file round-trips over Tauri's IPC as a
    // JSON string and is diffed synchronously, so there is a real cap —
    // over it, size is still reported honestly but text is withheld
    // rather than silently truncated (which would corrupt the diff).
    let oversized = vec![b'a'; CONFLICT_PREVIEW_TEXT_MAX_BYTES + 1];
    let too_big = content_side_from_bytes(oversized, ContentKind::Text);
    assert!(too_big.text.is_none());
    assert_eq!(too_big.size_bytes, Some((CONFLICT_PREVIEW_TEXT_MAX_BYTES + 1) as u64));
    assert!(too_big.unavailable_reason.unwrap().contains("too large"));
}

#[test]
fn test_base_version_parser_reads_current_version_prefix() {
    assert_eq!(parse_base_version_number(Some("7:1700000000:1024")), Some(7));
    assert_eq!(parse_base_version_number(Some("0:1700000000:1024")), None);
    assert_eq!(parse_base_version_number(Some("local:1700000000")), None);
    assert_eq!(parse_base_version_number(None), None);
}

#[test]
fn test_upload_init_request_sends_the_proposed_id_and_preserves_existing_base() {
    // SF-0001 (L13): the init request always carries the id this device chose,
    // for a create as much as for a replace.
    let req = upload_init_request_for_operation(
        "file-new",
        "{\"cipher_suite\":\"V1Aes256Gcm\"}",
        Some("image/png".into()),
        Some("folder-1".into()),
        11,
        None,
    );
    assert_eq!(req.file_id.as_deref(), Some("file-new"));
    assert_eq!(req.file_size_bytes, 11);
    assert_eq!(req.profile, "desktop");
    assert!(req.is_media);
    assert_eq!(req.chunk_count, Some(1));

    let replace = upload_init_request_for_operation(
        "file-existing",
        "{\"cipher_suite\":\"V1Aes256Gcm\"}",
        Some("text/plain".into()),
        None,
        42,
        Some(7),
    );
    assert_eq!(replace.file_id.as_deref(), Some("file-existing"));
    assert_eq!(replace.base_version_number, Some(7));
    assert!(!replace.is_media);
}

pub(super) fn write_test_png(path: &Path) {
    let image = image::RgbaImage::from_fn(8, 6, |x, y| {
        image::Rgba([(x * 31) as u8, (y * 41) as u8, ((x + y) * 17) as u8, 255])
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    std::fs::write(path, png.into_inner()).unwrap();
}

#[test]
fn test_prepare_thumbnail_uploads_encrypts_medium_and_large_with_blurhash() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("photo.png");
    write_test_png(&payload);

    let master_key = beebeeb_core::kdf::MasterKey::from_bytes([31u8; 32]);
    let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-file-1");
    let uploads = prepare_thumbnail_uploads_for_plaintext_media(&payload, Some("image/png"), &file_key).unwrap();

    assert_eq!(uploads.len(), 2);
    assert_eq!(uploads[0].variant, ThumbnailUploadVariant::Medium);
    assert_eq!(uploads[1].variant, ThumbnailUploadVariant::Large);
    assert!(uploads[0].blurhash.as_ref().is_some_and(|hash| hash.len() <= 64));
    assert!(uploads[1].blurhash.is_none());

    for upload in &uploads {
        assert!(upload.encrypted.len() <= upload.variant.encrypted_max_bytes());
        let plaintext = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &upload.encrypted).unwrap();
        assert!(plaintext.starts_with(b"RIFF"), "thumbnail plaintext must be WebP");
        assert_ne!(&*upload.encrypted, plaintext.as_slice());
    }
}

#[test]
fn test_prepare_thumbnail_uploads_extracts_video_frame_when_ffmpeg_available() {
    if std::process::Command::new("ffmpeg").arg("-version").output().is_err() {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("clip.mp4");
    let status = std::process::Command::new("ffmpeg")
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("testsrc=size=16x16:rate=1")
        .arg("-t")
        .arg("1")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg(&payload)
        .status()
        .unwrap();
    if !status.success() {
        return;
    }

    let master_key = beebeeb_core::kdf::MasterKey::from_bytes([32u8; 32]);
    let file_key = beebeeb_core::kdf::derive_file_key(&master_key, b"server-video-1");
    let uploads = prepare_thumbnail_uploads_for_plaintext_media(&payload, Some("video/mp4"), &file_key).unwrap();

    assert_eq!(uploads.len(), 2);
    assert_eq!(uploads[0].variant, ThumbnailUploadVariant::Medium);
    assert_eq!(uploads[1].variant, ThumbnailUploadVariant::Large);
    assert!(uploads[0].blurhash.is_none(), "video thumbnails do not carry blurhash");
    assert!(uploads[1].blurhash.is_none());

    for upload in &uploads {
        let plaintext = beebeeb_core::encrypt::decrypt_chunk_raw(&file_key, &upload.encrypted).unwrap();
        assert!(plaintext.starts_with(b"RIFF"), "video thumbnail plaintext must be WebP");
    }
}

#[test]
fn test_download_chunk_decrypts_raw_and_json_formats() {
    let file_id = uuid::Uuid::new_v4().to_string();
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes([9u8; 32]);
    let file_key = beebeeb_core::kdf::derive_file_key(&master_key, file_id.as_bytes());

    let raw = beebeeb_core::encrypt::encrypt_chunk_raw(&file_key, b"raw bytes").unwrap();
    assert_eq!(test_open_content_frame(&file_key, &raw).unwrap(), b"raw bytes");

    let blob = beebeeb_core::encrypt::encrypt_chunk(&file_key, b"json bytes").unwrap();
    let json = serde_json::to_vec(&blob).unwrap();
    assert_eq!(test_open_content_frame(&file_key, &json).unwrap(), b"json bytes");
}

#[test]
fn test_staging_payload_copies_to_durable_location() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("report.txt");
    std::fs::write(&source, b"finder data").unwrap();
    let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
    let staged = crate::staged_payload::StagedPayload::copy(db.clone(), &source, dir.path().join("staging")).unwrap();
    assert_eq!(std::fs::read(staged.path()).unwrap(), b"finder data");
    assert_eq!(db.staged_payloads_for_signout().unwrap().len(), 1);
}

include!("fixture_upload_server.rs");
