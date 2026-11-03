//! Custody-key states reported to the surfaces (tests sf2258_*).
//!
//! Owns: "upgrade required" as its own non-retryable state, and the key write naming its format
//! only when custody is proven.
//! Must not: run the preparation or repair passes (`prepare`, `index_repair`).
//! Tests: the whole file is the test module.

use super::*;

#[test]
fn sf2258_upgrade_required_is_its_own_state_with_no_retry() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000006";
    let folder = FileKey::from_bytes([55; 32]);
    let blob = beebeeb_core::kdf::wrap_invite_owner_copy(&master, root.as_bytes(), &folder).unwrap();
    let mut copy = blob.nonce;
    copy.extend_from_slice(&blob.ciphertext);
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
    let log = seen.clone();
    let body_log = bodies.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (
                200,
                json!({"invites":[{
                    "id":"inv-6","file_id":root,"is_folder_share":true,"status":"approved",
                    "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":STANDARD.encode(&copy)
                }]}),
            )
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            body_log.lock().unwrap().push(req.body.clone());
            (
                409,
                json!({"error":"upgrade_required","message":"This folder's keys must be upgraded first; legacy key writes are refused."}),
            )
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("upgrade_required must not be swallowed");
    assert!(error.to_string().contains("must upgrade"), "{error}");
    assert!(
        !crate::api_client::upgrade_required(&error),
        "the caller sees the folder state, not the raw transport error"
    );
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["format"], "l7", "a custody-folder write declares L7");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen.iter().filter(|line| line.ends_with("/key")).count(),
        1,
        "the custody write is never retried: {seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("/folder-keys")),
        "it is never retried as a legacy read: {seen:?}"
    );
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2258_key_writes_name_their_format_only_with_custody() {
    assert_eq!(custody_write_format(true), Some("l7"));
    assert_eq!(custody_write_format(false), None);
}

