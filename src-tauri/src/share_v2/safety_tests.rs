//! B05e: the safety number view, the "compared" mark, the seal bound to it (core v0.2.8 item 15)
//! and the PATCH through core's transport.
use base64::Engine;
use beebeeb_core::auth_flow::{HttpResponse, Method};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::ShareFlowError;
use beebeeb_core::share_flow::member::{
    self, InviteOutcome, InviteStep, MemberInvite, PeerKeyExpectation, ShareTarget,
};
use beebeeb_core::share_flow::safety::key_fingerprint;
use serde_json::json;

use super::safety::*;
use crate::state_db::{MarkKind, StateDb};

const ALICE_PK: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
const BOB_PK: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
const UID_A: &str = "0b3c7a52-5d0e-4f1a-9d2e-1c6f3a8b9e01";
const UID_B: &str = "7f9e2d14-3a6b-4c8d-8e1f-2b4a6c8d0e12";
const INVITE: &str = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";
const INVITE_2: &str = "1a2b3c4d-5e6f-4a1b-8c2d-3e4f5a6b7c8d";
const INVITE_3: &str = "9d8c7b6a-5f4e-4d3c-9b2a-1f0e9d8c7b6a";
const FILE: &str = "2c4e6a8b-0d1f-4a3c-8e5b-7d9f1b3d5f70";
const EMAIL: &str = "jonas@northwind.studio";

fn key(h: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect();
    v.try_into().unwrap()
}

/// A third valid sharing key: the one an account with this master key publishes.
fn third_key() -> [u8; 32] {
    let private = beebeeb_core::opaque::derive_x25519_private(&MasterKey::from_bytes([9; 32]));
    beebeeb_core::opaque::derive_x25519_public(&private)
}

fn b64(k: &[u8; 32]) -> String {
    base64::engine::general_purpose::STANDARD.encode(k)
}

fn fp_hex(k: &[u8; 32]) -> String {
    hex(&key_fingerprint(k).unwrap())
}

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    (dir, db)
}

/// My sent invites: `INVITE` to Jonas, whose account id the server lists.
fn sent() -> serde_json::Value {
    json!({ "invites": [
        { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": UID_B, "status": "claimed" },
    ]})
}

#[test]
fn the_view_is_twelve_groups_from_the_sealed_key_and_nothing_else() {
    let master = MasterKey::from_bytes([1; 32]);
    let (groups, fp) = safety_view(&master, UID_A, UID_B, &key(BOB_PK)).unwrap();
    assert_eq!(groups.len(), 12);
    assert!(
        groups
            .iter()
            .all(|g| g.len() == 5 && g.bytes().all(|c| c.is_ascii_digit()))
    );
    // The own key is derived in core from the master key: another master gives another number.
    let (other, _) = safety_view(&MasterKey::from_bytes([2; 32]), UID_A, UID_B, &key(BOB_PK)).unwrap();
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
fn a_mark_is_read_against_each_share_key() {
    let fp = "f35e5616160a30bf3c6e79fa73c576d40205e8fc3ba4e1c6dcf93e6b98e857b4";
    let other = "00".repeat(32);
    assert_eq!(trust(None, fp, None), Trust::NotCompared);
    assert_eq!(trust(Some((fp, MarkKind::Matched)), fp, None), Trust::Compared);
    assert_eq!(trust(Some((fp, MarkKind::NotMatched)), fp, None), Trust::Mismatch);
    assert_eq!(
        trust(Some((fp, MarkKind::Accepted)), fp, None),
        Trust::NotCompared,
        "an accepted key never reads as compared"
    );
    assert_eq!(trust(Some((&other, MarkKind::Matched)), fp, None), Trust::KeyChanged);
    assert_eq!(trust(Some((&other, MarkKind::NotMatched)), fp, None), Trust::KeyChanged);
    assert_eq!(
        trust(Some((&other, MarkKind::Accepted)), fp, None),
        Trust::NotCompared,
        "an accepted key is no reference to differ from"
    );
    assert_eq!(
        trust(Some((&other, MarkKind::Matched)), fp, Some(fp)),
        Trust::NotCompared,
        "this share's key was accepted over People"
    );
    assert_eq!(
        trust(Some((&other, MarkKind::Matched)), fp, Some(&other)),
        Trust::KeyChanged,
        "an acceptance of another key does not cover this one"
    );
    assert_eq!(trust_fields(Trust::KeyChanged), json!({ "keyChanged": true }));
    assert_eq!(trust_fields(Trust::Compared), json!({ "compared": true }));
    assert_eq!(trust_fields(Trust::Mismatch), json!({ "mismatch": true }));
    assert_eq!(trust_fields(Trust::NotCompared), json!({}));
}

#[test]
fn the_peer_comes_from_the_sent_invite() {
    let sent = json!({ "invites": [
        { "id": INVITE, "recipient_email": EMAIL, "recipient_id": UID_B, "status": "approved" },
        { "id": UID_A, "recipient_email": "mia@example.com", "status": "invited" },
    ]});
    assert_eq!(peer_user_id(&sent, INVITE).unwrap(), UID_B);
    assert!(peer_user_id(&sent, UID_A).unwrap_err().starts_with("unsupported"));
    assert_eq!(peer_user_id(&sent, UID_B), Err("not-found".to_string()));
}

#[test]
fn share_anyway_finds_the_claimed_invite_of_this_item_and_address() {
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": UID_A, "recipient_email": EMAIL, "status": "claimed" },
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
    assert_eq!(refused_invite(&sent, UID_B, EMAIL), Err("not-found".to_string()));
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
    assert_eq!(
        set_access_request(&INVITE.to_uppercase(), "read").unwrap_err(),
        "not-found",
        "never normalized"
    );
}

#[test]
fn every_seal_states_an_expectation_from_the_mark() {
    let (_dir, db) = db();
    let bob = || PeerKeyExpectation::Compared(key_fingerprint(&key(BOB_PK)).unwrap());
    // No mark: not compared.
    assert_eq!(
        expectation_in(&db, &sent(), UID_A, EMAIL).unwrap(),
        PeerKeyExpectation::NotCompared
    );
    // They match, and an accepted key, pin that key.
    for kind in [MarkKind::Matched, MarkKind::Accepted] {
        db.set_share_mark(UID_A, UID_B, &fp_hex(&key(BOB_PK)), kind).unwrap();
        assert_eq!(
            expectation_in(&db, &sent(), INVITE_3, EMAIL).unwrap(),
            PeerKeyExpectation::NotCompared,
            "another account's mark does not count"
        );
        assert_eq!(
            expectation_in(&db, &sent(), UID_A, "mia@example.com").unwrap(),
            PeerKeyExpectation::NotCompared,
            "another address names nobody with a mark"
        );
        assert_eq!(
            expectation_in(&db, &sent(), UID_A, " JONAS@northwind.studio").unwrap(),
            bob(),
            "{kind:?}"
        );
    }
    // A sent list without the person's account id (today's server) gives no mark to find.
    let no_id = json!({ "invites": [{ "id": INVITE, "recipient_email": EMAIL, "status": "claimed" }] });
    assert_eq!(
        expectation_in(&db, &no_id, UID_A, EMAIL).unwrap(),
        PeerKeyExpectation::NotCompared
    );
}

#[test]
fn a_dont_match_mark_lets_no_key_through_not_even_the_one_it_was_made_on() {
    let (_dir, db) = db();
    let master = MasterKey::from_bytes([1; 32]);
    let file_key = FileKey::from_bytes([7; 32]);
    let k2 = key(BOB_PK);
    db.set_share_mark(UID_A, UID_B, &fp_hex(&k2), MarkKind::NotMatched)
        .unwrap();
    let expected = expectation_in(&db, &sent(), UID_A, EMAIL).unwrap();
    assert_ne!(expected, PeerKeyExpectation::NotCompared);
    let mut refused = 0;
    for k in [k2, key(ALICE_PK), third_key()] {
        let r = member::approve_request(INVITE, FILE, &b64(&k), &master, &file_key, expected);
        assert_eq!(r.unwrap_err(), ShareFlowError::PeerKeyChanged);
        refused += 1;
    }
    assert_eq!(refused, 3, "every key stops at DW-0515 until the person chooses");
}

#[test]
fn a_mark_that_cannot_be_read_stops_the_share() {
    let (_dir, db) = db();
    db.set_share_mark(UID_A, UID_B, "not-a-fingerprint", MarkKind::Matched)
        .unwrap();
    assert_eq!(
        expectation_in(&db, &sent(), UID_A, EMAIL),
        Err("failed".to_string()),
        "never NotCompared"
    );
}

#[test]
fn share_anyway_pins_every_account_id_the_address_names() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    // Jonas registered again: two account ids for one address, the old one compared on k1.
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": UID_B, "status": "approved" },
        { "id": INVITE_2, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": UID_A, "status": "claimed" },
    ]});
    let own = INVITE_3;
    db.set_share_mark(own, UID_B, &fp_hex(&k1), MarkKind::Matched).unwrap();
    let peers = peers_for_email(&sent, EMAIL);
    assert_eq!(peers, [UID_B, UID_A]);
    record_anyway_in(&db, own, INVITE_2, &peers, &k2).unwrap();
    for peer in [UID_B, UID_A] {
        assert_eq!(
            db.share_mark(own, peer).unwrap(),
            Some((fp_hex(&k2), MarkKind::Accepted)),
            "{peer}"
        );
    }
    assert_eq!(
        expectation_in(&db, &sent, own, EMAIL).unwrap(),
        PeerKeyExpectation::Compared(key_fingerprint(&k2).unwrap()),
        "the next share expects the accepted key, not the old account's"
    );
}

#[test]
fn a_refused_invite_compares_the_key_it_was_offered() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    assert!(
        number_key_in(&db, UID_A, INVITE)
            .unwrap_err()
            .starts_with("unsupported")
    );
    db.set_share_offered_key(UID_A, INVITE, &hex(&k2)).unwrap();
    assert_eq!(
        number_key_in(&db, UID_A, INVITE).unwrap(),
        k2,
        "DW-0515 Compare: the offered key"
    );
    db.set_share_sealed_key(UID_A, INVITE, &hex(&k1)).unwrap();
    assert_eq!(
        number_key_in(&db, UID_A, INVITE).unwrap(),
        k1,
        "a sealed share: the key it was sealed to"
    );
}

#[test]
fn share_anyway_seals_to_the_carried_bytes_and_a_further_swap_is_refused() {
    let (_dir, db) = db();
    let master = MasterKey::from_bytes([1; 32]);
    let file_key = FileKey::from_bytes([7; 32]);
    let (k1, k2, k3) = (key(ALICE_PK), key(BOB_PK), third_key());
    // Jonas was compared on k1. The server then offers k2 for a new share: core refuses it and the
    // invite records the offered bytes (the ones DW-0515 showed).
    db.set_share_mark(UID_A, UID_B, &fp_hex(&k1), MarkKind::Matched)
        .unwrap();
    let expected = expectation_in(&db, &sent(), UID_A, EMAIL).unwrap();
    assert_eq!(expected, PeerKeyExpectation::Compared(key_fingerprint(&k1).unwrap()));
    let target = ShareTarget {
        item_id: FILE.into(),
        is_folder: false,
    };
    let (mut flow, _) = MemberInvite::start(&target, EMAIL, &master, &file_key, expected).unwrap();
    let claimed = HttpResponse::new(
        200,
        json!({ "invite_id": INVITE, "status": "claimed", "recipient_public_key": b64(&k2) }),
    );
    let InviteStep::Done(outcome) = flow.on_response(claimed).unwrap() else {
        panic!("a changed key is never sealed to");
    };
    assert_eq!(
        record_outcome_in(&db, UID_A, &outcome).unwrap(),
        json!({ "status": "key-changed", "memberId": INVITE })
    );
    assert_eq!(db.share_offered_key(UID_A, INVITE).unwrap(), Some(hex(&k2)));

    // "Share anyway": sealed to exactly the carried bytes, with Compared(fingerprint of them).
    let carried = key(db.share_offered_key(UID_A, INVITE).unwrap().as_deref().unwrap());
    let sealed = anyway_seal(INVITE, FILE, &carried, &master, &file_key).unwrap();
    assert_eq!(sealed.recipient_public_key, k2);
    assert_eq!(sealed.request.path, format!("/api/v1/shares/invites/{INVITE}/approve"));
    // Core's own check behind it: the same expectation refuses any other key.
    let refused = member::approve_request(
        INVITE,
        FILE,
        &b64(&k3),
        &master,
        &file_key,
        PeerKeyExpectation::Compared(key_fingerprint(&carried).unwrap()),
    );
    assert_eq!(refused.unwrap_err(), ShareFlowError::PeerKeyChanged);

    // The approval went through: the share keeps k2, and the mark is pinned to k2 (not compared).
    record_anyway_in(&db, UID_A, INVITE, &[UID_B.to_string()], &sealed.recipient_public_key).unwrap();
    assert_eq!(db.share_sealed_key(UID_A, INVITE).unwrap(), Some(hex(&k2)));
    assert_eq!(db.share_offered_key(UID_A, INVITE).unwrap(), None);
    assert_eq!(
        db.share_mark(UID_A, UID_B).unwrap(),
        Some((fp_hex(&k2), MarkKind::Accepted))
    );
    assert_eq!(share_trust_in(&db, UID_A, UID_B, INVITE), Some(Trust::NotCompared));

    // The server swaps again (k3) on the next share: refused before any seal byte, DW-0515 again.
    let expected = expectation_in(&db, &sent(), UID_A, EMAIL).unwrap();
    assert_eq!(expected, PeerKeyExpectation::Compared(key_fingerprint(&k2).unwrap()));
    let (mut flow, _) = MemberInvite::start(&target, EMAIL, &master, &file_key, expected).unwrap();
    let swapped = HttpResponse::new(
        200,
        json!({ "invite_id": INVITE_2, "status": "claimed", "recipient_public_key": b64(&k3) }),
    );
    match flow.on_response(swapped).unwrap() {
        InviteStep::Done(InviteOutcome::PeerKeyChanged {
            invite_id,
            recipient_public_key,
        }) => {
            assert_eq!(invite_id, INVITE_2);
            assert_eq!(recipient_public_key, k3);
        }
        other => panic!("the swapped key was not refused: {other:?}"),
    }
}

#[test]
fn a_mark_set_later_never_vouches_for_a_share_sealed_earlier() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    // Two shares to Jonas, sealed while nothing was compared: INVITE to k1, INVITE_2 to k2.
    db.set_share_sealed_key(UID_A, INVITE, &hex(&k1)).unwrap();
    db.set_share_sealed_key(UID_A, INVITE_2, &hex(&k2)).unwrap();
    assert_eq!(
        share_trust_in(&db, UID_A, UID_B, INVITE),
        None,
        "no mark: nothing to say"
    );
    // Later, the numbers of INVITE_2 are compared and match.
    db.set_share_mark(UID_A, UID_B, &fp_hex(&k2), MarkKind::Matched)
        .unwrap();
    assert_eq!(share_trust_in(&db, UID_A, UID_B, INVITE_2), Some(Trust::Compared));
    assert_eq!(
        share_trust_in(&db, UID_A, UID_B, INVITE),
        Some(Trust::KeyChanged),
        "the earlier share was sealed to another key"
    );
    // A share sealed before this app kept the key (no record): the mark never vouches for it.
    assert_eq!(share_trust_in(&db, UID_A, UID_B, INVITE_3), None);
}

#[test]
fn share_anyway_over_people_keeps_the_mark() {
    let (_dir, db) = db();
    let (k1, k2, k3) = (key(ALICE_PK), key(BOB_PK), third_key());
    db.set_share_sealed_key(UID_A, INVITE, &hex(&k1)).unwrap();
    db.set_share_sealed_key(UID_A, INVITE_2, &hex(&k3)).unwrap();
    db.set_share_mark(UID_A, UID_B, &fp_hex(&k2), MarkKind::Matched)
        .unwrap();
    assert_eq!(share_trust_in(&db, UID_A, UID_B, INVITE), Some(Trust::KeyChanged));
    accept_share_key_in(&db, UID_A, INVITE).unwrap();
    assert_eq!(share_trust_in(&db, UID_A, UID_B, INVITE), Some(Trust::NotCompared));
    assert_eq!(
        db.share_mark(UID_A, UID_B).unwrap(),
        Some((fp_hex(&k2), MarkKind::Matched)),
        "the mark still guards the next seal"
    );
    assert_eq!(
        share_trust_in(&db, UID_A, UID_B, INVITE_2),
        Some(Trust::KeyChanged),
        "only that share was accepted"
    );
    assert!(
        accept_share_key_in(&db, UID_A, INVITE_3)
            .unwrap_err()
            .starts_with("unsupported"),
        "no sealed key recorded"
    );
}

#[test]
fn marks_round_trip_in_the_state_db() {
    let (_dir, db) = db();
    assert_eq!(db.share_mark(UID_A, UID_B).unwrap(), None);
    db.set_share_mark(UID_A, UID_B, "ab", MarkKind::Matched).unwrap();
    assert_eq!(
        db.share_mark(UID_A, UID_B).unwrap(),
        Some(("ab".to_string(), MarkKind::Matched))
    );
    assert_eq!(db.share_mark(UID_B, UID_B).unwrap(), None, "keyed by account");
    db.set_share_mark(UID_A, UID_B, "cd", MarkKind::NotMatched).unwrap();
    assert_eq!(
        db.share_mark(UID_A, UID_B).unwrap(),
        Some(("cd".to_string(), MarkKind::NotMatched))
    );
    db.set_share_mark(UID_A, UID_B, "ef", MarkKind::Accepted).unwrap();
    assert_eq!(
        db.share_mark(UID_A, UID_B).unwrap(),
        Some(("ef".to_string(), MarkKind::Accepted))
    );
}

#[test]
fn sealed_and_offered_keys_round_trip_in_the_state_db() {
    let (_dir, db) = db();
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
