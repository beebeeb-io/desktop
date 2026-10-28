// Review-only tests. All keys, identities and plaintext are synthetic.
fn rv_folder_fixture(status: u16) -> (ContentServer, tempfile::TempDir, EngineBridge, SharedRootMapping) {
    let folder = FileKey::from_bytes([55; 32]);
    let sender = MasterKey::from_bytes([8; 32]);
    let recipient = MasterKey::from_bytes([7; 32]);
    let private = beebeeb_core::opaque::derive_x25519_private(&sender);
    let public = beebeeb_core::opaque::derive_x25519_public(&private);
    let recipient_public = beebeeb_core::opaque::derive_x25519_public(
        &beebeeb_core::opaque::derive_x25519_private(&recipient));
    let secret = beebeeb_core::opaque::x25519_shared_secret(&private, &recipient_public).unwrap();
    let share = beebeeb_core::opaque::derive_share_key(&secret, ROOT.as_bytes());
    let encrypted_folder = beebeeb_core::encrypt::encrypt_chunk_raw(
        &FileKey::from_bytes(*share), folder.as_bytes()).unwrap();
    let invite = json!({"id":"rv-invite", "file_id":ROOT, "status":"approved",
        "is_folder_share":true, "access_level":"read",
        "sender_public_key":base64::engine::general_purpose::STANDARD.encode(public),
        "encrypted_folder_key":base64::engine::general_purpose::STANDARD.encode(encrypted_folder)});
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
        w.rv_routes.insert("/api/v1/shares/invites/rv-invite/folder-keys".into(),
            (200, serde_json::to_vec(&json!({"keys":[{"file_id":ID,
                "encrypted_file_key":base64::engine::general_purpose::STANDARD.encode(wrap)}]})).unwrap()));
        let body = if status == 501 {json!({"error":"shared_folders_v1", "message":"This server does not support shared folders yet."})}
            else {json!({"folder_id":ROOT,"role":"member","key_version":1})};
        w.rv_routes.insert(format!("/api/v1/shared-folders/{ROOT}"), (status, serde_json::to_vec(&body).unwrap()));
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
    assert_eq!(&*runtime().block_on(bridge.do_hydrate(ID, None)).unwrap(), b"guest bytes!");
}

#[test]
fn rv173_flag_off_501_keeps_legacy_folder_readable() {
    let (_server, _temp, bridge, _) = rv_folder_fixture(501);
    let read = runtime().block_on(bridge.do_hydrate(ID, None));
    assert!(read.is_ok(), "a flag-off invite-only folder must remain readable; got {read:?}");
    assert_eq!(&*read.unwrap(), b"guest bytes!");
}

#[test]
fn rv173_custody_404_cannot_downgrade_to_replayed_unbound_child() {
    let (server, temp, bridge, _) = rv_folder_fixture(200);
    // The same bridge has already observed custody and opened an L7 child.
    assert_eq!(&*runtime().block_on(bridge.do_hydrate(ID, None)).unwrap(), b"guest bytes!");
    let final_path = temp.path().join("rv-result.txt");
    runtime().block_on(bridge.hydrate_file(ID, &final_path, &[temp.path()])).unwrap();
    assert_eq!(std::fs::read(&final_path).unwrap(), b"guest bytes!");
    let other_id = "70000000-0000-4000-8000-000000000007";
    let other_key = FileKey::from_bytes([99; 32]);
    let folder = FileKey::from_bytes([55; 32]);
    // Replay an older legacy child A's valid wrap/name/v1 content as row B.
    let old_wrap = beebeeb_core::encrypt::encrypt_chunk_raw(&folder, other_key.as_bytes()).unwrap();
    {
        let mut w = server.state.lock().unwrap();
        w.rv_routes.insert(format!("/api/v1/shared-folders/{ROOT}"), (404, b"{}".to_vec()));
        w.rv_routes.insert("/api/v1/shares/invites/rv-invite/folder-keys".into(),
            (200, serde_json::to_vec(&json!({"keys":[{"file_id":ID,
                "encrypted_file_key":base64::engine::general_purpose::STANDARD.encode(old_wrap)}]})).unwrap()));
        w.meta["name_encrypted"] = json!(named(&other_key,"other-file.txt",ExpectedChunkFormat::Unrecorded));
        w.frames = frames(&other_key, other_id, b"other bytes!", ExpectedChunkFormat::Unrecorded);
    }
    let result = runtime().block_on(bridge.hydrate_file(ID, &final_path, &[temp.path()]));
    if result.is_ok() {
        assert_eq!(std::fs::read(&final_path).unwrap(), b"other bytes!");
    }
    assert!(result.is_err(), "server 404 downgraded a previously observed custody folder and published another item's plaintext over its final file");
}

#[test]
fn rv173_control_custody_200_rejects_unbound_child() {
    let (server, _temp, bridge, _) = rv_folder_fixture(200);
    let folder = FileKey::from_bytes([55; 32]);
    let wrap = beebeeb_core::encrypt::encrypt_chunk_raw(&folder, random_key().as_bytes()).unwrap();
    server.state.lock().unwrap().rv_routes.insert("/api/v1/shares/invites/rv-invite/folder-keys".into(),
        (200, serde_json::to_vec(&json!({"keys":[{"file_id":ID,
            "encrypted_file_key":base64::engine::general_purpose::STANDARD.encode(wrap)}]})).unwrap()));
    assert!(runtime().block_on(bridge.do_hydrate(ID, None)).is_err());
}

#[tokio::test]
async fn rv173_deleted_before_sparse_update_does_not_pin_cursor() {
    let f = Convergence::new().await;
    let old_cursor = f.bridge.db.get_sync_cursor().unwrap().unwrap();
    f.event("file_update", json!({"id":ID,"version_number":2,"size_bytes":12}));
    f.event("file_delete", json!({"id":ID}));
    f.server.state.lock().unwrap().rv_routes.insert(format!("/api/v1/files/{ID}"), (404, b"{}".to_vec()));
    let first = sync_tick_outcome(&f.bridge, &f.root).await;
    let second = sync_tick_outcome(&f.bridge, &f.root).await;
    let cursor = f.bridge.db.get_sync_cursor().unwrap().unwrap();
    assert!(first.is_ok() && second.is_ok(),
        "terminal 404 on a superseded event must not block the later delete; attempts {first:?} / {second:?}, cursor {old_cursor}->{cursor}");
    assert!(cursor > old_cursor);
    assert!(f.bridge.db.get_file(ID).unwrap().is_none());
}
