//! `Engine::invite_for` and `Engine::share_target_of` on a real handle, against a fake server on loopback that
//! answers the invite routes in the server's shape and records every request, with REAL core crypto: the seal a test
//! reads back is opened with the invented recipient's own key. A state database of each test's own holds the items
//! (`Engine::use_local_state_for_test`). Every name, address, id and key is invented.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use beebeeb_core::kdf::MasterKey;
use beebeeb_core::opaque::{derive_x25519_private, derive_x25519_public};
use beebeeb_core::share_flow::safety::key_fingerprint;
use beebeeb_types::backup::SystemRole;
use serde_json::{Value, json};

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, ItemKind};
use crate::test_api::{FakeApi, Reply, Request};

thread_local! {
    /// Test seam: the next custody writes on this thread fail (`remember_custody`).
    pub(super) static CUSTODY_WRITE_FAILS: Cell<bool> = const { Cell::new(false) };
}

type TestEngine = Engine<MemoryCredentials>;

const OWN: &str = "d0000000-0000-4000-8000-000000000001";
const PEER: &str = "d0000000-0000-4000-8000-000000000002";
const FILE: &str = "a0000000-0000-4000-8000-000000000101";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000102";
const CHILD: &str = "a0000000-0000-4000-8000-000000000103";
const THEIRS: &str = "a0000000-0000-4000-8000-000000000104";
const BACKUPS: &str = "a0000000-0000-4000-8000-000000000105";
const BACKED: &str = "a0000000-0000-4000-8000-000000000106";
/// A foreign id (version nibble 8) with no owned-key record: no key on this computer.
const FOREIGN: &str = "c0000000-0000-8000-8000-000000000107";
const INVITE: &str = "b0000000-0000-4000-8000-000000000201";
const OLD_INVITE: &str = "b0000000-0000-4000-8000-000000000202";
const EMAIL: &str = "Person@Example.eu";
const OWN_EMAIL: &str = "owner@example.eu";

const INVITES: &str = "POST /api/v1/shares/invites";
const APPROVE: &str = "POST /api/v1/shares/invites/b0000000-0000-4000-8000-000000000201/approve";
const CANCEL: &str = "DELETE /api/v1/shares/invites/b0000000-0000-4000-8000-000000000201";
const CUSTODY: &str = "GET /api/v1/shared-folders/a0000000-0000-4000-8000-000000000102";
const CUSTODY_PUT: &str = "PUT /api/v1/shared-folders/a0000000-0000-4000-8000-000000000102/key";
const LISTING: &str = "GET /api/v1/files";
const FLAGS_ROUTE: &str = "GET /api/v1/feature-flags/evaluate";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
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
    /// The key the invite's answer carries (claimed at once); `None`: the person has no account yet.
    recipient: Option<[u8; 32]>,
    /// `Some(on)`: the flag's value; `None`: the flag read fails.
    flags: Option<bool>,
    approve_status: u16,
    /// What a withdrawal (`DELETE` of the invite) answers.
    cancel_status: u16,
    /// The account id `/auth/me` answers; `None`: it answers 404.
    me: Option<&'static str>,
    /// The folder's custody record, as the last PUT stored it.
    custody: Option<String>,
    /// Hang up once on this route.
    hang_once: Option<&'static str>,
    /// Every request: its route, and whether the device remembered the folder's custody when it arrived.
    seen: Vec<(String, bool)>,
    /// Runs once when the invite arrives, before it is answered (a session change while the invite is out).
    on_invite: Option<Box<dyn FnOnce() + Send>>,
}

struct Fixture {
    api: FakeApi,
    server: Arc<Mutex<Server>>,
    engine: Arc<TestEngine>,
    db: Arc<StateDb>,
    owner_master: [u8; 32],
    _dir: tempfile::TempDir,
}

fn row(db: &StateDb, id: &str, path: &str, folder: bool, received: bool) {
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
    db.upsert_file_with_contract(&entry, |contract| {
        contract.item_kind = entry.item_kind.clone();
        if received {
            contract.namespace = Namespace::SharedWithMe;
        }
    })
    .unwrap();
}

fn handler(server: Arc<Mutex<Server>>, probe: Arc<StateDb>) -> crate::test_api::Handler {
    Arc::new(move |req: &Request| {
        let route = req.route();
        let mut s = server.lock().unwrap();
        let remembered = probe.has_seen_folder_custody(FOLDER).unwrap();
        s.seen.push((route.clone(), remembered));
        if s.hang_once == Some(route.as_str()) {
            s.hang_once = None;
            return Reply::hang_up();
        }
        if route == INVITES
            && let Some(hook) = s.on_invite.take()
        {
            hook();
        }
        match route.as_str() {
            "GET /api/v1/shares/invites/sent" => Reply::ok(s.sent.clone()),
            INVITES => match s.recipient {
                Some(key) => Reply::json(
                    201,
                    json!({ "invite_id": INVITE, "status": "claimed", "recipient_public_key": b64(&key) }),
                ),
                None => Reply::json(201, json!({ "invite_id": INVITE, "status": "pending" })),
            },
            APPROVE => Reply::json(s.approve_status, json!({})),
            CANCEL => Reply::json(s.cancel_status, json!({})),
            "GET /api/v1/auth/me" => match s.me {
                Some(id) => Reply::ok(json!({ "user_id": id, "email": OWN_EMAIL })),
                None => Reply::error(404, "not_found"),
            },
            FLAGS_ROUTE => match s.flags {
                Some(on) => Reply::ok(json!({ "flags": { "shared_folders_v1": on } })),
                None => Reply::error(503, "unavailable"),
            },
            CUSTODY => match &s.custody {
                Some(record) => Reply::ok(json!({ "owner_wrapped_folder_key": record, "key_version": 1 })),
                None => Reply::error(404, "not_found"),
            },
            CUSTODY_PUT => {
                s.custody = req
                    .json()
                    .get("owner_wrapped_folder_key")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Reply::ok(json!({}))
            }
            LISTING if req.query("parent_id") == Some(FOLDER) => {
                Reply::ok(json!({ "files": [{ "id": CHILD, "is_folder": false }] }))
            }
            LISTING => Reply::ok(json!({ "files": [] })),
            _ => Reply::error(404, "not_found"),
        }
    })
}

fn engine_at(base: &str) -> Arc<TestEngine> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-invite-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
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

fn fixture(recipient: Option<[u8; 32]>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = Arc::new(StateDb::open(&path).unwrap());
    row(&db, FILE, "Docs/Invented plan.pdf", false, false);
    row(&db, FOLDER, "Trip", true, false);
    row(&db, CHILD, "Trip/Invented note.txt", false, false);
    row(&db, THEIRS, "Shared/Invented theirs.pdf", false, true);
    row(&db, BACKUPS, "Backups", true, false);
    db.set_system_role(BACKUPS, Some(SystemRole::BackupsRoot)).unwrap();
    row(&db, BACKED, "Backups/Invented PC/kept.txt", false, false);
    row(&db, FOREIGN, "Docs/Invented foreign.txt", false, false);
    db.set_owner(&Identity::new(Some(OWN), Some(OWN_EMAIL))).unwrap();
    let server = Arc::new(Mutex::new(Server {
        sent: json!({ "invites": [] }),
        recipient,
        flags: Some(true),
        approve_status: 200,
        cancel_status: 200,
        me: Some(OWN),
        custody: None,
        hang_once: None,
        seen: Vec::new(),
        on_invite: None,
    }));
    let probe = Arc::new(StateDb::open(&path).unwrap());
    let api = FakeApi::start(handler(server.clone(), probe));
    let engine = engine_at(&api.base);
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

impl Fixture {
    fn invite(&self, item: &str) -> InviteAnswer {
        self.engine.invite_for(self.engine.session_turn(), item, EMAIL)
    }
    fn routes(&self) -> Vec<String> {
        self.api.routes()
    }
    fn bodies(&self, route: &str) -> Vec<Value> {
        self.api
            .requests()
            .iter()
            .filter(|r| r.route() == route)
            .map(Request::json)
            .collect()
    }
    fn file_key(&self) -> [u8; 32] {
        *beebeeb_core::kdf::derive_file_key(&MasterKey::from_bytes(self.owner_master), FILE.as_bytes()).as_bytes()
    }
    fn owner_public(&self) -> [u8; 32] {
        derive_x25519_public(&derive_x25519_private(&MasterKey::from_bytes(self.owner_master)))
    }
}

/// Any request body that carries a sealed key for a person (the approve's field, in any request).
fn sealed_fields(api: &FakeApi) -> Vec<String> {
    api.requests()
        .iter()
        .filter_map(|r| {
            r.json()
                .get("encrypted_file_key")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

/// Everything `body` logs through `tracing`, at every level, on this thread.
fn capture_logs<T>(body: impl FnOnce() -> T) -> (T, String) {
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer: Arc<Mutex<Vec<u8>>> = Arc::default();
    let writer = Capture(buffer.clone());
    let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let out = body();
    drop(guard);
    let text = String::from_utf8_lossy(&buffer.lock().unwrap()).into_owned();
    (out, text)
}

// Rule 1.
#[test]
fn share_invite_to_a_person_with_a_key_seals_the_files_key_that_opens_with_their_key() {
    let them = person(41);
    let f = fixture(Some(them.public));
    assert_eq!(
        f.invite(FILE),
        InviteAnswer::Shared {
            invite_id: INVITE.into()
        }
    );

    let invites = f.bodies(INVITES);
    assert_eq!(invites.len(), 1);
    assert_eq!(invites[0]["file_id"], FILE);
    assert_eq!(
        invites[0]["recipient_email"], "person@example.eu",
        "core lower-cases the address"
    );
    assert!(
        invites[0].get("permission").is_none(),
        "core's invite carries no access: the server's default"
    );
    let sealed = sealed_fields(&f.api);
    assert_eq!(sealed.len(), 1, "one approval with the sealed key");
    let bytes = base64::engine::general_purpose::STANDARD.decode(&sealed[0]).unwrap();
    // The recipient opens it with their own key and the owner's public key: the file's key.
    let opened = share_flow::open_member_file_key(&MasterKey::from_bytes(them.master), &f.owner_public(), FILE, &bytes)
        .expect("the seal opens with the recipient's key");
    assert_eq!(opened.as_bytes(), &f.file_key());
    // Neither the vault key nor the file key left in any request.
    let wire: String = f
        .api
        .requests()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    for secret in [
        b64(&f.owner_master),
        hexfmt::encode(&f.owner_master),
        b64(&f.file_key()),
        hexfmt::encode(&f.file_key()),
    ] {
        assert!(!wire.contains(&secret));
    }
    // The share's sealed key is the exact key of this flow's answer.
    assert_eq!(
        f.db.share_sealed_key(OWN, INVITE).unwrap(),
        Some(hexfmt::encode(&them.public))
    );
}

// Rule 2.
#[test]
fn share_invite_to_someone_without_an_account_is_an_invitation_and_seals_nothing() {
    let f = fixture(None);
    assert_eq!(
        f.invite(FILE),
        InviteAnswer::Invited {
            invite_id: INVITE.into()
        }
    );
    assert_eq!(f.bodies(INVITES).len(), 1);
    assert!(sealed_fields(&f.api).is_empty());
    assert!(!f.routes().iter().any(|r| r == APPROVE));
}

// Rule 3.
#[test]
fn share_invite_to_a_key_that_is_not_the_compared_one_seals_nothing() {
    let compared = person(42);
    let swapped = person(43);
    let f = fixture(Some(swapped.public));
    // The sent list names the person's account id; the mark was made under it against another key.
    f.server.lock().unwrap().sent = json!({ "invites": [
        { "id": OLD_INVITE, "file_id": FOLDER, "recipient_email": "person@example.eu", "recipient_id": PEER,
          "status": "revoked", "is_folder_share": true }
    ] });
    let fingerprint = hexfmt::encode(&key_fingerprint(&compared.public).unwrap());
    f.db.set_person_marks(OWN, None, &[PEER.to_string()], &fingerprint, MarkKind::Matched, false)
        .unwrap();

    assert_eq!(
        f.invite(FILE),
        InviteAnswer::PeerKeyChanged {
            invite_id: INVITE.into()
        }
    );
    assert!(sealed_fields(&f.api).is_empty(), "no sealed key bytes were sent");
    assert!(!f.routes().iter().any(|r| r == APPROVE));
    // The key that answer carried is kept as the invite's offered key, for "Share anyway" later.
    assert_eq!(
        f.db.share_offered_key(OWN, INVITE).unwrap(),
        Some(hexfmt::encode(&swapped.public))
    );
    assert_eq!(f.db.share_sealed_key(OWN, INVITE).unwrap(), None);

    // "They don't match" on the very key the server answers: still nothing sealed.
    let g = fixture(Some(compared.public));
    g.db.set_person_marks(
        OWN,
        Some("person@example.eu"),
        &[],
        &fingerprint,
        MarkKind::NotMatched,
        false,
    )
    .unwrap();
    assert_eq!(
        g.invite(FILE),
        InviteAnswer::PeerKeyChanged {
            invite_id: INVITE.into()
        }
    );
    assert!(sealed_fields(&g.api).is_empty());

    // The same mark on the key the server answers seals (the check above can see a seal).
    let h = fixture(Some(compared.public));
    h.db.set_person_marks(
        OWN,
        Some("person@example.eu"),
        &[],
        &fingerprint,
        MarkKind::Matched,
        false,
    )
    .unwrap();
    assert_eq!(
        h.invite(FILE),
        InviteAnswer::Shared {
            invite_id: INVITE.into()
        }
    );
    assert_eq!(sealed_fields(&h.api).len(), 1);
}

#[test]
fn share_invite_stops_on_a_mark_that_cannot_be_read() {
    let f = fixture(Some(person(44).public));
    // Not 64 hex digits: unreadable as a key, never "not compared".
    f.db.set_person_marks(
        OWN,
        Some("person@example.eu"),
        &[],
        "not-a-fingerprint",
        MarkKind::Matched,
        false,
    )
    .unwrap();
    assert_eq!(f.invite(FILE), InviteAnswer::Refused(InviteRefused::Failed));
    assert!(!f.routes().iter().any(|r| r == INVITES));
}

// Rule 4.
#[test]
fn share_invite_of_a_folder_remembers_its_custody_before_the_request_that_uses_it() {
    let f = fixture(None);
    assert_eq!(
        f.invite(FOLDER),
        InviteAnswer::Invited {
            invite_id: INVITE.into()
        }
    );
    let seen = f.server.lock().unwrap().seen.clone();
    let at = |route: &str| seen.iter().find(|(r, _)| r == route).map(|(_, remembered)| *remembered);
    // The flag was read at send time, the key registered once while nothing was remembered yet (the probe can say
    // "not yet"), and remembered before the listing and the invite that use it.
    assert_eq!(seen[0].0, "GET /api/v1/shares/invites/sent");
    assert_eq!(at(FLAGS_ROUTE), Some(false));
    assert_eq!(at(CUSTODY_PUT), Some(false));
    assert_eq!(at(LISTING), Some(true));
    assert_eq!(at(INVITES), Some(true));
    let invite = &f.bodies(INVITES)[0];
    assert_eq!(invite["is_folder_share"], true);
    assert_eq!(invite["format"], "l7");
    let wraps: Vec<&str> = invite["folder_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["file_id"].as_str().unwrap())
        .collect();
    assert_eq!(wraps, [FOLDER, CHILD], "the root's own name key, then its child");

    // A device that never saw this folder, and the server holds its custody record (the one registered above, under
    // the same vault key): the record is opened, remembered before the listing that uses its key, and no key is made.
    let record = f
        .server
        .lock()
        .unwrap()
        .custody
        .clone()
        .expect("a record was registered");
    let g = fixture(None);
    g.server.lock().unwrap().custody = Some(record);
    assert!(!g.db.has_seen_folder_custody(FOLDER).unwrap());
    assert_eq!(
        g.invite(FOLDER),
        InviteAnswer::Invited {
            invite_id: INVITE.into()
        }
    );
    let seen = g.server.lock().unwrap().seen.clone();
    let at = |route: &str| seen.iter().find(|(r, _)| r == route).map(|(_, remembered)| *remembered);
    assert_eq!(at(CUSTODY), Some(false), "read while not remembered yet");
    assert_eq!(at(LISTING), Some(true));
    assert_eq!(at(INVITES), Some(true));
    assert_eq!(at(CUSTODY_PUT), None, "the record's key is used: no new one");
    assert_eq!(
        g.bodies(INVITES)[0]["folder_key_id"],
        f.bodies(INVITES)[0]["folder_key_id"],
        "the same folder key"
    );
}

#[test]
fn share_invite_of_a_folder_whose_custody_write_fails_sends_nothing_that_uses_it() {
    let f = fixture(None);
    CUSTODY_WRITE_FAILS.with(|fails| fails.set(true));
    let answer = f.invite(FOLDER);
    CUSTODY_WRITE_FAILS.with(|fails| fails.set(false));
    assert_eq!(answer, InviteAnswer::Refused(InviteRefused::Failed));
    let routes = f.routes();
    assert!(
        routes.iter().any(|r| r == CUSTODY_PUT),
        "the key was registered first (core's order)"
    );
    assert!(!routes.iter().any(|r| r == LISTING || r == INVITES), "{routes:?}");
    assert!(!f.db.has_seen_folder_custody(FOLDER).unwrap());
}

// Rule 5.
#[test]
fn share_invite_of_a_folder_with_invites_from_before_custody_needs_the_upgrade_and_writes_nothing() {
    let f = fixture(None);
    f.server.lock().unwrap().sent = json!({ "invites": [
        { "id": OLD_INVITE, "file_id": FOLDER, "recipient_email": "else@example.eu", "status": "invited",
          "is_folder_share": true, "created_at": "2026-10-01T10:00:00Z", "encrypted_owner_folder_key": "AAAA" }
    ] });
    assert_eq!(
        f.invite(FOLDER),
        InviteAnswer::Refused(InviteRefused::FolderNeedsUpgrade)
    );
    let routes = f.routes();
    assert!(
        !routes.iter().any(|r| r == CUSTODY_PUT || r == INVITES || r == LISTING),
        "{routes:?}"
    );
    assert!(!f.db.has_seen_folder_custody(FOLDER).unwrap());
}

// Rule 6.
#[test]
fn share_invite_of_a_folder_when_the_flag_cannot_be_read_asks_nothing_more() {
    let f = fixture(None);
    f.server.lock().unwrap().flags = None;
    assert_eq!(
        f.invite(FOLDER),
        InviteAnswer::Refused(InviteRefused::SharedFoldersFlagUnknown)
    );
    let routes = f.routes();
    assert_eq!(routes.last().map(String::as_str), Some(FLAGS_ROUTE), "{routes:?}");
    assert!(!routes.iter().any(|r| r == CUSTODY || r == INVITES));
    // A file needs no flag.
    let g = fixture(None);
    g.server.lock().unwrap().flags = None;
    assert_eq!(
        g.invite(FILE),
        InviteAnswer::Invited {
            invite_id: INVITE.into()
        }
    );
    assert!(!g.routes().iter().any(|r| r == FLAGS_ROUTE));
}

// Rule 7.
#[test]
fn share_invite_cut_off_mid_flow_is_not_sent_and_a_retry_keeps_the_one_folder_key() {
    let f = fixture(None);
    f.server.lock().unwrap().hang_once = Some(INVITES);
    let first = f.invite(FOLDER);
    assert_eq!(first, InviteAnswer::Refused(InviteRefused::Offline));
    // The retry finds the first run's custody record: no second key is made.
    assert_eq!(
        f.invite(FOLDER),
        InviteAnswer::Invited {
            invite_id: INVITE.into()
        }
    );
    let routes = f.routes();
    assert_eq!(routes.iter().filter(|r| *r == CUSTODY_PUT).count(), 1, "{routes:?}");
    let invites = f.bodies(INVITES);
    assert_eq!(invites.len(), 2);
    assert_eq!(
        invites[0]["folder_key_id"], invites[1]["folder_key_id"],
        "one key per folder"
    );

    // A file whose approval gets no answer: the claimed invite, which holds no key, is withdrawn; not sent.
    let g = fixture(Some(person(45).public));
    g.server.lock().unwrap().hang_once = Some(APPROVE);
    assert_eq!(g.invite(FILE), InviteAnswer::Refused(InviteRefused::Failed));
    assert!(g.routes().iter().any(|r| r == CANCEL));
    // An approval the server refused: the same.
    let h = fixture(Some(person(45).public));
    h.server.lock().unwrap().approve_status = 500;
    assert_eq!(h.invite(FILE), InviteAnswer::Refused(InviteRefused::Failed));
    assert!(h.routes().iter().any(|r| r == CANCEL));
    assert_eq!(h.db.share_sealed_key(OWN, INVITE).unwrap(), None);
}

/// Review of desktop#424 (crypto P2-3): the session's turn moves between the invite (answered "claimed") and its
/// approval. Nothing is sealed or approved, and the claimed invite, which holds no key, is withdrawn while the same
/// account is still signed in. Another account: nothing more is sent, and the answer never says nothing was shared
/// while the claimed invite stays.
#[test]
fn share_invite_whose_session_moves_after_the_claim_is_withdrawn_for_the_same_account() {
    let f = fixture(Some(person(47).public));
    let (engine, master) = (f.engine.clone(), f.owner_master);
    f.server.lock().unwrap().on_invite = Some(Box::new(move || signed_in(&engine, master)));
    assert_eq!(f.invite(FILE), InviteAnswer::Refused(InviteRefused::SessionEnded));
    let routes = f.routes();
    assert!(!routes.iter().any(|r| r == APPROVE), "{routes:?}");
    assert!(
        routes.iter().any(|r| r == CANCEL),
        "the claimed invite is withdrawn: {routes:?}"
    );
    assert!(sealed_fields(&f.api).is_empty());

    let g = fixture(Some(person(48).public));
    let engine = g.engine.clone();
    g.server.lock().unwrap().on_invite = Some(Box::new(move || signed_in(&engine, [9; 32])));
    let answer = g.invite(FILE);
    let routes = g.routes();
    assert!(!routes.iter().any(|r| r == APPROVE || r == CANCEL), "{routes:?}");
    assert!(sealed_fields(&g.api).is_empty());
    assert!(
        !matches!(answer, InviteAnswer::Refused(_)),
        "a claimed invite that stays is never answered as nothing shared: {answer:?}"
    );
}

// Rule 8 (engine side; the window's side is the host's test).
#[test]
fn share_invite_for_a_session_that_is_over_sends_nothing() {
    let f = fixture(Some(person(46).public));
    let old = f.engine.session_turn();
    signed_in(&f.engine, f.owner_master);
    assert_eq!(
        f.engine.invite_for(old, FILE, EMAIL),
        InviteAnswer::Refused(InviteRefused::SessionEnded)
    );
    let now = f.engine.session_turn();
    let _ = f.engine.lock();
    assert_eq!(
        f.engine.invite_for(now, FILE, EMAIL),
        InviteAnswer::Refused(InviteRefused::SessionEnded)
    );
    assert!(f.routes().is_empty(), "{:?}", f.routes());
    assert_eq!(f.engine.share_target_of(now, FILE), None);
}

// Rule 9.
#[test]
fn share_invite_answers_and_logs_carry_no_key_material() {
    let them = person(47);
    let f = fixture(Some(them.public));
    // Approvals fail and so do the withdrawals: the module logs its warning, so the capture has a line to read.
    f.server.lock().unwrap().approve_status = 500;
    f.server.lock().unwrap().cancel_status = 500;
    let (answers, logs) = capture_logs(|| {
        vec![
            f.invite(FILE),
            f.invite(FOLDER),
            f.engine.invite_for(f.engine.session_turn(), FILE, "not an address"),
        ]
    });
    assert_eq!(answers[2], InviteAnswer::Refused(InviteRefused::InvalidEmail));
    // The withdrawal failed, so the claimed invite stays: never answered as nothing shared (review of desktop#424).
    assert_eq!(
        answers[0],
        InviteAnswer::Unfinished {
            invite_id: INVITE.into(),
            approval_sent: true,
        },
        "a 500 is not a refusal: the approval may have been applied"
    );
    assert!(
        logs.contains("an invite that holds no key could not be withdrawn"),
        "the capture reads this module's lines"
    );
    let mut secrets = vec![
        b64(&f.owner_master),
        hexfmt::encode(&f.owner_master),
        b64(&f.file_key()),
        hexfmt::encode(&f.file_key()),
        b64(&them.public),
        hexfmt::encode(&them.public),
        "an-invented-session-token".to_string(),
        "person@example.eu".to_string(),
    ];
    secrets.extend(sealed_fields(&f.api));
    let custody = f
        .server
        .lock()
        .unwrap()
        .custody
        .clone()
        .expect("the folder's key was registered");
    secrets.push(custody);
    assert!(
        secrets.len() >= 9,
        "the sealed key and the custody record are in the list"
    );
    let shown = format!(
        "{answers:?} {}",
        answers
            .iter()
            .filter_map(|a| match a {
                InviteAnswer::Refused(r) => Some(r.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    );
    for secret in &secrets {
        assert!(!shown.contains(secret.as_str()), "an answer carries key material");
        assert!(!logs.contains(secret.as_str()), "a log line carries key material");
    }
}

#[test]
fn share_invite_refuses_someone_already_on_it_and_what_is_not_the_accounts() {
    let f = fixture(Some(person(48).public));
    f.server.lock().unwrap().sent = json!({ "invites": [
        { "id": OLD_INVITE, "file_id": FILE, "recipient_email": "PERSON@example.eu ", "status": "claimed" }
    ] });
    assert_eq!(f.invite(FILE), InviteAnswer::Refused(InviteRefused::AlreadyOnIt));
    for item in [
        THEIRS,
        BACKUPS,
        BACKED,
        FOREIGN,
        "a0000000-0000-4000-8000-0000000009ff",
        "not-an-id",
    ] {
        assert_eq!(f.invite(item), InviteAnswer::Refused(InviteRefused::NotYours), "{item}");
    }
    assert!(!f.routes().iter().any(|r| r == INVITES));
}

// Rule 11.
#[test]
fn share_of_names_the_accounts_own_items_only() {
    let f = fixture(None);
    let now = f.engine.session_turn();
    assert_eq!(f.engine.share_target_of(now, FILE), Some(FILE.to_string()));
    assert_eq!(f.engine.share_target_of(now, FOLDER), Some(FOLDER.to_string()));
    for item in [THEIRS, BACKUPS, BACKED, FOREIGN, "a0000000-0000-4000-8000-0000000009ff"] {
        assert_eq!(f.engine.share_target_of(now, item), None, "{item}");
    }
    // Another session's question gets no answer.
    signed_in(&f.engine, f.owner_master);
    assert_eq!(f.engine.share_target_of(now, FILE), None);
    assert!(
        f.routes().is_empty(),
        "local state only: nothing is asked of the server"
    );
}

#[test]
fn share_invite_asks_beebeeb_whose_account_it_is_when_the_session_does_not_say() {
    // A session restored without a recorded owner: the marks are read under the id Beebeeb names for it.
    let them = person(49);
    let f = fixture(Some(them.public));
    f.engine.remember_session_user_id(None);
    let fingerprint = hexfmt::encode(&key_fingerprint(&person(50).public).unwrap());
    f.db.set_person_marks(
        OWN,
        Some("person@example.eu"),
        &[],
        &fingerprint,
        MarkKind::Matched,
        false,
    )
    .unwrap();
    assert_eq!(
        f.invite(FILE),
        InviteAnswer::PeerKeyChanged {
            invite_id: INVITE.into()
        }
    );
    assert!(f.routes().iter().any(|r| r == "GET /api/v1/auth/me"));
    assert!(
        sealed_fields(&f.api).is_empty(),
        "the account's mark was found: nothing sealed"
    );

    // Beebeeb names no account: nothing is sent.
    let g = fixture(Some(them.public));
    g.engine.remember_session_user_id(None);
    g.server.lock().unwrap().me = None;
    assert_eq!(g.invite(FILE), InviteAnswer::Refused(InviteRefused::Failed));
    assert!(!g.routes().iter().any(|r| r == INVITES));
}

/// Review of desktop#426 (P2): an approval that LEFT and got no answer may have been applied, so when the
/// withdrawal fails too the answer says so (`approval_sent`), for a file and for a folder alike.
#[test]
fn share_invite_unfinished_says_whether_the_approval_left_without_an_answer() {
    for item in [FILE, FOLDER] {
        let f = fixture(Some(person(48).public));
        f.server.lock().unwrap().hang_once = Some(APPROVE);
        f.server.lock().unwrap().cancel_status = 500;
        assert_eq!(
            f.invite(item),
            InviteAnswer::Unfinished {
                invite_id: INVITE.into(),
                approval_sent: true,
            },
            "{item}: the approval left, its answer was lost, and the withdrawal failed"
        );
    }
}

/// Review of desktop#427 (P2): only a 4xx proves the approval was refused; a 5xx may have been applied.
#[test]
fn share_invite_unfinished_is_sent_for_a_500_and_not_for_a_4xx() {
    for (status, sent) in [(500u16, true), (503, true), (403, false), (409, false)] {
        for item in [FILE, FOLDER] {
            let f = fixture(Some(person(49).public));
            f.server.lock().unwrap().approve_status = status;
            f.server.lock().unwrap().cancel_status = 500;
            assert_eq!(
                f.invite(item),
                InviteAnswer::Unfinished {
                    invite_id: INVITE.into(),
                    approval_sent: sent,
                },
                "{item}: approve answered {status}"
            );
        }
    }
}

/// Review of desktop#427 (delta 2, P2): an approval that was never made (the key could not be sealed) cannot have
/// landed, whatever the status; a sealed one may have, unless a 4xx proves it was refused.
#[test]
fn an_approval_may_have_landed_only_when_one_was_made_and_no_4xx_refused_it() {
    let waiting = |key: Option<[u8; 32]>| InviteOutcome::AwaitingApproval {
        invite_id: INVITE.into(),
        recipient_public_key: key,
    };
    for status in [0u16, 500, 503] {
        assert!(approval_may_have_landed(&waiting(Some([7; 32])), status), "{status}");
        assert!(
            !approval_may_have_landed(&waiting(None), status),
            "no approval was made: {status}"
        );
    }
    assert!(!approval_may_have_landed(&waiting(Some([7; 32])), 403));
    assert!(!approval_may_have_landed(&waiting(Some([7; 32])), 409));
}
