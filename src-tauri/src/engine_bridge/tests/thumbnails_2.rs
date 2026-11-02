//! `thumbnails_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
    /// Review Focus 3: an empty save lands, and a fetch while it is queued is an empty file.
    #[tokio::test]
    async fn an_empty_save_lands_and_is_served_from_the_queue() {
        let master_key = [94u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "empty");
        fp_save(&bridge, dir.path(), "empty", "notes.txt", b"", "1");
        let dest = dir.path().join("fetch").join("notes.txt");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        assert_eq!(
            bridge.serve_hydrate("empty", &dest, &[dir.path()], None).await.unwrap(),
            HydrateSource::Queue
        );
        assert_eq!(
            std::fs::metadata(&dest).unwrap().len(),
            0,
            "an empty file, not an error"
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.files["empty"].versions.len(), 2);
        assert!(state.latest_plaintext("empty", master_key).is_empty());
    }

    // ── Rule 5: the item's status follows the queue (spec §9.1–§9.2) ──────────

    /// T33 (§9.2): a Finder upload that failed once and will retry keeps the row `Uploading`, so
    /// the item stays writable and is never evicted.
    #[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
    #[tokio::test]
    async fn a_retrying_upload_keeps_the_item_writable() {
        let (dir, sync_root, server, bridge) = rule3_setup([95u8; 32]);
        seed_uploaded_row(&bridge, &server, "retrying");
        fp_save(&bridge, dir.path(), "retrying", "notes.txt", b"edit", "1");
        server
            .state
            .lock()
            .unwrap()
            .fail_first_chunk_once
            .insert("session-1".into());
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // one transient failure
        let op = bridge.db.list_operations_for_file("retrying").unwrap().remove(0);
        assert_eq!(op.attempts, 1, "precondition: one failed attempt, to be retried");
        assert_eq!(
            bridge.db.get_file("retrying").unwrap().unwrap().status,
            FileStatus::Uploading,
            "the row is not rolled back to Error"
        );
        let entry = bridge.db.get_file("retrying").unwrap().unwrap();
        let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
        assert_eq!(payload.status, "uploading");
        assert_ne!(payload.capabilities & crate::ipc_socket::CAP_WRITE, 0, "writable");
        drop(server.finish());
    }

    /// T60 (§9.1, m-8): a `Trashing` row keeps its trash presentation while its Finder upload is
    /// queued: never `uploading`, no write capability, under the trash container.
    #[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
    #[tokio::test]
    async fn a_trashing_row_is_never_presented_uploading() {
        let (dir, _sync_root, server, bridge) = rule3_setup([96u8; 32]);
        seed_uploaded_row(&bridge, &server, "trashed-here");
        let w = fp_save(&bridge, dir.path(), "trashed-here", "notes.txt", b"edit", "1");
        bridge.queue_finder_delete("trashed-here", w.token).unwrap();
        let entry = bridge.db.get_file("trashed-here").unwrap().unwrap();
        assert_eq!(entry.status, FileStatus::Trashing, "precondition: in the trash");
        assert!(
            bridge
                .db
                .item_presentation("trashed-here")
                .unwrap()
                .unwrap()
                .unparked_finder_upload,
            "precondition: its save is still queued"
        );
        let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
        assert_ne!(payload.status, "uploading");
        assert_eq!(
            payload.capabilities & crate::ipc_socket::CAP_WRITE,
            0,
            "no write capability in the trash"
        );
        assert_eq!(
            payload.parent_identifier,
            crate::ipc_socket::FP_TRASH_APPLE,
            "under the trash container"
        );
        drop(server.finish());
    }

    /// §9.1 (m-8): a save queued before a Finder trash lands while its row is
    /// `Trashing`, before the trash op runs. The landing leaves the row in the trash: never `Local`,
    /// so never writable or evictable, and still under the trash container.
    #[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
    #[tokio::test]
    async fn a_save_that_lands_under_a_trashing_row_keeps_it_in_the_trash() {
        let master_key = [102u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        seed_uploaded_row(&bridge, &server, "lands-in-trash");
        let w = fp_save(&bridge, dir.path(), "lands-in-trash", "notes.txt", b"edit", "1");
        bridge.queue_finder_delete("lands-in-trash", w.token).unwrap();
        // The save is listed first, so it lands before the trash op runs. The seam fires once, at
        // the save's outcome: the trash op has not reached the server yet.
        let seen = Arc::new(Mutex::new(None));
        let (db, server_state, seen_in_seam) = (bridge.db.clone(), Arc::clone(&server.state), Arc::clone(&seen));
        bridge.seams.arm("outcome:before_tx", move || {
            let trashes = server_state.lock().unwrap().trashes.len();
            let entry = db.get_file("lands-in-trash").unwrap().unwrap();
            let payload = crate::ipc_socket::file_entry_payload_for_db(&db, &entry, "root");
            *seen_in_seam.lock().unwrap() = Some((trashes, entry.status, payload));
        });
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
        let (trashes, status, payload) = seen.lock().unwrap().take().expect("the seam fired");
        assert_eq!(trashes, 0, "precondition: the save's outcome, before the trash op");
        assert_eq!(status, FileStatus::Trashing, "the landing leaves the row in the trash");
        assert!(
            payload.status != "local" && payload.status != "uploading",
            "never presented writable: {}",
            payload.status
        );
        assert_eq!(
            payload.capabilities & crate::ipc_socket::CAP_WRITE,
            0,
            "no write capability"
        );
        assert_eq!(payload.parent_identifier, crate::ipc_socket::FP_TRASH_APPLE);
        assert_eq!(
            bridge.db.get_file("lands-in-trash").unwrap().unwrap().status,
            FileStatus::Trashing,
            "still in the trash after the trash op"
        );
        let state = server.finish();
        assert_eq!(
            state.latest_plaintext("lands-in-trash", master_key),
            b"edit",
            "precondition: the save landed"
        );
        assert_eq!(
            state.trashes,
            vec!["lands-in-trash".to_string()],
            "precondition: the trash op ran"
        );
    }

    /// T31 / Review Focus 2: a restart in the middle of a save's upload. Every chunk was
    /// acknowledged and the completion was lost when the process died holding its claim. After the
    /// relaunch the item is still writable, it keeps the token it had (no re-download), and the
    /// upload resumes its session.
    #[cfg(unix)] // asserts a token through `held_content_version`
    #[tokio::test]
    async fn a_restart_mid_upload_resumes_and_keeps_the_token() {
        let master_key = [97u8; 32];
        let (dir, sync_root, server, bridge) = rule3_setup(master_key);
        let db_path = dir.path().join("state.db");
        seed_uploaded_row(&bridge, &server, "restart");
        let token = fp_save(
            &bridge,
            dir.path(),
            "restart",
            "notes.txt",
            b"saved before the restart",
            "1",
        )
        .token
        .unwrap();
        server
            .state
            .lock()
            .unwrap()
            .fail_complete_once
            .insert("session-1".into());
        bridge.process_due_operations(&sync_root, now_secs()).await.unwrap(); // every chunk acknowledged, no completion
        let op_id = bridge.db.list_operations_for_file("restart").unwrap().remove(0).op_id;
        let resume = bridge
            .db
            .get_upload_resume(&op_id)
            .unwrap()
            .expect("its session is kept");
        assert_eq!(
            resume.acked_chunks, resume.chunk_count,
            "precondition: every chunk acknowledged"
        );
        assert!(resume.completed_version.is_none(), "precondition: no completion");
        let _crashed_runner = bridge.db.claim_operation(&op_id, now_secs()).unwrap(); // the process dies holding a claim
        drop(bridge);

        // Relaunch: a new process opens the same state.db.
        let bridge = test_bridge_with_api(&db_path, server.base_url.clone(), master_key);
        assert_eq!(bridge.db.engine_start_repair().unwrap().claims_cleared, 1);
        bridge.db.reconcile_stale_in_flight_on_startup().unwrap();
        assert_eq!(
            bridge.db.get_file("restart").unwrap().unwrap().status,
            FileStatus::Uploading,
            "still writable"
        );
        assert_eq!(
            held_content_version(&bridge, "restart"),
            token,
            "same name: no re-download"
        );
        drain_upload_queue(&bridge, &sync_root).await;
        let state = server.finish();
        assert_eq!(state.inits.len(), 1, "the session resumed: {:?}", state.init_summary());
        assert_eq!(
            state.latest_plaintext("restart", master_key),
            b"saved before the restart"
        );
        assert_eq!(held_content_version(&bridge, "restart"), token);
    }

    /// RF5 / Review Focus 5 (§9.1, m-8): a save of a file that was trashed on the web while the
    /// save was queued. The server refuses its `init` with "file is in trash", class `other`: the
    /// save retries with its bytes kept, and the item in the trash never looks writable.
    #[cfg(unix)] // names `crate::ipc_socket::file_entry_payload_for_db`
    #[tokio::test]
    async fn a_save_to_a_file_trashed_elsewhere_keeps_its_bytes() {
        let (dir, sync_root, server, bridge) = rule3_setup([98u8; 32]);
        seed_uploaded_row(&bridge, &server, "trashed-on-web");
        fp_save(
            &bridge,
            dir.path(),
            "trashed-on-web",
            "notes.txt",
            b"an edit the person made",
            "1",
        );
        let trash = crate::api_client::SyncOp {
            seq_id: 7,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": "trashed-on-web" }),
        };
        apply_sync_op(&bridge, &sync_root, &trash, now_secs(), &mut Vec::new()).unwrap();
        server
            .state
            .lock()
            .unwrap()
            .trashed_files
            .insert("trashed-on-web".into());
        drain_upload_queue(&bridge, &sync_root).await;
        let op = bridge.db.list_operations_for_file("trashed-on-web").unwrap().remove(0);
        assert!(
            op.attempts < op.max_attempts,
            "class other: retried, not parked at once"
        );
        assert_eq!(
            std::fs::read(op.payload_path.as_deref().unwrap()).unwrap(),
            b"an edit the person made",
            "the bytes are kept"
        );
        let entry = bridge.db.get_file("trashed-on-web").unwrap().unwrap();
        let payload = crate::ipc_socket::file_entry_payload_for_db(&bridge.db, &entry, "root");
        assert_ne!(payload.status, "uploading", "an item in the trash never looks writable");
        assert_eq!(
            payload.capabilities & crate::ipc_socket::CAP_WRITE,
            0,
            "no write capability in the trash"
        );
        let state = server.finish();
        assert!(
            !state.inits.is_empty() && state.inits.iter().all(|(_, status)| *status == 409),
            "precondition: the server refused every attempt: {:?}",
            state.init_summary()
        );
    }

