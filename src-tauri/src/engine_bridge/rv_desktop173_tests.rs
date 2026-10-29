// Adopted review regressions (review head aeeebc5). All keys, identities and plaintext are synthetic.
fn rv_folder_fixture(status: u16) -> (ContentServer, tempfile::TempDir, EngineBridge, SharedRootMapping) {
    let folder = FileKey::from_bytes([55; 32]);
    let sender = MasterKey::from_bytes([8; 32]);
    let recipient = MasterKey::from_bytes([7; 32]);
    let private = beebeeb_core::opaque::derive_x25519_private(&sender);
    let public = beebeeb_core::opaque::derive_x25519_public(&private);
    let recipient_public =
        beebeeb_core::opaque::derive_x25519_public(&beebeeb_core::opaque::derive_x25519_private(&recipient));
    let secret = beebeeb_core::opaque::x25519_shared_secret(&private, &recipient_public).unwrap();
    let share = beebeeb_core::opaque::derive_share_key(&secret, ROOT.as_bytes());
    let encrypted_folder =
        beebeeb_core::encrypt::encrypt_chunk_raw(&FileKey::from_bytes(*share), folder.as_bytes()).unwrap();
    let invite = json!({"id":"rv-invite", "file_id":ROOT, "status":"approved",
        "is_folder_share":true, "access_level":"read",
        "sender_public_key":base64::engine::general_purpose::STANDARD.encode(public),
        "encrypted_folder_key":base64::engine::general_purpose::STANDARD.encode(&encrypted_folder)});
    let roots = shared_roots_from_invite_response(&json!({"invites":[invite.clone()]}));
    assert_eq!(roots.len(), 1);
    let s = ContentServer::start(&random_key(), ExpectedChunkFormat::Unrecorded);
    {
        let mut w = s.state.lock().unwrap();
        w.invites = json!({"invites":[invite]});
        let wrap = if status == 200 {
            beebeeb_sync::key_resolution::wrap_child_key(&folder, ROOT, ID, 1, &random_key()).unwrap()
        } else {
            beebeeb_core::encrypt::encrypt_chunk_raw(&folder, random_key().as_bytes()).unwrap()
        };
        w.rv_routes.insert(
            "/api/v1/shares/invites/rv-invite/folder-keys".into(),
            (
                200,
                serde_json::to_vec(&json!({"keys":[{"file_id":ID,
                "encrypted_file_key":base64::engine::general_purpose::STANDARD.encode(wrap)}]}))
                .unwrap(),
            ),
        );
        let body = if status == 501 {
            json!({"error":"shared_folders_v1", "message":"This server does not support shared folders yet."})
        } else {
            json!({"folder_id":ROOT,"role":"member","key_version":1,"owner_wrapped_folder_key":null,
                "wrapped_folder_key":base64::engine::general_purpose::STANDARD.encode(&encrypted_folder)})
        };
        w.rv_routes.insert(
            format!("/api/v1/shared-folders/{ROOT}"),
            (status, serde_json::to_vec(&body).unwrap()),
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), false);
    let mut contract = b.db.get_file_contract_state(ID).unwrap().unwrap();
    contract.namespace = Namespace::SharedWithMe;
    contract.permission_bits = PERMISSION_READ;
    contract.share_id = Some("rv-invite".into());
    contract.shared_root_id = Some(ROOT.into());
    b.db.set_file_contract_state(&contract).unwrap();
    (s, temp, b, roots.into_iter().next().unwrap())
}

#[test]
fn rv173_control_legacy_404_opens() {
    let (_server, _temp, bridge, _) = rv_folder_fixture(404);
    assert_eq!(
        &*runtime().block_on(bridge.do_hydrate(ID, None)).unwrap(),
        b"guest bytes!"
    );
}

#[test]
fn rv173_flag_off_501_keeps_legacy_folder_readable() {
    let (_server, _temp, bridge, _) = rv_folder_fixture(501);
    let read = runtime().block_on(bridge.do_hydrate(ID, None));
    assert!(
        read.is_ok(),
        "a flag-off invite-only folder must remain readable; got {read:?}"
    );
    assert_eq!(&*read.unwrap(), b"guest bytes!");
}

#[test]
fn rv173_custody_404_cannot_downgrade_to_replayed_unbound_child() {
    let (server, temp, bridge, root) = rv_folder_fixture(200);
    // The same bridge has already observed custody and opened an L7 child.
    assert_eq!(
        &*runtime().block_on(bridge.do_hydrate(ID, None)).unwrap(),
        b"guest bytes!"
    );
    let final_path = temp.path().join("rv-result.txt");
    runtime()
        .block_on(bridge.hydrate_file(ID, &final_path, &[temp.path()]))
        .unwrap();
    assert_eq!(std::fs::read(&final_path).unwrap(), b"guest bytes!");
    let other_id = "70000000-0000-4000-8000-000000000007";
    let other_key = FileKey::from_bytes([99; 32]);
    let folder = FileKey::from_bytes([55; 32]);
    // Replay an older legacy child A's valid wrap/name/v1 content as row B.
    let old_wrap = beebeeb_core::encrypt::encrypt_chunk_raw(&folder, other_key.as_bytes()).unwrap();
    {
        let mut w = server.state.lock().unwrap();
        w.rv_routes
            .insert(format!("/api/v1/shared-folders/{ROOT}"), (404, b"{}".to_vec()));
        w.rv_routes.insert(
            "/api/v1/shares/invites/rv-invite/folder-keys".into(),
            (
                200,
                serde_json::to_vec(&json!({"keys":[{"file_id":ID,
                "encrypted_file_key":base64::engine::general_purpose::STANDARD.encode(old_wrap)}]}))
                .unwrap(),
            ),
        );
        w.meta["name_encrypted"] = json!(named(&other_key, "other-file.txt", ExpectedChunkFormat::Unrecorded));
        w.frames = frames(&other_key, other_id, b"other bytes!", ExpectedChunkFormat::Unrecorded);
    }
    let result = runtime().block_on(bridge.hydrate_file(ID, &final_path, &[temp.path()]));
    if result.is_ok() {
        assert_eq!(std::fs::read(&final_path).unwrap(), b"other bytes!");
    }
    assert!(
        result.is_err(),
        "server 404 downgraded a previously observed custody folder and published another item's plaintext over its final file"
    );
    assert_eq!(result.err().unwrap().to_string(), "This folder's keys are unavailable");
    assert_eq!(std::fs::read(&final_path).unwrap(), b"guest bytes!");
    drop(bridge);
    // A new engine/DB connection has no in-memory custody history.
    let bridge = test_bridge_with_api(&temp.path().join("db"), server.base.clone(), [7; 32]);
    LEGACY_CHILD_READS.with(|n| n.set(0));
    for status in [404, 405, 501] {
        server
            .state
            .lock()
            .unwrap()
            .rv_routes
            .insert(format!("/api/v1/shared-folders/{ROOT}"), (status, b"{}".to_vec()));
        let error = runtime()
            .block_on(bridge.shared_folder_material(&root))
            .err()
            .expect("durable custody cannot downgrade");
        assert_eq!(error.to_string(), "This folder's keys are unavailable");
        assert!(
            runtime()
                .block_on(bridge.hydrate_file(ID, &final_path, &[temp.path()]))
                .is_err()
        );
        assert_eq!(std::fs::read(&final_path).unwrap(), b"guest bytes!");
    }
    assert_eq!(LEGACY_CHILD_READS.with(|n| n.get()), 0);
}

#[test]
fn rv173_control_custody_200_rejects_unbound_child() {
    let (server, _temp, bridge, _) = rv_folder_fixture(200);
    let folder = FileKey::from_bytes([55; 32]);
    let wrap = beebeeb_core::encrypt::encrypt_chunk_raw(&folder, random_key().as_bytes()).unwrap();
    server.state.lock().unwrap().rv_routes.insert(
        "/api/v1/shares/invites/rv-invite/folder-keys".into(),
        (
            200,
            serde_json::to_vec(&json!({"keys":[{"file_id":ID,
            "encrypted_file_key":base64::engine::general_purpose::STANDARD.encode(wrap)}]}))
            .unwrap(),
        ),
    );
    assert!(runtime().block_on(bridge.do_hydrate(ID, None)).is_err());
}

#[tokio::test]
async fn rv173_deleted_before_sparse_update_does_not_pin_cursor() {
    let f = Convergence::new().await;
    let old_cursor = f.bridge.db.get_sync_cursor().unwrap().unwrap();
    f.event("file_update", json!({"id":ID,"version_number":2,"size_bytes":12}));
    f.event("file_delete", json!({"id":ID}));
    f.event("folder_rename", json!({"id":A,"new_name_encrypted":named(&hydration_test_key([7;32], A), "later", ExpectedChunkFormat::Unrecorded)}));
    f.server
        .state
        .lock()
        .unwrap()
        .rv_routes
        .insert(format!("/api/v1/files/{ID}"), (404, b"{}".to_vec()));
    let first = sync_tick_outcome(&f.bridge, &f.root).await;
    let second = sync_tick_outcome(&f.bridge, &f.root).await;
    let cursor = f.bridge.db.get_sync_cursor().unwrap().unwrap();
    assert!(
        first.is_ok() && second.is_ok(),
        "terminal 404 on a superseded event must not block the later delete; attempts {first:?} / {second:?}, cursor {old_cursor}->{cursor}"
    );
    assert_eq!(cursor, old_cursor + 3);
    assert!(f.bridge.db.get_file(ID).unwrap().is_none());
    assert_eq!(f.bridge.db.get_file(A).unwrap().unwrap().path, "later");
    let reopened = StateDb::open(f.dir.path().join("db")).unwrap();
    assert_eq!(reopened.get_sync_cursor().unwrap(), Some(cursor));
    assert!(reopened.get_file(ID).unwrap().is_none());
}

#[test]
fn rv173_l7c_custody_memory_clears_with_account_keys() {
    let (server, _temp, bridge, root) = rv_folder_fixture(200);
    let valid = server.state.lock().unwrap().rv_routes[&format!("/api/v1/shared-folders/{ROOT}")].clone();
    for full_reset in [false, true] {
        server
            .state
            .lock()
            .unwrap()
            .rv_routes
            .insert(format!("/api/v1/shared-folders/{ROOT}"), valid.clone());
        runtime().block_on(bridge.shared_folder_material(&root)).unwrap();
        server
            .state
            .lock()
            .unwrap()
            .rv_routes
            .insert(format!("/api/v1/shared-folders/{ROOT}"), (501, b"{}".to_vec()));
        assert!(
            runtime().block_on(bridge.shared_folder_material(&root)).is_err(),
            "seen custody must refuse before account purge"
        );
        if full_reset {
            bridge.db.clear_account_data(false).unwrap();
        } else {
            bridge.db.purge_all_local_state().unwrap();
        }
        let (_, keys) = runtime().block_on(bridge.shared_folder_material(&root)).unwrap();
        assert_eq!(
            keys["desktop_has_custody"], false,
            "next account inherits no custody history"
        );
    }
}

#[test]
fn rv173_l7c_invalid_member_custody_is_not_remembered() {
    let (server, _temp, bridge, root) = rv_folder_fixture(200);
    server.state.lock().unwrap().rv_routes.insert(format!("/api/v1/shared-folders/{ROOT}"),
        (200, serde_json::to_vec(&json!({"folder_id":ROOT,"role":"member","key_version":1,"wrapped_folder_key":base64::engine::general_purpose::STANDARD.encode([0;60])})).unwrap()));
    assert!(
        runtime().block_on(bridge.shared_folder_material(&root)).is_err(),
        "unopened custody must not become durable knowledge"
    );
    server
        .state
        .lock()
        .unwrap()
        .rv_routes
        .insert(format!("/api/v1/shared-folders/{ROOT}"), (405, b"{}".to_vec()));
    let (_, keys) = runtime().block_on(bridge.shared_folder_material(&root)).unwrap();
    assert_eq!(keys["desktop_has_custody"], false);
}

#[test]
fn rv173_l7c_owner_custody_is_remembered_only_after_l6_open() {
    let (server, temp, bridge, _) = rv_folder_fixture(404);
    server.state.lock().unwrap().rv_routes.insert(
        format!("/api/v1/shared-folders/{ROOT}"),
        (
            200,
            serde_json::to_vec(&json!({"folder_id":ROOT,"role":"owner","key_version":1,
            "owner_wrapped_folder_key":base64::engine::general_purpose::STANDARD.encode([0;60])}))
            .unwrap(),
        ),
    );
    assert!(runtime().block_on(bridge.shared_folder_custody_key(ROOT)).is_err());
    server
        .state
        .lock()
        .unwrap()
        .rv_routes
        .insert(format!("/api/v1/shared-folders/{ROOT}"), (405, b"{}".to_vec()));
    assert_eq!(
        runtime()
            .block_on(bridge.shared_folder_custody_key(ROOT))
            .err()
            .unwrap()
            .to_string(),
        "This folder has no custody key here."
    );
    let owner_wrap = beebeeb_sync::key_resolution::wrap_folder_custody(
        &MasterKey::from_bytes([7; 32]),
        ROOT,
        &FileKey::from_bytes([55; 32]),
    )
    .unwrap();
    let record = json!({"folder_id":ROOT,"role":"owner","key_version":1,"owner_wrapped_folder_key":base64::engine::general_purpose::STANDARD.encode(owner_wrap)});
    server.state.lock().unwrap().rv_routes.insert(
        format!("/api/v1/shared-folders/{ROOT}"),
        (200, serde_json::to_vec(&record).unwrap()),
    );
    assert_eq!(
        runtime()
            .block_on(bridge.shared_folder_custody_key(ROOT))
            .unwrap()
            .as_bytes(),
        &[55; 32]
    );
    drop(bridge);
    let bridge = test_bridge_with_api(&temp.path().join("db"), server.base.clone(), [7; 32]);
    for status in [404, 405, 501] {
        server
            .state
            .lock()
            .unwrap()
            .rv_routes
            .insert(format!("/api/v1/shared-folders/{ROOT}"), (status, b"{}".to_vec()));
        let error = runtime()
            .block_on(bridge.shared_folder_custody_key(ROOT))
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "This folder's keys are unavailable");
    }
}

#[tokio::test]
async fn rv173_canonical_auth_and_server_errors_still_retry() {
    for status in [401, 403, 500] {
        let f = Convergence::new().await;
        let cursor = f.bridge.db.get_sync_cursor().unwrap();
        f.event("file_update", json!({"id":ID,"version_number":2,"size_bytes":12}));
        f.event("file_delete", json!({"id":ID}));
        f.server
            .state
            .lock()
            .unwrap()
            .rv_routes
            .insert(format!("/api/v1/files/{ID}"), (status, b"{}".to_vec()));
        assert!(sync_tick_outcome(&f.bridge, &f.root).await.is_err());
        assert_eq!(
            f.bridge.db.get_sync_cursor().unwrap(),
            cursor,
            "retryable enrichment never acknowledges later events"
        );
        assert!(f.bridge.db.get_file(ID).unwrap().is_some());
        f.server
            .state
            .lock()
            .unwrap()
            .rv_routes
            .insert(format!("/api/v1/files/{ID}"), (404, b"{}".to_vec()));
        sync_tick_outcome(&f.bridge, &f.root).await.unwrap();
        assert_eq!(f.bridge.db.get_sync_cursor().unwrap(), cursor.map(|n| n + 2));
        assert!(f.bridge.db.get_file(ID).unwrap().is_none());
    }
}

#[test]
fn rv173_l7c_late_custody_write_cannot_cross_account_reset() {
    let (_server, _temp, bridge, _) = rv_folder_fixture(200);
    for full_reset in [false, true] {
        let revision = bridge.db.account_revision();
        let scope = bridge.owner_key_scope();
        if full_reset {
            bridge.db.clear_account_data(false).unwrap();
        } else {
            bridge.db.purge_all_local_state().unwrap();
        }
        assert!(scope.remember_custody(ROOT).is_err());
        assert!(
            bridge.db.remember_folder_custody_at_revision(ROOT, revision).is_err(),
            "final database write checks the captured account while locked"
        );
        assert!(!bridge.db.has_seen_folder_custody(ROOT).unwrap());
    }
}

#[test]
fn rv173_l7c_stale_inventory_cannot_reenable_legacy_reader() {
    let (_server, _temp, bridge, mut root) = rv_folder_fixture(200);
    let (folder, mut keys) = runtime().block_on(bridge.shared_folder_material(&root)).unwrap();
    // An earlier request's 404/inventory may finish after this valid custody
    // response. Its stale format flag cannot authorize an unbound read.
    keys["desktop_has_custody"] = json!(false);
    keys["keys"] = json!([{"file_id":ROOT,"encrypted_file_key":
        base64::engine::general_purpose::STANDARD.encode(
            beebeeb_core::encrypt::encrypt_chunk_raw(&FileKey::from_bytes(*folder), random_key().as_bytes()).unwrap())}]);
    root.file_name_encrypted = Some(named(&random_key(), "replayed-root", ExpectedChunkFormat::Unrecorded));
    LEGACY_CHILD_READS.with(|n| n.set(0));
    assert!(bridge.decrypted_folder_root_name(&root, &folder, &keys).is_none());
    assert_eq!(
        LEGACY_CHILD_READS.with(|n| n.get()),
        0,
        "every dispatch rechecks durable custody, including stale in-flight inventories"
    );
}
