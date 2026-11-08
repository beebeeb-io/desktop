//! B05e: the safety number view, the "compared" mark and the PATCH through core's transport.
use beebeeb_core::auth_flow::Method;
use serde_json::json;

use super::safety::*;

const ALICE_PK: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
const BOB_PK: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
const UID_A: &str = "0b3c7a52-5d0e-4f1a-9d2e-1c6f3a8b9e01";
const UID_B: &str = "7f9e2d14-3a6b-4c8d-8e1f-2b4a6c8d0e12";
const INVITE: &str = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";

fn key(h: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect();
    v.try_into().unwrap()
}

#[test]
fn the_view_is_twelve_groups_of_the_core_vector_and_nothing_else() {
    let (groups, fp) = safety_view(UID_A, &key(ALICE_PK), UID_B, &key(BOB_PK)).unwrap();
    assert_eq!(groups.len(), 12);
    assert_eq!(
        groups.concat(),
        "125734655894184515799576627809760712380416734985096630590130"
    );
    assert!(groups.iter().all(|g| g.len() == 5));
    assert_eq!(fp, "f35e5616160a30bf3c6e79fa73c576d40205e8fc3ba4e1c6dcf93e6b98e857b4");
    let answer = groups_answer(&groups);
    assert_eq!(answer.as_object().unwrap().len(), 1, "only the groups leave Rust");
}

#[test]
fn a_mark_is_read_against_the_current_key() {
    let fp = "f35e5616160a30bf3c6e79fa73c576d40205e8fc3ba4e1c6dcf93e6b98e857b4";
    assert_eq!(trust(None, fp), Trust::NotCompared);
    assert_eq!(trust(Some((fp.into(), true)), fp), Trust::Compared);
    assert_eq!(trust(Some((fp.into(), false)), fp), Trust::Mismatch);
    assert_eq!(trust(Some(("00".repeat(32), true)), fp), Trust::KeyChanged);
    assert_eq!(trust(Some(("00".repeat(32), false)), fp), Trust::KeyChanged);
    assert_eq!(trust_fields(Trust::KeyChanged), json!({ "keyChanged": true }));
    assert_eq!(trust_fields(Trust::Compared), json!({ "compared": true }));
    assert_eq!(trust_fields(Trust::Mismatch), json!({ "mismatch": true }));
    assert_eq!(trust_fields(Trust::NotCompared), json!({}));
}

#[test]
fn the_peer_comes_from_the_sent_invite() {
    let sent = json!({ "invites": [
        { "id": INVITE, "recipient_email": "jonas@northwind.studio", "recipient_id": UID_B, "status": "approved" },
        { "id": UID_A, "recipient_email": "mia@example.com", "status": "invited" },
    ]});
    assert_eq!(peer_user_id(&sent, INVITE).unwrap(), UID_B);
    assert!(peer_user_id(&sent, UID_A).unwrap_err().starts_with("unsupported"));
    assert_eq!(peer_user_id(&sent, UID_B), Err("not-found".to_string()));
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key(BOB_PK));
    assert_eq!(
        public_key_of(&json!({ "user_id": UID_B, "public_key": b64 }), UID_B).unwrap(),
        key(BOB_PK)
    );
    assert_eq!(
        public_key_of(&json!({ "user_id": UID_A, "public_key": b64 }), UID_B),
        Err("failed".to_string())
    );
    assert_eq!(
        public_key_of(&json!({ "user_id": UID_B, "public_key": "AAAA" }), UID_B),
        Err("failed".to_string())
    );
}

#[test]
fn set_access_is_a_core_patch() {
    let r = set_access_request(INVITE, true).unwrap();
    assert_eq!(r.method, Method::Patch);
    assert_eq!(r.path, format!("/api/v1/shares/invites/{INVITE}"));
    assert_eq!(r.body, Some(json!({ "permission": "write" })));
    assert_eq!(set_access_request("../x", true).unwrap_err(), "not-found");
}

#[test]
fn marks_round_trip_in_the_state_db() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(db.share_mark(UID_A, UID_B).unwrap(), None);
    db.set_share_mark(UID_A, UID_B, "ab", true).unwrap();
    assert_eq!(db.share_mark(UID_A, UID_B).unwrap(), Some(("ab".to_string(), true)));
    assert_eq!(db.share_mark(UID_B, UID_B).unwrap(), None, "keyed by account");
    db.set_share_mark(UID_A, UID_B, "cd", false).unwrap();
    assert_eq!(db.share_mark(UID_A, UID_B).unwrap(), Some(("cd".to_string(), false)));
    db.clear_share_mark(UID_A, UID_B).unwrap();
    assert_eq!(db.share_mark(UID_A, UID_B).unwrap(), None);
}
