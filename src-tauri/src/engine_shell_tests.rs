//! Engine tests whose last step is the shell's sign-out purge or its account reset (`local_state_purge`,
//! `session_reauth`), which the engine crate cannot name. They run here on the engine's own test fixtures: the
//! fixture files are the engine's, included as text, so both crates build the same set-up.

use crate::api_client::ApiClient;
use crate::engine_bridge::*;
use crate::state_db::{
    BackupEnqueue, ClaimOutcome, FileEntry, FileStatus, ItemKind, Namespace, PERMISSION_READ, StateDb,
};
use base64::Engine as _;
use beebeeb_engine::content_record::ContentRecord;
use serde_json::json;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

include!("../../crates/beebeeb-engine/src/engine_bridge/tests/fixture_http.rs");
include!("../../crates/beebeeb-engine/src/engine_bridge/tests/fixture_upload_server.rs");
include!("../../crates/beebeeb-engine/src/engine_bridge/tests/fixture_backup.rs");
include!("../../crates/beebeeb-engine/src/engine_bridge/tests/fixture_seed.rs");

fn count_requests(requests: &[RecordedRequest], method: &str, path: &str) -> usize {
    requests.iter().filter(|r| r.method == method && r.path == path).count()
}

/// Spec §6.1 point 4, the sign-out step of the engine's test of the same name (which checks the four other steps
/// and this one's set-up): with backup ops queued, one with an open session, the sign-out purge (the same function
/// the R10 reset runs) drops the queue and the session and deletes nothing of the source.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_upload_leaves_source_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let in_flight = person_file(dir.path(), "in-flight.txt", b"chunk 0 fails, the session stays");
    let waiting = person_file(dir.path(), "waiting.txt", b"queued, never started");
    let before = (source_state(&in_flight), source_state(&waiting));
    let server = UploadMockServer::start(true);
    let (bridge, sync_root) = backup_bridge(dir.path(), server.base_url.clone(), ALICE);
    let in_flight_op = enqueue(&bridge, &in_flight);
    let outcome = bridge.process_due_operations(&sync_root, 200).await.unwrap();
    assert_eq!(
        outcome.retried_op_ids,
        vec![in_flight_op.clone()],
        "chunk 0 failed, so it retries"
    );
    assert!(
        bridge.db.backup_upload_resume(&in_flight_op).unwrap().is_some(),
        "with its session kept"
    );
    enqueue(&bridge, &waiting);
    assert_eq!(backup_ops(&bridge), 2, "precondition: two backup ops are queued");
    crate::local_state_purge::purge_local_state_files_in(&bridge.db, Some(&sync_root), &[], None).unwrap();
    assert_eq!(backup_ops(&bridge), 0, "the sign-out purge drops the queue");
    assert_eq!(
        count_rows(&bridge, "backup_upload_resume"),
        0,
        "and the session with it"
    );
    server.finish();
    assert_eq!(
        (source_state(&in_flight), source_state(&waiting)),
        before,
        "sign-out: a source changed"
    );
}

/// Review Focus 3 (R10): an account switch with backups queued never uses the old account's records or ops for
/// the new one. The R10 reset (the engine start's `bind_before_engine_start`, as production runs it) clears the
/// records, the ops and their sessions; on Windows the start is refused instead. Even when no reset ran, the new
/// account reads none of the old one's records and its uploader sends none of the old one's ops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn account_switch_resets_backup_records_and_ops() {
    // Alice backed up one file and has another queued, its session open.
    async fn alices_backups(dir: &Path, server: &str) -> (EngineBridge, PathBuf, String) {
        let (bridge, sync_root) = backup_bridge(dir, server.to_string(), ALICE);
        let done = person_file(dir, "done.txt", b"backed up for alice");
        let queued = person_file(dir, "queued.txt", b"queued for alice");
        enqueue(&bridge, &done);
        bridge.process_due_operations(&sync_root, 200).await.unwrap();
        assert_eq!(
            bridge.db.backup_records("pictures").unwrap().len(),
            1,
            "precondition: alice has a record"
        );
        let queued_op = enqueue(&bridge, &queued);
        (bridge, sync_root, queued_op)
    }

    // The R10 reset.
    {
        let dir = tempfile::tempdir().unwrap();
        let server = UploadMockServer::start(false);
        let (bridge, _sync_root, queued_op) = alices_backups(dir.path(), &server.base_url).await;
        bridge
            .db
            .put_backup_upload_resume_claimed(
                &crate::state_db::BackupUploadResume {
                    op_id: queued_op.clone(),
                    server_file_id: "00000000-0000-4000-8000-000000000001".into(),
                    upload_session_id: "s".into(),
                    object_version_id: "o".into(),
                    chunk_size_bytes: 8,
                    chunk_count: 1,
                    acked_chunks: 0,
                    chunk_format: 1,
                    fingerprint: beebeeb_backup::Fingerprint {
                        size: 1,
                        modified_ms: 1,
                        file_id: None,
                    },
                },
                &{
                    let ClaimOutcome::Claimed(claimed) = bridge.db.claim_operation(&queued_op, 300).unwrap() else {
                        panic!("claimable");
                    };
                    claimed.claim_id
                },
            )
            .unwrap();
        assert_eq!(
            (
                count_rows(&bridge, "backup_records"),
                backup_ops(&bridge),
                count_rows(&bridge, "backup_upload_resume")
            ),
            (1, 1, 1),
            "precondition: alice's record, op and session"
        );
        let data = crate::session_reauth::StateDbLocalData {
            db: StateDb::open(dir.path().join("state.db")).unwrap(),
            sync_root: None,
            staging_dirs: Vec::new(),
            kept: Default::default(),
        };
        let bound =
            crate::account_binding::bind_before_engine_start(&data, &identity(BOB), !cfg!(target_os = "windows"));
        if cfg!(target_os = "windows") {
            assert_eq!(bound, Err(crate::account_binding::OTHER_ACCOUNT_ON_WINDOWS.to_string()));
            assert_eq!(
                bridge.db.owner().unwrap(),
                Some(identity(ALICE)),
                "Windows refuses; nothing starts"
            );
        } else {
            assert_eq!(bound, Ok(crate::account_binding::Bound::Reset));
            assert_eq!(
                (
                    count_rows(&bridge, "backup_records"),
                    backup_ops(&bridge),
                    count_rows(&bridge, "backup_upload_resume")
                ),
                (0, 0, 0),
                "the reset cleared alice's records, ops and sessions"
            );
            assert_eq!(bridge.db.owner().unwrap(), Some(identity(BOB)));
            assert!(bridge.db.backup_records("pictures").unwrap().is_empty());
        }
        server.finish();
    }

    // No reset ran (defence in depth): bob reads none of alice's records, and alice's op is never sent for him.
    {
        let dir = tempfile::tempdir().unwrap();
        let server = UploadMockServer::start(false);
        let (bridge, sync_root, queued_op) = alices_backups(dir.path(), &server.base_url).await;
        let inits_before = count_requests(&server.requests.lock().unwrap(), "POST", "/api/v1/uploads/init");
        bridge.db.set_owner(&identity(BOB)).unwrap();
        assert!(
            bridge.db.backup_records("pictures").unwrap().is_empty(),
            "alice's records are not bob's"
        );
        assert_eq!(
            count_rows(&bridge, "backup_records"),
            1,
            "they are still on disk, unread"
        );
        let outcome = bridge.process_due_operations(&sync_root, 300).await.unwrap();
        assert_eq!(
            outcome.retried_op_ids,
            vec![queued_op],
            "alice's op is dropped, not sent"
        );
        assert_eq!(backup_ops(&bridge), 0);
        let requests = server.finish();
        assert_eq!(
            count_requests(&requests, "POST", "/api/v1/uploads/init"),
            inits_before,
            "no upload was started for bob from alice's queue"
        );
        // And a record is only ever written for the owner the op was queued for.
        bridge.db.set_owner(&identity(ALICE)).unwrap();
        assert_eq!(
            bridge.db.backup_records("pictures").unwrap().len(),
            1,
            "alice's record is hers again"
        );
    }
}

mod sf2237d {
    use super::*;
    use crate::engine_bridge::LEGACY_CHILD_READS;
    include!("../../crates/beebeeb-engine/src/engine_bridge/tests/fixture_2237d.rs");
    include!("../../crates/beebeeb-engine/src/engine_bridge/tests/fixture_rv173.rs");

    /// L7d (review 2279 F2): the custody memory belongs to the device. A sign-out,
    /// an R10 reset or an account switch keeps it, so a server 404 after the next
    /// sign-in still refuses instead of falling back to the legacy reader.
    #[test]
    fn rv173_l7d_custody_memory_survives_sign_out_and_account_switch() {
        let (server, _temp, bridge, root) = rv_folder_fixture(200);
        let custody = server.state.lock().unwrap().rv_routes[&format!("/api/v1/shared-folders/{ROOT}")].clone();
        let folder_keys = |server: &ContentServer| {
            server
                .state
                .lock()
                .unwrap()
                .requests
                .iter()
                .filter(|request| request.path.ends_with("/folder-keys"))
                .count()
        };
        for wipe in ["sign-out", "account switch", "windows sign-out"] {
            server
                .state
                .lock()
                .unwrap()
                .rv_routes
                .insert(format!("/api/v1/shared-folders/{ROOT}"), custody.clone());
            runtime()
                .block_on(bridge.shared_folder_material_for_test(&root))
                .unwrap();
            assert!(
                bridge.db.has_seen_folder_custody(ROOT).unwrap(),
                "{wipe}: the fixture starts with the custody memory"
            );
            // The server now denies the folder. Without the durable memory this
            // device would read the invite's legacy unbound wrap instead.
            server
                .state
                .lock()
                .unwrap()
                .rv_routes
                .insert(format!("/api/v1/shared-folders/{ROOT}"), (404, b"{}".to_vec()));
            match wipe {
                // The macOS/Linux sign-out: purge the files and rows, then clear
                // every account row and the owner.
                "sign-out" => {
                    crate::local_state_purge::purge_local_state_files_in(&bridge.db, None, &[], None).unwrap();
                    bridge.db.clear_account_data(false).unwrap();
                }
                // R10 reset and the account switch.
                "account switch" => bridge.db.clear_account_data(true).unwrap(),
                // The Windows sign-out clears the rows in one transaction.
                _ => {
                    bridge.db.purge_all_local_state().unwrap();
                }
            }
            assert!(
                bridge.db.has_seen_folder_custody(ROOT).unwrap(),
                "{wipe}: the custody memory must survive"
            );
            let before = folder_keys(&server);
            LEGACY_CHILD_READS.with(|n| n.set(0));
            let error = runtime()
                .block_on(bridge.shared_folder_material_for_test(&root))
                .expect_err("a seen-custody 404 after sign-out must refuse");
            assert_eq!(
                error.to_string(),
                "This folder's keys are unavailable",
                "{wipe}: the typed seen-custody error"
            );
            assert!(
                runtime().block_on(bridge.do_hydrate(ID, None)).is_err(),
                "{wipe}: nothing opens after the purge"
            );
            assert_eq!(LEGACY_CHILD_READS.with(|n| n.get()), 0, "{wipe}: no legacy child read");
            assert_eq!(folder_keys(&server), before, "{wipe}: no /folder-keys request");
        }
    }
}
