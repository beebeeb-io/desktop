include!("tests/fixture_2237d.rs");
#[test]
fn sf2237d_t5_whole_and_range_open() {
    for foreign in [true, false] {
        for pin in [ExpectedChunkFormat::Unrecorded, ExpectedChunkFormat::V2] {
            // L12: the id class decides, so an owner flow needs an owner id.
            let id = if foreign { ID } else { OWNER };
            let key = if foreign {
                random_key()
            } else {
                hydration_test_key([7; 32], id)
            };
            let server = if foreign {
                ContentServer::start(&key, pin)
            } else {
                ContentServer::start_owner(&key, pin)
            };
            let temp = tempfile::tempdir().unwrap();
            let b = server.bridge(&temp.path().join("db"), foreign);
            let whole = runtime().block_on(b.do_hydrate(id, None)).unwrap();
            let opened_hash = sha256_hex(&whole);
            let guest_hash = sha256_hex(b"guest bytes!");
            assert_eq!(opened_hash, guest_hash);
            let range = runtime().block_on(b.hydrate_file_range(id, 4, 4)).unwrap().unwrap();
            assert_eq!(&*range.data, b"t by");
        }
    }
    // Published core known-answer frames, not a desktop encrypt/decrypt round-trip.
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../../../../src-tauri/tests/fixtures/core-1943.json")).unwrap();
    let get = |name: &str| {
        vectors["vectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .unwrap()
    };
    for v2 in [false, true] {
        let v = get(if v2 { "chunk_v2_stream" } else { "chunk_encrypt_decrypt" });
        let deriv = get("file_key_derivation");
        let hexbytes = |v: &serde_json::Value| {
            v.as_str()
                .unwrap()
                .as_bytes()
                .chunks_exact(2)
                .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
                .collect::<Vec<u8>>()
        };
        let master = if v2 {
            hexbytes(&v["master_key_hex"])
        } else {
            hexbytes(&deriv["master_key_hex"])
        };
        let id = if v2 {
            v["file_id"].as_str().unwrap().to_string()
        } else {
            String::from_utf8(hexbytes(&deriv["file_id_hex"])).unwrap()
        };
        let key = FileKey::from_bytes(hexbytes(&v["file_key_hex"]).try_into().unwrap());
        let expected = hexbytes(&v["plaintext_hex"]);
        let s = ContentServer::start(&key, ExpectedChunkFormat::Unrecorded);
        {
            let mut w = s.state.lock().unwrap();
            w.frames = if v2 {
                v["frames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| hexbytes(&f["frame_hex"]))
                    .collect()
            } else {
                vec![[hexbytes(&v["nonce_hex"]), hexbytes(&v["ciphertext_hex"])].concat()]
            };
            // The published v2 frames open only against their version id (core 0.3.0), so the name is
            // sealed here with the commitment to the vector's `version_id_hex`. The published name
            // vector records the v2 pin and no commitment: it still opens as a name, and v2 content
            // never opens under it (the second half of the loop body below).
            let name = if v2 {
                let version_id: [u8; 12] = hexbytes(&v["version_id_hex"]).try_into().unwrap();
                beebeeb_core::encrypt::encrypt_name_for_version(
                    &key,
                    "vector.bin",
                    None,
                    ExpectedChunkFormat::V2,
                    beebeeb_core::chunk_stream::VersionCommitment::new(version_id, 1).unwrap(),
                )
                .unwrap()
            } else {
                named(&key, "legacy.txt", ExpectedChunkFormat::Unrecorded)
            };
            w.meta = json!({"id":id,"name_encrypted":name,"chunk_count":w.frames.len(),"chunk_size_bytes":if v2 {32}else{expected.len()},"size_bytes":expected.len()});
        }
        let temp = tempfile::tempdir().unwrap();
        let b = test_bridge_with_api(&temp.path().join("db"), s.base.clone(), master.try_into().unwrap());
        assert_eq!(
            sha256_hex(&runtime().block_on(b.do_hydrate(&id, None)).unwrap()),
            sha256_hex(&expected)
        );
        if v2 {
            let published = get("metadata_v2_name");
            let sealed = published["name_encrypted"].as_str().unwrap();
            let opened = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&key, sealed).unwrap();
            assert_eq!(opened.name, published["expected_name"].as_str().unwrap());
            assert_eq!(opened.chunk_format, ExpectedChunkFormat::V2);
            assert!(
                opened.version.is_none(),
                "the published name predates version commitments"
            );
            s.state.lock().unwrap().meta["name_encrypted"] = json!(sealed);
            assert!(
                runtime().block_on(b.do_hydrate(&id, None)).is_err(),
                "v2 frames never open under a name without their commitment"
            );
        }
    }

    // Exercise the direct transport with the same recorded pins and random keys.
    for foreign in [false, true] {
        for pin in [ExpectedChunkFormat::Unrecorded, ExpectedChunkFormat::V2] {
            let id = if foreign { ID } else { OWNER };
            let key = if foreign {
                random_key()
            } else {
                hydration_test_key([7; 32], id)
            };
            let pair = crate::direct_download::test_support::Pair::start_with([7; 32], id, 4, b"guest bytes!".to_vec());
            pair.knobs().version.frames = frames(&key, id, b"guest bytes!", pin);
            let temp = tempfile::tempdir().unwrap();
            let b = test_bridge_with_api(&temp.path().join("db"), pair.api.base.clone(), [7; 32]);
            let name = named(&key, "direct.txt", pin);
            if foreign {
                b.db.put_owned_key_record(&OwnedKeyRecord {
                    file_id: id.into(),
                    name_encrypted: name.clone(),
                    owner_wrap: Some(
                        beebeeb_sync::key_resolution::wrap_owner_file_key(&MasterKey::from_bytes([7; 32]), id, &key)
                            .unwrap(),
                    ),
                    foreign_added: true,
                    ..Default::default()
                })
                .unwrap();
            }
            let mut meta = json!({"id":id,"name_encrypted":name,"size_bytes":12,"chunk_count":3,"chunk_size_bytes":4,"download_version":pair.knobs().version.manifest()});
            assert_eq!(
                &*runtime().block_on(b.do_hydrate_with_meta(id, &meta, None)).unwrap(),
                b"guest bytes!"
            );
            assert_eq!(pair.storage_requests(), 3);
            assert!(pair.api_chunk_requests().is_empty());
            if pin == ExpectedChunkFormat::V2 {
                {
                    let mut k = pair.knobs();
                    k.version.frames.pop();
                    k.version.plain.truncate(8);
                    meta["download_version"] = k.version.manifest();
                }
                meta["chunk_count"] = json!(2);
                meta["size_bytes"] = json!(8);
                assert!(
                    runtime().block_on(b.do_hydrate_with_meta(id, &meta, None)).is_err(),
                    "direct finish must reject truncated v2"
                );
                assert!(
                    pair.api_chunk_requests().is_empty(),
                    "finish cannot fall back to a legacy reader"
                );
            }
        }
    }
}
#[test]
fn sf2237d_t5_thumbnail_preview_and_versions() {
    let s = ContentServer::start(&random_key(), ExpectedChunkFormat::V2);
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), true);
    assert_eq!(
        &*runtime().block_on(b.fetch_thumbnail_to_memory(ID, "small")).unwrap(),
        b"thumbnail"
    );
    let (preview, _) = runtime().block_on(b.remote_content_preview(ID, ContentKind::Text));
    assert_eq!(preview.text.as_deref(), Some("guest bytes!"));
    {
        let mut w = s.state.lock().unwrap();
        w.meta["download_version"] = manifest(&w.frames, 12);
    }
    let meta = s.state.lock().unwrap().meta.clone();
    assert_eq!(
        &*runtime().block_on(b.do_hydrate_with_meta(ID, &meta, None)).unwrap(),
        b"guest bytes!"
    );
    assert!(
        s.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.path.contains("/chunks/"))
            .any(|r| r.path.contains(OBJECT)),
        "version reads remain bound to the object identity"
    );
}
#[test]
fn sf2237d_t5_rename_and_replacement_keep_key() {
    let s = ContentServer::start(&random_key(), ExpectedChunkFormat::V2);
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), true);
    let name = runtime()
        .block_on(b.encrypt_owned_name(ID, "renamed.txt", ExpectedChunkFormat::V2))
        .unwrap();
    let m = beebeeb_core::encrypt::decrypt_file_metadata_with_key(&random_key(), &name).unwrap();
    assert_eq!(m.name, "renamed.txt");
    assert_eq!(m.chunk_format, ExpectedChunkFormat::V2);
    let payload = temp.path().join("replacement.txt");
    std::fs::write(&payload, b"replacement!").unwrap();
    let mut op = resumable_create_op(&payload);
    op.file_id = Some(ID.into());
    let session = UploadResume {
        op_id: op.op_id.clone(),
        payload_path: payload.to_string_lossy().into_owned(),
        payload_size: 12,
        payload_mtime_ns: payload_mtime_ns(&payload),
        upload_session_id: "session".into(),
        server_file_id: ID.into(),
        object_version_id: OBJECT.into(),
        chunk_size_bytes: 4,
        chunk_count: 3,
        acked_chunks: 0,
        metadata_applied: true,
        is_create: false,
        self_minted: false,
        completed_version: None,
        completed_object_version_id: None,
        completed_mime_type: None,
    };
    b.db.put_upload_resume(&session).unwrap();
    runtime()
        .block_on(b.upload_session_body(
            LocalFileId::new(ID),
            &op,
            &json!({}),
            &name,
            Some("text/plain".into()),
            &payload,
            &session,
            None,
        ))
        .unwrap();
    let wire = s.state.lock().unwrap().uploads.clone();
    assert_eq!(wire.len(), 3);
    let sealed =
        b.db.upload_completion_name(&op.op_id)
            .unwrap()
            .expect("the session stored the name `complete` publishes");
    assert_eq!(opened_as(&random_key(), ID, &wire, &sealed), b"replacement!");
    let queued = owned_metadata_for_name(&b.db, &[7; 32], ID, "moved.txt", Some("text/plain")).unwrap();
    assert_eq!(
        beebeeb_core::encrypt::decrypt_file_metadata_with_key(&random_key(), &queued)
            .unwrap()
            .chunk_format,
        ExpectedChunkFormat::V2
    );
}
#[test]
fn sf2237d_t5_move_out_restart_without_folder() {
    let s = ContentServer::start(&random_key(), ExpectedChunkFormat::Unrecorded);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db");
    drop(s.bridge(&path, true));
    let b = test_bridge_with_api(&path, s.base.clone(), [7; 32]);
    assert_eq!(&*runtime().block_on(b.do_hydrate(ID, None)).unwrap(), b"guest bytes!");
    assert_eq!(
        s.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.path.contains("shares") || r.path.contains("shared-folders"))
            .count(),
        0
    );
}
#[test]
fn sf2237d_t5_bad_key_never_publishes() {
    let s = ContentServer::start(&random_key(), ExpectedChunkFormat::V2);
    s.state.lock().unwrap().frames[1][15] ^= 1;
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), true);
    let final_path = temp.path().join("result.txt");
    assert!(
        runtime()
            .block_on(b.hydrate_file(ID, &final_path, &[temp.path()]))
            .is_err()
    );
    assert!(!final_path.exists());
    assert_eq!(b.db.get_file_contract_state(ID).unwrap().unwrap().cache_bytes, 0);
    assert_eq!(b.db.get_file(ID).unwrap().unwrap().status, FileStatus::CloudOnly);
    assert_eq!(
        crate::owner_keys::cannot_open_message(None, false),
        "Can't open this file (added via link)"
    );
    assert_eq!(
        crate::owner_keys::cannot_open_message(None, true),
        "Can't open this file (added via shared folder)"
    );
}
#[test]
fn sf2237d_t5_request_and_sharedwithme_unchanged() {
    let key = hydration_test_key([7; 32], OWNER);
    let s = ContentServer::start_owner(&key, ExpectedChunkFormat::Unrecorded);
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), false);
    assert_eq!(
        &*runtime().block_on(b.do_hydrate(OWNER, None)).unwrap(),
        b"guest bytes!"
    );
    let sender = MasterKey::from_bytes([8; 32]);
    let recipient = MasterKey::from_bytes([7; 32]);
    let sp = beebeeb_core::opaque::derive_x25519_public(&beebeeb_core::opaque::derive_x25519_private(&sender));
    let rp = beebeeb_core::opaque::derive_x25519_public(&beebeeb_core::opaque::derive_x25519_private(&recipient));
    let secret =
        beebeeb_core::opaque::x25519_shared_secret(&beebeeb_core::opaque::derive_x25519_private(&sender), &rp).unwrap();
    let share = beebeeb_core::opaque::derive_share_key(&secret, ID.as_bytes());
    let wrapped =
        beebeeb_core::encrypt::encrypt_chunk_raw(&FileKey::from_bytes(*share), random_key().as_bytes()).unwrap();
    let unwrapped = unwrap_direct_shared_file_key(
        &[7; 32],
        &base64::engine::general_purpose::STANDARD.encode(sp),
        ID,
        &base64::engine::general_purpose::STANDARD.encode(&wrapped),
    )
    .unwrap();
    assert_eq!(unwrapped.as_bytes(), random_key().as_bytes());
    let s = ContentServer::start(&random_key(), ExpectedChunkFormat::V2);
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), false);
    s.state.lock().unwrap().invites = json!({"invites":[{"id":"invite","file_id":ID,"status":"approved","sender_public_key":base64::engine::general_purpose::STANDARD.encode(sp),"encrypted_file_key":base64::engine::general_purpose::STANDARD.encode(wrapped),"is_folder_share":false,"access_level":"read"}]});
    let mut contract = b.db.get_file_contract_state(ID).unwrap().unwrap();
    contract.namespace = Namespace::SharedWithMe;
    contract.permission_bits = PERMISSION_READ;
    contract.share_id = Some("invite".into());
    contract.shared_root_id = Some(ID.into());
    b.db.set_file_contract_state(&contract).unwrap();
    assert_eq!(&*runtime().block_on(b.do_hydrate(ID, None)).unwrap(), b"guest bytes!");
    assert_eq!(
        b.db.list_owned_key_records().unwrap().len(),
        0,
        "SharedWithMe does not adopt owner keys"
    );
    // Existing request uploads whose random key already has an owner wrap need no custody.
    let s = ContentServer::start(&random_key(), ExpectedChunkFormat::Unrecorded);
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), true);
    let mut request = b.db.get_owned_key_record(ID).unwrap().unwrap();
    request.foreign_added = false;
    request.shared_root_id = None;
    b.db.put_owned_key_record(&request).unwrap();
    assert_eq!(&*runtime().block_on(b.do_hydrate(ID, None)).unwrap(), b"guest bytes!");
    assert_eq!(
        s.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.path.contains("shared-folders"))
            .count(),
        0
    );
}
#[test]
fn sf2237d_t5_pin_and_object_change() {
    let key = hydration_test_key([7; 32], OWNER);
    let s = ContentServer::start_owner(&key, ExpectedChunkFormat::V2);
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), false);
    s.state.lock().unwrap().frames = frames(&key, OWNER, b"guest bytes!", ExpectedChunkFormat::Unrecorded);
    assert!(
        runtime().block_on(b.do_hydrate(OWNER, None)).is_err(),
        "v2 metadata cannot accept v1 frames"
    );
    {
        let mut w = s.state.lock().unwrap();
        w.frames = frames(&key, OWNER, b"guest bytes!", ExpectedChunkFormat::V2);
        w.meta["download_version"] = manifest(&w.frames, 12);
        w.meta_reads = 0;
        w.gone = 2;
    }
    assert!(runtime().block_on(b.do_hydrate(OWNER, None)).is_err());
    let w = s.state.lock().unwrap();
    let object_restarts = w.meta_reads - 1;
    assert!(object_restarts <= 1);
    assert_eq!(w.meta_reads, 2);
    let unpinned_after_restart = w
        .requests
        .iter()
        .rev()
        .take(4)
        .filter(|r| r.path.contains("/chunks/") && !r.path.contains(OBJECT))
        .count();
    assert_eq!(unpinned_after_restart, 0);
}
#[test]
fn sf2237d_1943_truncated_v2_never_publishes() {
    let key = hydration_test_key([7; 32], OWNER);
    let s = ContentServer::start_owner(&key, ExpectedChunkFormat::V2);
    {
        let mut w = s.state.lock().unwrap();
        w.frames.pop();
        w.meta["chunk_count"] = json!(2);
        w.meta["size_bytes"] = json!(8);
    }
    let temp = tempfile::tempdir().unwrap();
    let b = s.bridge(&temp.path().join("db"), false);
    let final_path = temp.path().join("result.txt");
    assert!(
        runtime()
            .block_on(b.hydrate_file(OWNER, &final_path, &[temp.path()]))
            .is_err()
    );
    assert!(!final_path.exists());
    assert_eq!(b.db.get_file_contract_state(OWNER).unwrap().unwrap().cache_bytes, 0);
}
#[test]
fn sf2237d_l7b_custody_never_uses_legacy_reader() {
    use base64::Engine as _;
    LEGACY_CHILD_READS.with(|n| n.set(0));
    let folder = FileKey::from_bytes([23; 32]);
    let wrap = beebeeb_sync::key_resolution::wrap_child_key(&folder, ROOT, ID, 1, &random_key()).unwrap();
    let encoded = base64::engine::general_purpose::STANDARD.encode(wrap);
    let got = unwrap_folder_child_for_format(folder.as_bytes(), ROOT, ID, 1, &encoded, KeyCustody::Custody);
    assert_eq!(
        LEGACY_CHILD_READS.with(|n| n.get()),
        0,
        "custody must never try the unbound reader"
    );
    assert_eq!(got.unwrap().as_bytes(), random_key().as_bytes());
    let legacy = base64::engine::general_purpose::STANDARD
        .encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, random_key().as_bytes()).unwrap());
    assert!(unwrap_folder_child_for_format(folder.as_bytes(), ROOT, ID, 1, &legacy, KeyCustody::Custody).is_err());
    assert_eq!(LEGACY_CHILD_READS.with(|n| n.get()), 0);
    assert_eq!(
        unwrap_folder_child_for_format(folder.as_bytes(), ROOT, ID, 1, &legacy, KeyCustody::Invite)
            .unwrap()
            .as_bytes(),
        random_key().as_bytes()
    );
    assert_eq!(LEGACY_CHILD_READS.with(|n| n.get()), 1);
}

include!("2237d_convergence_tests.rs");

include!("rv_desktop173_tests.rs");
