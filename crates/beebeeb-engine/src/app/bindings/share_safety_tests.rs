//! The rules of `share_safety.rs` on their own, against a state database of each test's own and REAL core crypto:
//! the Tauri shell's `share_v2/safety_tests.rs` and `share_v2/marks_tests.rs`, ported under the same names. The seal's
//! expectation is read with the very functions `invite_for` reads it with (`share_invite_api.rs`). The engine calls
//! themselves, on a handle and a fake server, are in `share_safety_engine_tests.rs`. Every name, address, id and key
//! is invented (the two keys are RFC 7748's test vectors).

use beebeeb_core::auth_flow::HttpResponse;
use beebeeb_core::share_flow::member::{InviteOutcome, InviteStep, MemberInvite, ShareTarget};
use beebeeb_core::share_flow::safety::key_fingerprint;
use serde_json::json;

use super::*;
use crate::app::share_invite_api::expectation_of;

const ALICE_PK: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
const BOB_PK: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
const OWN: &str = "0b3c7a52-5d0e-4f1a-9d2e-1c6f3a8b9e01";
const UID_B: &str = "7f9e2d14-3a6b-4c8d-8e1f-2b4a6c8d0e12";
const UID_B2: &str = "3c5e7a9b-1d2f-4e6a-8b0c-9d1e2f3a4b5c";
const INVITE: &str = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";
const INVITE_2: &str = "1a2b3c4d-5e6f-4a1b-8c2d-3e4f5a6b7c8d";
const INVITE_3: &str = "9d8c7b6a-5f4e-4d3c-9b2a-1f0e9d8c7b6a";
const FILE: &str = "2c4e6a8b-0d1f-4a3c-8e5b-7d9f1b3d5f70";
const FOLDER: &str = "5a7c9e1b-3d5f-4a7c-9e1b-3d5f7a9c1e3b";
const EMAIL: &str = "jonas@northwind.studio";

fn key(h: &str) -> [u8; 32] {
    hexfmt::decode32(h).unwrap()
}

/// A third valid sharing key: the one an account with this vault key publishes.
fn third_key() -> [u8; 32] {
    let private = beebeeb_core::opaque::derive_x25519_private(&MasterKey::from_bytes([9; 32]));
    beebeeb_core::opaque::derive_x25519_public(&private)
}

fn b64(k: &[u8; 32]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(k)
}

fn fp_hex(k: &[u8; 32]) -> String {
    hexfmt::encode(&key_fingerprint(k).unwrap())
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Jonas's account id, as the slots a mark of his is kept in.
fn bob() -> Vec<String> {
    ids(&[UID_B])
}

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    (dir, db)
}

/// My sent invites: `INVITE` to Jonas, whose account id the server lists.
fn sent() -> Value {
    json!({ "invites": [
        { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": UID_B, "status": "claimed" },
    ]})
}

/// The seal's expectation for `email`, read exactly as `invite_for` reads it (`share_invite_api.rs`, `invite_run`):
/// the address normalized, the account ids my sent invites to it name, every mark of the person in one read, then
/// `expectation_of`. A read or a mark that fails is "failed", never `NotCompared`.
fn expectation_in(db: &StateDb, sent: &Value, own: &str, email: &str) -> Result<PeerKeyExpectation, &'static str> {
    let person = normalized_email(email);
    let peers = peers_for_email(sent, &person);
    let marks = db.person_marks(own, Some(&person), &peers).map_err(|_| "failed")?;
    expectation_of(&marks).ok_or("failed")
}

/// Every key a seal could be offered: none of them may be sealed to while the expectation is "no key".
fn refused_for_every_key(expected: PeerKeyExpectation) -> usize {
    let master = MasterKey::from_bytes([1; 32]);
    let file_key = FileKey::from_bytes([7; 32]);
    [key(ALICE_PK), key(BOB_PK), third_key()]
        .iter()
        .filter(|k| {
            member::approve_request(INVITE, FILE, &b64(k), &master, &file_key, expected).err()
                == Some(ShareFlowError::PeerKeyChanged)
        })
        .count()
}

/// A mark row written as `value`, as a newer or a damaged build could have left it.
fn plant(db: &StateDb, key: &str, value: &str) {
    db.hold_lock_for_test()
        .execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )
        .unwrap();
}

// ---- safety_tests.rs ----

#[test]
fn the_view_is_twelve_groups_from_the_sealed_key_and_nothing_else() {
    let master = MasterKey::from_bytes([1; 32]);
    let (groups, fp) = safety_view(&master, OWN, UID_B, &key(BOB_PK)).unwrap();
    assert_eq!(groups.len(), 12);
    assert!(
        groups
            .iter()
            .all(|g| g.len() == 5 && g.bytes().all(|c| c.is_ascii_digit()))
    );
    // The own key is derived in core from the vault key: another vault key gives another number.
    let (other, _) = safety_view(&MasterKey::from_bytes([2; 32]), OWN, UID_B, &key(BOB_PK)).unwrap();
    assert_ne!(groups, other);
    assert_eq!(fp, "f35e5616160a30bf3c6e79fa73c576d40205e8fc3ba4e1c6dcf93e6b98e857b4");
    assert_eq!(
        safety_view(&master, OWN, OWN, &key(BOB_PK)),
        Err(SafetyRefused::Failed),
        "own == peer is refused"
    );
    // What leaves the engine is the groups and nothing else: a pattern with no `..`, so a field added to the answer
    // (a key, a fingerprint) stops this test from building.
    let SafetyNumber { groups: only } = SafetyNumber { groups };
    assert_eq!(only.len(), 12);
}

#[test]
fn a_mark_is_read_against_each_share_key() {
    let fp = "f35e5616160a30bf3c6e79fa73c576d40205e8fc3ba4e1c6dcf93e6b98e857b4";
    let other = "00".repeat(32);
    assert_eq!(trust(None, fp, None), ShareTrust::NotCompared);
    assert_eq!(trust(Some((fp, MarkKind::Matched)), fp, None), ShareTrust::Compared);
    assert_eq!(trust(Some((fp, MarkKind::NotMatched)), fp, None), ShareTrust::Mismatch);
    assert_eq!(
        trust(Some((fp, MarkKind::Accepted)), fp, None),
        ShareTrust::NotCompared,
        "an accepted key never reads as compared"
    );
    assert_eq!(
        trust(Some((&other, MarkKind::Matched)), fp, None),
        ShareTrust::KeyChanged
    );
    assert_eq!(
        trust(Some((&other, MarkKind::NotMatched)), fp, None),
        ShareTrust::KeyChanged
    );
    assert_eq!(
        trust(Some((&other, MarkKind::Accepted)), fp, None),
        ShareTrust::KeyChanged,
        "an accepted key still pins: a share sealed to a third key is flagged"
    );
    assert_eq!(
        trust(Some((&other, MarkKind::Matched)), fp, Some(fp)),
        ShareTrust::NotCompared,
        "this share's key was accepted over People"
    );
    assert_eq!(
        trust(Some((&other, MarkKind::Matched)), fp, Some(&other)),
        ShareTrust::KeyChanged,
        "an acceptance of another key does not cover this one"
    );
    // The least trusted wins: the order the declaration gives.
    let mut all = [
        ShareTrust::Unreadable,
        ShareTrust::Compared,
        ShareTrust::Mismatch,
        ShareTrust::NotCompared,
        ShareTrust::KeyChanged,
    ];
    all.sort();
    assert_eq!(
        all,
        [
            ShareTrust::Compared,
            ShareTrust::NotCompared,
            ShareTrust::KeyChanged,
            ShareTrust::Mismatch,
            ShareTrust::Unreadable
        ]
    );
}

#[test]
fn the_peer_comes_from_the_sent_invite() {
    let sent = json!({ "invites": [
        { "id": INVITE, "recipient_email": EMAIL, "recipient_id": UID_B, "status": "approved" },
        { "id": OWN, "recipient_email": "mia@example.com", "status": "invited" },
        { "id": INVITE_2, "recipient_email": EMAIL, "recipient_id": UID_B.to_uppercase(), "status": "approved" },
    ]});
    assert_eq!(peer_user_id(&sent, INVITE).unwrap(), UID_B);
    assert_eq!(peer_user_id(&sent, OWN), Err(SafetyRefused::NoPeerId));
    assert_eq!(peer_user_id(&sent, UID_B), Err(SafetyRefused::NotFound));
    assert_eq!(
        peer_user_id(&sent, INVITE_2),
        Err(SafetyRefused::NotFound),
        "an id is never normalized"
    );
}

#[test]
fn share_anyway_seals_only_the_refused_invite_the_dialog_named() {
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "status": "claimed" },
        { "id": INVITE_2, "file_id": FILE, "recipient_email": EMAIL, "status": "claimed" },
        { "id": INVITE_3, "file_id": FILE, "recipient_email": "mia@example.com", "status": "approved" },
    ]});
    // Two claimed invites to Jonas for this file: only the named one, never the first found.
    assert_eq!(
        refused_invite(&sent, INVITE_2, FILE, " Jonas@Northwind.Studio ").unwrap(),
        INVITE_2
    );
    assert_eq!(refused_invite(&sent, INVITE, FILE, EMAIL).unwrap(), INVITE);
    let mut refused = 0;
    for (member, item, email) in [
        (INVITE_3, FILE, "mia@example.com"),
        (INVITE, OWN, EMAIL),
        (INVITE, FILE, "mia@example.com"),
        (UID_B, FILE, EMAIL),
    ] {
        assert_eq!(
            refused_invite(&sent, member, item, email),
            Err(SafetyRefused::NotFound),
            "{member} {item} {email}"
        );
        refused += 1;
    }
    assert_eq!(refused, 4);
}

#[test]
fn a_they_match_on_the_offered_key_survives_the_seal() {
    let (_dir, db) = db();
    let k2 = key(BOB_PK);
    // DW-0515 Compare on the refused invite, They match, then the seal of that invite.
    db.set_share_mark(OWN, UID_B, &fp_hex(&k2), MarkKind::Matched).unwrap();
    record_anyway_in(&db, OWN, INVITE, None, &bob(), &k2).unwrap();
    assert_eq!(
        db.share_mark(OWN, UID_B).unwrap(),
        Some((fp_hex(&k2), MarkKind::Matched))
    );
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE),
        Some(ShareTrust::Compared)
    );
}

#[test]
fn every_seal_states_an_expectation_from_the_mark() {
    let (_dir, db) = db();
    let bob_key = || PeerKeyExpectation::Compared(key_fingerprint(&key(BOB_PK)).unwrap());
    // No mark: not compared.
    assert_eq!(
        expectation_in(&db, &sent(), OWN, EMAIL).unwrap(),
        PeerKeyExpectation::NotCompared
    );
    // They match, and an accepted key, pin that key.
    for kind in [MarkKind::Matched, MarkKind::Accepted] {
        db.set_share_mark(OWN, UID_B, &fp_hex(&key(BOB_PK)), kind).unwrap();
        assert_eq!(
            expectation_in(&db, &sent(), INVITE_3, EMAIL).unwrap(),
            PeerKeyExpectation::NotCompared,
            "another account's mark does not count"
        );
        assert_eq!(
            expectation_in(&db, &sent(), OWN, "mia@example.com").unwrap(),
            PeerKeyExpectation::NotCompared,
            "another address names nobody with a mark"
        );
        assert_eq!(
            expectation_in(&db, &sent(), OWN, " JONAS@northwind.studio").unwrap(),
            bob_key(),
            "{kind:?}"
        );
    }
    // A sent list without the person's account id gives no mark under it to find.
    let no_id = json!({ "invites": [{ "id": INVITE, "recipient_email": EMAIL, "status": "claimed" }] });
    assert_eq!(
        expectation_in(&db, &no_id, OWN, EMAIL).unwrap(),
        PeerKeyExpectation::NotCompared
    );
}

#[test]
fn a_dont_match_mark_lets_no_key_through_not_even_the_one_it_was_made_on() {
    let (_dir, db) = db();
    db.set_share_mark(OWN, UID_B, &fp_hex(&key(BOB_PK)), MarkKind::NotMatched)
        .unwrap();
    let expected = expectation_in(&db, &sent(), OWN, EMAIL).unwrap();
    assert_ne!(expected, PeerKeyExpectation::NotCompared);
    assert_eq!(
        refused_for_every_key(expected),
        3,
        "every key stops at DW-0515 until the person chooses"
    );
}

#[test]
fn a_mark_that_cannot_be_read_stops_the_share() {
    let (_dir, db) = db();
    db.set_share_mark(OWN, UID_B, "not-a-fingerprint", MarkKind::Matched)
        .unwrap();
    assert_eq!(
        expectation_in(&db, &sent(), OWN, EMAIL),
        Err("failed"),
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
        { "id": INVITE_2, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": UID_B2, "status": "claimed" },
    ]});
    db.set_share_mark(OWN, UID_B, &fp_hex(&k1), MarkKind::Matched).unwrap();
    let peers = peers_for_email(&sent, EMAIL);
    assert_eq!(peers, [UID_B, UID_B2]);
    record_anyway_in(&db, OWN, INVITE_2, None, &peers, &k2).unwrap();
    for peer in [UID_B, UID_B2] {
        assert_eq!(
            db.share_mark(OWN, peer).unwrap(),
            Some((fp_hex(&k2), MarkKind::Accepted)),
            "{peer}"
        );
    }
    assert_eq!(
        expectation_in(&db, &sent, OWN, EMAIL).unwrap(),
        PeerKeyExpectation::Compared(key_fingerprint(&k2).unwrap()),
        "the next share expects the accepted key, not the old account's"
    );
}

#[test]
fn a_refused_invite_compares_the_key_it_was_offered() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    assert_eq!(number_key_in(&db, OWN, INVITE), Err(SafetyRefused::NoSealedKey));
    db.set_share_offered_key(OWN, INVITE, &hexfmt::encode(&k2)).unwrap();
    assert_eq!(
        number_key_in(&db, OWN, INVITE).unwrap(),
        (k2, NumberKey::Offered),
        "DW-0515 Compare: the offered key"
    );
    db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k1)).unwrap();
    assert_eq!(
        number_key_in(&db, OWN, INVITE).unwrap(),
        (k1, NumberKey::Sealed),
        "a sealed share: the key it was sealed to"
    );
}

#[test]
fn share_anyway_seals_to_the_carried_bytes_and_a_further_swap_is_refused() {
    let (_dir, db) = db();
    let master = MasterKey::from_bytes([1; 32]);
    let file_key = FileKey::from_bytes([7; 32]);
    let (k1, k2, k3) = (key(ALICE_PK), key(BOB_PK), third_key());
    // Jonas was compared on k1. The server then offers k2 for a new share: core refuses it, and the invite's offered
    // key is the k2 bytes that answer carried (as `invite_for` records them).
    db.set_share_mark(OWN, UID_B, &fp_hex(&k1), MarkKind::Matched).unwrap();
    let expected = expectation_in(&db, &sent(), OWN, EMAIL).unwrap();
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
    let InviteStep::Done(InviteOutcome::PeerKeyChanged {
        recipient_public_key, ..
    }) = flow.on_response(claimed).unwrap()
    else {
        panic!("a changed key is never sealed to");
    };
    db.set_share_offered_key(OWN, INVITE, &hexfmt::encode(&recipient_public_key))
        .unwrap();

    // "Share anyway": sealed to exactly the carried bytes, with Compared(fingerprint of them).
    let carried = key(db.share_offered_key(OWN, INVITE).unwrap().as_deref().unwrap());
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
    assert_eq!(refused.err(), Some(ShareFlowError::PeerKeyChanged));

    // The approval went through: the share keeps k2, and the mark is pinned to k2 (not compared).
    record_anyway_in(&db, OWN, INVITE, None, &bob(), &sealed.recipient_public_key).unwrap();
    assert_eq!(db.share_sealed_key(OWN, INVITE).unwrap(), Some(hexfmt::encode(&k2)));
    assert_eq!(db.share_offered_key(OWN, INVITE).unwrap(), None);
    assert_eq!(
        db.share_mark(OWN, UID_B).unwrap(),
        Some((fp_hex(&k2), MarkKind::Accepted))
    );
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE),
        Some(ShareTrust::NotCompared)
    );

    // The server swaps again (k3) on the next share: refused before any seal byte, DW-0515 again.
    let expected = expectation_in(&db, &sent(), OWN, EMAIL).unwrap();
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
        _ => panic!("the swapped key was not refused"),
    }
}

#[test]
fn a_mark_set_later_never_vouches_for_a_share_sealed_earlier() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    // Two shares to Jonas, sealed while nothing was compared: INVITE to k1, INVITE_2 to k2.
    db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k1)).unwrap();
    db.set_share_sealed_key(OWN, INVITE_2, &hexfmt::encode(&k2)).unwrap();
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE),
        None,
        "no mark: nothing to say"
    );
    // Later, the numbers of INVITE_2 are compared and match.
    db.set_share_mark(OWN, UID_B, &fp_hex(&k2), MarkKind::Matched).unwrap();
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE_2),
        Some(ShareTrust::Compared)
    );
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE),
        Some(ShareTrust::KeyChanged),
        "the earlier share was sealed to another key"
    );
    // A share sealed before this app kept the key (no record): the mark never vouches for it.
    assert_eq!(share_trust_in(&db, OWN, None, &bob(), INVITE_3), None);
}

#[test]
fn share_anyway_over_people_keeps_the_mark() {
    let (_dir, db) = db();
    let (k1, k2, k3) = (key(ALICE_PK), key(BOB_PK), third_key());
    db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k1)).unwrap();
    db.set_share_sealed_key(OWN, INVITE_2, &hexfmt::encode(&k3)).unwrap();
    db.set_share_mark(OWN, UID_B, &fp_hex(&k2), MarkKind::Matched).unwrap();
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE),
        Some(ShareTrust::KeyChanged)
    );
    accept_share_key_in(&db, OWN, INVITE).unwrap();
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE),
        Some(ShareTrust::NotCompared)
    );
    assert_eq!(
        db.share_mark(OWN, UID_B).unwrap(),
        Some((fp_hex(&k2), MarkKind::Matched)),
        "the mark still guards the next seal"
    );
    assert_eq!(
        share_trust_in(&db, OWN, None, &bob(), INVITE_2),
        Some(ShareTrust::KeyChanged),
        "only that share was accepted"
    );
    assert_eq!(
        accept_share_key_in(&db, OWN, INVITE_3),
        Err(SafetyRefused::NoSealedKey),
        "no sealed key recorded"
    );
}

#[test]
fn marks_round_trip_in_the_state_db() {
    let (_dir, db) = db();
    assert_eq!(db.share_mark(OWN, UID_B).unwrap(), None);
    db.set_share_mark(OWN, UID_B, "ab", MarkKind::Matched).unwrap();
    assert_eq!(
        db.share_mark(OWN, UID_B).unwrap(),
        Some(("ab".to_string(), MarkKind::Matched))
    );
    assert_eq!(db.share_mark(UID_B, UID_B).unwrap(), None, "keyed by account");
    db.set_share_mark(OWN, UID_B, "cd", MarkKind::NotMatched).unwrap();
    assert_eq!(
        db.share_mark(OWN, UID_B).unwrap(),
        Some(("cd".to_string(), MarkKind::NotMatched))
    );
    db.set_share_mark(OWN, UID_B, "ef", MarkKind::Accepted).unwrap();
    assert_eq!(
        db.share_mark(OWN, UID_B).unwrap(),
        Some(("ef".to_string(), MarkKind::Accepted))
    );
}

#[test]
fn sealed_and_offered_keys_round_trip_in_the_state_db() {
    let (_dir, db) = db();
    assert_eq!(db.share_sealed_key(OWN, INVITE).unwrap(), None);
    db.set_share_offered_key(OWN, INVITE, ALICE_PK).unwrap();
    assert_eq!(db.share_offered_key(OWN, INVITE).unwrap().as_deref(), Some(ALICE_PK));
    db.set_share_sealed_key(OWN, INVITE, BOB_PK).unwrap();
    assert_eq!(db.share_sealed_key(OWN, INVITE).unwrap().as_deref(), Some(BOB_PK));
    assert_eq!(
        db.share_offered_key(OWN, INVITE).unwrap(),
        None,
        "sealing clears the offer"
    );
    assert_eq!(db.share_sealed_key(UID_B, INVITE).unwrap(), None, "keyed by account");
}

// ---- marks_tests.rs ----

/// Compare, They don't match, Remove the last share, invite the same address again. The server's DELETE took the
/// invite row, so the sent list names no account id for the person: the mark is found under the address, and the
/// seal is refused.
#[test]
fn a_dont_match_mark_is_found_after_the_last_invite_to_the_person_is_gone() {
    let (_dir, db) = db();
    let k2 = key(BOB_PK);
    mark_compared_in(&db, OWN, Some(EMAIL), &bob(), &fp_hex(&k2), false, NumberKey::Sealed).unwrap();
    let gone = json!({ "invites": [] });
    let expected = expectation_in(&db, &gone, OWN, " Jonas@Northwind.Studio ").unwrap();
    assert_ne!(expected, PeerKeyExpectation::NotCompared, "never NotCompared");
    assert_eq!(
        refused_for_every_key(expected),
        3,
        "k2 included: nothing is sealed until they choose"
    );
}

/// The same for a "They match": the pin to the compared key outlives the last invite.
#[test]
fn a_they_match_pin_is_found_after_the_last_invite_to_the_person_is_gone() {
    let (_dir, db) = db();
    let k1 = key(ALICE_PK);
    mark_compared_in(&db, OWN, Some(EMAIL), &bob(), &fp_hex(&k1), true, NumberKey::Sealed).unwrap();
    let gone = json!({ "invites": [] });
    assert_eq!(
        expectation_in(&db, &gone, OWN, EMAIL).unwrap(),
        PeerKeyExpectation::Compared(key_fingerprint(&k1).unwrap()),
        "the compared key, and only it"
    );
    assert_eq!(
        expectation_in(&db, &gone, OWN, "mia@example.com").unwrap(),
        PeerKeyExpectation::NotCompared,
        "another address has no mark"
    );
}

/// A mark row that is there but cannot be decoded stops the share.
#[test]
fn a_mark_row_that_cannot_be_decoded_stops_the_share() {
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": UID_B, "status": "approved" },
    ]});
    let fp = fp_hex(&key(BOB_PK));
    let mut stopped = 0;
    for (name, row_key, value) in [
        ("no colon", format!("share_compared:{OWN}:{UID_B}"), fp.clone()),
        (
            "unknown kind",
            format!("share_compared:{OWN}:{UID_B}"),
            format!("{fp}:x"),
        ),
        (
            "a newer kind",
            format!("share_compared:{OWN}:{UID_B}"),
            format!("{fp}:2"),
        ),
        (
            "the address row",
            StateDb::email_mark_key_for_test(OWN, EMAIL),
            format!("{fp}:?"),
        ),
    ] {
        let (_dir, db) = db();
        plant(&db, &row_key, &value);
        assert!(
            db.share_mark(OWN, UID_B).is_err() || db.share_email_mark(OWN, EMAIL).is_err(),
            "{name}: an error, never \"no mark\""
        );
        assert_eq!(
            expectation_in(&db, &sent, OWN, EMAIL),
            Err("failed"),
            "{name}: never NotCompared"
        );
        stopped += 1;
    }
    assert_eq!(stopped, 4);
}

/// One address, two account ids. Share anyway pins both to k2; the person later compares on one share and chooses
/// They don't match. Whichever id the server lists first, the next seal expects no key: never k2.
#[test]
fn two_account_ids_for_one_address_never_let_a_dont_match_through() {
    let k2 = key(BOB_PK);
    for first in [UID_B, UID_B2] {
        let (_dir, db) = db();
        let second = if first == UID_B { UID_B2 } else { UID_B };
        let sent = json!({ "invites": [
            { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": first, "status": "approved" },
            { "id": INVITE_2, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": second, "status": "approved" },
        ]});
        let peers = peers_for_email(&sent, EMAIL);
        record_anyway_in(&db, OWN, INVITE_2, Some(EMAIL), &peers, &k2).unwrap();
        // The comparison on one share, with the slots `resolve` reads: the address and its ids.
        let (email, slots) = person_of(&sent, invite_of(&sent, INVITE).unwrap());
        assert_eq!(slots, peers);
        mark_compared_in(
            &db,
            OWN,
            email.as_deref(),
            &slots,
            &fp_hex(&k2),
            false,
            NumberKey::Sealed,
        )
        .unwrap();
        let expected = expectation_in(&db, &sent, OWN, EMAIL).unwrap();
        assert_ne!(
            expected,
            PeerKeyExpectation::Compared(key_fingerprint(&k2).unwrap()),
            "{first} first: the key marked as not matching is not expected"
        );
        assert_eq!(refused_for_every_key(expected), 3, "{first} first");
    }
}

/// Marks that pin two different keys, or one that cannot be read, stop the share in either order.
#[test]
fn two_account_ids_with_different_or_unreadable_marks_stop_the_share() {
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    for first in [UID_B, UID_B2] {
        let second = if first == UID_B { UID_B2 } else { UID_B };
        let sent = json!({ "invites": [
            { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": first, "status": "approved" },
            { "id": INVITE_2, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": second, "status": "approved" },
        ]});
        let (_dir, db) = db();
        db.set_share_mark(OWN, UID_B, &fp_hex(&k1), MarkKind::Matched).unwrap();
        db.set_share_mark(OWN, UID_B2, &fp_hex(&k2), MarkKind::Matched).unwrap();
        let expected = expectation_in(&db, &sent, OWN, EMAIL).unwrap();
        assert_eq!(
            refused_for_every_key(expected),
            3,
            "{first} first: two keys pinned, none expected"
        );

        plant(&db, &format!("share_compared:{OWN}:{UID_B2}"), "not a mark");
        assert_eq!(
            expectation_in(&db, &sent, OWN, EMAIL),
            Err("failed"),
            "{first} first: one mark cannot be read"
        );
    }
}

/// Every mark write (They match, They don't match, Share anyway) goes to the address and every account id.
#[test]
fn every_mark_write_goes_to_the_address_and_every_account_id() {
    let k2 = key(BOB_PK);
    let peers = ids(&[UID_B, UID_B2]);
    let every = |db: &StateDb| {
        [
            db.share_email_mark(OWN, EMAIL).unwrap(),
            db.share_mark(OWN, UID_B).unwrap(),
            db.share_mark(OWN, UID_B2).unwrap(),
        ]
    };
    for (matched, kind) in [(true, MarkKind::Matched), (false, MarkKind::NotMatched)] {
        let (_dir, db) = db();
        mark_compared_in(&db, OWN, Some(EMAIL), &peers, &fp_hex(&k2), matched, NumberKey::Sealed).unwrap();
        let want = Some((fp_hex(&k2), kind));
        assert_eq!(every(&db), [want.clone(), want.clone(), want], "{kind:?}");
    }
    let (_dir, db) = db();
    record_anyway_in(&db, OWN, INVITE, Some(" JONAS@northwind.studio"), &peers, &k2).unwrap();
    let want = Some((fp_hex(&k2), MarkKind::Accepted));
    assert_eq!(every(&db), [want.clone(), want.clone(), want], "Share anyway");
}

/// "They don't match" on a refused invite's offered key keeps the person's They match on the key their live share
/// was sealed to; the host withdraws the refused invite.
#[test]
fn a_dont_match_on_an_offered_key_keeps_the_mark_on_the_live_key() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    let peers = bob();
    db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k1)).unwrap();
    mark_compared_in(&db, OWN, Some(EMAIL), &peers, &fp_hex(&k1), true, NumberKey::Sealed).unwrap();
    // A new invite is refused (the server offers k2); compared, They don't match.
    db.set_share_offered_key(OWN, INVITE_2, &hexfmt::encode(&k2)).unwrap();
    mark_compared_in(&db, OWN, Some(EMAIL), &peers, &fp_hex(&k2), false, NumberKey::Offered).unwrap();
    let matched = Some((fp_hex(&k1), MarkKind::Matched));
    assert_eq!(db.share_mark(OWN, UID_B).unwrap(), matched);
    assert_eq!(db.share_email_mark(OWN, EMAIL).unwrap(), matched);
    assert_eq!(
        share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE),
        Some(ShareTrust::Compared),
        "the live share still reads as compared"
    );
    assert_eq!(
        expectation_in(&db, &json!({ "invites": [] }), OWN, EMAIL).unwrap(),
        PeerKeyExpectation::Compared(key_fingerprint(&k1).unwrap()),
        "k2 is refused again on the next share"
    );
    // With no mark at all, the refusal is written down: nothing goes to them unasked.
    let (_dir2, fresh) = self::db();
    mark_compared_in(
        &fresh,
        OWN,
        Some(EMAIL),
        &peers,
        &fp_hex(&k2),
        false,
        NumberKey::Offered,
    )
    .unwrap();
    assert_eq!(
        fresh.share_email_mark(OWN, EMAIL).unwrap(),
        Some((fp_hex(&k2), MarkKind::NotMatched))
    );
}

/// People shows a mark that cannot be read, never an unmarked row; with several marks, the least trusted one.
#[test]
fn people_shows_a_mark_that_cannot_be_read_and_the_least_trusted_mark() {
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    let peers = bob();
    for (name, value) in [
        ("undecodable", "no colon here".to_string()),
        ("not a fingerprint", "zz:1".to_string()),
    ] {
        let (_dir, db) = db();
        db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k1)).unwrap();
        plant(&db, &format!("share_compared:{OWN}:{UID_B}"), &value);
        assert_eq!(
            share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE),
            Some(ShareTrust::Unreadable),
            "{name}"
        );
    }

    let (_dir, db) = db();
    db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k1)).unwrap();
    db.set_share_email_mark(OWN, EMAIL, &fp_hex(&k1), MarkKind::Matched)
        .unwrap();
    db.set_share_mark(OWN, UID_B, &fp_hex(&k2), MarkKind::Accepted).unwrap();
    assert_eq!(
        share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE),
        Some(ShareTrust::KeyChanged),
        "one mark pins another key: not shown as compared"
    );
}

/// Share anyway on a folder withdraws the refused folder invite the dialog named (core cannot seal a folder to a
/// changed key yet), and only that one.
#[test]
fn share_anyway_on_a_folder_withdraws_the_named_refused_invite() {
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": FOLDER, "is_folder_share": true, "recipient_email": EMAIL, "status": "claimed" },
        { "id": INVITE_2, "file_id": FOLDER, "is_folder_share": true, "recipient_email": EMAIL, "status": "claimed" },
        { "id": INVITE_3, "file_id": FOLDER, "is_folder_share": true, "recipient_email": EMAIL, "status": "approved" },
    ]});
    assert_eq!(
        refused_invite(&sent, INVITE_2, FOLDER, " Jonas@Northwind.Studio").unwrap(),
        INVITE_2
    );
    let mut refused = 0;
    for (member, item) in [(INVITE_3, FOLDER), (INVITE, FILE), (UID_B, FOLDER)] {
        assert_eq!(
            refused_invite(&sent, member, item, EMAIL),
            Err(SafetyRefused::NotFound),
            "{member} {item}"
        );
        refused += 1;
    }
    assert_eq!(refused, 3);
}

/// A read that fails on the database (here: the table is gone) says nothing about the mark. People leaves the row as
/// it was; the seal still stops.
#[test]
fn a_failed_database_read_is_not_shown_as_an_unreadable_mark() {
    let (_dir, db) = db();
    let peers = bob();
    db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&key(ALICE_PK)))
        .unwrap();
    db.set_share_mark(OWN, UID_B, &fp_hex(&key(ALICE_PK)), MarkKind::Matched)
        .unwrap();
    db.hold_lock_for_test()
        .execute("ALTER TABLE sync_state RENAME TO sync_state_elsewhere", [])
        .unwrap();
    assert_eq!(share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE), None);
    assert_eq!(
        expectation_in(&db, &json!({ "invites": [] }), OWN, EMAIL),
        Err("failed"),
        "the seal still stops"
    );
}

/// The marks of one person are written in one transaction. A write that fails partway (here: the last key holds a
/// row this build cannot read) leaves every key as it was, so no two keys pin different fingerprints.
#[test]
fn a_mark_write_that_fails_partway_changes_no_key() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    let peers = ids(&[UID_B, UID_B2]);
    mark_compared_in(&db, OWN, Some(EMAIL), &bob(), &fp_hex(&k1), true, NumberKey::Sealed).unwrap();
    plant(&db, &format!("share_compared:{OWN}:{UID_B2}"), "not a mark");
    assert_eq!(
        record_anyway_in(&db, OWN, INVITE, Some(EMAIL), &peers, &k2),
        Err(SafetyRefused::Failed)
    );
    let k1_matched = Some((fp_hex(&k1), MarkKind::Matched));
    assert_eq!(
        db.share_email_mark(OWN, EMAIL).unwrap(),
        k1_matched,
        "the address row is as it was"
    );
    assert_eq!(
        db.share_mark(OWN, UID_B).unwrap(),
        k1_matched,
        "the first id is as it was"
    );
}

/// "They don't match" on an offered key reads the person's marks first (to keep them): a mark that cannot be read
/// stops it, and nothing is written over it.
#[test]
fn a_dont_match_on_an_offered_key_over_a_mark_that_cannot_be_read_writes_nothing() {
    let (_dir, db) = db();
    let k2 = key(BOB_PK);
    plant(&db, &format!("share_compared:{OWN}:{UID_B}"), "not a mark");
    assert_eq!(
        mark_compared_in(&db, OWN, Some(EMAIL), &bob(), &fp_hex(&k2), false, NumberKey::Offered),
        Err(SafetyRefused::Failed)
    );
    assert_eq!(
        db.share_email_mark(OWN, EMAIL).unwrap(),
        None,
        "the address row stays empty"
    );
}
