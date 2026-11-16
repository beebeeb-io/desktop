//! File requests at the engine (`Engine::file_request_create_for`, `Engine::file_request_copy_for`), ported from the
//! Tauri shell's `share_v2/requests.rs` (`file_request_create`, `file_request_copy`), against the fake API on loopback
//! (its file-request routes answered here in Beebeeb's shape) and a state database of each test's own recorded as the
//! signed-in account's. The session is put in memory as a sign-in and an unlock leave it. Every name and key is
//! invented.

use std::sync::{Arc, Mutex};

use beebeeb_core::kdf::MasterKey;
use beebeeb_core::share_flow::request::{CreateRequest, RequestOptions, rebuild_request_link};
use beebeeb_types::backup::SystemRole;
use serde_json::{Value, json};

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace, StateDb};
use crate::test_api::{FakeApi, Handler, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const OWN: &str = "d0000000-0000-4000-8000-000000000801";
const OWN_EMAIL: &str = "owner@example.eu";
const MASTER: [u8; 32] = [7; 32];
const APP: &str = "http://localhost:5173";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000801";
const FILE: &str = "a0000000-0000-4000-8000-000000000802";
const THEIRS: &str = "a0000000-0000-4000-8000-000000000803";
const BACKUPS: &str = "a0000000-0000-4000-8000-000000000804";
const BACKED: &str = "a0000000-0000-4000-8000-000000000805";
const UNKNOWN: &str = "a0000000-0000-4000-8000-000000000806";
const FOREIGN_REQUEST: &str = "e0000000-0000-4000-8000-0000000008ff";

const CREATE: &str = "POST /api/v1/file-requests";
const LIST: &str = "GET /api/v1/file-requests";

/// What the fake holds and answers.
struct Server {
    /// The account's requests, in Beebeeb's listing shape.
    rows: Vec<Value>,
    /// Not 200: every file-request route answers this status.
    status: u16,
    /// Runs once when a new request arrives, before it is answered (a session change while it is out).
    on_create: Option<Box<dyn FnOnce() + Send>>,
    /// Runs once when the list of requests is read, before it is answered.
    on_list: Option<Box<dyn FnOnce() + Send>>,
}

struct Fixture {
    api: FakeApi,
    server: Arc<Mutex<Server>>,
    engine: Arc<TestEngine>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn count(&self, route: &str) -> usize {
        self.api.routes().iter().filter(|seen| *seen == route).count()
    }

    fn sent(&self) -> usize {
        self.count(CREATE) + self.count(LIST)
    }

    fn create(&self, choices: &RequestChoices) -> Result<MadeLink, ShareLinkError> {
        self.engine.file_request_create_for(self.engine.session_turn(), choices)
    }

    fn copy(&self, id: &str) -> Result<MadeLink, RequestCopyError> {
        self.engine.file_request_copy_for(self.engine.session_turn(), id)
    }

    fn status(&self, status: u16) {
        self.server.lock().unwrap().status = status;
    }
}

fn handler(server: Arc<Mutex<Server>>) -> Handler {
    Arc::new(move |req: &Request| {
        let route = req.route();
        let mut s = server.lock().unwrap();
        if route != CREATE && route != LIST {
            return Reply::error(404, "not_found");
        }
        if s.status != 200 {
            return Reply::json(s.status, json!({ "error": "account_frozen" }));
        }
        if route == LIST {
            if let Some(hook) = s.on_list.take() {
                hook();
            }
            return Reply::ok(json!({ "file_requests": s.rows }));
        }
        if let Some(hook) = s.on_create.take() {
            hook();
        }
        let body = req.json();
        let n = s.rows.len() + 1;
        let id = format!("e0000000-0000-4000-8000-{n:012}");
        let token = format!("invented-request-token-{n}");
        s.rows.push(json!({
            "id": id, "title": body["title"], "token": token, "files_received": 0, "closed": false,
            "wrapped_private_key": body["wrapped_private_key"], "wrap_nonce": body["wrap_nonce"],
        }));
        Reply::json(201, json!({ "id": id, "token": token }))
    })
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

/// A session in memory, as a sign-in and an unlock leave it. Called again, it is a new session of the same account.
fn signed_in(engine: &TestEngine) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: "an-invented-session-token".into(),
        master_key: MASTER,
        email: Some(OWN_EMAIL.into()),
    });
    engine.remember_session_user_id(Some(OWN.into()));
    engine.set_auth_present(true);
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, FOLDER, "Invented drops", true, false);
    row(&db, FILE, "Invented plan.pdf", false, false);
    row(&db, THEIRS, "Shared/Invented theirs", true, true);
    row(&db, BACKUPS, "Backups", true, false);
    db.set_system_role(BACKUPS, Some(SystemRole::BackupsRoot)).unwrap();
    row(&db, BACKED, "Backups/Invented PC", true, false);
    db.set_owner(&Identity::new(Some(OWN), Some(OWN_EMAIL))).unwrap();
    let server = Arc::new(Mutex::new(Server {
        rows: Vec::new(),
        status: 200,
        on_create: None,
        on_list: None,
    }));
    let api = FakeApi::start(handler(server.clone()));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-request-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"));
    engine.use_local_state_for_test(path);
    signed_in(&engine);
    Fixture {
        api,
        server,
        engine,
        _dir: dir,
    }
}

fn choices(title: &str) -> RequestChoices {
    RequestChoices {
        title: title.into(),
        ..RequestChoices::default()
    }
}

fn text<'a>(v: &'a Value, field: &str) -> &'a str {
    v.get(field).and_then(Value::as_str).unwrap_or_default()
}

#[test]
fn a_new_request_is_one_post_and_its_link_carries_the_key_after_the_hash_only() {
    let f = fixture();
    let turn = f.engine.session_turn();
    let asked = RequestChoices {
        title: "  Invented drop  ".into(),
        note: Some("Invented note".into()),
        folder_id: Some(FOLDER.into()),
        max_files: Some(20),
        max_bytes: Some(1_000_000),
        expires_in_days: Some(7),
    };
    let made = f.engine.file_request_create_for(turn, &asked).expect("a request");
    assert_eq!(f.api.routes(), [CREATE], "one request, nothing else");
    assert_eq!(made.session, turn);

    let posted = f.api.requests()[0].json();
    assert_eq!(text(&posted, "title"), "Invented drop");
    assert_eq!(text(&posted, "description"), "Invented note");
    assert_eq!(text(&posted, "target_folder_id"), FOLDER);
    assert_eq!(posted["max_files"], json!(20));
    assert_eq!(posted["max_total_bytes"], json!(1_000_000));
    assert_eq!(posted["expires_in_secs"], json!(7 * 86_400));

    // The link: the web app's request page for the token Beebeeb answered, the key after `#` and nowhere else.
    let row = f.server.lock().unwrap().rows[0].clone();
    let token = text(&row, "token").to_string();
    let link = made.expose().to_string();
    let (page, key) = link.split_once('#').expect("a key part after #");
    assert_eq!(page, format!("{APP}/r/{token}"));
    assert!(!key.is_empty());
    assert!(!link.contains('?'), "nothing of the key in a query");
    // It is the very link core rebuilds from what Beebeeb stored, under this account's vault key.
    let rebuilt = rebuild_request_link(
        APP,
        &MasterKey::from_bytes(MASTER),
        Some(&token),
        Some(text(&row, "wrapped_private_key")),
        Some(text(&row, "wrap_nonce")),
    )
    .expect("core rebuilds it");
    assert_eq!(rebuilt.as_str(), link, "core's link, as core made it");
    // The key part never went to Beebeeb, in a path or a body.
    for sent in f.api.requests() {
        assert!(!sent.path.contains(key), "the key in a path");
        assert!(!String::from_utf8_lossy(&sent.body).contains(key), "the key in a body");
    }
    // What may be drawn or printed holds nothing of the key part or the token.
    assert_eq!(made.display(), page);
    let printed = format!("{made:?}");
    assert!(!printed.contains(key) && !printed.contains(&token), "{printed}");
}

#[test]
fn copying_a_request_rebuilds_the_same_link_from_what_beebeeb_stored() {
    let f = fixture();
    let made = f.create(&choices("Invented drop")).expect("a request");
    let id = text(&f.server.lock().unwrap().rows[0], "id").to_string();
    let again = f.copy(&id).expect("the same request's link");
    assert_eq!(again.expose(), made.expose(), "the same link");
    assert_eq!(f.api.routes(), [CREATE, LIST], "the copy reads, and makes nothing");

    // A request Beebeeb does not list for this account, or lists as taken off the list: not found.
    assert_eq!(f.copy(UNKNOWN).unwrap_err(), ShareLinkError::NotFound);
    f.server.lock().unwrap().rows[0]["hidden"] = json!(true);
    assert_eq!(f.copy(&id).unwrap_err(), ShareLinkError::NotFound);
    f.server.lock().unwrap().rows[0]["hidden"] = json!(false);

    // A stored key that is gone, or one that does not open under this vault key: never handed out.
    let stored = f.server.lock().unwrap().rows[0].clone();
    f.server.lock().unwrap().rows[0]["wrapped_private_key"] = json!("");
    assert_eq!(f.copy(&id).unwrap_err(), ShareLinkError::KeyUnavailable);
    f.server.lock().unwrap().rows[0] = stored;
    let other = RequestOptions {
        title: "Another account's".into(),
        ..RequestOptions::default()
    };
    let (_, foreign) = CreateRequest::start(APP, &other, &MasterKey::from_bytes([9; 32])).unwrap();
    let foreign = foreign.body.unwrap();
    f.server.lock().unwrap().rows.push(json!({
        "id": FOREIGN_REQUEST, "title": "x", "token": "invented-foreign-token",
        "wrapped_private_key": foreign["wrapped_private_key"], "wrap_nonce": foreign["wrap_nonce"],
    }));
    assert_eq!(f.copy(FOREIGN_REQUEST).unwrap_err(), ShareLinkError::KeyUnavailable);
    assert_eq!(f.copy(&id).expect("its own still opens").expose(), made.expose());

    // An id that is not a canonical id is refused before anything is sent.
    let before = f.sent();
    assert_eq!(f.copy(&id.to_uppercase()).unwrap_err(), ShareLinkError::NotFound);
    assert_eq!(f.sent(), before);
}

#[test]
fn a_refused_choice_or_a_press_of_another_session_sends_nothing() {
    let f = fixture();
    assert_eq!(f.create(&choices("   ")).unwrap_err(), ShareLinkError::Invalid);
    let into = |folder: &str| RequestChoices {
        folder_id: Some(folder.into()),
        ..choices("Invented drop")
    };
    // Delivered only into one of the account's own folders.
    assert_eq!(f.create(&into(FILE)).unwrap_err(), ShareLinkError::Invalid, "a file");
    assert_eq!(f.create(&into(UNKNOWN)).unwrap_err(), ShareLinkError::NotFound);
    assert_eq!(
        f.create(&into(THEIRS)).unwrap_err(),
        ShareLinkError::NotFound,
        "a received share"
    );
    assert_eq!(
        f.create(&into(BACKED)).unwrap_err(),
        ShareLinkError::NotFound,
        "the Backups tree"
    );
    assert_eq!(f.create(&into("../x")).unwrap_err(), ShareLinkError::NotFound);
    assert_eq!(f.sent(), 0, "nothing was sent");

    // A press made under a session that is gone: nothing is sent.
    let turn = f.engine.session_turn();
    signed_in(&f.engine);
    assert_eq!(
        f.engine
            .file_request_create_for(turn, &choices("Invented drop"))
            .unwrap_err(),
        ShareLinkError::NotSignedIn
    );
    assert_eq!(
        f.engine.file_request_copy_for(turn, FOREIGN_REQUEST).unwrap_err(),
        ShareLinkError::NotSignedIn
    );
    assert_eq!(f.sent(), 0, "nothing was sent");
    // The top level (no folder) under the session in memory: one request.
    f.create(&choices("Invented drop")).expect("a request");
    assert_eq!(f.sent(), 1);
    assert_eq!(f.api.requests()[0].json().get("target_folder_id"), None);
}

#[test]
fn a_403_is_beebeeb_refusing_never_a_session_that_ended() {
    let f = fixture();
    let mut checked = 0;
    for (status, want) in [
        (403, ShareLinkError::Refused),
        (401, ShareLinkError::SessionEnded),
        (500, ShareLinkError::Refused),
    ] {
        f.status(status);
        assert_eq!(
            f.create(&choices("Invented drop")).unwrap_err(),
            want,
            "create, {status}"
        );
        assert_eq!(f.copy(FOREIGN_REQUEST).unwrap_err(), want, "copy, {status}");
        checked += 2;
    }
    assert_eq!(checked, 6);
    assert_eq!(f.sent(), 6, "each asked once");
}

#[test]
fn a_request_made_for_a_session_that_ended_meanwhile_is_not_handed_over() {
    let f = fixture();
    let turn = f.engine.session_turn();
    let engine = f.engine.clone();
    f.server.lock().unwrap().on_create = Some(Box::new(move || signed_in(&engine)));
    assert_eq!(
        f.engine
            .file_request_create_for(turn, &choices("Invented drop"))
            .unwrap_err(),
        ShareLinkError::NotSignedIn
    );
    assert_eq!(f.count(CREATE), 1, "it was sent before the session moved");
}

#[test]
fn a_copied_link_read_for_a_session_that_ended_meanwhile_is_not_handed_over() {
    let f = fixture();
    f.create(&choices("Invented drop")).expect("a request");
    let id = text(&f.server.lock().unwrap().rows[0], "id").to_string();
    let turn = f.engine.session_turn();
    let engine = f.engine.clone();
    f.server.lock().unwrap().on_list = Some(Box::new(move || signed_in(&engine)));
    assert_eq!(
        f.engine.file_request_copy_for(turn, &id).unwrap_err(),
        ShareLinkError::NotSignedIn
    );
    assert_eq!(f.count(LIST), 1, "it was read before the session moved");
}

/// The session moves at the top of the press's next request, before that request's own session check.
fn before_next_send(engine: &Arc<TestEngine>) {
    let engine = engine.clone();
    crate::app::bindings::link_session::tests::BEFORE_SEND
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move || signed_in(&engine))));
}

#[test]
fn a_press_of_another_session_is_refused_before_any_of_its_choices_is_looked_at() {
    // The run's session check (`link_session.rs`, `Engine::link_run`): a press of a session that is gone is refused
    // as not signed in, never told "not found" about a folder it chose.
    let f = fixture();
    let turn = f.engine.session_turn();
    signed_in(&f.engine);
    let into_nothing = RequestChoices {
        folder_id: Some("../x".into()),
        ..choices("Invented drop")
    };
    assert_eq!(
        f.engine.file_request_create_for(turn, &into_nothing).unwrap_err(),
        ShareLinkError::NotSignedIn
    );
    assert_eq!(f.sent(), 0, "nothing was sent");
}

#[test]
fn a_session_that_moves_before_the_request_leaves_sends_nothing() {
    // The send's session check (`link_session.rs`, `Engine::link_send`): the session moves after the press's run
    // and before its request; the request never leaves.
    let f = fixture();
    before_next_send(&f.engine);
    assert_eq!(
        f.create(&choices("Invented drop")).unwrap_err(),
        ShareLinkError::NotSignedIn
    );
    assert_eq!(f.count(CREATE), 0, "the request never left");
    before_next_send(&f.engine);
    assert_eq!(f.copy(FOREIGN_REQUEST).unwrap_err(), ShareLinkError::NotSignedIn);
    assert_eq!(f.count(LIST), 0, "the list was never read");
}

#[test]
fn a_closed_requests_link_is_refused_with_words_and_nothing_is_copied() {
    let f = fixture();
    f.create(&choices("Invented drop")).expect("a request");
    let id = text(&f.server.lock().unwrap().rows[0], "id").to_string();
    f.server.lock().unwrap().rows[0]["closed"] = json!(true);
    let refused = f.copy(&id).unwrap_err();
    assert_eq!(refused, RequestCopyError::Closed);
    assert_eq!(refused.words(), "This request is closed");
    // Closed is said first, as the Shared list draws it, even when its expiry has passed too.
    f.server.lock().unwrap().rows[0]["expires_at"] = json!("2020-01-02T03:04:05Z");
    assert_eq!(f.copy(&id).unwrap_err(), RequestCopyError::Closed);
    assert_eq!(f.count(CREATE), 1, "nothing new was made");
}

#[test]
fn an_expired_requests_link_is_refused_with_words_and_nothing_is_copied() {
    let f = fixture();
    let made = f.create(&choices("Invented drop")).expect("a request");
    let id = text(&f.server.lock().unwrap().rows[0], "id").to_string();
    f.server.lock().unwrap().rows[0]["expires_at"] = json!("2020-01-02T03:04:05Z");
    let refused = f.copy(&id).unwrap_err();
    assert_eq!(refused, RequestCopyError::Expired);
    assert_eq!(refused.words(), "This request has expired");
    // An expiry still ahead: the link is copied as before.
    f.server.lock().unwrap().rows[0]["expires_at"] = json!("2999-01-02T03:04:05Z");
    assert_eq!(f.copy(&id).expect("an open request's link").expose(), made.expose());
    assert_eq!(f.count(CREATE), 1, "nothing new was made");
}
