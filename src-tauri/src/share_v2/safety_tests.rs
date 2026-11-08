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
fn the_view_is_twelve_groups_from_the_sealed_key_and_nothing_else() {
    let master = beebeeb_core::kdf::MasterKey::from_bytes([1; 32]);
    let (groups, fp) = safety_view(&master, UID_A, UID_B, &key(BOB_PK)).unwrap();
    assert_eq!(groups.len(), 12);
    assert!(
        groups
            .iter()
            .all(|g| g.len() == 5 && g.bytes().all(|c| c.is_ascii_digit()))
    );
    // The own key is derived in core from the master key: another master gives another number.
    let (other, _) = safety_view(
        &beebeeb_core::kdf::MasterKey::from_bytes([2; 32]),
        UID_A,
        UID_B,
        &key(BOB_PK),
    )
    .unwrap();
    assert_ne!(groups, other);
    assert_eq!(fp, "f35e5616160a30bf3c6e79fa73c576d40205e8fc3ba4e1c6dcf93e6b98e857b4");
    assert!(
        safety_view(&master, UID_A, UID_A, &key(BOB_PK)).is_err(),
        "own == peer is refused"
    );
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
}

#[test]
fn share_anyway_finds_the_claimed_invite_of_this_item_and_address() {
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": UID_A, "recipient_email": "jonas@northwind.studio", "status": "claimed" },
        { "id": UID_B, "file_id": UID_A, "recipient_email": "mia@example.com", "status": "approved" },
    ]});
    assert_eq!(
        refused_invite(&sent, UID_A, " Jonas@Northwind.Studio ").unwrap(),
        INVITE
    );
    assert_eq!(
        refused_invite(&sent, UID_A, "mia@example.com"),
        Err("not-found".to_string())
    );
    assert_eq!(
        refused_invite(&sent, UID_B, "jonas@northwind.studio"),
        Err("not-found".to_string())
    );
}

#[test]
fn set_access_is_a_core_patch() {
    let mut n = 0;
    for p in ["read", "add", "write", "admin"] {
        let r = set_access_request(INVITE, p).unwrap();
        assert_eq!(r.method, Method::Patch);
        assert_eq!(r.path, format!("/api/v1/shares/invites/{INVITE}"));
        assert_eq!(r.body, Some(json!({ "permission": p })));
        n += 1;
    }
    assert_eq!(n, 4);
    assert_eq!(set_access_request(INVITE, "owner").unwrap_err(), "failed");
    assert_eq!(set_access_request("../x", "read").unwrap_err(), "not-found");
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

#[test]
fn sealed_and_offered_keys_round_trip_in_the_state_db() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(db.share_sealed_key(UID_A, INVITE).unwrap(), None);
    db.set_share_offered_key(UID_A, INVITE, ALICE_PK).unwrap();
    assert_eq!(db.share_offered_key(UID_A, INVITE).unwrap().as_deref(), Some(ALICE_PK));
    db.set_share_sealed_key(UID_A, INVITE, BOB_PK).unwrap();
    assert_eq!(db.share_sealed_key(UID_A, INVITE).unwrap().as_deref(), Some(BOB_PK));
    assert_eq!(
        db.share_offered_key(UID_A, INVITE).unwrap(),
        None,
        "sealing clears the offer"
    );
    assert_eq!(db.share_sealed_key(UID_B, INVITE).unwrap(), None, "keyed by account");
}
