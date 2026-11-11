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
