//! `Engine::{safety_number_for, mark_compared_for, accept_changed_key_for, share_anyway_for, share_trust_for}` on a
//! real handle, against a fake server on loopback that answers the share routes in the server's shape and records
//! every request, with REAL core crypto: a seal a test reads back is opened with the invented recipient's own key.
//! A state database of each test's own holds the items (`Engine::use_local_state_for_test`). Also the Tauri
//! `marks_tests.rs` ports whose rule now lives in `invite_for` (the withdrawals). Every name, address, id and key is
//! invented.

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use beebeeb_core::opaque::{derive_x25519_private, derive_x25519_public};
use beebeeb_core::share_flow::safety::key_fingerprint;
use serde_json::json;

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::app::share_invite_api::{InviteAnswer, InviteRefused};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use crate::test_api::{FakeApi, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const OWN: &str = "d0000000-0000-4000-8000-000000000001";
const PEER: &str = "d0000000-0000-4000-8000-000000000002";
const PEER_2: &str = "d0000000-0000-4000-8000-000000000003";
const FILE: &str = "a0000000-0000-4000-8000-000000000101";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000102";
/// An item the sent list names that is not this test's FILE: an earlier share to the same person.
const OTHER_FILE: &str = "a0000000-0000-4000-8000-000000000109";
/// An id this computer has no row for.
const UNKNOWN: &str = "a0000000-0000-4000-8000-000000000110";
const INVITE: &str = "b0000000-0000-4000-8000-000000000201";
const OLD_INVITE: &str = "b0000000-0000-4000-8000-000000000202";
const THIRD_INVITE: &str = "b0000000-0000-4000-8000-000000000203";
const EMAIL: &str = "Person@Example.eu";
const PERSON: &str = "person@example.eu";
const OWN_EMAIL: &str = "owner@example.eu";

const SENT_ROUTE: &str = "GET /api/v1/shares/invites/sent";
const INVITES: &str = "POST /api/v1/shares/invites";
const APPROVE: &str = "POST /api/v1/shares/invites/b0000000-0000-4000-8000-000000000201/approve";
const CANCEL: &str = "DELETE /api/v1/shares/invites/b0000000-0000-4000-8000-000000000201";
const CANCEL_OLD: &str = "DELETE /api/v1/shares/invites/b0000000-0000-4000-8000-000000000202";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn fp(public: &[u8; 32]) -> String {
    hexfmt::encode(&key_fingerprint(public).unwrap())
}

/// An invented person with an account: a vault key and the sharing key derived from it.
struct Person {
    master: [u8; 32],
    public: [u8; 32],
}

fn person(seed: u8) -> Person {
    let master = [seed; 32];
    let public = derive_x25519_public(&derive_x25519_private(&MasterKey::from_bytes(master)));
    Person { master, public }
}

/// What the fake answers, and what it saw.
struct Server {
    sent: Value,
    /// The key an invite's answer carries (claimed at once).
    recipient: [u8; 32],
    approve_status: u16,
    cancel_status: u16,
    hang_once: Option<&'static str>,
    /// Runs once when the sent list is read, before it is answered (a session change while the call is out).
    on_sent: Option<Box<dyn FnOnce() + Send>>,
}

struct Fixture {
    api: FakeApi,
    server: Arc<Mutex<Server>>,
    engine: Arc<TestEngine>,
    db: Arc<StateDb>,
    owner_master: [u8; 32],
    _dir: tempfile::TempDir,
}

fn row(db: &StateDb, id: &str, path: &str, folder: bool) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: FileStatus::CloudOnly,
        size_bytes: if folder { 0 } else { 1_000 },
        modified_at: 100,
        content_hash: None,
        remote_updated_at: 100,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

fn handler(server: Arc<Mutex<Server>>) -> crate::test_api::Handler {
    Arc::new(move |req: &Request| {
        let route = req.route();
        let mut s = server.lock().unwrap();
        if s.hang_once == Some(route.as_str()) {
            s.hang_once = None;
            return Reply::hang_up();
        }
        if route == SENT_ROUTE
            && let Some(hook) = s.on_sent.take()
        {
            hook();
        }
        match route.as_str() {
            SENT_ROUTE => Reply::ok(s.sent.clone()),
            INVITES => Reply::json(
                201,
                json!({ "invite_id": INVITE, "status": "claimed", "recipient_public_key": b64(&s.recipient) }),
            ),
            r if r.starts_with("POST /api/v1/shares/invites/") && r.ends_with("/approve") => {
                Reply::json(s.approve_status, json!({}))
            }
            r if r.starts_with("DELETE /api/v1/shares/invites/") => Reply::json(s.cancel_status, json!({})),
            "GET /api/v1/auth/me" => Reply::ok(json!({ "user_id": OWN, "email": OWN_EMAIL })),
            _ => Reply::error(404, "not_found"),
        }
    })
}

/// A session in memory, as a sign-in and an unlock leave it.
fn signed_in(engine: &TestEngine, master: [u8; 32]) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-session-token".into(),
        master_key: master,
        email: Some(OWN_EMAIL.into()),
    });
    engine.remember_session_user_id(Some(OWN.into()));
    engine.set_auth_present(true);
}

fn fixture(recipient: [u8; 32]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = Arc::new(StateDb::open(&path).unwrap());
    row(&db, FILE, "Docs/Invented plan.pdf", false);
    row(&db, FOLDER, "Trip", true);
    db.set_owner(&Identity::new(Some(OWN), Some(OWN_EMAIL))).unwrap();
    let server = Arc::new(Mutex::new(Server {
        sent: json!({ "invites": [] }),
        recipient,
        approve_status: 200,
        cancel_status: 200,
        hang_once: None,
        on_sent: None,
    }));
    let api = FakeApi::start(handler(server.clone()));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-safety-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
    engine.use_local_state_for_test(path);
    let owner_master = [7u8; 32];
    signed_in(&engine, owner_master);
    Fixture {
        api,
        server,
        engine,
        db,
        owner_master,
        _dir: dir,
    }
}

/// One row of my sent list.
fn invite_row(id: &str, item: &str, status: &str, peer: Option<&str>) -> Value {
    let mut row = json!({ "id": id, "file_id": item, "recipient_email": EMAIL, "status": status });
    if let Some(peer) = peer {
        row["recipient_id"] = json!(peer);
    }
    row
}

impl Fixture {
    fn turn(&self) -> SessionGeneration {
        self.engine.session_turn()
    }
    fn invite(&self) -> InviteAnswer {
        self.engine.invite_for(self.turn(), FILE, EMAIL)
    }
    fn set_sent(&self, rows: Vec<Value>) {
        self.server.lock().unwrap().sent = json!({ "invites": rows });
    }
    fn routes(&self) -> Vec<String> {
        self.api.routes()
    }
    fn count(&self, route: &str) -> usize {
        self.routes().iter().filter(|r| r.as_str() == route).count()
    }
    /// Every sealed file key any request carried (the approve's field).
    fn sealed(&self) -> Vec<Vec<u8>> {
        self.api
            .requests()
            .iter()
            .filter_map(|r| {
                r.json()
                    .get("encrypted_file_key")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .map(|s| base64::engine::general_purpose::STANDARD.decode(s).unwrap())
            .collect()
    }
    fn file_key(&self) -> [u8; 32] {
        *beebeeb_core::kdf::derive_file_key(&MasterKey::from_bytes(self.owner_master), FILE.as_bytes()).as_bytes()
    }
    fn owner_public(&self) -> [u8; 32] {
        derive_x25519_public(&derive_x25519_private(&MasterKey::from_bytes(self.owner_master)))
    }
    /// Whether `sealed` opens, with `them`'s own vault key, to the file's key.
    fn opens_for(&self, them: &Person, sealed: &[u8]) -> bool {
        share_flow::open_member_file_key(&MasterKey::from_bytes(them.master), &self.owner_public(), FILE, sealed)
            .is_ok_and(|key| key.as_bytes() == &self.file_key())
    }
    /// Writes of `prefix` rows into the state database fail from now on (a full disk, a damaged file).
    fn refuse_writes_of(&self, prefix: &str) {
        let sql = format!(
            "CREATE TRIGGER refuse_{prefix}_insert BEFORE INSERT ON sync_state WHEN NEW.key LIKE '{prefix}:%' \
             BEGIN SELECT RAISE(ABORT, 'refused'); END; \
             CREATE TRIGGER refuse_{prefix}_update BEFORE UPDATE ON sync_state WHEN NEW.key LIKE '{prefix}:%' \
             BEGIN SELECT RAISE(ABORT, 'refused'); END;"
        );
        self.db.hold_lock_for_test().execute_batch(&sql).unwrap();
    }
}

/// DW-0513 on a live share: the 12 groups core computes from the key the share was sealed to, and the very digits the
/// other person computes on their side. The refusals say what is missing, never a number from another key.
#[test]
fn safety_number_for_answers_the_number_both_people_compute_from_the_sealed_key() {
    let them = person(41);
    let f = fixture(them.public);
    assert_eq!(
        f.invite(),
        InviteAnswer::Shared {
            invite_id: INVITE.into()
        }
    );
    f.set_sent(vec![invite_row(INVITE, FILE, "approved", Some(PEER))]);

    let number = f.engine.safety_number_for(f.turn(), INVITE).expect("a number");
    assert_eq!(number.groups.len(), 12);
    let mine = safety::safety_number(&MasterKey::from_bytes(f.owner_master), OWN, PEER, &them.public).unwrap();
    let theirs = safety::safety_number(&MasterKey::from_bytes(them.master), PEER, OWN, &f.owner_public()).unwrap();
    assert_eq!(number.groups, mine.groups());
    assert_eq!(number.groups, theirs.groups(), "both sides read the same digits");
    let other = safety::safety_number(&MasterKey::from_bytes(f.owner_master), OWN, PEER, &person(42).public).unwrap();
    assert_ne!(number.groups, other.groups(), "another key gives another number");

    // The server lists no account id for the person: no number.
    f.set_sent(vec![invite_row(INVITE, FILE, "approved", None)]);
    assert_eq!(
        f.engine.safety_number_for(f.turn(), INVITE),
        Err(SafetyRefused::NoPeerId)
    );
    // A share this app holds no sealed key for: no number.
    f.set_sent(vec![invite_row(OLD_INVITE, OTHER_FILE, "approved", Some(PEER))]);
    assert_eq!(
        f.engine.safety_number_for(f.turn(), OLD_INVITE),
        Err(SafetyRefused::NoSealedKey)
    );
    // Not one of my sent invites, or not an id as the server writes it (nothing is asked for it).
    assert_eq!(
        f.engine.safety_number_for(f.turn(), INVITE),
        Err(SafetyRefused::NotFound)
    );
    let asked = f.routes().len();
    assert_eq!(
        f.engine.safety_number_for(f.turn(), &INVITE.to_uppercase()),
        Err(SafetyRefused::NotFound)
    );
    assert_eq!(f.routes().len(), asked, "nothing sent for an id that is not canonical");
}

/// DW-0515 "Compare safety numbers" on a refused invite: the number is the offered key's. They match / They don't
/// match write every slot of the person, and the next invite honours the mark, also once the last invite is gone.
#[test]
fn mark_compared_for_writes_every_slot_and_the_next_invite_honours_it() {
    let them = person(43);
    let f = fixture(them.public);
    assert!(matches!(f.invite(), InviteAnswer::Shared { .. }));
    f.set_sent(vec![invite_row(INVITE, FILE, "approved", Some(PEER))]);

    assert_eq!(f.engine.mark_compared_for(f.turn(), INVITE, true), Ok(()));
    let matched = Some((fp(&them.public), MarkKind::Matched));
    assert_eq!(f.db.share_email_mark(OWN, PERSON).unwrap(), matched);
    assert_eq!(f.db.share_mark(OWN, PEER).unwrap(), matched);

    // The last invite to them is gone (Remove access): the next one finds the pin under the address.
    let swapped = person(44);
    f.set_sent(vec![]);
    f.server.lock().unwrap().recipient = swapped.public;
    assert_eq!(
        f.invite(),
        InviteAnswer::PeerKeyChanged {
            invite_id: INVITE.into()
        }
    );
    assert_eq!(f.sealed().len(), 1, "only the first share was sealed");
    // DW-0515 Compare on that refused invite: the number is the OFFERED key's.
    // The fake names every invite INVITE: the first share's sealed-key record is not this refused invite's.
    f.set_sent(vec![invite_row(INVITE, FILE, "claimed", Some(PEER))]);
    f.db.hold_lock_for_test()
        .execute(
            "DELETE FROM sync_state WHERE key = ?1",
            rusqlite::params![format!("share_sealed:{OWN}:{INVITE}")],
        )
        .unwrap();
    let number = f.engine.safety_number_for(f.turn(), INVITE).unwrap();
    let offered = safety::safety_number(&MasterKey::from_bytes(f.owner_master), OWN, PEER, &swapped.public).unwrap();
    assert_eq!(number.groups, offered.groups());
    // They don't match on it: the "They match" on their real key stays, so the swapped key is refused again.
    assert_eq!(f.engine.mark_compared_for(f.turn(), INVITE, false), Ok(()));
    assert_eq!(f.db.share_mark(OWN, PEER).unwrap(), matched);

    // A fresh person compared on a live share and chose They don't match: no key passes, not even theirs.
    let g = fixture(them.public);
    assert!(matches!(g.invite(), InviteAnswer::Shared { .. }));
    g.set_sent(vec![invite_row(INVITE, FILE, "approved", Some(PEER))]);
    assert_eq!(g.engine.mark_compared_for(g.turn(), INVITE, false), Ok(()));
    assert_eq!(
        g.db.share_email_mark(OWN, PERSON).unwrap(),
        Some((fp(&them.public), MarkKind::NotMatched))
    );
    g.set_sent(vec![]);
    assert_eq!(
        g.invite(),
        InviteAnswer::PeerKeyChanged {
            invite_id: INVITE.into()
        }
    );
    assert_eq!(g.sealed().len(), 1, "nothing sealed after They don't match");
}

/// The named mutant of this slice: "Share anyway" seals to exactly the bytes the changed-key warning carried, never
/// to a key Beebeeb answers now. The sent list and the invite route both offer a third, live key here: a seal to it
/// does not open for the person the warning showed.
#[test]
fn share_anyway_for_seals_to_the_key_the_warning_carried_and_never_to_a_live_one() {
    let compared = person(45);
    let carried = person(46);
    let live = person(47);
    let f = fixture(carried.public);
    // The person was compared on another key; the server answers `carried`: refused, its bytes recorded.
    f.set_sent(vec![invite_row(OLD_INVITE, OTHER_FILE, "revoked", Some(PEER))]);
    f.db.set_person_marks(
        OWN,
        None,
        &[PEER.to_string()],
        &fp(&compared.public),
        MarkKind::Matched,
        false,
    )
    .unwrap();
    assert_eq!(
        f.invite(),
        InviteAnswer::PeerKeyChanged {
            invite_id: INVITE.into()
        }
    );
    assert_eq!(
        f.db.share_offered_key(OWN, INVITE).unwrap(),
        Some(hexfmt::encode(&carried.public))
    );
    assert!(f.sealed().is_empty());

    // Now Beebeeb answers a third key wherever one could be read again.
    let mut refused = invite_row(INVITE, FILE, "claimed", Some(PEER));
    refused["recipient_public_key"] = json!(b64(&live.public));
    f.set_sent(vec![refused, invite_row(OLD_INVITE, OTHER_FILE, "revoked", Some(PEER))]);
    f.server.lock().unwrap().recipient = live.public;

    assert_eq!(
        f.engine.share_anyway_for(f.turn(), INVITE, FILE, EMAIL),
        Ok(ShareAnyway::Shared {
            invite_id: INVITE.into()
        })
    );
    assert_eq!(f.count(APPROVE), 1);
    let sealed = f.sealed();
    assert_eq!(sealed.len(), 1);
    assert!(
        f.opens_for(&carried, &sealed[0]),
        "sealed to the key the warning showed"
    );
    assert!(!f.opens_for(&live, &sealed[0]), "never to the live key");
    // The share keeps that key; every slot of the person is pinned to it, as accepted (not compared).
    assert_eq!(
        f.db.share_sealed_key(OWN, INVITE).unwrap(),
        Some(hexfmt::encode(&carried.public))
    );
    assert_eq!(f.db.share_offered_key(OWN, INVITE).unwrap(), None);
    let pinned = Some((fp(&carried.public), MarkKind::Accepted));
    assert_eq!(f.db.share_mark(OWN, PEER).unwrap(), pinned);
    assert_eq!(f.db.share_email_mark(OWN, PERSON).unwrap(), pinned);

    // A further swap on the next share is refused before any seal byte.
    f.set_sent(vec![invite_row(INVITE, OTHER_FILE, "approved", Some(PEER))]);
    assert_eq!(
        f.invite(),
        InviteAnswer::PeerKeyChanged {
            invite_id: INVITE.into()
        }
    );
    assert_eq!(f.sealed().len(), 1, "no second seal");
}

/// Share anyway on a folder: the refused invite the dialog named is withdrawn, and only that one; nothing is sealed.
/// One already gone is withdrawn too.
#[test]
fn share_anyway_for_a_folder_withdraws_only_the_named_refused_invite() {
    for cancel_status in [200u16, 404] {
        let f = fixture(person(48).public);
        f.server.lock().unwrap().cancel_status = cancel_status;
        f.set_sent(vec![
            invite_row(OLD_INVITE, FOLDER, "claimed", Some(PEER)),
            invite_row(INVITE, FOLDER, "claimed", Some(PEER)),
        ]);
        assert_eq!(
            f.engine.share_anyway_for(f.turn(), INVITE, FOLDER, EMAIL),
            Ok(ShareAnyway::Withdrawn {
                invite_id: INVITE.into()
            }),
            "{cancel_status}"
        );
        assert_eq!(f.count(CANCEL), 1);
        assert_eq!(f.count(CANCEL_OLD), 0, "never another claimed invite");
        assert!(f.sealed().is_empty());
        assert!(!f.routes().iter().any(|r| r.ends_with("/approve")));
    }
}

/// Share anyway refuses what the dialog did not name, and says when the approval may have landed. Nothing is recorded
/// unless Beebeeb took the approval.
#[test]
fn share_anyway_for_refuses_what_the_dialog_did_not_name_and_says_when_the_approval_may_have_landed() {
    let carried = person(49);
    let ready = |status: &str, approve_status: u16, hang: bool| {
        let f = fixture(carried.public);
        f.set_sent(vec![invite_row(INVITE, FILE, status, Some(PEER))]);
        f.db.set_share_offered_key(OWN, INVITE, &hexfmt::encode(&carried.public))
            .unwrap();
        f.server.lock().unwrap().approve_status = approve_status;
        if hang {
            f.server.lock().unwrap().hang_once = Some(APPROVE);
        }
        f
    };
    let nothing_recorded = |f: &Fixture| {
        assert_eq!(f.db.share_sealed_key(OWN, INVITE).unwrap(), None);
        assert_eq!(f.db.share_mark(OWN, PEER).unwrap(), None);
        assert_eq!(f.db.share_email_mark(OWN, PERSON).unwrap(), None);
    };

    // Not the refused invite the dialog named: nothing is sealed or sent but the read.
    let f = ready("approved", 200, false);
    for (member, item, email) in [
        (INVITE, FILE, "other@example.eu"),
        (OLD_INVITE, FILE, EMAIL),
        (INVITE, FILE, EMAIL),
    ] {
        assert_eq!(
            f.engine.share_anyway_for(f.turn(), member, item, email),
            Err(SafetyRefused::NotFound),
            "{member} {email}"
        );
    }
    // Not one of the account's own items: nothing is asked at all.
    assert_eq!(
        f.engine.share_anyway_for(f.turn(), INVITE, UNKNOWN, EMAIL),
        Err(SafetyRefused::NotYours)
    );
    assert!(f.routes().iter().all(|r| r == SENT_ROUTE), "{:?}", f.routes());
    nothing_recorded(&f);

    // A refused invite with no offered key recorded: there are no bytes to seal to.
    let f = fixture(carried.public);
    f.set_sent(vec![invite_row(INVITE, FILE, "claimed", Some(PEER))]);
    assert_eq!(
        f.engine.share_anyway_for(f.turn(), INVITE, FILE, EMAIL),
        Err(SafetyRefused::NotFound)
    );
    assert_eq!(f.count(APPROVE), 0);

    // The approval: a 4xx proves it was not taken; a 5xx or a lost answer may have landed; a 401 ended the session.
    for (approve_status, hang, want) in [
        (403u16, false, SafetyRefused::Failed),
        (409, false, SafetyRefused::Failed),
        (500, false, SafetyRefused::ApprovalUnanswered),
        (503, false, SafetyRefused::ApprovalUnanswered),
        (200, true, SafetyRefused::ApprovalUnanswered),
        (401, false, SafetyRefused::SessionEnded),
    ] {
        let f = ready("claimed", approve_status, hang);
        assert_eq!(
            f.engine.share_anyway_for(f.turn(), INVITE, FILE, EMAIL),
            Err(want),
            "{approve_status} hang {hang}"
        );
        nothing_recorded(&f);
    }
}

/// DW-0515 over People: "Share anyway" keeps one share as it is. Only that share's key is accepted; the person's mark
/// stays and still guards the next seal. Nothing is sent.
#[test]
fn accept_changed_key_for_accepts_this_share_only_and_keeps_the_mark() {
    let (k1, k2, k3) = (person(50).public, person(51).public, person(52).public);
    let f = fixture(k2);
    f.db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k1)).unwrap();
    f.db.set_share_sealed_key(OWN, OLD_INVITE, &hexfmt::encode(&k3))
        .unwrap();
    f.db.set_person_marks(
        OWN,
        Some(PERSON),
        &[PEER.to_string()],
        &fp(&k2),
        MarkKind::Matched,
        false,
    )
    .unwrap();
    f.set_sent(vec![
        invite_row(INVITE, FILE, "approved", Some(PEER)),
        invite_row(OLD_INVITE, OTHER_FILE, "approved", Some(PEER)),
    ]);
    let trust = |f: &Fixture| f.engine.share_trust_for(f.turn()).unwrap();
    assert_eq!(
        trust(&f),
        [
            MemberTrust {
                invite_id: INVITE.into(),
                trust: ShareTrust::KeyChanged
            },
            MemberTrust {
                invite_id: OLD_INVITE.into(),
                trust: ShareTrust::KeyChanged
            },
        ]
    );

    let asked = f.routes().len();
    assert_eq!(f.engine.accept_changed_key_for(f.turn(), INVITE), Ok(()));
    assert_eq!(f.routes().len(), asked, "nothing is sent");
    assert_eq!(
        trust(&f),
        [
            MemberTrust {
                invite_id: INVITE.into(),
                trust: ShareTrust::NotCompared
            },
            MemberTrust {
                invite_id: OLD_INVITE.into(),
                trust: ShareTrust::KeyChanged
            },
        ],
        "only that share was accepted"
    );
    assert_eq!(
        f.db.share_mark(OWN, PEER).unwrap(),
        Some((fp(&k2), MarkKind::Matched)),
        "the mark still guards the next seal"
    );
    assert_eq!(
        f.engine.accept_changed_key_for(f.turn(), THIRD_INVITE),
        Err(SafetyRefused::NoSealedKey)
    );
}

/// People's marks in one read of the sent list: each share read against its own sealed key, the least trusted mark
/// of a person, a mark that cannot be read said so, and a share with nothing to say left out.
#[test]
fn share_trust_for_reads_each_share_against_its_own_key_in_one_read() {
    let (k1, k2) = (person(53).public, person(54).public);
    let f = fixture(k2);
    f.db.set_share_sealed_key(OWN, INVITE, &hexfmt::encode(&k2)).unwrap();
    f.db.set_share_sealed_key(OWN, OLD_INVITE, &hexfmt::encode(&k1))
        .unwrap();
    f.db.set_share_sealed_key(OWN, THIRD_INVITE, &hexfmt::encode(&k1))
        .unwrap();
    f.db.set_person_marks(
        OWN,
        Some(PERSON),
        &[PEER.to_string()],
        &fp(&k2),
        MarkKind::Matched,
        false,
    )
    .unwrap();
    // Another person, whose only mark this build cannot read.
    f.db.hold_lock_for_test()
        .execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, 'not a mark')",
            rusqlite::params![format!("share_compared:{OWN}:{PEER_2}")],
        )
        .unwrap();
    let mut other = invite_row(THIRD_INVITE, OTHER_FILE, "approved", Some(PEER_2));
    other["recipient_email"] = json!("someone@example.eu");
    let mut unmarked = invite_row(UNKNOWN, OTHER_FILE, "approved", None);
    unmarked["recipient_email"] = json!("nobody@example.eu");
    f.set_sent(vec![
        invite_row(INVITE, FILE, "approved", Some(PEER)),
        invite_row(OLD_INVITE, OTHER_FILE, "approved", Some(PEER)),
        other,
        unmarked,
    ]);
    assert_eq!(
        f.engine.share_trust_for(f.turn()),
        Ok(vec![
            MemberTrust {
                invite_id: INVITE.into(),
                trust: ShareTrust::Compared
            },
            MemberTrust {
                invite_id: OLD_INVITE.into(),
                trust: ShareTrust::KeyChanged
            },
            MemberTrust {
                invite_id: THIRD_INVITE.into(),
                trust: ShareTrust::Unreadable
            },
        ])
    );
    assert_eq!(f.count(SENT_ROUTE), 1, "one read for every row");
}

/// The session rule: an answer for a session that moved while the call was out is dropped and no mark is written; a
/// call for a session that is over, or behind a locked screen, sends nothing.
#[test]
fn a_session_that_moved_drops_the_answer_and_writes_no_mark() {
    let them = person(55);
    let f = fixture(them.public);
    assert!(matches!(f.invite(), InviteAnswer::Shared { .. }));
    f.set_sent(vec![invite_row(INVITE, FILE, "approved", Some(PEER))]);

    let moved = |f: &Fixture| {
        let (engine, master) = (f.engine.clone(), f.owner_master);
        f.server.lock().unwrap().on_sent = Some(Box::new(move || signed_in(&engine, master)));
    };
    moved(&f);
    assert_eq!(
        f.engine.safety_number_for(f.turn(), INVITE),
        Err(SafetyRefused::NotSignedIn)
    );
    moved(&f);
    assert_eq!(
        f.engine.mark_compared_for(f.turn(), INVITE, true),
        Err(SafetyRefused::NotSignedIn)
    );
    assert_eq!(f.db.share_mark(OWN, PEER).unwrap(), None, "no mark for a moved session");
    assert_eq!(f.db.share_email_mark(OWN, PERSON).unwrap(), None);
    moved(&f);
    assert_eq!(f.engine.share_trust_for(f.turn()), Err(SafetyRefused::NotSignedIn));

    // Asked under a session that is over: nothing is sent.
    let old = f.turn();
    signed_in(&f.engine, f.owner_master);
    let asked = f.routes().len();
    assert_eq!(f.engine.safety_number_for(old, INVITE), Err(SafetyRefused::NotSignedIn));
    assert_eq!(
        f.engine.share_anyway_for(old, INVITE, FILE, EMAIL),
        Err(SafetyRefused::NotSignedIn)
    );
    assert_eq!(
        f.engine.accept_changed_key_for(old, INVITE),
        Err(SafetyRefused::NotSignedIn)
    );
    // Behind a locked screen: refused as locked, nothing sent.
    *f.engine.lock_flow.lock().unwrap() = beebeeb_api::flows::LockFlow::at_start(beebeeb_api::flows::StartFacts {
        session_stored: true,
        ask_password_at_start: Some(false),
        vault_closed: Some(false),
        app_lock: Some(false),
    });
    f.engine.lock_screen().expect("an open lock locks the screen");
    let now = f.turn();
    assert_eq!(f.engine.safety_number_for(now, INVITE), Err(SafetyRefused::Locked));
    assert_eq!(
        f.engine.mark_compared_for(now, INVITE, false),
        Err(SafetyRefused::Locked)
    );
    assert_eq!(f.engine.share_trust_for(now), Err(SafetyRefused::Locked));
    assert_eq!(f.routes().len(), asked, "{:?}", f.routes());
}

/// Tauri `marks_tests.rs` (`an_approval_that_did_not_go_through_withdraws_the_invite`,
/// `an_approval_that_got_no_answer_ends_as_a_withdrawal`), now `invite_for`'s rule: a claimed invite whose approval
/// failed or got no answer holds no key the person can open, so it is withdrawn and nothing is recorded. Before the
/// claim, nothing was made and nothing is withdrawn.
#[test]
fn an_invite_whose_approval_did_not_go_through_is_withdrawn() {
    for (approve_status, hang) in [(500u16, false), (403, false), (200, true)] {
        let f = fixture(person(56).public);
        f.server.lock().unwrap().approve_status = approve_status;
        if hang {
            f.server.lock().unwrap().hang_once = Some(APPROVE);
        }
        assert_eq!(
            f.invite(),
            InviteAnswer::Refused(InviteRefused::Failed),
            "{approve_status} hang {hang}"
        );
        assert_eq!(f.count(CANCEL), 1, "{approve_status} hang {hang}");
        assert_eq!(f.db.share_sealed_key(OWN, INVITE).unwrap(), None);
    }
    let f = fixture(person(57).public);
    f.server.lock().unwrap().hang_once = Some(INVITES);
    assert_eq!(f.invite(), InviteAnswer::Refused(InviteRefused::Offline));
    assert_eq!(f.count(CANCEL), 0, "nothing claimed, nothing withdrawn");
}

/// Tauri `marks_tests.rs` `a_refused_invite_whose_offered_key_cannot_be_recorded_is_withdrawn`: "Share anyway" seals
/// only recorded bytes, so a refused invite whose offered key cannot be written is withdrawn. A live share whose
/// sealed-key record cannot be written still answers shared.
#[test]
fn a_refused_invite_whose_offered_key_cannot_be_recorded_is_withdrawn() {
    let compared = person(58);
    let f = fixture(person(59).public);
    f.db.set_person_marks(OWN, Some(PERSON), &[], &fp(&compared.public), MarkKind::Matched, false)
        .unwrap();
    f.refuse_writes_of("share_offered");
    assert_eq!(f.invite(), InviteAnswer::Refused(InviteRefused::Failed));
    assert_eq!(f.count(CANCEL), 1, "the refused invite is withdrawn");
    assert!(f.sealed().is_empty());

    let g = fixture(person(60).public);
    g.refuse_writes_of("share_sealed");
    assert_eq!(
        g.invite(),
        InviteAnswer::Shared {
            invite_id: INVITE.into()
        },
        "the share is live: never answered as failed"
    );
    assert_eq!(g.count(CANCEL), 0);
    assert_eq!(g.db.share_sealed_key(OWN, INVITE).unwrap(), None);
}
