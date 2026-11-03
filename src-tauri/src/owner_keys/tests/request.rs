//! T3.2 (task 2275, L12/L13): 2236 request rows on the desktop.
//!
//! Owns: a request row's keys — content under the sealed request key `C`, name
//! under the derived string convention first — and the read-only, never-adopted
//! rule.
//! Must not: assert anything about other row classes.
//! Tests: `a9c_`.

use super::*;

const OWNER: &str = "40000000-0000-4000-8000-00000000000b";
const REQUEST: &str = "0f000000-0000-4000-8000-000000000001";

/// A request row whose name an old client re-sealed under the derived (string)
/// convention while its content stayed under `C` (A9c).
#[test]
fn a9c_request_row_keeps_content_under_c_and_name_under_derived() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key, owner_file_key_for_name};
    use beebeeb_core::opaque::derive_x25519_public;

    let master = MasterKey::from_bytes([7; 32]);
    let content = [0x77u8; 32];
    let r_priv = [0x21u8; 32];
    let r_pub = derive_x25519_public(&r_priv);
    // Pre-2236: the uploader seals with the empty id, the owner wraps with the
    // empty context. Both are the R5 legacy shape core cannot open.
    let sealed = beebeeb_core::file_request::seal_to_request(&r_pub, b"", &content).unwrap();
    let (wrapped_private, wrap_nonce) = beebeeb_core::file_request::wrap_request_private(&master, b"", &r_priv).unwrap();

    let derived = derive_file_key(&master, OWNER.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({
        "id": OWNER,
        "name_encrypted": name,
        "parent_id": null,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
        "acted_via_share_id": "fixture-share",
        "uploaded_by": null,
        "file_request_id": REQUEST,
        "sender_ephemeral_pubkey": STANDARD.encode(sealed.e_pub),
        "wrapped_content_key": STANDARD.encode(&sealed.wrapped_key),
    });

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let served = row.clone();
    let requests = json!({"file_requests": [{
        "id": REQUEST,
        "wrapped_private_key": STANDARD.encode(&wrapped_private),
        "wrap_nonce": STANDARD.encode(&wrap_nonce),
    }]});
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let body = if req.path.contains("/file-requests") {
            requests.clone()
        } else {
            served.clone()
        };
        Some((200, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );

    let prepared = run(bridge.prepare_owned_rows(&[row.clone()])).unwrap();
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable, "the derived name authenticates");
    assert!(!prepared[0].record.foreign_added);
    assert!(prepared[0].record.pending_owner_wrap.is_none(), "a request row is never adopted");
    assert!(prepared[0].record.owner_wrap.is_none());

    // The name is the derived convention's, so the display path works offline.
    assert_eq!(
        crate::owner_keys::display_name(&row, OWNER, bridge.api().master_key()).as_deref(),
        Some("report.txt")
    );
    assert!(matches!(
        owner_file_key_for_name(&master, OWNER, row["name_encrypted"].as_str().unwrap()).map(|(_, c)| c),
        Ok(beebeeb_core::kdf::OwnerKeyConvention::String)
    ));

    // The content key is the sealed request key `C`, never the derived one.
    let resolved = run(bridge.request_content_key(&row)).unwrap();
    assert_eq!(resolved.as_bytes(), &content);
    assert_ne!(resolved.as_bytes(), derived.as_bytes());

    // Read-only: nothing was adopted or published.
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
}
