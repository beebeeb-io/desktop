//! B05e, desktop #265 delta 1: a person's marks under their address and every account id, the seal
//! that reads all of them, the People row that says when one cannot be read, and the invites this
//! app withdraws when it cannot finish them.
use beebeeb_core::auth_flow::{HttpResponse, Method};
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_core::share_flow::ShareFlowError;
use beebeeb_core::share_flow::member::{
    self, InviteOutcome, InviteStep, MemberInvite, PeerKeyExpectation, ShareTarget,
};
use beebeeb_core::share_flow::safety::key_fingerprint;
use serde_json::json;

use super::safety::*;
use crate::hexfmt::encode as hex;
use crate::state_db::{MarkKind, StateDb};

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
    crate::hexfmt::decode32(h).unwrap()
}

fn third_key() -> [u8; 32] {
    let private = beebeeb_core::opaque::derive_x25519_private(&MasterKey::from_bytes([9; 32]));
    beebeeb_core::opaque::derive_x25519_public(&private)
}

fn fp_hex(k: &[u8; 32]) -> String {
    hex(&key_fingerprint(k).unwrap())
}

fn b64(k: &[u8; 32]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(k)
}

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    (dir, db)
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
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

/// Item 2 (P1, review r4508300554): compare, They don't match, Remove the last share, invite the
/// same address again. The server's DELETE took the invite row, so the sent list names no account
/// id for the person: the mark is found under the address, and the seal is refused.
#[test]
fn a_dont_match_mark_is_found_after_the_last_invite_to_the_person_is_gone() {
    let (_dir, db) = db();
    let k2 = key(BOB_PK);
    mark_compared_in(
        &db,
        OWN,
        Some(EMAIL),
        &ids(&[UID_B]),
        &fp_hex(&k2),
        false,
        NumberKey::Sealed,
    )
    .unwrap();
    let gone = json!({ "invites": [] });
    let expected = expectation_in(&db, &gone, OWN, " Jonas@Northwind.Studio ").unwrap();
    assert_ne!(expected, PeerKeyExpectation::NotCompared, "never NotCompared");
    assert_eq!(
        refused_for_every_key(expected),
        3,
        "k2 included: nothing is sealed until they choose"
    );

    // Through core's invite: the new invite claimed with the key just marked as not matching.
    let target = ShareTarget {
        item_id: FILE.into(),
        is_folder: false,
    };
    let master = MasterKey::from_bytes([1; 32]);
    let (mut flow, _) = MemberInvite::start(&target, EMAIL, &master, &FileKey::from_bytes([7; 32]), expected).unwrap();
    let claimed = HttpResponse::new(
        200,
        json!({ "invite_id": INVITE_2, "status": "claimed", "recipient_public_key": b64(&k2) }),
    );
    assert!(
        matches!(
            flow.on_response(claimed).unwrap(),
            InviteStep::Done(InviteOutcome::PeerKeyChanged { .. })
        ),
        "the seal is refused"
    );
}

/// The same for a "They match": the pin to the compared key outlives the last invite.
#[test]
fn a_they_match_pin_is_found_after_the_last_invite_to_the_person_is_gone() {
    let (_dir, db) = db();
    let k1 = key(ALICE_PK);
    mark_compared_in(
        &db,
        OWN,
        Some(EMAIL),
        &ids(&[UID_B]),
        &fp_hex(&k1),
        true,
        NumberKey::Sealed,
    )
    .unwrap();
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

/// Item 3 (P1, review r4508300626): a mark row that is there but cannot be decoded stops the share.
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
            Err("mark_unreadable".to_string()),
            "{name}: never NotCompared, and its own reason"
        );
        stopped += 1;
    }
    assert_eq!(stopped, 4);
}

/// Item 4 (P2): one address, two account ids. Share anyway pins both to k2; the person later
/// compares on one share and chooses They don't match. Whichever id the server lists first, the
/// next seal expects no key: never k2.
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
        // The comparison on UID_B's share, with the slots `resolve` reads: the address and its ids.
        mark_compared_in(&db, OWN, Some(EMAIL), &peers, &fp_hex(&k2), false, NumberKey::Sealed).unwrap();
        let expected = expectation_in(&db, &sent, OWN, EMAIL).unwrap();
        assert_ne!(
            expected,
            PeerKeyExpectation::Compared(key_fingerprint(&k2).unwrap()),
            "{first} first: the key marked as not matching is not expected"
        );
        assert_eq!(refused_for_every_key(expected), 3, "{first} first");
    }
}

/// Item 4 (P2): marks that pin two different keys, or one that cannot be read, stop the share in
/// either order.
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
            Err("mark_unreadable".to_string()),
            "{first} first: one mark cannot be read"
        );
    }
}

/// Item 4 (P2): every mark write (They match, They don't match, Share anyway) goes to the address
/// and every account id.
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

/// P3 (review r4508300810): "They don't match" on a refused invite's offered key keeps the person's
/// They match on the key their live share was sealed to; the window withdraws the refused invite.
#[test]
fn a_dont_match_on_an_offered_key_keeps_the_mark_on_the_live_key() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    let peers = ids(&[UID_B]);
    db.set_share_sealed_key(OWN, INVITE, &hex(&k1)).unwrap();
    mark_compared_in(&db, OWN, Some(EMAIL), &peers, &fp_hex(&k1), true, NumberKey::Sealed).unwrap();
    // A new invite is refused (the server offers k2); compared, They don't match.
    db.set_share_offered_key(OWN, INVITE_2, &hex(&k2)).unwrap();
    mark_compared_in(&db, OWN, Some(EMAIL), &peers, &fp_hex(&k2), false, NumberKey::Offered).unwrap();
    let matched = Some((fp_hex(&k1), MarkKind::Matched));
    assert_eq!(db.share_mark(OWN, UID_B).unwrap(), matched);
    assert_eq!(db.share_email_mark(OWN, EMAIL).unwrap(), matched);
    assert_eq!(
        share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE),
        Some(Trust::Compared),
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

/// P3 (review r4508301253): People shows a mark that cannot be read, never an unmarked row; with
/// several marks, the least trusted one.
#[test]
fn people_shows_a_mark_that_cannot_be_read_and_the_least_trusted_mark() {
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    let peers = ids(&[UID_B]);
    for (name, value) in [
        ("undecodable", "no colon here".to_string()),
        ("not a fingerprint", "zz:1".to_string()),
    ] {
        let (_dir, db) = db();
        db.set_share_sealed_key(OWN, INVITE, &hex(&k1)).unwrap();
        plant(&db, &format!("share_compared:{OWN}:{UID_B}"), &value);
        assert_eq!(
            share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE),
            Some(Trust::Unreadable),
            "{name}"
        );
    }
    assert_eq!(trust_fields(Trust::Unreadable), json!({ "markUnreadable": true }));

    let (_dir, db) = db();
    db.set_share_sealed_key(OWN, INVITE, &hex(&k1)).unwrap();
    db.set_share_email_mark(OWN, EMAIL, &fp_hex(&k1), MarkKind::Matched)
        .unwrap();
    db.set_share_mark(OWN, UID_B, &fp_hex(&k2), MarkKind::Accepted).unwrap();
    assert_eq!(
        share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE),
        Some(Trust::KeyChanged),
        "one mark pins another key: not shown as compared"
    );
}

/// P3 (review r4508300738): core sealed but the approval did not go through. The claimed invite
/// holds no key: it is withdrawn and the window hears that nothing was shared, never "invited".
#[test]
fn an_approval_that_did_not_go_through_withdraws_the_invite() {
    let (_dir, db) = db();
    let k2 = key(BOB_PK);
    let target = ShareTarget {
        item_id: FILE.into(),
        is_folder: false,
    };
    let master = MasterKey::from_bytes([1; 32]);
    let (mut flow, _) = MemberInvite::start(
        &target,
        EMAIL,
        &master,
        &FileKey::from_bytes([7; 32]),
        PeerKeyExpectation::NotCompared,
    )
    .unwrap();
    let claimed = HttpResponse::new(
        200,
        json!({ "invite_id": INVITE, "status": "claimed", "recipient_public_key": b64(&k2) }),
    );
    let InviteStep::Send(_) = flow.on_response(claimed).unwrap() else {
        panic!("the approval is sent");
    };
    let InviteStep::Done(outcome) = flow.on_response(HttpResponse::new(500, json!({}))).unwrap() else {
        panic!("a failed approval ends the flow");
    };
    assert!(matches!(
        outcome,
        InviteOutcome::AwaitingApproval {
            recipient_public_key: Some(_),
            ..
        }
    ));
    let mut withdrawn = 0;
    for outcome in [
        outcome,
        InviteOutcome::AwaitingApproval {
            invite_id: INVITE.into(),
            recipient_public_key: None,
        },
    ] {
        assert_eq!(
            record_outcome_in(Some(&db), OWN, &outcome),
            Finish::Withdraw {
                invite_id: INVITE.into(),
                then: NOT_SHARED
            }
        );
        withdrawn += 1;
    }
    assert_eq!(withdrawn, 2);
    assert_eq!(
        db.share_sealed_key(OWN, INVITE).unwrap(),
        None,
        "no share to keep a key for"
    );
}

/// P3 (review summary, `safety.rs:230`): a refused invite whose offered key cannot be recorded can
/// never be sealed by Share anyway: it is withdrawn. A live share whose record cannot be written
/// still answers `shared`.
#[test]
fn a_refused_invite_whose_offered_key_cannot_be_recorded_is_withdrawn() {
    let k2 = key(BOB_PK);
    let refused = InviteOutcome::PeerKeyChanged {
        invite_id: INVITE.into(),
        recipient_public_key: k2,
    };
    assert_eq!(
        record_outcome_in(None, OWN, &refused),
        Finish::Withdraw {
            invite_id: INVITE.into(),
            then: "failed"
        }
    );
    let shared = InviteOutcome::Shared {
        invite_id: INVITE.into(),
        recipient_public_key: k2,
    };
    assert_eq!(
        record_outcome_in(None, OWN, &shared),
        Finish::Answer(json!({ "status": "shared" }))
    );
    let (_dir, db) = db();
    assert_eq!(
        record_outcome_in(Some(&db), OWN, &refused),
        Finish::Answer(json!({ "status": "key-changed", "memberId": INVITE }))
    );
}

/// P3 (review r4508300863): Share anyway on a folder withdraws the refused folder invite the dialog
/// named (core cannot seal a folder to a changed key yet), and only that one.
#[test]
fn share_anyway_on_a_folder_withdraws_the_named_refused_invite() {
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": FOLDER, "is_folder_share": true, "recipient_email": EMAIL, "status": "claimed" },
        { "id": INVITE_2, "file_id": FOLDER, "is_folder_share": true, "recipient_email": EMAIL, "status": "claimed" },
        { "id": INVITE_3, "file_id": FOLDER, "is_folder_share": true, "recipient_email": EMAIL, "status": "approved" },
    ]});
    let req = folder_anyway_withdrawal(&sent, INVITE_2, FOLDER, " Jonas@Northwind.Studio").unwrap();
    assert_eq!(req.method, Method::Delete);
    assert_eq!(req.path, format!("/api/v1/shares/invites/{INVITE_2}"));
    let mut refused = 0;
    for (member, item) in [(INVITE_3, FOLDER), (INVITE, FILE), (UID_B, FOLDER)] {
        assert_eq!(
            folder_anyway_withdrawal(&sent, member, item, EMAIL).map(|r| r.path),
            Err("not-found".to_string()),
            "{member} {item}"
        );
        refused += 1;
    }
    assert_eq!(refused, 3);
}

/// Review of delta 1 (`flows.rs:211`): the approval request got no answer (the network dropped after
/// the invite was claimed). Core's flow ends as `AwaitingApproval`, which is withdrawn; before the
/// approval, nothing was claimed and the transport's reason stands.
#[test]
fn an_approval_that_got_no_answer_ends_as_a_withdrawal() {
    let k2 = key(BOB_PK);
    let target = ShareTarget {
        item_id: FILE.into(),
        is_folder: false,
    };
    let master = MasterKey::from_bytes([1; 32]);
    let start = || {
        MemberInvite::start(
            &target,
            EMAIL,
            &master,
            &FileKey::from_bytes([7; 32]),
            PeerKeyExpectation::NotCompared,
        )
        .unwrap()
        .0
    };
    let mut before = start();
    assert_eq!(
        super::flows::unanswered_invite(&mut before),
        None,
        "nothing claimed yet"
    );

    let mut flow = start();
    let claimed = HttpResponse::new(
        200,
        json!({ "invite_id": INVITE, "status": "claimed", "recipient_public_key": b64(&k2) }),
    );
    let InviteStep::Send(_) = flow.on_response(claimed).unwrap() else {
        panic!("the approval is sent");
    };
    let outcome = super::flows::unanswered_invite(&mut flow).expect("the claimed invite is withdrawn");
    let (_dir, db) = db();
    assert_eq!(
        record_outcome_in(Some(&db), OWN, &outcome),
        Finish::Withdraw {
            invite_id: INVITE.into(),
            then: NOT_SHARED
        }
    );
}

/// Task 2440 item 2: a mark this build cannot decode gets its own reason, so the window can offer
/// Compare again instead of a retry that would fail the same way for good.
#[test]
fn mark_unreadable_refuses_a_share_with_its_own_reason() {
    let (_dir, db) = db();
    let sent = json!({ "invites": [
        { "id": INVITE, "file_id": FILE, "recipient_email": EMAIL, "recipient_id": UID_B, "status": "approved" },
    ]});
    plant(&db, &format!("share_compared:{OWN}:{UID_B}"), "no colon here");
    assert_eq!(
        expectation_in(&db, &sent, OWN, EMAIL),
        Err("mark_unreadable".to_string()),
        "a mark we cannot decode, not a failed read"
    );
}

/// Task 2440 item 2: a fingerprint that is not 32 bytes of hex is the same class: a mark we cannot
/// read, never NotCompared (which would seal to a swapped key unpinned).
#[test]
fn mark_unreadable_covers_an_undecodable_fingerprint() {
    assert_eq!(
        expectation_of(&[("zz".into(), MarkKind::Matched)]),
        Err("mark_unreadable".to_string()),
        "not a fingerprint, so a mark we cannot read"
    );
}

/// Review of delta 1 (`safety.rs:595`): a read that fails on the database (here: the table is gone)
/// says nothing about the mark. People leaves the row as it was; the seal still stops, and the
/// reason stays `failed`, never `mark_unreadable` (there is no mark to compare again).
#[test]
fn mark_unreadable_keeps_a_busy_database_as_failed() {
    let (_dir, db) = db();
    let peers = ids(&[UID_B]);
    db.set_share_sealed_key(OWN, INVITE, &hex(&key(ALICE_PK))).unwrap();
    db.set_share_mark(OWN, UID_B, &fp_hex(&key(ALICE_PK)), MarkKind::Matched)
        .unwrap();
    db.hold_lock_for_test()
        .execute("ALTER TABLE sync_state RENAME TO sync_state_elsewhere", [])
        .unwrap();
    assert_eq!(share_trust_in(&db, OWN, Some(EMAIL), &peers, INVITE), None);
    assert_eq!(
        expectation_in(&db, &json!({ "invites": [] }), OWN, EMAIL),
        Err("failed".to_string()),
        "the seal still stops"
    );
    assert_ne!(
        expectation_in(&db, &json!({ "invites": [] }), OWN, EMAIL),
        Err("mark_unreadable".to_string()),
        "a busy database is not an unreadable mark"
    );
}

/// Review of delta 1 (`safety.rs:241`): the marks of one person are written in one transaction. A
/// write that fails partway (here: the last key holds a row this build cannot read) leaves every key
/// as it was, so no two keys pin different fingerprints.
#[test]
fn a_mark_write_that_fails_partway_changes_no_key() {
    let (_dir, db) = db();
    let (k1, k2) = (key(ALICE_PK), key(BOB_PK));
    let peers = ids(&[UID_B, UID_B2]);
    mark_compared_in(
        &db,
        OWN,
        Some(EMAIL),
        &ids(&[UID_B]),
        &fp_hex(&k1),
        true,
        NumberKey::Sealed,
    )
    .unwrap();
    plant(&db, &format!("share_compared:{OWN}:{UID_B2}"), "not a mark");
    assert_eq!(
        record_anyway_in(&db, OWN, INVITE, Some(EMAIL), &peers, &k2),
        Err("failed".to_string())
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
