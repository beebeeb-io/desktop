//! Write-queue request-id dedup over the real socket: one request id queues one
//! upload, and cached replies never mask a changed or vanished row.

use super::*;

#[test]
fn concurrent_creates_with_one_request_id_queue_exactly_one_upload() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "big.bin");
    let replies = send_overlapping(&fx, 8, || create_request("big.bin", &path, Some("key-concurrent")));
    assert_eq!(replies.len(), 8);
    for r in &replies {
        assert_write_queued(r);
        assert_eq!(r, &replies[0], "every repeat must get the SAME WriteQueued reply");
    }
    assert_eq!(queued_operation_count(&fx), 1, "one logical create must queue exactly one upload");
}

#[test]
fn a_repeat_after_completion_returns_the_cached_reply_without_queueing() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("a.bin", &path, Some("key-after"))).await;
        assert_write_queued(&first);
        assert_eq!(queued_operation_count(&fx), 1);
        // The retry comes on a NEW connection, well after the first finished.
        let retry = send_one(&fx, create_request("a.bin", &path, Some("key-after"))).await;
        assert_eq!(retry, first);
    });
    assert_eq!(queued_operation_count(&fx), 1, "the retry must not queue a second upload");
}

#[test]
fn different_request_ids_each_queue_their_own_upload() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let one = send_one(&fx, create_request("a.bin", &path, Some("key-one"))).await;
        let two = send_one(&fx, create_request("a.bin", &path, Some("key-two"))).await;
        assert_write_queued(&one);
        assert_write_queued(&two);
        assert_ne!(
            one["WriteQueued"]["item"]["identifier"], two["WriteQueued"]["item"]["identifier"],
            "two different keys are two different creates"
        );
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn a_request_without_a_request_id_behaves_exactly_as_before() {
    // Version skew: the 0.8.6 extension sends no key. Two identical key-less
    // requests are two creates, as they always were.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let one = send_one(&fx, create_request("a.bin", &path, None)).await;
        let two = send_one(&fx, create_request("a.bin", &path, None)).await;
        assert_write_queued(&one);
        assert_write_queued(&two);
        assert_ne!(one, two);
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn a_cached_create_is_not_returned_once_its_row_is_gone() {
    // The false-dedup guard: the same key arriving after the user deleted the
    // file is a NEW create, not a retry.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let first = send_one(&fx, create_request("a.bin", &path, Some("key-gone"))).await;
        assert_write_queued(&first);
        let id = first["WriteQueued"]["item"]["identifier"].as_str().unwrap().to_string();
        fx.db.delete_file(&id).unwrap();
        let again = send_one(&fx, create_request("a.bin", &path, Some("key-gone"))).await;
        assert_write_queued(&again);
        assert_ne!(again["WriteQueued"]["item"]["identifier"], first["WriteQueued"]["item"]["identifier"]);
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn a_request_id_reused_for_another_file_name_is_not_merged() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        let a = send_one(&fx, create_request("a.bin", &path, Some("key-collide"))).await;
        let b = send_one(&fx, create_request("b.bin", &path, Some("key-collide"))).await;
        assert_write_queued(&a);
        assert_write_queued(&b);
        assert_ne!(a["WriteQueued"]["item"]["identifier"], b["WriteQueued"]["item"]["identifier"]);
    });
    assert_eq!(queued_operation_count(&fx), 2);
}

#[test]
fn concurrent_modifies_with_one_request_id_queue_exactly_one_version() {
    let fx = IpcFixture::start(|db| {
        db.upsert_file(&FileEntry {
            file_id: "00000000-0000-0000-0000-00000000aaaa".into(),
            path: "doc.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    });
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "doc.txt");
    let replies = send_overlapping(&fx, 6, || {
        modify_request("00000000-0000-0000-0000-00000000aaaa", "doc.txt", &path, Some("key-modify"))
    });
    for r in &replies {
        assert!(r.get("WriteQueued").is_some(), "expected WriteQueued, got {r}");
        assert_eq!(r, &replies[0]);
    }
    assert_eq!(queued_operation_count(&fx), 1, "one logical modify must queue exactly one version");
}

#[test]
fn an_unusable_request_id_is_refused_not_ignored() {
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let path = source_file(&src, "a.bin");
    fx.rt.block_on(async {
        for bad in ["".to_string(), "x".repeat(129), "line\nbreak".to_string()] {
            let reply = send_one(&fx, create_request("a.bin", &path, Some(&bad))).await;
            let message = reply["Error"]["message"].as_str().unwrap_or_default();
            assert!(message.contains("request_id"), "a bad key must be refused, got {reply}");
        }
    });
    assert_eq!(queued_operation_count(&fx), 0, "a refused request must queue nothing");
}

#[test]
fn a_failed_create_is_not_remembered_so_the_retry_actually_runs() {
    // The first attempt fails (its source file is unreadable), which must not
    // poison the key: the system's retry, now with a readable file, has to queue.
    let fx = IpcFixture::start(|_| {});
    let src = tempfile::tempdir().unwrap();
    let good = source_file(&src, "a.bin");
    let missing = src.path().join("does-not-exist.bin").to_string_lossy().into_owned();
    fx.rt.block_on(async {
        let failed = send_one(&fx, create_request("a.bin", &missing, Some("key-fail"))).await;
        assert!(failed.get("Error").is_some(), "the first attempt must fail, got {failed}");
        assert_eq!(queued_operation_count(&fx), 0);
        let retried = send_one(&fx, create_request("a.bin", &good, Some("key-fail"))).await;
        assert_write_queued(&retried);
    });
    assert_eq!(queued_operation_count(&fx), 1);
}
