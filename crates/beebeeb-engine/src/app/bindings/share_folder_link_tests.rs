//! Folder links, bundles and "Stop sharing" a folder's links at the engine (`Engine::folder_link_for`,
//! `Engine::bundle_link_for`, `Engine::folder_links_revoke_for`), ported from the Tauri shell's
//! `share_v2/folder_flows.rs`; and the live-link rule of the file link (`Engine::share_link_for`), ported from
//! `share_v2/tests.rs::a_live_link_is_rebuilt_only_with_a_stored_wrapped_key_that_opens`. Against the fake API on
//! loopback (the custody, listing and link routes answered here in Beebeeb's shape) and a state database of each
//! test's own recorded as the signed-in account's. The session is put in memory as a sign-in and an unlock leave it.
//! Every name and key is invented.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use beebeeb_core::kdf::MasterKey;
use beebeeb_core::share_flow::folder::rebuild_folder_link;
use beebeeb_core::share_flow::link::open_link_file_key;
use beebeeb_types::backup::SystemRole;
use serde_json::{Value, json};

use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, FileStatus, ItemKind, Namespace};
use crate::test_api::{FakeApi, Handler, Reply, Request};

thread_local! {
    /// Test seam: the next custody writes on this thread fail (`remember_custody`).
    pub(super) static CUSTODY_WRITE_FAILS: Cell<bool> = const { Cell::new(false) };
}

type TestEngine = Engine<MemoryCredentials>;

const OWN: &str = "d0000000-0000-4000-8000-000000000901";
const OWN_EMAIL: &str = "owner@example.eu";
const MASTER: [u8; 32] = [7; 32];
const APP: &str = "http://localhost:5173";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000901";
const CHILD: &str = "a0000000-0000-4000-8000-000000000902";
const OTHER_FOLDER: &str = "a0000000-0000-4000-8000-000000000903";
const NOTE: &str = "a0000000-0000-4000-8000-000000000904";
const PLAN: &str = "a0000000-0000-4000-8000-000000000905";
const THEIRS: &str = "a0000000-0000-4000-8000-000000000906";
const BACKUPS: &str = "a0000000-0000-4000-8000-000000000907";
const BACKED: &str = "a0000000-0000-4000-8000-000000000908";
/// A foreign id (version nibble 8) with no owned-key record: no key on this computer.
const NO_KEY_HERE: &str = "c0000000-0000-8000-8000-000000000909";
const UNKNOWN: &str = "a0000000-0000-4000-8000-00000000090a";

const LINKS: &str = "GET /api/v1/shares/folder-links";
const MAKE_FOLDER_LINK: &str = "POST /api/v1/shares/folder-links";
const REVOKE: &str = "DELETE /api/v1/shares/folder-links/";
const CUSTODY: &str = "GET /api/v1/shared-folders/";
const MAKE_LINK: &str = "POST /api/v1/shares";
const MINE: &str = "GET /api/v1/shares/mine";

/// What the fake holds and answers, and what it saw.
struct Server {
    /// The folders' custody records, as the last PUT stored them.
    custody: Vec<(String, String)>,
    /// The account's folder links, in Beebeeb's listing shape (revoked ones stay, with `revoked_at` set).
    folder_links: Vec<Value>,
    /// The account's file and bundle links, in the shape of its link list.
    links: Vec<Value>,
    /// The folder-link listing answers every folder's links, whatever was asked.
    list_every_folder: bool,
    /// Not 200: a new link (folder, file or bundle) is answered with this status.
    make_status: u16,
    /// Not 200: the folder-link listing answers this status.
    list_status: u16,
    /// Not 200: a revoke answers this status.
    revoke_status: u16,
    /// Runs once when the first request of this route arrives, before it is answered.
    on_route: Option<(&'static str, Box<dyn FnOnce() + Send>)>,
    /// Every request: its route, and whether the device remembered FOLDER's custody when it arrived.
    seen: Vec<(String, bool)>,
}

struct Fixture {
    api: FakeApi,
    server: Arc<Mutex<Server>>,
    engine: Arc<TestEngine>,
    db: Arc<StateDb>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn count(&self, route: &str) -> usize {
        self.api.routes().iter().filter(|seen| seen.starts_with(route)).count()
    }

    fn sent(&self) -> usize {
        self.api.routes().len()
    }

    fn folder_link(&self, id: &str, choices: Option<&LinkChoices>) -> Result<MadeLink, FolderLinkError> {
        self.engine.folder_link_for(self.engine.session_turn(), id, choices)
    }

    fn bundle(&self, ids: &[&str]) -> Result<MadeLink, FolderLinkError> {
        let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
        self.engine.bundle_link_for(self.engine.session_turn(), &ids, None)
    }

    fn revoke(&self, id: &str) -> Result<(), FolderLinkError> {
        self.engine.folder_links_revoke_for(self.engine.session_turn(), id)
    }

    fn with<T>(&self, change: impl FnOnce(&mut Server) -> T) -> T {
        change(&mut self.server.lock().unwrap())
    }

    /// The live folder link row of `folder` Beebeeb holds (the first one made).
    fn folder_row(&self, folder: &str) -> usize {
        self.with(|s| {
            s.folder_links
                .iter()
                .position(|row| row["folder_id"] == folder && row["revoked_at"].is_null())
                .expect("a live folder link")
        })
    }

    fn link_row(&self, file: &str) -> usize {
        self.with(|s| s.links.iter().position(|row| row["file_id"] == file).expect("a link"))
    }
}

fn custody_of(s: &Server, folder: &str) -> Option<String> {
    s.custody
        .iter()
        .find(|(id, _)| id == folder)
        .map(|(_, record)| record.clone())
}

fn handler(server: Arc<Mutex<Server>>, probe: Arc<StateDb>) -> Handler {
    Arc::new(move |req: &Request| {
        let route = req.route();
        let mut s = server.lock().unwrap();
        let remembered = probe.has_seen_folder_custody(FOLDER).unwrap();
        s.seen.push((route.clone(), remembered));
        if s.on_route.as_ref().is_some_and(|(at, _)| route.starts_with(at)) {
            let (_, hook) = s.on_route.take().unwrap();
            hook();
        }
        let refused = |status: u16| match status {
            401 => Reply::json(401, json!({ "error": "unauthorized" })),
            403 => Reply::json(403, json!({ "error": "account_frozen" })),
            other => Reply::json(other, json!({ "error": "a server body" })),
        };
        let path = req.path.split('?').next().unwrap_or_default().to_string();
        match req.method.as_str() {
            "GET" if route == LINKS => {
                if s.list_status != 200 {
                    return refused(s.list_status);
                }
                let asked = req.query("folder_id").unwrap_or_default().to_string();
                let every = s.list_every_folder;
                let links: Vec<Value> = s
                    .folder_links
                    .iter()
                    .filter(|row| row["revoked_at"].is_null() && (every || row["folder_id"] == asked.as_str()))
                    .cloned()
                    .collect();
                Reply::ok(json!({ "links": links }))
            }
            "POST" if route == MAKE_FOLDER_LINK => {
                if s.make_status != 200 {
                    return refused(s.make_status);
                }
                let body = req.json();
                let expires_at = body["expires_in_hours"]
                    .as_i64()
                    .map(|h| (chrono::Utc::now() + chrono::Duration::hours(h)).to_rfc3339());
                s.folder_links.push(json!({
                    "id": body["id"], "folder_id": body["folder_id"], "access_level": body["access_level"],
                    "expires_at": expires_at, "max_opens": body["max_opens"], "open_count": 0,
                    "passphrase_protected": body.get("passphrase").is_some(), "revoked_at": null,
                    "wrapped_folder_key": body["wrapped_folder_key"], "owner_wrapped_key": body["owner_wrapped_key"],
                    "owner_wrapped_token": body["owner_wrapped_token"],
                }));
                Reply::json(
                    201,
                    json!({ "id": body["id"], "folder_id": body["folder_id"], "key_version": 1 }),
                )
            }
            "DELETE" if route.starts_with(REVOKE) => {
                if s.revoke_status != 200 {
                    return refused(s.revoke_status);
                }
                let id = path.rsplit('/').next().unwrap_or_default().to_string();
                match s
                    .folder_links
                    .iter_mut()
                    .find(|row| row["id"] == id.as_str() && row["revoked_at"].is_null())
                {
                    Some(row) => {
                        row["revoked_at"] = json!("2026-11-15T12:00:00Z");
                        row["wrapped_folder_key"] = Value::Null;
                        row["owner_wrapped_key"] = Value::Null;
                        Reply::ok(json!({}))
                    }
                    None => Reply::error(404, "not_found"),
                }
            }
            "GET" if route.starts_with(CUSTODY) => match custody_of(&s, path.rsplit('/').next().unwrap_or_default()) {
                Some(record) => Reply::ok(json!({ "owner_wrapped_folder_key": record, "key_version": 1 })),
                None => Reply::error(404, "not_found"),
            },
            "PUT" if path.starts_with("/api/v1/shared-folders/") && path.ends_with("/key") => {
                let folder = path
                    .trim_end_matches("/key")
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let record = text(&req.json(), "owner_wrapped_folder_key").to_string();
                s.custody.push((folder, record));
                Reply::ok(json!({}))
            }
            "GET" if route == "GET /api/v1/shares/invites/sent" => Reply::ok(json!({ "invites": [] })),
            "GET" if route == "GET /api/v1/files" && req.query("parent_id") == Some(FOLDER) => {
                Reply::ok(json!({ "files": [{ "id": CHILD, "is_folder": false }] }))
            }
            "GET" if route == "GET /api/v1/files" => Reply::ok(json!({ "files": [] })),
            "POST" if route == MAKE_LINK => {
                if s.make_status != 200 {
                    return refused(s.make_status);
                }
                let mut body = req.json();
                let id = format!("f0000000-0000-4000-8000-{:012}", s.links.len() + 1);
                body["id"] = json!(id);
                if body.get("share_type").is_none() {
                    body["share_type"] = json!("file");
                }
                s.links.push(body);
                Reply::json(201, json!({ "id": id }))
            }
            "GET" if route == MINE => Reply::ok(json!({ "shares": s.links })),
            _ => Reply::error(404, "not_found"),
        }
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
    let db = Arc::new(StateDb::open(&path).unwrap());
    row(&db, FOLDER, "Trip", true, false);
    row(&db, CHILD, "Trip/Invented note.txt", false, false);
    row(&db, OTHER_FOLDER, "Plans", true, false);
    row(&db, NOTE, "Invented note.txt", false, false);
    row(&db, PLAN, "Invented plan.txt", false, false);
    row(&db, THEIRS, "Shared/Invented theirs", true, true);
    row(&db, BACKUPS, "Backups", true, false);
    db.set_system_role(BACKUPS, Some(SystemRole::BackupsRoot)).unwrap();
    row(&db, BACKED, "Backups/Invented PC", true, false);
    row(&db, NO_KEY_HERE, "Invented adopted.txt", false, false);
    db.set_owner(&Identity::new(Some(OWN), Some(OWN_EMAIL))).unwrap();
    let server = Arc::new(Mutex::new(Server {
        custody: Vec::new(),
        folder_links: Vec::new(),
        links: Vec::new(),
        list_every_folder: false,
        make_status: 200,
        list_status: 200,
        revoke_status: 200,
        on_route: None,
        seen: Vec::new(),
    }));
    let probe = Arc::new(StateDb::open(&path).unwrap());
    let api = FakeApi::start(handler(server.clone(), probe));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-folder-link-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"));
    engine.use_local_state_for_test(path);
    signed_in(&engine);
    Fixture {
        api,
        server,
        engine,
        db,
        _dir: dir,
    }
}

fn text<'a>(v: &'a Value, field: &str) -> &'a str {
    v.get(field).and_then(Value::as_str).unwrap_or_default()
}

/// The part of a link after `#key=`.
fn key_part(link: &str) -> &str {
    link.split_once("#key=").expect("a link with its key part").1
}

fn expiring(hours: u32) -> LinkChoices {
    LinkChoices {
        expires_in_hours: Some(hours),
        passphrase: None,
    }
}

#[test]
fn a_new_folder_link_is_cores_and_its_custody_is_remembered_before_the_request_that_uses_it() {
    let f = fixture();
    let turn = f.engine.session_turn();
    let made = f.engine.folder_link_for(turn, FOLDER, None).expect("a folder link");
    assert_eq!(made.session, turn);
    assert_eq!(f.count(MAKE_FOLDER_LINK), 1, "one link");
    assert_eq!(
        f.api.routes()[0],
        LINKS,
        "a plain Copy link reads the folder's links first"
    );

    // L7d: the folder was not remembered when its custody was first read, and was when the link was asked for.
    let seen = f.with(|s| s.seen.clone());
    let at = |route: &str| seen.iter().find(|(r, _)| r.starts_with(route)).expect(route).1;
    assert!(!at(CUSTODY), "nothing remembered before the custody read");
    assert!(
        at("GET /api/v1/files"),
        "remembered before the listing, the request the mark rides on"
    );
    assert!(
        at(MAKE_FOLDER_LINK),
        "remembered before the link that uses the folder's key"
    );
    assert!(f.db.has_seen_folder_custody(FOLDER).unwrap());

    let posted = f.with(|s| s.folder_links[0].clone());
    assert_eq!(text(&posted, "folder_id"), FOLDER);
    assert_eq!(text(&posted, "access_level"), "view");
    assert_eq!(posted["passphrase_protected"], json!(false), "it asks for no password");
    assert!(posted["expires_at"].is_null(), "it never expires");

    // The link: the web app's folder page, the key after `#key=`; core rebuilds the very same link from what Beebeeb
    // stored, and the stored folder key opens under its key (core's check).
    let link = made.expose().to_string();
    let rebuilt = rebuild_folder_link(
        APP,
        &MasterKey::from_bytes(MASTER),
        text(&posted, "id"),
        Some(text(&posted, "owner_wrapped_key")),
        Some(text(&posted, "owner_wrapped_token")),
        Some(text(&posted, "wrapped_folder_key")),
        false,
    )
    .expect("core rebuilds it");
    assert_eq!(rebuilt.as_str(), link);
    assert!(link.starts_with(&format!("{APP}/f/")), "the folder route");
    let key = key_part(&link).to_string();
    for sent in f.api.requests() {
        assert!(!sent.path.contains(&key), "the key in a path");
        assert!(!String::from_utf8_lossy(&sent.body).contains(&key), "the key in a body");
    }
    assert_eq!(made.display(), link.split('#').next().unwrap());
    assert!(!format!("{made:?}").contains(&key));
}

#[test]
fn a_custody_write_that_fails_sends_nothing_that_uses_it() {
    let f = fixture();
    CUSTODY_WRITE_FAILS.with(|fails| fails.set(true));
    let answer = f.folder_link(FOLDER, None);
    CUSTODY_WRITE_FAILS.with(|fails| fails.set(false));
    assert_eq!(answer.unwrap_err(), FolderLinkError::Refused);
    assert_eq!(
        f.count("GET /api/v1/files"),
        0,
        "the request that carried the mark was not sent"
    );
    assert_eq!(f.count(MAKE_FOLDER_LINK), 0);
    // The next press reads the record Beebeeb now holds and makes the link.
    f.folder_link(FOLDER, None).expect("a folder link");
    assert_eq!(f.count(MAKE_FOLDER_LINK), 1);
}

#[test]
fn a_plain_copy_link_hands_out_the_folders_live_link_and_never_makes_a_second() {
    let f = fixture();
    let first = f.folder_link(FOLDER, None).expect("the first link");
    let again = f.folder_link(FOLDER, None).expect("the live link again");
    assert_eq!(again.expose(), first.expose(), "the same link");
    assert_eq!(f.count(MAKE_FOLDER_LINK), 1, "none was made");

    // A real choice asks for a new link, a live one or not.
    let chosen = f.folder_link(FOLDER, Some(&expiring(24))).expect("a link that expires");
    assert_ne!(chosen.expose(), first.expose());
    assert_eq!(f.count(MAKE_FOLDER_LINK), 2);
    assert!(
        !f.with(|s| s.folder_links[1]["expires_at"].is_null()),
        "the expiry was sent"
    );

    // Stop sharing: every live link of the folder is revoked, one request each.
    f.revoke(FOLDER).expect("stopped");
    assert_eq!(f.count(REVOKE), 2);
    // None is live now: the next plain Copy link makes a new one.
    let new = f.folder_link(FOLDER, None).expect("a new link");
    assert_ne!(new.expose(), first.expose());
    assert_eq!(f.count(MAKE_FOLDER_LINK), 3);

    // A link already gone counts as done; a revoke Beebeeb fails is not.
    f.with(|s| s.revoke_status = 404);
    f.revoke(FOLDER).expect("already gone is done");
    f.with(|s| s.revoke_status = 500);
    assert_eq!(f.revoke(FOLDER).unwrap_err(), FolderLinkError::Refused);
}

#[test]
fn a_live_folder_link_is_handed_out_only_when_core_could_check_it() {
    let f = fixture();
    let first = f.folder_link(FOLDER, None).expect("the folder's link");
    let others = f.folder_link(OTHER_FOLDER, None).expect("another folder's link");
    assert_eq!(f.count(MAKE_FOLDER_LINK), 2);
    let at = f.folder_row(FOLDER);
    let other_at = f.folder_row(OTHER_FOLDER);
    let other_key = f.with(|s| s.folder_links[other_at]["wrapped_folder_key"].clone());
    let stored = f.with(|s| s.folder_links[at].clone());

    // A stored folder key that is empty, gone, or another link's (it does not open under this link's key); a
    // password on it (core can check it only with the password); no owner copy: never handed out, and no second
    // link is made behind it.
    for (field, value) in [
        ("wrapped_folder_key", json!("")),
        ("wrapped_folder_key", Value::Null),
        ("wrapped_folder_key", other_key),
        ("passphrase_protected", json!(true)),
        ("owner_wrapped_key", Value::Null),
    ] {
        f.with(|s| s.folder_links[at][field] = value.clone());
        assert_eq!(
            f.folder_link(FOLDER, None).unwrap_err(),
            FolderLinkError::HasLink,
            "{field} = {value}"
        );
        f.with(|s| s.folder_links[at] = stored.clone());
    }
    assert_eq!(f.count(MAKE_FOLDER_LINK), 2, "no second link");
    let again = f.folder_link(FOLDER, None).expect("restored, it is handed out");
    assert_eq!(again.expose(), first.expose());

    // A row that names another folder is not this folder's link, whatever the listing answers.
    f.with(|s| {
        s.folder_links[at]["revoked_at"] = json!("2026-11-15T12:00:00Z");
        s.list_every_folder = true;
    });
    let own = f.folder_link(FOLDER, None).expect("this folder's own new link");
    assert_ne!(own.expose(), others.expose(), "never the other folder's link");
    assert_eq!(f.count(MAKE_FOLDER_LINK), 3);

    // Past its expiry, or its opens used up: not live, so a plain Copy link makes a new one.
    let now_at = f.folder_row(FOLDER);
    f.with(|s| s.folder_links[now_at]["expires_at"] = json!("2026-01-01T00:00:00Z"));
    f.folder_link(FOLDER, None).expect("a new link");
    assert_eq!(f.count(MAKE_FOLDER_LINK), 4);
    f.with(|s| {
        for row in s.folder_links.iter_mut().filter(|row| row["folder_id"] == FOLDER) {
            row["max_opens"] = json!(3);
            row["open_count"] = json!(3);
        }
    });
    f.folder_link(FOLDER, None).expect("a new link");
    assert_eq!(f.count(MAKE_FOLDER_LINK), 5);
}

#[test]
fn a_live_file_link_is_rebuilt_only_with_a_stored_wrapped_key_that_opens() {
    let f = fixture();
    let turn = f.engine.session_turn();
    let first = f.engine.share_link_for(turn, NOTE, None).expect("the file's link");
    f.engine.share_link_for(turn, PLAN, None).expect("another file's link");
    assert_eq!(f.count(MAKE_LINK), 2);
    let at = f.link_row(NOTE);
    let plans = f.link_row(PLAN);
    let stored = f.with(|s| s.links[at].clone());
    let foreign = f.with(|s| s.links[plans]["wrapped_file_key"].clone());

    // With the stored wrapped key that opens: the very same link, and none made.
    let again = f.engine.share_link_for(turn, NOTE, None).expect("the live link again");
    assert_eq!(again.expose(), first.expose());
    // Without it, empty, or another link's (it does not open under this link's key): nothing is handed out.
    for value in [Value::Null, json!(""), foreign] {
        f.with(|s| s.links[at]["wrapped_file_key"] = value.clone());
        assert_eq!(
            f.engine.share_link_for(turn, NOTE, None).unwrap_err(),
            ShareLinkError::HasLink,
            "wrapped_file_key = {value}"
        );
    }
    f.with(|s| s.links[at] = stored);
    assert_eq!(f.count(MAKE_LINK), 2, "no second link");
}

#[test]
fn a_bundle_is_one_link_over_each_files_own_key_and_a_refused_set_sends_nothing() {
    let f = fixture();
    let made = f.bundle(&[NOTE, PLAN]).expect("a bundle link");
    assert_eq!(f.count(MAKE_LINK), 1);
    let posted = f.with(|s| s.links[0].clone());
    assert_eq!(text(&posted, "share_type"), "bundle");
    let items = posted["items"].as_array().expect("the items").clone();
    assert_eq!(items.len(), 2);
    let link = made.expose().to_string();
    assert_eq!(made.display(), format!("{APP}/s/{}", text(&posted, "token")));
    let master = MasterKey::from_bytes(MASTER);
    for (position, (item, id)) in items.iter().zip([NOTE, PLAN]).enumerate() {
        assert_eq!(text(item, "file_id"), id);
        assert_eq!(item["position"], json!(position));
        let opened = open_link_file_key(key_part(&link), text(item, "wrapped_file_key")).expect("it opens");
        let own = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
        assert_eq!(opened.as_bytes(), own.as_bytes(), "each file's own key");
    }
    assert!(!String::from_utf8_lossy(&f.api.requests()[0].body).contains(key_part(&link)));

    let many: Vec<String> = (0..=BUNDLE_MAX_ITEMS)
        .map(|n| format!("a0000000-0000-4000-8000-{n:012}"))
        .collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    let mut checked = 0;
    for (ids, want) in [
        (vec![], FolderLinkError::Invalid),
        (vec![NOTE, NOTE], FolderLinkError::Invalid),
        (vec![NOTE, FOLDER], FolderLinkError::NotAFile),
        (vec![NOTE, "../x"], FolderLinkError::NotFound),
        (vec![NOTE, UNKNOWN], FolderLinkError::NotFound),
        (vec![NOTE, NO_KEY_HERE], FolderLinkError::KeyUnavailable),
        (many, FolderLinkError::TooLarge),
    ] {
        assert_eq!(f.bundle(&ids).unwrap_err(), want, "{} items", ids.len());
        checked += 1;
    }
    assert_eq!(checked, 7);
    let blank = LinkChoices {
        expires_in_hours: None,
        passphrase: Some(zeroize::Zeroizing::new("   ".into())),
    };
    let ids = [NOTE.to_string(), PLAN.to_string()];
    assert_eq!(
        f.engine
            .bundle_link_for(f.engine.session_turn(), &ids, Some(&blank))
            .unwrap_err(),
        FolderLinkError::Invalid
    );
    assert_eq!(f.sent(), 1, "nothing more was sent");
}

#[test]
fn a_folder_link_is_made_for_a_folder_of_the_accounts_own_and_nothing_else_is_sent() {
    let f = fixture();
    let mut checked = 0;
    for (id, want) in [
        (NOTE, FolderLinkError::NotAFolder),
        (THEIRS, FolderLinkError::NotFound),
        (BACKED, FolderLinkError::NotFound),
        (UNKNOWN, FolderLinkError::NotFound),
        ("../x", FolderLinkError::NotFound),
    ] {
        assert_eq!(f.folder_link(id, None).unwrap_err(), want, "{id}");
        checked += 1;
    }
    assert_eq!(checked, 5);
    let blank = LinkChoices {
        expires_in_hours: None,
        passphrase: Some(zeroize::Zeroizing::new("   ".into())),
    };
    assert_eq!(
        f.folder_link(FOLDER, Some(&blank)).unwrap_err(),
        FolderLinkError::Invalid
    );
    assert_eq!(f.sent(), 0, "nothing was sent");
}

#[test]
fn a_press_under_one_session_is_never_sent_under_another() {
    let f = fixture();
    let turn = f.engine.session_turn();
    signed_in(&f.engine);
    assert_eq!(
        f.engine.folder_link_for(turn, FOLDER, None).unwrap_err(),
        FolderLinkError::NotSignedIn
    );
    let ids = [NOTE.to_string()];
    assert_eq!(
        f.engine.bundle_link_for(turn, &ids, None).unwrap_err(),
        FolderLinkError::NotSignedIn
    );
    assert_eq!(
        f.engine.folder_links_revoke_for(turn, FOLDER).unwrap_err(),
        FolderLinkError::NotSignedIn
    );
    assert_eq!(f.sent(), 0, "nothing was sent");

    // The session moves while the custody is read: the next request of the press is never sent.
    let engine = f.engine.clone();
    f.with(|s| s.on_route = Some((CUSTODY, Box::new(move || signed_in(&engine)))));
    assert_eq!(f.folder_link(FOLDER, None).unwrap_err(), FolderLinkError::NotSignedIn);
    assert_eq!(f.count(MAKE_FOLDER_LINK), 0);
    assert_eq!(
        f.api.routes().last().map(String::as_str),
        Some(format!("{CUSTODY}{FOLDER}").as_str()),
        "nothing after the custody read"
    );
}

#[test]
fn a_403_is_beebeeb_refusing_never_a_session_that_ended() {
    let f = fixture();
    let mut checked = 0;
    for (status, want) in [(403, FolderLinkError::Refused), (401, FolderLinkError::SessionEnded)] {
        f.with(|s| s.make_status = status);
        assert_eq!(
            f.folder_link(FOLDER, Some(&expiring(24))).unwrap_err(),
            want,
            "a new folder link, {status}"
        );
        assert_eq!(f.bundle(&[NOTE]).unwrap_err(), want, "a bundle, {status}");
        f.with(|s| {
            s.make_status = 200;
            s.list_status = status;
        });
        assert_eq!(
            f.folder_link(FOLDER, None).unwrap_err(),
            want,
            "the links read, {status}"
        );
        assert_eq!(f.revoke(FOLDER).unwrap_err(), want, "stop sharing, {status}");
        f.with(|s| s.list_status = 200);
        checked += 4;
    }
    assert_eq!(checked, 8);
    assert_eq!(f.count(MAKE_FOLDER_LINK), 2, "each new folder link was asked once");
}

#[test]
fn a_bundle_made_for_a_session_that_ended_meanwhile_is_not_handed_over() {
    let f = fixture();
    let turn = f.engine.session_turn();
    let engine = f.engine.clone();
    f.with(|s| s.on_route = Some((MAKE_LINK, Box::new(move || signed_in(&engine)))));
    let ids = [NOTE.to_string(), PLAN.to_string()];
    assert_eq!(
        f.engine.bundle_link_for(turn, &ids, None).unwrap_err(),
        FolderLinkError::NotSignedIn
    );
    assert_eq!(f.count(MAKE_LINK), 1, "it was sent before the session moved");
}

#[test]
fn a_rebuilt_folder_link_read_for_a_session_that_ended_meanwhile_is_not_handed_over() {
    let f = fixture();
    f.folder_link(FOLDER, None).expect("the folder's link");
    let turn = f.engine.session_turn();
    let engine = f.engine.clone();
    f.with(|s| s.on_route = Some((LINKS, Box::new(move || signed_in(&engine)))));
    assert_eq!(
        f.engine.folder_link_for(turn, FOLDER, None).unwrap_err(),
        FolderLinkError::NotSignedIn
    );
    assert_eq!(f.count(MAKE_FOLDER_LINK), 1, "none was made");
    assert_eq!(
        f.count(LINKS),
        2,
        "the links were read under the old session, then nothing"
    );
}

/// The session moves at the top of the press's next request, before that request's own session check.
fn before_next_send(engine: &Arc<TestEngine>) {
    let engine = engine.clone();
    crate::app::bindings::link_session::tests::BEFORE_SEND
        .with(|hook| *hook.borrow_mut() = Some(Box::new(move || signed_in(&engine))));
}

#[test]
fn a_bundle_pressed_under_another_session_is_refused_before_any_file_is_looked_at() {
    // The run's session check (`link_session.rs`, `Engine::link_run`): a bundle pressed under a session that is gone
    // is refused as not signed in, never told "not found" about a file it holds.
    let f = fixture();
    let turn = f.engine.session_turn();
    signed_in(&f.engine);
    let ids = [UNKNOWN.to_string()];
    assert_eq!(
        f.engine.bundle_link_for(turn, &ids, None).unwrap_err(),
        FolderLinkError::NotSignedIn
    );
    assert_eq!(f.sent(), 0, "nothing was sent");
}

#[test]
fn a_session_that_moves_before_a_request_leaves_sends_nothing() {
    // The send's session check (`link_session.rs`, `Engine::link_send`): the session moves after the press's run
    // and before its first request; nothing leaves.
    let f = fixture();
    before_next_send(&f.engine);
    assert_eq!(f.revoke(FOLDER).unwrap_err(), FolderLinkError::NotSignedIn);
    before_next_send(&f.engine);
    assert_eq!(f.bundle(&[NOTE]).unwrap_err(), FolderLinkError::NotSignedIn);
    assert_eq!(f.sent(), 0, "nothing was sent");
}
