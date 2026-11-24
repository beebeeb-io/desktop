//! Upload staging: the daemon uploads from its own copy, deletes the handed-over
//! copy, and refuses contents outside the staging directory.

use super::*;

#[test]
fn staged_create_uploads_from_the_daemons_own_copy_and_deletes_the_handed_over_one() {
    let fx = IpcFixture::start_staged(|_| {});
    let copy = staged_copy(&fx, b"finder file contents");
    fx.rt.block_on(async {
        let reply = send_one(
            &fx,
            create_request("notes.txt", &copy.to_string_lossy(), Some("key-staged")),
        )
        .await;
        assert_write_queued(&reply);
    });
    assert!(
        !copy.exists(),
        "the handed-over copy must be deleted once the daemon has its own"
    );
    assert!(
        staging_entries(&fx).is_empty(),
        "nothing may be left in the staging dir"
    );
    assert_uploads_from_own_copy(&fx, b"finder file contents");
}

/// Spec 2026-10-09 §8.4 (the staging folder, macOS): when the app's own staging folder cannot be
/// created or written to, a create and a modify are answered `WriteRetryLater` with a fixed reason and
/// no path. The extension reports that as a transient error, so the system keeps the change and
/// retries the write. Nothing is staged anywhere, nothing is queued, no row is added, and the
/// handed-over copy is deleted as after any answer. Once the folder can take a copy again, the
/// system's retry with the same request id is queued, exactly once.
#[cfg(target_os = "macos")]
#[test]
fn an_unwritable_staging_folder_answers_write_retry_later_and_stages_and_queues_nothing() {
    const FILE_ID: &str = "00000000-0000-0000-0000-00000000cccc";
    let fx = IpcFixture::start_staged(|db| {
        db.upsert_file(&FileEntry {
            file_id: FILE_ID.into(),
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
    let bases_dir = tempfile::tempdir().unwrap();
    let data = bases_dir.path().join("data");
    std::fs::create_dir(&data).unwrap();
    // A file where the staging folder's parent would be: `beebeeb/finder-writes` cannot be created.
    std::fs::write(data.join("beebeeb"), b"in the way").unwrap();
    fx.bridge.seams.stage_under(crate::engine_bridge::FinderStagingBases {
        folder: "beebeeb",
        data: Some(data.clone()),
        cache: Some(bases_dir.path().join("cache")),
        temp: bases_dir.path().join("temp"),
    });
    let rows_before = fx.db.list_files().unwrap().len();

    for kind in ["create", "modify"] {
        let copy = staged_copy(&fx, b"a save the app cannot stage");
        let contents = copy.to_string_lossy().into_owned();
        let request = match kind {
            "create" => create_request("new.txt", &contents, Some("key-retry-create")),
            _ => modify_request_on_base(FILE_ID, "doc.txt", &contents, "1"),
        };
        let reply = fx.rt.block_on(send_one(&fx, request));
        let message = reply["WriteRetryLater"]["message"]
            .as_str()
            .unwrap_or_else(|| panic!("{kind}: expected WriteRetryLater, got {reply}"));
        assert_eq!(
            reply,
            serde_json::json!({ "WriteRetryLater": { "message": message } }),
            "{kind}: the wire shape"
        );
        assert!(
            message.starts_with("the staging folder is unavailable") && !message.contains('/'),
            "{kind}: a fixed reason, no path: {message}"
        );
        assert!(
            !copy.exists(),
            "{kind}: the handed-over copy is deleted after the answer"
        );
    }

    assert_eq!(queued_operation_count(&fx), 0, "nothing is queued");
    assert!(
        fx.db.staged_payloads_for_signout().unwrap().is_empty(),
        "nothing is journalled"
    );
    assert_eq!(
        fx.db.list_files().unwrap().len(),
        rows_before,
        "the create added no row"
    );
    let mut left = Vec::new();
    let mut stack = vec![bases_dir.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                left.push(path);
            }
        }
    }
    assert_eq!(left, vec![data.join("beebeeb")], "no copy was written anywhere");
    assert!(staging_entries(&fx).is_empty(), "nothing is left in upload staging");

    // The folder can take a copy again. The system's retry carries the SAME request id: the
    // refusal was not remembered, so the retry is accepted and queued, and a repeat of it is
    // answered from that result, so the save is queued exactly once.
    std::fs::remove_file(data.join("beebeeb")).unwrap();
    for attempt in ["the retry", "a repeat of the retry"] {
        let copy = staged_copy(&fx, b"a save the app could not stage before");
        let reply = fx.rt.block_on(send_one(
            &fx,
            create_request("new.txt", &copy.to_string_lossy(), Some("key-retry-create")),
        ));
        assert!(
            reply.get("WriteQueued").is_some(),
            "{attempt}: expected WriteQueued, got {reply}"
        );
        assert_eq!(
            queued_operation_count(&fx),
            1,
            "{attempt}: the save is queued exactly once"
        );
    }
    let ops = fx.db.list_due_operations(i64::MAX).unwrap();
    let own = ops[0].payload_path.as_deref().expect("the upload carries its copy");
    assert!(
        std::path::Path::new(own).starts_with(data.join("beebeeb").join("finder-writes")),
        "the copy is staged in the data root: {own}"
    );
}

#[test]
fn staged_modify_uploads_from_the_daemons_own_copy_and_deletes_the_handed_over_one() {
    let fx = IpcFixture::start_staged(|db| {
        db.upsert_file(&FileEntry {
            file_id: "00000000-0000-0000-0000-00000000bbbb".into(),
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
    let copy = staged_copy(&fx, b"edited contents");
    fx.rt.block_on(async {
        let reply = send_one(
            &fx,
            modify_request(
                "00000000-0000-0000-0000-00000000bbbb",
                "doc.txt",
                &copy.to_string_lossy(),
                Some("key-staged-modify"),
            ),
        )
        .await;
        assert!(reply.get("WriteQueued").is_some(), "expected WriteQueued, got {reply}");
    });
    assert!(
        !copy.exists(),
        "the handed-over copy must be deleted once the daemon has its own"
    );
    assert!(staging_entries(&fx).is_empty());
    assert_uploads_from_own_copy(&fx, b"edited contents");
}

#[test]
fn contents_not_directly_in_the_staging_dir_are_refused_with_their_category_and_never_deleted() {
    let fx = IpcFixture::start_staged(|_| {});
    // A real file right next to the staging dir: outside it, reachable by `..`.
    let outside = fx.staging_parent().join("outside.txt");
    std::fs::write(&outside, b"not handed over").unwrap();
    let traversal = fx.staging_dir().join("..").join("outside.txt");
    let link = fx.staging_dir().join("link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let missing = fx.staging_dir().join("never-staged");
    let cases = [
        (outside.clone(), "outside_staging"),
        (traversal, "traversal"),
        (link.clone(), "symlink"),
        (missing, "missing"),
    ];
    fx.rt.block_on(async {
        for (path, category) in &cases {
            let reply = send_one(&fx, create_request("a.txt", &path.to_string_lossy(), None)).await;
            let message = reply["Error"]["message"].as_str().unwrap_or_default();
            assert!(
                message.contains(category),
                "{category}: the refusal must name its category, got {reply}"
            );
            assert!(
                !message.contains("outside.txt"),
                "a refusal must not echo the path, got {reply}"
            );
        }
    });
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"not handed over",
        "a refused path is never deleted"
    );
    assert!(
        std::fs::symlink_metadata(&link).is_ok(),
        "a refused entry is left for the age-bound purge, not deleted on the request path"
    );
    assert_eq!(queued_operation_count(&fx), 0, "a refused request must queue nothing");
}

#[test]
fn a_write_refused_after_its_contents_were_accepted_still_deletes_the_handed_over_copy() {
    let fx = IpcFixture::start_staged(|db| seed_read_only_shared_folder(db, "shared-read-only"));
    // 1. The engine refuses: a new file in a read-only shared folder.
    let engine_refused = staged_copy(&fx, b"a");
    // 2. The socket refuses: the "Shared with me" namespace root.
    let namespace_refused = staged_copy(&fx, b"b");
    // 3. The idempotency key is unusable.
    let key_refused = staged_copy(&fx, b"c");
    fx.rt.block_on(async {
        let reply = send_one(
            &fx,
            create_request_in(
                "shared-read-only",
                "a.txt",
                &engine_refused.to_string_lossy(),
                Some("key-ro"),
            ),
        )
        .await;
        assert!(reply.get("Error").is_some(), "the engine must refuse, got {reply}");
        let reply = send_one(
            &fx,
            create_request_in(
                "namespace:shared_with_me",
                "b.txt",
                &namespace_refused.to_string_lossy(),
                None,
            ),
        )
        .await;
        assert!(
            reply.get("Error").is_some(),
            "the namespace root must refuse, got {reply}"
        );
        let reply = send_one(&fx, create_request("c.txt", &key_refused.to_string_lossy(), Some(""))).await;
        assert!(
            reply.get("Error").is_some(),
            "an empty key must be refused, got {reply}"
        );
    });
    for copy in [&engine_refused, &namespace_refused, &key_refused] {
        assert!(
            !copy.exists(),
            "a refused request's handed-over copy must be deleted: {}",
            copy.display()
        );
    }
    assert_eq!(queued_operation_count(&fx), 0);
}

#[test]
fn concurrent_staged_creates_with_one_request_id_queue_one_upload_and_delete_every_copy() {
    // Each retry of a timed-out create hands over its OWN copy. Only the first
    // is read; every copy, read or not, must be gone once its request is
    // answered.
    let fx = IpcFixture::start_staged(|_| {});
    let staging = fx.staging_dir();
    let replies = send_overlapping(&fx, 6, || {
        let copy = staging.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&copy, b"big file").unwrap();
        create_request("big.bin", &copy.to_string_lossy(), Some("key-staged-concurrent"))
    });
    assert_eq!(replies.len(), 6);
    for r in &replies {
        assert_write_queued(r);
        assert_eq!(r, &replies[0], "every repeat must get the SAME WriteQueued reply");
    }
    assert!(
        staging_entries(&fx).is_empty(),
        "every handed-over copy must be deleted"
    );
    assert_uploads_from_own_copy(&fx, b"big file");
}

#[test]
fn a_create_still_in_flight_when_the_socket_stops_keeps_its_copy_until_the_engine_has_read_it() {
    // The first attempt of a keyed write runs detached from its connection (the
    // engine finishes it even if the client is gone). When the socket stops,
    // every connection task is aborted; the handed-over copy must stay until
    // that detached work has read it, or the create fails after the fact.
    let mut fx = IpcFixture::start_staged(|_| {});
    let copy = staged_copy(&fx, b"in flight");
    let (release, holder) = hold_database(&fx);
    fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(&create_request(
                "late.txt",
                &copy.to_string_lossy(),
                Some("key-in-flight"),
            ))
            .await
            .unwrap();
        // Blocking on purpose (see `send_overlapping`): let the work reach the held lock.
        std::thread::sleep(Duration::from_millis(300));
    });
    let _ = fx.cancel.take().expect("server running").send(());
    let server = fx.server.take().expect("server running");
    fx.rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("the socket stops")
            .expect("server task")
            .expect("server result");
        // Let the aborted connection tasks be dropped.
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    assert!(
        copy.exists(),
        "the copy must outlive the stopped connection while the engine still needs it"
    );
    release.send(()).unwrap();
    holder.join().unwrap();
    let deadline = std::time::Instant::now() + READ_DEADLINE;
    while (copy.exists() || queued_operation_count(&fx) == 0) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_uploads_from_own_copy(&fx, b"in flight");
    assert!(
        !copy.exists(),
        "once the engine has its own copy the handed-over one is deleted"
    );
}

#[test]
fn a_staged_copy_swapped_for_a_symlink_after_validation_uploads_what_was_handed_over() {
    // The daemon validates the handed-over copy, then the engine reads it,
    // later and on another thread. Whatever happens to the entry in between
    // (here: renamed away and replaced by a symlink to a file the requester
    // never handed over), the engine must read the file that was validated.
    let fx = IpcFixture::start_staged(|_| {});
    let copy = staged_copy(&fx, b"handed over");
    let secret = fx.staging_parent().join("not-handed-over.txt");
    std::fs::write(&secret, b"never handed over").unwrap();
    let (release, holder) = hold_database(&fx);
    let reply = fx.rt.block_on(async {
        let mut client = fx.connect().await;
        client
            .write_all(&create_request(
                "swapped.txt",
                &copy.to_string_lossy(),
                Some("key-swapped"),
            ))
            .await
            .unwrap();
        // Blocking on purpose (see `send_overlapping`): the request is
        // validated, and its work then waits behind the held database.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::rename(&copy, fx.staging_dir().join("moved-away")).unwrap();
        std::os::unix::fs::symlink(&secret, &copy).unwrap();
        release.send(()).unwrap();
        parse(&read_line(&mut client).await)
    });
    holder.join().unwrap();
    assert_write_queued(&reply);
    assert_uploads_from_own_copy(&fx, b"handed over");
    assert_eq!(
        std::fs::read(&secret).unwrap(),
        b"never handed over",
        "the symlink's target is never touched"
    );
}
