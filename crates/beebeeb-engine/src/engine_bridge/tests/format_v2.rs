//! Chunk format v2 through the engine (core 0.3.0): what an upload sends, what a read accepts, and
//! what a retry does. The unit rules are in `content_record_tests.rs`; here the same rules run through
//! `EngineBridge` against a mock that behaves like the server. Each test names the mutant it fails on.

use super::*;
use crate::content_record::ContentRecord;

const WRITE_CAPABILITIES: &str = "file-write-v2, version-metadata-v1";

fn key_of(master_key: [u8; 32], file_id: &str) -> beebeeb_core::kdf::FileKey {
    hydration_test_key(master_key, file_id)
}

fn record_on_server(server: &VersionedServerMock, master_key: [u8; 32], file_id: &str) -> ContentRecord {
    let name = server.state.lock().unwrap().authenticated_names[file_id].clone();
    ContentRecord::of(
        &beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key_of(master_key, file_id), &name)
            .expect("the server's name for the file opens under its key"),
    )
}

/// A file created through the engine and landed on the mock: its server id.
async fn created(
    bridge: &EngineBridge,
    server: &VersionedServerMock,
    dir: &Path,
    sync_root: &Path,
    bytes: &[u8],
) -> String {
    let contents = dir.join(format!("create-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&contents, bytes).unwrap();
    bridge
        .queue_file_provider_create(finder_file_target(None, "notes.txt", &contents, None))
        .unwrap();
    drain_upload_queue(bridge, sync_root).await;
    let ids = server.state.lock().unwrap().files_with_content();
    assert_eq!(ids.len(), 1, "one file on the server after the create");
    ids[0].clone()
}

/// v2 write, then read, through the engine: a created file's content comes back from the server
/// byte for byte, and so does the next version of it. The name the server holds carries the
/// commitment of the version it serves, with the counter one higher per version.
/// Mutants: a reader built from the pin alone (the fetch fails: v2 opens only against a commitment);
/// a writer that sends no completion name (the mock then holds no name with a commitment).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_v2_upload_reads_back_through_the_engine_and_so_does_its_next_version() {
    let master_key = [171u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    server.state.lock().unwrap().serve_downloads = true;
    let first = b"the first version, written as chunk format v2";
    let id = created(&bridge, &server, dir.path(), &sync_root, first).await;

    let record = record_on_server(&server, master_key, &id);
    assert_eq!(
        record.format,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::V2,
        "a new file records the format its content was written in"
    );
    assert_eq!(record.version.map(|version| version.counter), Some(1));
    assert_eq!(
        &**bridge.hydrate_file_to_memory_metered(&id, None).await.unwrap().bytes,
        first
    );

    let second = b"the second version: longer, and under its own commitment";
    fp_save(
        &bridge,
        dir.path(),
        &id,
        "notes.txt",
        second,
        &held_content_version(&bridge, &id),
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let replaced = record_on_server(&server, master_key, &id);
    assert_eq!(replaced.format, record.format, "a new version keeps the file's pin");
    assert_eq!(
        replaced.version.map(|version| version.counter),
        Some(2),
        "a replacement's counter follows the one before it"
    );
    assert_ne!(
        replaced.version.map(|version| version.version_id),
        record.version.map(|version| version.version_id),
        "every upload has its own version id"
    );
    assert_eq!(
        &**bridge.hydrate_file_to_memory_metered(&id, None).await.unwrap().bytes,
        second
    );

    let state = server.finish();
    assert_eq!(state.latest_plaintext(&id, master_key), second);
    // The replacement's init named the file's current sealed name, as the server requires for a
    // protected version; the create's named none.
    let inits: Vec<&serde_json::Value> = state.inits.iter().map(|(body, _)| body).collect();
    assert_eq!(inits.len(), 2);
    assert!(inits[0].get("version_metadata_encrypted").is_none());
    let current_at_replace = inits[1]["version_metadata_encrypted"]
        .as_str()
        .expect("a replacement of a protected version names the current metadata");
    let opened =
        beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key_of(master_key, &id), current_at_replace).unwrap();
    assert_eq!(ContentRecord::of(&opened), record);
    // No upload ever sent a name in a metadata update: the name travels with `complete`.
    assert!(
        state
            .patches
            .iter()
            .all(|(_, body)| body.get("name_encrypted").is_none()),
        "{:?}",
        state.patches
    );
}

/// Core's ruling of 2026-11-07, through the engine: v2 content whose name carries no commitment is
/// refused, whatever the name's pin says, and the same content is read once the name carries it.
/// Mutant: a reader that takes the version id from the served frame 0 (both refusals then pass).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_content_under_a_name_without_a_commitment_is_refused() {
    let master_key = [172u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    server.state.lock().unwrap().serve_downloads = true;
    let bytes = b"content that must not open without its commitment";
    let id = created(&bridge, &server, dir.path(), &sync_root, bytes).await;
    let committed = server.state.lock().unwrap().authenticated_names[&id].clone();
    let key = key_of(master_key, &id);

    for (what, pin) in [
        ("a v2 pin", beebeeb_core::chunk_stream::ExpectedChunkFormat::V2),
        ("no pin", beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded),
    ] {
        // The name a server would hold had the commitment been dropped (or never published).
        let stripped = beebeeb_core::encrypt::encrypt_name_for(&key, "notes.txt", None, pin).unwrap();
        server
            .state
            .lock()
            .unwrap()
            .authenticated_names
            .insert(id.clone(), stripped);
        assert!(
            bridge.hydrate_file_to_memory_metered(&id, None).await.is_err(),
            "{what} and no commitment: the v2 content must not open"
        );
    }
    // The control: the refusals above are the missing commitment's, not the fetch's.
    server
        .state
        .lock()
        .unwrap()
        .authenticated_names
        .insert(id.clone(), committed);
    assert_eq!(
        &**bridge.hydrate_file_to_memory_metered(&id, None).await.unwrap().bytes,
        bytes
    );
    drop(server.finish());
}

/// A file from before chunk format v2 (v1 frames, a name that records nothing) is still read, and
/// its next version is v2 under the kept unrecorded pin, with a commitment.
/// Mutant: a writer that records `chunk_format: 2` on a file with v1 history.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_v1_file_is_still_read_and_its_next_version_is_v2_under_the_kept_pin() {
    let master_key = [173u8; 32];
    let file = "d6a1e3f0-5c0b-5b7e-9c55-0f6a3a1c7b21";
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    let key = key_of(master_key, file);
    let old = b"a file an older build uploaded";
    {
        // Version 1 as an older build left it: one v1 frame, and the seeded name records nothing.
        let mut state = server.state.lock().unwrap();
        state.serve_downloads = true;
        state.files.get_mut(file).unwrap().versions[0] =
            vec![beebeeb_core::encrypt::encrypt_chunk_raw(&key, old).unwrap()];
    }
    assert_eq!(record_on_server(&server, master_key, file), ContentRecord::UNRECORDED);
    assert_eq!(
        &**bridge.hydrate_file_to_memory_metered(file, None).await.unwrap().bytes,
        old
    );

    let new = b"the same file, saved again by this build";
    fp_save(&bridge, dir.path(), file, "notes.txt", new, "1");
    drain_upload_queue(&bridge, &sync_root).await;
    let record = record_on_server(&server, master_key, file);
    assert_eq!(
        record.format,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
        "v2 is never recorded on a file with v1 history"
    );
    assert_eq!(record.version.map(|version| version.counter), Some(1));
    assert_eq!(
        &**bridge.hydrate_file_to_memory_metered(file, None).await.unwrap().bytes,
        new
    );
    assert_eq!(server.finish().latest_plaintext(file, master_key), new);
}

/// `complete` committed on the server and its answer was lost. The retry finds every chunk
/// acknowledged and sends `complete` again with the name it stored before the first chunk: no second
/// init, no chunk sent twice, and never an abandon (which would be aimed at a version that is now
/// the file's current one). The repeat's answer names nothing, so the name is published by a
/// metadata update before the upload lands.
/// Mutant: a retry that restarts whenever a session row exists (a second init, an abandon).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_complete_answer_is_completed_again_and_never_abandoned() {
    let master_key = [174u8; 32];
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    server
        .state
        .lock()
        .unwrap()
        .complete_reply_lost_once
        .insert("session-1".into());
    let bytes = b"completed on the server, answer lost on the way";
    let id = created(&bridge, &server, dir.path(), &sync_root, bytes).await;

    let state = server.finish();
    let count = |method: &str, path: &str| state.requests.iter().filter(|(m, p)| m == method && p == path).count();
    assert_eq!(
        count("POST", "/api/v1/uploads/init"),
        1,
        "one session: {:?}",
        state.requests
    );
    assert_eq!(
        count("PUT", "/api/v1/uploads/session-1/chunks/0"),
        1,
        "the chunk is sent once"
    );
    assert_eq!(
        count("POST", "/api/v1/uploads/session-1/complete"),
        2,
        "complete, and complete again"
    );
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|(_, p)| p.ends_with("/upload/abandon"))
            .count(),
        0,
        "a session whose complete may have committed is never abandoned"
    );
    assert_eq!(state.files[&id].versions.len(), 1, "one version, not two");
    assert_eq!(state.latest_plaintext(&id, master_key), bytes);
    // The repeat answered `already_completed` with no name, so the stored completion name went
    // out as a metadata update: exactly the name the server already holds from the first complete.
    let published: Vec<&serde_json::Value> = state
        .patches
        .iter()
        .filter(|(file, body)| file == &id && body.get("name_encrypted").is_some())
        .map(|(_, body)| body)
        .collect();
    assert_eq!(published.len(), 1, "{:?}", state.patches);
    assert_eq!(
        published[0]["name_encrypted"],
        serde_json::json!(state.authenticated_names[&id])
    );
    assert_eq!(bridge.db.highest_file_version(&id).unwrap(), Some(1));
}

/// `X-Beebeeb-Capabilities` goes on the requests whose server behaviour it gates, with the value
/// core's own client sends, and on no other (core's rule: never globally). Here: init, the metadata
/// update, the chunk and complete carry it; the abandon of a refused upload does not.
/// Mutant: an upload request without the header (the server's stage 2 would refuse the write).
#[tokio::test]
async fn the_upload_session_and_the_metadata_update_declare_their_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.txt");
    std::fs::write(&payload, b"live upload payload").unwrap();
    let op = |op_id: &str, file_id: &str, max_attempts: i64| PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(file_id.into()),
        parent_id: Some("846c7080-936d-5ffd-879c-39f630bfb30d".into()),
        target_path: Some("Reports/report.txt".into()),
        metadata_json: Some(
            serde_json::json!({
                "operation": "create_file",
                "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\"}",
                "display_name": "report.txt",
                "content_type": "text/plain"
            })
            .to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    };

    let server = UploadMockServer::start(false);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [175u8; 32]);
    bridge
        .db
        .enqueue_operation(&op("op-capabilities", "6cc778db-0fc3-5a51-927a-b6e4a06d4864", 5))
        .unwrap();
    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert_eq!(outcome.completed_op_ids, vec!["op-capabilities".to_string()]);
    let requests = server.finish();
    let seen: Vec<(String, Option<&str>)> = requests
        .iter()
        .map(|r| (format!("{} {}", r.method, r.path), r.capabilities.as_deref()))
        .collect();
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(seen[0].0.starts_with("POST /api/v1/uploads/init"));
    assert!(seen[1].0.starts_with("PATCH /api/v1/files/"));
    assert!(seen[2].0.starts_with("PUT /api/v1/uploads/"));
    assert!(seen[3].0.ends_with("/complete"));
    for (request, capabilities) in &seen {
        assert_eq!(*capabilities, Some(WRITE_CAPABILITIES), "{request}");
    }

    // A refused chunk, retries used up: the abandon is not a gated request and declares nothing.
    let payload_2 = dir.path().join("payload-2.txt");
    std::fs::write(&payload_2, b"refused").unwrap();
    let server = UploadMockServer::start(true);
    let bridge = test_bridge_with_api(&dir.path().join("state-2.db"), server.base_url.clone(), [176u8; 32]);
    let mut refused = op("op-refused", "78cd3eb2-6aee-527c-ad1c-9130e4ed62ad", 1);
    refused.payload_path = Some(payload_2.to_string_lossy().into_owned());
    refused.parent_id = None;
    bridge.db.enqueue_operation(&refused).unwrap();
    bridge.process_due_operations(dir.path(), 200).await.unwrap();
    let requests = server.finish();
    let abandon: Vec<&RecordedRequest> = requests
        .iter()
        .filter(|r| r.path.ends_with("/upload/abandon"))
        .collect();
    assert_eq!(abandon.len(), 1);
    assert_eq!(abandon[0].capabilities, None, "an abandon declares no capability");
}

/// A rename queued before an upload of the same file completed carries the previous version's
/// commitment. Sent as queued it would make the new version unreadable, so it is sealed again at
/// send time with the item's current record. A queued name that already carries it is sent as it is.
/// Mutant: sending the queued name unchanged (its record is the old one).
#[test]
fn a_queued_name_is_sealed_again_when_the_items_record_moved_on() {
    let dir = tempfile::tempdir().unwrap();
    let master = [177u8; 32];
    let bridge = test_bridge(&dir.path().join("state.db"));
    let id = "5b1f7d0e-6a39-4f6e-8a57-2d9b6f0c4e11";
    let key = key_of(master, id);
    let seal = |name: &str, counter: u64| {
        ContentRecord::pin_only(beebeeb_core::chunk_stream::ExpectedChunkFormat::V2)
            .seal_name_for_version(
                &key,
                name,
                Some("text/plain"),
                beebeeb_core::chunk_stream::VersionCommitment::new([counter as u8; 12], counter).unwrap(),
            )
            .unwrap()
    };
    let record_of = |sealed: &str| {
        let opened = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, sealed).unwrap();
        (ContentRecord::of(&opened), opened.name, opened.mime_type)
    };
    // Version 1 is current when the rename is queued; version 2 lands before it is sent.
    let queued = seal("renamed.txt", 1);
    let published = seal("notes.txt", 2);
    bridge.db.observe_file_version(id, 2, Some(&published)).unwrap();

    let sent = upload_v2::queued_name_for_sending(&bridge.db, &master, id, &queued);
    assert_ne!(
        sent, queued,
        "the queued name carries the previous version's commitment"
    );
    let (record, name, mime) = record_of(&sent);
    assert_eq!(
        record,
        record_of(&published).0,
        "the sent name carries the current record"
    );
    assert_eq!((name.as_str(), mime.as_deref()), ("renamed.txt", Some("text/plain")));

    // A name queued after the upload landed already carries the current record: sent as queued.
    let current = record_of(&published).0.seal_name(&key, "again.txt", None).unwrap();
    assert_eq!(
        upload_v2::queued_name_for_sending(&bridge.db, &master, id, &current),
        current
    );
    // A name this device cannot open under the item's key is sent as queued, never dropped.
    assert_eq!(
        upload_v2::queued_name_for_sending(&bridge.db, &master, id, "not a sealed name"),
        "not a sealed name"
    );
}

/// The resume rule: `complete` again only when the server holds every chunk of the same bytes;
/// anything else restarts. Mutant: "some chunks" taken for "all of them".
#[test]
fn only_a_session_with_every_chunk_acknowledged_is_completed_again() {
    use upload_v2::{ResumeStep, resume_step};
    assert_eq!(resume_step(true, 3, 3), ResumeStep::CompleteOnly);
    for (same_payload, acked, count) in [(true, 0, 3), (true, 1, 3), (true, 2, 3), (false, 3, 3), (true, 0, 0)] {
        assert_eq!(
            resume_step(same_payload, acked, count),
            ResumeStep::Restart,
            "same_payload {same_payload}, {acked} of {count}"
        );
    }
}

/// A replacement's counter is above both the record's and the one this device remembers (core's
/// rule), whichever is higher. Mutant: taking the record's counter alone.
#[test]
fn a_new_versions_counter_is_above_the_record_and_the_remembered_one() {
    use upload_v2::previous_counter;
    assert_eq!(previous_counter(None, None), None);
    assert_eq!(previous_counter(Some(2), None), Some(2));
    assert_eq!(previous_counter(None, Some(5)), Some(5));
    assert_eq!(previous_counter(Some(2), Some(5)), Some(5));
    assert_eq!(previous_counter(Some(7), Some(5)), Some(7));
}

/// A version restore of a protected file publishes fresh metadata, as core's client builds it: the
/// restored version's own id under a counter above every one in sight, with the file's current name.
/// A file with no protected version restores with no body; an unprotected version is never restored
/// over a protected file; a version that is not listed is refused.
#[test]
fn a_restore_publishes_the_restored_versions_id_under_a_higher_counter() {
    let master = [178u8; 32];
    let id = "0c2f4b7a-93d1-4b0e-a1c4-7e5d2a9b8f33";
    let key = key_of(master, id);
    let seal = |name: &str, tag: u8, counter: u64| {
        ContentRecord::pin_only(beebeeb_core::chunk_stream::ExpectedChunkFormat::V2)
            .seal_name_for_version(
                &key,
                name,
                Some("text/plain"),
                beebeeb_core::chunk_stream::VersionCommitment::new([tag; 12], counter).unwrap(),
            )
            .unwrap()
    };
    let v1 = seal("draft.txt", 1, 1);
    let v2 = seal("final.txt", 2, 2);
    let versions = serde_json::json!({ "file_id": id, "versions": [
        { "id": "version-2", "version_metadata_encrypted": v2 },
        { "id": "version-1", "version_metadata_encrypted": v1 },
    ]});

    // Restore version 1 while version 2 is current and this device once saw counter 4.
    let (sealed, counter) = upload_v2::restore_metadata(&key, &v2, &versions, "version-1", Some(4))
        .unwrap()
        .expect("a protected file restores with fresh metadata");
    assert_eq!(counter, 5, "above the record's 2 and the remembered 4");
    let opened = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, &sealed).unwrap();
    let version = opened.version.unwrap();
    assert_eq!(
        version.version_id, [1; 12],
        "the restored chunks keep their own version id"
    );
    assert_eq!(version.counter, 5);
    assert_eq!(opened.name, "final.txt", "the file keeps its current name");

    // No version protected, nothing remembered: the restore sends no body, as before.
    let plain = ContentRecord::UNRECORDED.seal_name(&key, "old.txt", None).unwrap();
    let unprotected = serde_json::json!({ "file_id": id, "versions": [{ "id": "version-1" }] });
    assert!(
        upload_v2::restore_metadata(&key, &plain, &unprotected, "version-1", None)
            .unwrap()
            .is_none()
    );
    // An unprotected version over a protected file has no commitment to open against.
    let mixed = serde_json::json!({ "file_id": id, "versions": [
        { "id": "version-2", "version_metadata_encrypted": v2 },
        { "id": "version-1" },
    ]});
    assert!(upload_v2::restore_metadata(&key, &v2, &mixed, "version-1", None).is_err());
    // A version the listing does not name.
    assert!(upload_v2::restore_metadata(&key, &v2, &versions, "version-9", None).is_err());
}

/// What this device remembers of a file's versions is monotonic: a lower counter never replaces a
/// higher one, nor does its name; a new session's row carries no completion name of the old one.
#[test]
fn the_remembered_counter_and_name_only_move_forward() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    assert_eq!(db.highest_file_version("f").unwrap(), None);
    assert_eq!(db.published_name("f").unwrap(), None);
    db.observe_file_version("f", 2, Some("name-2")).unwrap();
    db.observe_file_version("f", 1, Some("name-1")).unwrap();
    assert_eq!(db.highest_file_version("f").unwrap(), Some(2));
    assert_eq!(db.published_name("f").unwrap().as_deref(), Some("name-2"));
    db.observe_file_version("f", 3, None).unwrap();
    assert_eq!(db.highest_file_version("f").unwrap(), Some(3));
    assert_eq!(
        db.published_name("f").unwrap(),
        None,
        "a higher counter seen elsewhere retires the name published under the lower one"
    );

    let resume = |session: &str| UploadResume {
        op_id: "op".into(),
        payload_path: "/staged/op".into(),
        payload_size: 3,
        payload_mtime_ns: 1,
        upload_session_id: session.into(),
        server_file_id: "f".into(),
        object_version_id: "object".into(),
        chunk_size_bytes: 3,
        chunk_count: 1,
        acked_chunks: 0,
        metadata_applied: false,
        is_create: false,
        self_minted: true,
        completed_version: None,
        completed_object_version_id: None,
        completed_mime_type: None,
    };
    db.put_upload_resume(&resume("session-1")).unwrap();
    db.set_upload_completion_name("op", "sealed-for-session-1").unwrap();
    assert_eq!(
        db.upload_completion_name("op").unwrap().as_deref(),
        Some("sealed-for-session-1")
    );
    db.put_upload_resume(&resume("session-2")).unwrap();
    assert_eq!(
        db.upload_completion_name("op").unwrap(),
        None,
        "a fresh session never completes with the name of the one before it"
    );
}
