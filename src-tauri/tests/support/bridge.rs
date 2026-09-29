//! Included by the library's test-only module so the production bridge stays private.
#[path = "native_parity.rs"]
mod fixture;
use crate::{
    api_client::{ApiClient, DesktopUploadInitRequest},
    engine_bridge::{EngineBridge, sync_tick},
    state_db::StateDb,
};
use beebeeb_core::{
    encrypt::{decrypt_chunk_raw, encrypt_chunk},
    kdf::{MasterKey, derive_file_key},
};
use fixture::{Corpus, Fixture, hash, id};
use serde_json::json;
use std::sync::Arc;

fn assert_clean(f: Fixture) {
    let report = f.cleanup().unwrap();
    assert_eq!((report.removed_objects, report.removed_accounts), (14, 2));
    assert_eq!(
        (
            report.remaining_objects,
            report.remaining_accounts,
            report.remaining_uploads,
            report.remaining_paths,
            report.remaining_listeners
        ),
        (0, 0, 0, 0, 0)
    );
    println!("cleanup={}", serde_json::to_string(&report).unwrap());
}
fn plaintext(f: &Fixture, account: &str, n: usize) -> Vec<u8> {
    let row = f
        .request(account, "GET", &format!("/api/v1/files/{}", id(n)), b"")
        .unwrap();
    assert_eq!(row.status, 200);
    let meta = row.value();
    let owner = &f.corpus.objects[n - 1].owner;
    let (_, master) = f.credentials(owner);
    let key = derive_file_key(&MasterKey::from_bytes(*master), id(n).as_bytes());
    let mut bytes = vec![];
    for i in 0..meta["chunk_count"].as_u64().unwrap() {
        let chunk = f
            .request(account, "GET", &format!("/api/v1/files/{}/chunks/{i}", id(n)), b"")
            .unwrap();
        assert_eq!(chunk.status, 200);
        bytes.extend(decrypt_chunk_raw(&key, &chunk.body).unwrap());
    }
    bytes
}
#[test]
fn corpus_setup_cleanup_twice() {
    for _ in 0..2 {
        let f = Fixture::setup().unwrap();
        assert_eq!(
            (
                f.corpus.objects.len(),
                f.corpus.scenarios.len(),
                f.corpus.accounts.len()
            ),
            (14, 9, 2)
        );
        std::fs::write(f.root.path().join("owned-test-data"), b"synthetic").unwrap();
        assert_eq!(std::fs::read_dir(f.root.path()).unwrap().count(), 1);
        assert_clean(f);
    }
}
#[test]
fn corpus_encrypted_bytes_namespaces_and_thumbnails() {
    let f = Fixture::setup().unwrap();
    let mut files = 0;
    let mut thumbnails = 0;
    assert_eq!(
        f.request("alice", "GET", "/api/v1/sync/snapshot", b"").unwrap().value()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    assert_eq!(
        f.request("bob", "GET", "/api/v1/sync/snapshot", b"").unwrap().value()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for (i, o) in f
        .corpus
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.kind == "file" && o.id != id(10))
    {
        let bytes = plaintext(&f, &o.owner, i + 1);
        assert_eq!(hash(&bytes), o.versions[0].sha256, "plaintext oracle for {}", o.id);
        files += 1;
        let (_, master) = f.credentials(&o.owner);
        let key = derive_file_key(&MasterKey::from_bytes(*master), o.id.as_bytes());
        for thumb in &o.thumbnails {
            let response = f
                .request(
                    &o.owner,
                    "GET",
                    &format!("/api/v1/files/{}/thumbnail/{}", o.id, thumb.variant.as_ref().unwrap()),
                    b"",
                )
                .unwrap();
            assert_eq!(response.status, 200);
            let bytes = decrypt_chunk_raw(&key, &response.body).unwrap();
            assert_eq!(hash(&bytes), thumb.sha256);
            let image = image::load_from_memory(&bytes).unwrap();
            assert_eq!((image.width(), image.height()), (32, 32));
            thumbnails += 1;
        }
    }
    assert_eq!((files, thumbnails), (9, 4));
    assert_eq!(
        f.request("bob", "GET", &format!("/api/v1/files/{}", id(4)), b"")
            .unwrap()
            .status,
        403
    );
    assert_eq!(
        f.request("alice", "GET", &format!("/api/v1/files/{}", id(14)), b"")
            .unwrap()
            .status,
        403
    );
    assert_eq!(
        f.request(
            "alice",
            "GET",
            &format!("/api/v1/files/{}/thumbnail/small", id(11)),
            b""
        )
        .unwrap()
        .status,
        404
    );
    assert_eq!(f.counts().thumbnails_read, 4);
    assert!(f.counts().chunks_read > 0);
    println!("files={files} thumbnails={thumbnails} counts={:?}", f.counts());
    assert_clean(f);
}
#[test]
fn corpus_conflict_versions_and_stale_precondition() {
    let f = Fixture::setup().unwrap();
    assert_eq!(hash(&plaintext(&f, "alice", 7)), f.corpus.objects[6].versions[0].sha256);
    f.remote_edit();
    let meta = f
        .request("alice", "GET", &format!("/api/v1/files/{}", id(7)), b"")
        .unwrap()
        .value();
    assert_eq!(meta["version_number"], 2);
    assert_eq!(hash(&plaintext(&f, "alice", 7)), f.corpus.objects[6].versions[2].sha256);
    assert_ne!(
        f.corpus.objects[6].versions[1].sha256,
        f.corpus.objects[6].versions[2].sha256
    );
    let conflict = f
        .request(
            "alice",
            "POST",
            "/api/v1/uploads/init",
            &serde_json::to_vec(&json!({"file_id":id(7),"base_version_number":1})).unwrap(),
        )
        .unwrap();
    assert_eq!(conflict.status, 409);
    assert_clean(f);
}
#[test]
fn corpus_trash_subtree_preserves_unrelated_ids() {
    let f = Fixture::setup().unwrap();
    f.trash_subtree();
    for n in [8, 9] {
        assert_eq!(
            f.request("alice", "GET", &format!("/api/v1/files/{}", id(n)), b"")
                .unwrap()
                .status,
            404
        );
    }
    assert_eq!(
        f.request("alice", "GET", "/api/v1/sync/snapshot", b"").unwrap().value()["nodes"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let ops = f
        .request("alice", "GET", "/api/v1/sync/ops?since=0", b"")
        .unwrap()
        .value();
    assert_eq!(ops["ops"].as_array().unwrap().len(), 1);
    assert_eq!(ops["ops"][0]["op_type"], "file_trash");
    assert_clean(f);
}
#[test]
fn corpus_recipient_access_and_revocation() {
    let f = Fixture::setup().unwrap();
    use base64::Engine;
    use beebeeb_core::{kdf::FileKey, opaque};
    let invite = f
        .request("bob", "GET", "/api/v1/shares/invites/incoming", b"")
        .unwrap()
        .value()["invites"][0]
        .clone();
    let (_, bob_key) = f.credentials("bob");
    let private = opaque::derive_x25519_private(&MasterKey::from_bytes(*bob_key));
    let b64 = base64::engine::general_purpose::STANDARD;
    let public: [u8; 32] = b64
        .decode(invite["sender_public_key"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let secret = opaque::x25519_shared_secret(&private, &public).unwrap();
    let share_key = opaque::derive_share_key(&secret, id(13).as_bytes());
    let frame = b64.decode(invite["encrypted_file_key"].as_str().unwrap()).unwrap();
    let unwrapped: [u8; 32] = decrypt_chunk_raw(&FileKey::from_bytes(*share_key), &frame)
        .unwrap()
        .try_into()
        .unwrap();
    let chunk = f
        .request("bob", "GET", &format!("/api/v1/files/{}/chunks/0", id(13)), b"")
        .unwrap();
    assert_eq!(
        decrypt_chunk_raw(&FileKey::from_bytes(unwrapped), &chunk.body).unwrap(),
        b"owner Al"
    );

    assert_eq!(hash(&plaintext(&f, "bob", 13)), f.corpus.objects[12].versions[0].sha256);
    assert_eq!(
        f.request("bob", "GET", "/api/v1/shares/invites/incoming", b"")
            .unwrap()
            .value()["invites"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.revoke_share();
    assert_eq!(
        f.request("bob", "GET", &format!("/api/v1/files/{}", id(13)), b"")
            .unwrap()
            .status,
        403
    );
    assert_eq!(
        f.request("bob", "GET", "/api/v1/shares/invites/incoming", b"")
            .unwrap()
            .value()["invites"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_clean(f);
}
#[test]
fn corpus_expiry_is_account_scoped() {
    let f = Fixture::setup().unwrap();
    assert_eq!(
        f.request("alice", "GET", "/api/v1/sync/snapshot", b"").unwrap().status,
        200
    );
    f.expire("alice");
    assert_eq!(
        f.request("alice", "GET", "/api/v1/sync/snapshot", b"").unwrap().status,
        401
    );
    assert_eq!(
        f.request("bob", "GET", "/api/v1/sync/snapshot", b"").unwrap().status,
        200
    );
    assert_clean(f);
}
#[test]
fn corpus_interrupted_upload_retains_acknowledged_chunks_and_commits_once() {
    let f = Fixture::setup().unwrap();
    let init = f
        .request(
            "alice",
            "POST",
            "/api/v1/uploads/init",
            &serde_json::to_vec(&json!({"file_id":id(10),"file_size_bytes":20})).unwrap(),
        )
        .unwrap();
    assert_eq!(init.status, 201);
    assert_eq!(init.value()["chunk_count"], 3);
    let (_, master) = f.credentials("alice");
    let key = derive_file_key(&MasterKey::from_bytes(*master), id(10).as_bytes());
    let bytes = Corpus::bytes(&f.corpus.objects[9].versions[0]).unwrap();
    for (index, chunk) in bytes.chunks(8).enumerate() {
        let blob = encrypt_chunk(&key, chunk).unwrap();
        let wire = [blob.nonce, blob.ciphertext].concat();
        let path = format!("/api/v1/uploads/fixture-upload/chunks/{index}");
        let first = f.request("alice", "PUT", &path, &wire).unwrap();
        if index == 1 {
            assert_eq!(first.status, 503);
            assert_eq!(f.counts().chunk_writes, 1);
            assert_eq!(
                f.request("alice", "POST", "/api/v1/uploads/fixture-upload/complete", b"{}")
                    .unwrap()
                    .status,
                409
            );
            assert_eq!(f.request("alice", "PUT", &path, &wire).unwrap().status, 200);
        } else {
            assert_eq!(first.status, 200);
        }
    }
    for _ in 0..2 {
        let complete = f
            .request("alice", "POST", "/api/v1/uploads/fixture-upload/complete", b"{}")
            .unwrap();
        assert_eq!(complete.status, 200);
        assert_eq!(complete.value()["version_number"], 1);
    }
    assert_eq!(
        hash(&plaintext(&f, "alice", 10)),
        f.corpus.objects[9].versions[0].sha256
    );
    let counts = f.counts();
    assert_eq!(
        (counts.upload_inits, counts.faults, counts.chunk_writes, counts.commits),
        (1, 1, 3, 1)
    );
    println!("upload={counts:?}");
    assert_clean(f);
}
#[test]
fn corpus_size_mismatch_reports_expected_and_actual_bytes_and_hashes() {
    let corpus = Corpus::load().unwrap();
    let mut blob = corpus.objects[3].versions[0].clone();
    let bytes = Corpus::bytes(&blob).unwrap();
    // Model a manifest/checkout disagreement without mutating shared fixture files.
    blob.size += 1;
    blob.sha256 = "0".repeat(64);
    assert_eq!(
        Corpus::bytes(&blob).unwrap_err().to_string(),
        format!(
            "fixture size mismatch: {}; expected {} bytes, SHA-256 {}; actual {} bytes, SHA-256 {}",
            blob.source, blob.size, blob.sha256, bytes.len(), hash(&bytes)
        )
    );
}
#[test]
fn corpus_oracle_rejects_wrong_hash_and_missing_scenario() {
    let mut corpus = Corpus::load().unwrap();
    corpus.objects[3].versions[0].sha256 = "0".repeat(64);
    assert!(corpus.validate().unwrap_err().to_string().contains("SHA-256 mismatch"));
    let mut corpus = Corpus::load().unwrap();
    corpus.scenarios.pop();
    assert!(corpus.validate().unwrap_err().to_string().contains("scenario set"));
}
#[test]
fn corpus_reports_do_not_export_credentials() {
    let f = Fixture::setup().unwrap();
    let report = serde_json::to_string(&f.corpus).unwrap() + &serde_json::to_string(&f.counts()).unwrap();
    for account in ["alice", "bob"] {
        let (token, key) = f.credentials(account);
        assert!(!report.contains(token.as_str()));
        let hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert!(!report.contains(&hex));
    }
    assert_clean(f);
}
async fn hydrate(f: &Fixture, bridge: &EngineBridge, n: usize) -> anyhow::Result<Vec<u8>> {
    let path = f.root.path().join(format!("hydrated-{n}"));
    bridge.hydrate_file(&id(n), &path, &[f.root.path()]).await?;
    Ok(std::fs::read(path)?)
}
fn bridge(f: &Fixture, url: &str, account: &str) -> EngineBridge {
    let (token, key) = f.credentials(account);
    EngineBridge::new(
        Arc::new(StateDb::open(f.root.path().join(format!("{account}.db"))).unwrap()),
        Arc::new(ApiClient::new(url.into(), token.to_string(), *key)),
    )
}
#[tokio::test]
async fn http_bridge_nested_hydration_thumbnails_and_cleanup_twice() {
    for _ in 0..2 {
        let mut f = Fixture::setup().unwrap();
        let url = f.start_http().unwrap();
        {
            let bridge = bridge(&f, &url, "alice");
            sync_tick(&bridge, f.root.path()).await.unwrap();
            assert_eq!(bridge.db().list_files().unwrap().len(), 12);
            for n in [4, 5, 6] {
                let row = bridge.db().get_file(&id(n)).unwrap().unwrap();
                assert_eq!(row.path, f.corpus.objects[n - 1].path);
                let bytes = hydrate(&f, &bridge, n).await.unwrap();
                assert_eq!(hash(&bytes), f.corpus.objects[n - 1].versions[0].sha256);
            }
            for n in [11, 12] {
                let before = f.counts().chunks_read;
                let bytes = bridge.fetch_thumbnail_to_memory(&id(n), "medium").await.unwrap();
                assert_eq!(hash(&bytes), f.corpus.objects[n - 1].thumbnails[0].sha256);
                assert_eq!(f.counts().chunks_read, before);
            }
            f.trash_subtree();
            sync_tick(&bridge, f.root.path()).await.unwrap();
            assert_eq!(bridge.db().list_files().unwrap().len(), 10);
            for n in [8, 9] {
                assert!(bridge.db().get_file(&id(n)).unwrap().is_none());
            }
            f.expire("alice");
            assert!(
                bridge
                    .api()
                    .sync_snapshot()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("401")
            );
        }
        assert!(f.counts().requests >= 10);
        println!("http={:?}", f.counts());
        assert_clean(f);
    }
}
#[tokio::test]
async fn http_bridge_recipient_decrypt_then_revoke() {
    let mut f = Fixture::setup().unwrap();
    let url = f.start_http().unwrap();
    {
        let bridge = bridge(&f, &url, "bob");
        sync_tick(&bridge, f.root.path()).await.unwrap();
        bridge.refresh_shared_roots().await.unwrap();
        let bytes = hydrate(&f, &bridge, 13).await.unwrap();
        assert_eq!(hash(&bytes), f.corpus.objects[12].versions[0].sha256);
        f.revoke_share();
        assert!(hydrate(&f, &bridge, 13).await.is_err());
        bridge.refresh_shared_roots().await.unwrap();
        assert!(bridge.db().get_file(&id(13)).unwrap().is_none());
        assert_eq!(bridge.db().list_files().unwrap().len(), 1);
    }
    assert_clean(f);
}
#[tokio::test]
async fn http_api_interrupted_upload_round_trip() {
    let mut f = Fixture::setup().unwrap();
    let url = f.start_http().unwrap();
    {
        let bridge = bridge(&f, &url, "alice");
        let api = bridge.api();
        let upload = api
            .init_upload(&DesktopUploadInitRequest {
                file_id: Some(id(10)),
                file_name: "fixture".into(),
                file_size_bytes: 20,
                mime_type: None,
                parent_id: None,
                profile: "desktop".into(),
                is_media: false,
                chunk_size_bytes: Some(8),
                chunk_count: Some(3),
                base_version_number: None,
            })
            .await
            .unwrap();
        let (_, master) = f.credentials("alice");
        let key = derive_file_key(&MasterKey::from_bytes(*master), id(10).as_bytes());
        for (index, chunk) in Corpus::bytes(&f.corpus.objects[9].versions[0])
            .unwrap()
            .chunks(8)
            .enumerate()
        {
            let blob = encrypt_chunk(&key, chunk).unwrap();
            let wire = [blob.nonce, blob.ciphertext].concat();
            let result = api
                .upload_session_chunk(&upload.upload_session_id, index as u32, &wire)
                .await;
            if index == 1 {
                assert!(result.unwrap_err().to_string().contains("503"));
                api.upload_session_chunk(&upload.upload_session_id, index as u32, &wire)
                    .await
                    .unwrap();
            } else {
                result.unwrap();
            }
        }
        api.complete_upload_session(&upload.upload_session_id).await.unwrap();
        sync_tick(&bridge, f.root.path()).await.unwrap();
        let bytes = hydrate(&f, &bridge, 10).await.unwrap();
        assert_eq!(hash(&bytes), f.corpus.objects[9].versions[0].sha256);
        assert_eq!((f.counts().faults, f.counts().commits), (1, 1));
    }
    assert_clean(f);
}
