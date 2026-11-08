//! `shared` engine tests, split out of `engine_bridge.rs`.

use super::*;
    fn fixed_aes_gcm_frame(key: &[u8; 32], nonce_byte: u8, plaintext: &[u8]) -> Vec<u8> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

        let cipher = Aes256Gcm::new_from_slice(key).unwrap();
        let nonce_bytes = [nonce_byte; 12];
        let mut out = nonce_bytes.to_vec();
        let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce_bytes), plaintext).unwrap();
        out.extend_from_slice(&ciphertext);
        out
    }

    #[test]
    fn unwrap_folder_share_file_key_fixture_decrypts_content_chunk() {
        let owner_master = [0x11u8; 32];
        let recipient_master = [0x22u8; 32];
        let folder_id = "0bb0f451-986d-43e0-bbef-f1e8acb27a01";
        let child_id = "4a823d35-3ae2-4fc7-8266-762096405bc7";
        let folder_key = [0x33u8; 32];
        let child_file_key = [0x44u8; 32];
        let plaintext = b"shared plaintext fixture";

        let owner_mk = beebeeb_core::kdf::MasterKey::from_bytes(owner_master);
        let owner_private = beebeeb_core::opaque::derive_x25519_private(&owner_mk);
        let owner_public = beebeeb_core::opaque::derive_x25519_public(&owner_private);
        let recipient_mk = beebeeb_core::kdf::MasterKey::from_bytes(recipient_master);
        let recipient_private = beebeeb_core::opaque::derive_x25519_private(&recipient_mk);
        let recipient_public = beebeeb_core::opaque::derive_x25519_public(&recipient_private);
        let shared_secret = beebeeb_core::opaque::x25519_shared_secret(&owner_private, &recipient_public).unwrap();
        let share_key = beebeeb_core::opaque::derive_share_key(&shared_secret, folder_id.as_bytes());

        let encrypted_folder_key =
            base64::engine::general_purpose::STANDARD.encode(fixed_aes_gcm_frame(&share_key, 0xa1, &folder_key));
        let encrypted_child_file_key =
            base64::engine::general_purpose::STANDARD.encode(fixed_aes_gcm_frame(&folder_key, 0xb2, &child_file_key));
        let content_wire = fixed_aes_gcm_frame(&child_file_key, 0xc3, plaintext);

        let unwrapped = unwrap_folder_share_file_key(
            &recipient_master,
            &base64::engine::general_purpose::STANDARD.encode(owner_public),
            folder_id,
            &encrypted_folder_key,
            child_id,
            &serde_json::json!({
                "keys": [{ "file_id": child_id, "encrypted_file_key": encrypted_child_file_key }]
            }),
        )
        .unwrap();

        assert_eq!(unwrapped.as_bytes(), &child_file_key);
        assert_eq!(test_open_content_frame(&unwrapped, &content_wire).unwrap(), plaintext);
    }

    #[test]
    fn unwrap_folder_share_file_key_web_generated_vector_decrypts_child_key() {
        // Generated from repos/web/src/lib/folder-share-crypto.ts using the real
        // encryptFolderKeyForRecipient/encryptChildFileKey functions with
        // fixed inputs and beebeeb-wasm backing their crypto worker proxy.
        let recipient_master = [0x22u8; 32];
        let folder_id = "0bb0f451-986d-43e0-bbef-f1e8acb27a01";
        let child_id = "4a823d35-3ae2-4fc7-8266-762096405bc7";
        let owner_public_key = "zr4yc0fCIWlQgKPrOGWQG25jB3Kbm13rDDezBBz61Uc=";
        let encrypted_folder_key = "PVZ8a7JfzaV/m2p53h33xsIHsXtwaFmfcttbly6GoKnRVlnh8V5LbXiwCGFLbZhNLskOuwJX0NJQ3UzJ";
        let encrypted_child_file_key =
            "xbKO8C1APBQytc8VqT0mfT5oSaP+8h02CWObRL2kegAOsBbzFE6N4ZLfRQy2m63bLsPBjmNXLEUaHlPu";

        let folder_key =
            unwrap_folder_share_key(&recipient_master, owner_public_key, folder_id, encrypted_folder_key).unwrap();
        assert_eq!(&*folder_key, &[0x33u8; 32]);

        let child_key = unwrap_folder_share_file_key(
            &recipient_master,
            owner_public_key,
            folder_id,
            encrypted_folder_key,
            child_id,
            &serde_json::json!({
                "keys": [{ "file_id": child_id, "encrypted_file_key": encrypted_child_file_key }]
            }),
        )
        .unwrap();

        assert_eq!(child_key.as_bytes(), &[0x44u8; 32]);
    }

    #[test]
    fn decrypt_shared_name_with_key_rejects_unauthenticated_plaintext_name() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);

        assert_eq!(
            decrypt_shared_name_with_key(&file_key, "../../../../.ssh/authorized_keys"),
            None
        );
    }

    #[test]
    fn decrypt_shared_name_with_key_accepts_authenticated_metadata_name() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);
        let plaintext = serde_json::json!({
            "name": "Quarterly report.txt",
            "mime_type": "text/plain"
        })
        .to_string();
        let name_encrypted =
            serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(&file_key, &plaintext).unwrap()).unwrap();

        assert_eq!(
            decrypt_shared_name_with_key(&file_key, &name_encrypted),
            Some("Quarterly report.txt".to_string())
        );
    }

    #[test]
    fn shared_metadata_ignores_raw_server_name_fields_when_decrypt_fails() {
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x55u8; 32]);
        let metadata = serde_json::json!({
            "id": "child-file",
            "name_encrypted": "not authenticated ciphertext",
            "display_name": "../../../../.ssh/authorized_keys",
            "decrypted_name": "server-claimed.txt",
            "path": "nested/server-claimed.txt"
        });

        assert_eq!(
            shared_name_from_metadata(&metadata, &ItemKind::File, Some(&file_key)).as_deref(),
            Ok("Encrypted file")
        );
    }

    #[test]
    /// Task 2413: a decrypted name that is not one name is refused, as an own row's is (the placeholder name stays for a
    /// name that does not decrypt; several refused names under it would share one path).
    fn shared_metadata_refuses_an_authenticated_name_that_is_not_one_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = StateDb::open(dir.path().join("state.db")).expect("state db");
        let root = SharedRootMapping {
            invite_id: "invite-folder".into(),
            file_id: "929317e7-4e2f-583b-895f-8f0152b1530d".into(),
            display_name: "Shared folder".into(),
            is_folder: true,
            size_bytes: 0,
            content_type: None,
            owner_email: Some("owner@example.com".into()),
            sender_public_key: None,
            encrypted_file_key: None,
            encrypted_folder_key: None,
            file_name_encrypted: None,
            permission_bits: PERMISSION_READ,
            approved_at: None,
        };
        let file_key = beebeeb_core::kdf::FileKey::from_bytes([0x66u8; 32]);
        let malicious_name = serde_json::json!({
            "name": "../../../../.ssh/authorized_keys",
            "mime_type": "text/plain"
        })
        .to_string();
        let name_encrypted =
            serde_json::to_string(&beebeeb_core::encrypt::encrypt_metadata(&file_key, &malicious_name).unwrap())
                .unwrap();

        let entry = apply_shared_metadata_file_row(
            &db,
            &serde_json::json!({
                "id": "child-file",
                "name_encrypted": name_encrypted,
                "size_bytes": 12,
                "is_folder": false,
            }),
            &root,
            1_700_000_000,
            Some(root.file_id.clone()),
            "Shared with me/Shared folder",
            Some(&file_key),
        )
        .expect("apply shared metadata");

        assert!(entry.is_none(), "{entry:?}");
        assert!(db.get_file("child-file").unwrap().is_none(), "nothing is stored");
    }

    #[test]
    fn shared_hydrate_write_guard_rejects_persisted_traversal_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bridge = test_bridge(&dir.path().join("state.db"));
        bridge
            .db
            .upsert_file(&FileEntry {
                file_id: "9d27e191-f4bd-5042-a3a0-94ebd208d5c5".into(),
                path: "Shared with me/folder/../../outside.txt".into(),
                status: FileStatus::CloudOnly,
                size_bytes: 12,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        let mut contract = bridge
            .db
            .get_file_contract_state("9d27e191-f4bd-5042-a3a0-94ebd208d5c5")
            .unwrap()
            .unwrap();
        contract.namespace = Namespace::SharedWithMe;
        contract.shared_root_id = Some("929317e7-4e2f-583b-895f-8f0152b1530d".into());
        contract.share_id = Some("invite-folder".into());
        contract.permission_bits = PERMISSION_READ;
        bridge.db.set_file_contract_state(&contract).unwrap();

        let err = bridge
            .ensure_shared_hydrate_path_safe("9d27e191-f4bd-5042-a3a0-94ebd208d5c5")
            .unwrap_err()
            .to_string();

        assert!(err.contains("unsafe shared hydrate path"));
        assert!(err.contains(".."));
    }

    #[test]
    fn test_operation_error_classification_and_backoff() {
        assert_eq!(
            classify_operation_error("401 unauthorized Bearer token"),
            OperationFailureClass::Auth
        );
        assert_eq!(classify_operation_error("quota exceeded"), OperationFailureClass::Quota);
        assert_eq!(
            classify_operation_error("403 forbidden permission denied"),
            OperationFailureClass::Permission
        );
        assert_eq!(
            classify_operation_error("vault locked, unlock required"),
            OperationFailureClass::Locked
        );
        assert_eq!(
            classify_operation_error("connection reset by peer"),
            OperationFailureClass::Retryable
        );
        assert_eq!(retry_delay_seconds(1), 30);
        assert_eq!(retry_delay_seconds(2), 60);
        assert_eq!(retry_delay_seconds(10), 960);
    }

    #[test]
    fn test_classify_review_operation_recognizes_expired_session_as_auth_failure() {
        // Task 1546 finding 3: an expired-session (401) upload failure fell
        // through to the generic `failed_upload` bucket with the raw HTTP
        // error text and no "sign in again" path. `classify_review_operation`
        // is the function VersionCenter's list actually reads (NOT
        // `classify_operation_error`, which already classified this
        // correctly for backoff purposes but was never consulted here).
        let op = PendingOperation {
            op_id: "op-auth-1".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("6d2d20bc-321b-533d-931c-778b0b34a9fc".into()),
            parent_id: None,
            target_path: Some("Docs/notes.txt".into()),
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 1,
            max_attempts: 25,
            next_retry_at: 0,
            last_error: Some(
                "HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:8080/api/v1/uploads/init)"
                    .to_string(),
            ),
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        };

        let (kind, _status, detail, action) = classify_review_operation(&op);
        assert_eq!(kind, "auth_failure");
        assert_eq!(action, "sign_in_again");
        assert!(
            detail.to_ascii_lowercase().contains("sign in"),
            "detail should tell the user to sign in again, got: {detail}"
        );

        // A completely unrelated failure (no 401/unauthorized/invalid-token
        // substring) must still classify as a plain upload review, not auth —
        // this guards against the new check being too broad.
        let unrelated = PendingOperation {
            last_error: Some("500 Internal Server Error".to_string()),
            ..op
        };
        let (kind, _status, _detail, _action) = classify_review_operation(&unrelated);
        assert_eq!(kind, "failed_upload");
    }

    #[test]
    fn classify_ignores_status_like_digits_in_the_request_url() {
        // Task 1252: reqwest's Display suffixes `" for url (…)"`, and the URL's
        // ephemeral port / path ids can contain "401", "403", etc. Those are not
        // status codes and must never flip a retryable failure into a pausable
        // one. A real 5xx to such a port must stay Retryable.
        for port in ["45401", "40113", "34012", "50403", "44039"] {
            let err = format!(
                "HTTP status server error (500 Internal Server Error) for url (http://127.0.0.1:{port}/api/v1/uploads/upload-session-1/chunks/0)"
            );
            assert_eq!(
                classify_operation_error(&err),
                OperationFailureClass::Retryable,
                "500 to port {port} must stay Retryable, not be misread from the URL"
            );
        }
        // A connection error to the same kind of port is likewise Retryable.
        assert_eq!(
            classify_operation_error(
                "error sending request for url (http://127.0.0.1:45401/api/v1/uploads/upload-session-1/chunks/0)"
            ),
            OperationFailureClass::Retryable
        );
        // Genuine auth/permission failures still classify from reqwest's status
        // reason phrase (which sits BEFORE the stripped URL), even when the URL
        // also carries unrelated digits.
        assert_eq!(
            classify_operation_error(
                "HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:8080/api/v1/uploads/init)"
            ),
            OperationFailureClass::Auth
        );
        assert_eq!(
            classify_operation_error(
                "HTTP status client error (403 Forbidden) for url (http://127.0.0.1:8080/api/v1/files/abc)"
            ),
            OperationFailureClass::Permission
        );
    }

    #[test]
    fn test_metadata_file_row_persists_contract_for_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let metadata = serde_json::json!({
            "id": "file-1",
            "path": "spec.docx",
            "parent_id": "folder-1",
            "size_bytes": 4096,
            "updated_at": 1234,
            "content_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "version_number": 7,
            "object_version_id": "object-7"
        });

        // This fixture carries a plaintext `path` and no `name_encrypted`, so
        // it exercises the plaintext fallback in `resolve_relative_path`; the
        // master key is unused on that branch. Task 2413: the fallback is the
        // row's own name, one segment (its folders come from its parents).
        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            2000,
            &[0u8; 32],
            "",
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry.status, FileStatus::CloudOnly);
        assert_eq!(entry.remote_updated_at, 1234);
        let contract = db.get_file_contract_state("file-1").unwrap().unwrap();
        assert_eq!(contract.namespace, Namespace::MyFiles);
        assert_eq!(contract.parent_id.as_deref(), Some("folder-1"));
        assert_eq!(
            contract.permission_bits,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER
        );
        assert_eq!(
            contract.content_type.as_deref(),
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        );
        assert_eq!(contract.current_version, 7);
        assert_eq!(contract.current_object_version_id.as_deref(), Some("object-7"));
        assert_eq!(contract.last_sync_at, 2000);
    }

    #[test]
    fn test_metadata_file_row_decrypts_encrypted_name_into_path() {
        // E2EE invariant: the server returns the filename only as the
        // encrypted `name_encrypted` blob (no plaintext path). The ingest must
        // decrypt it with the master key and store the plaintext relative path
        // so placeholder seeding + hydration can resolve a real destination.
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let master_key = [7u8; 32];
        let file_id = "f1e2d3c4-0000-4000-8000-000000000001";
        // Encrypt the name exactly as every client does (shared core path).
        let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
        let name_encrypted =
            beebeeb_core::encrypt::encrypt_name(&mk, file_id, "Quarterly Report.pdf", Some("application/pdf")).unwrap();

        let metadata = serde_json::json!({
            "id": file_id,
            "name_encrypted": name_encrypted,
            "size_bytes": 2048,
            "updated_at": 5555,
            "is_folder": false,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            6000,
            &master_key,
            "",
        )
        .unwrap()
        .unwrap();

        // The decrypted name lands in `path` (a root-level item's relative
        // path under the sync root IS its name), not an empty string.
        assert_eq!(entry.path, "Quarterly Report.pdf");
        assert_eq!(entry.status, FileStatus::CloudOnly);
        let stored = db.get_file(file_id).unwrap().unwrap();
        assert_eq!(stored.path, "Quarterly Report.pdf");
    }

    #[test]
    fn test_metadata_file_row_falls_back_when_name_undecryptable() {
        // A garbled/foreign blob must not abort the sweep nor poison the row:
        // resolve_relative_path falls through to any plaintext field, else "".
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();

        let metadata = serde_json::json!({
            "id": "f1e2d3c4-0000-4000-8000-000000000002",
            "name_encrypted": "{\"cipher_suite\":\"V1Aes256Gcm\",\"nonce\":[],\"ciphertext\":[]}",
            "size_bytes": 10,
            "updated_at": 1,
            "is_folder": false,
        });

        let entry = apply_metadata_file_row(
            &db,
            &metadata,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            1,
            &[9u8; 32],
            "",
        )
        .unwrap()
        .unwrap();

        // No plaintext field present → empty path (caller skips seeding),
        // but the row still upserts so the rest of the sweep proceeds.
        assert_eq!(entry.path, "");
    }

