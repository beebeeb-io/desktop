//! A link to a file at the engine (`Engine::share_link_for`), against the engine's own doubles: two invented
//! accounts on the fake API on loopback (its share routes: make a link, list the account's), a state database of
//! each test's own recorded as the first account's. Sessions are put in memory as a sign-in and an unlock leave
//! them. Requests are counted at the fake as they arrive, before it looks at a token. Every name and key is
//! invented.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use super::super::{Engine, EngineConfig};
use super::*;
use crate::account::SessionTransition;
use crate::account_binding::Identity;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::state_db::{FileEntry, ItemKind};
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Handler, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const NOTE: &str = "a0000000-0000-4000-8000-000000000601";
const PLAN: &str = "a0000000-0000-4000-8000-000000000602";
const FOLDER: &str = "a0000000-0000-4000-8000-000000000603";
/// In the local state, not on the server.
const NOT_ON_SERVER: &str = "a0000000-0000-4000-8000-000000000604";
/// In no local state.
const UNKNOWN: &str = "a0000000-0000-4000-8000-000000000605";
/// An id of the class whose key is never derived, with no key record here.
const NO_KEY_HERE: &str = "a0000000-0000-8000-8000-000000000606";
/// A row this computer is moving to Trash.
const TRASHING: &str = "a0000000-0000-4000-8000-000000000607";
/// The second account's file; a row of it stands in the first's local state, as a stale one would.
const THEIRS: &str = "a0000000-0000-4000-8000-000000000701";

const POST: &str = "POST /api/v1/shares";
const LIST: &str = "GET /api/v1/shares/mine";

/// Holds every request of one route until released, and says when one arrived.
#[derive(Default)]
struct Hold {
    route: Mutex<Option<&'static str>>,
    released: Mutex<bool>,
    wake: Condvar,
    arrived: Mutex<Option<mpsc::Sender<()>>>,
}

impl Hold {
    /// From now on requests of `route` wait; the receiver hears of each that arrived.
    fn on(&self, route: &'static str) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel();
        *self.arrived.lock().unwrap() = Some(tx);
        *self.route.lock().unwrap() = Some(route);
        rx
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }

    fn wait(&self, route: &str) {
        if *self.route.lock().unwrap() != Some(route) {
            return;
        }
        if let Some(arrived) = self.arrived.lock().unwrap().as_ref() {
            let _ = arrived.send(());
        }
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }
}

struct Fake {
    api: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    hold: Arc<Hold>,
    /// Not 0: the list of the account's links answers with this status.
    list_status: Arc<AtomicU16>,
}

impl Fake {
    fn count(&self, route: &str) -> usize {
        self.api.routes().iter().filter(|seen| *seen == route).count()
    }
}

fn fake(tag: &str) -> Fake {
    let (first, _) = FakeAccount::register(&format!("{tag}-one@example.eu"), "an invented password", TEST_PHRASE);
    let (second, _) = FakeAccount::register(
        &format!("{tag}-two@example.eu"),
        "an invented password",
        "legal winner thank year wave sausage worth useful legal winner thank yellow",
    );
    let (first, second) = (Arc::new(first), Arc::new(second));
    assert_ne!(first.master_key, second.master_key);
    first.add_item(NOTE, None, "Invented note.txt", false, 10);
    first.add_item(PLAN, None, "Invented plan.txt", false, 10);
    first.add_item(FOLDER, None, "Invented folder", true, 0);
    first.add_item(NO_KEY_HERE, None, "Invented adopted.txt", false, 10);
    first.add_item(TRASHING, None, "Invented leaving.txt", false, 10);
    second.add_item(THEIRS, None, "Second account plan.txt", false, 10);
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let second_token = second.session_token.clone();
    let (hold, list_status) = (Arc::new(Hold::default()), Arc::new(AtomicU16::new(0)));
    let (hold_in, status_in) = (hold.clone(), list_status.clone());
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        hold_in.wait(&route);
        let status = status_in.load(Ordering::SeqCst);
        if route == LIST && status == 200 {
            // It answers, and not with a list of links.
            return Reply::ok(serde_json::json!({ "items": [] }));
        }
        if route == LIST && status != 0 {
            return Reply::json(status, serde_json::json!({ "error": "a server body" }));
        }
        if req.bearer() == Some(second_token.as_str()) {
            two(req)
        } else {
            one(req)
        }
    }));
    Fake {
        api,
        first,
        second,
        hold,
        list_status,
    }
}

fn engine_at(base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-link-slot".into(),
        state_dir,
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// A session in memory, as a sign-in and an unlock of `account` leave it.
fn signed_in(engine: &TestEngine, account: &FakeAccount) {
    let _writes = engine.writes.write().unwrap();
    engine
        .acct
        .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    *engine.acct.session.lock().unwrap() = Some(Session {
        token: account.session_token.clone(),
        master_key: account.master_key,
        email: Some(account.email.clone()),
    });
    engine.remember_session_user_id(Some(account.user_id.clone()));
    engine.set_auth_present(true);
}

fn row(db: &StateDb, id: &str, path: &str, folder: bool) {
    row_in(db, id, path, folder, FileStatus::CloudOnly);
}

fn row_in(db: &StateDb, id: &str, path: &str, folder: bool, status: FileStatus) {
    let entry = FileEntry {
        file_id: id.into(),
        path: path.into(),
        status,
        size_bytes: 10,
        modified_at: 1_794_000_000,
        content_hash: None,
        remote_updated_at: 1_794_000_000,
        parent_id: None,
        item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
    };
    db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
        .unwrap();
}

/// A state database recorded as `owner`'s, and the engine reading it.
fn local_state(engine: &TestEngine, owner: &FakeAccount) -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    row(&db, NOTE, "Invented note.txt", false);
    row(&db, PLAN, "Invented plan.txt", false);
    row(&db, FOLDER, "Invented folder", true);
    row(&db, NOT_ON_SERVER, "Invented gone.txt", false);
    row(&db, NO_KEY_HERE, "Invented adopted.txt", false);
    row(&db, THEIRS, "Second account plan.txt", false);
    row_in(&db, TRASHING, "Invented leaving.txt", false, FileStatus::Trashing);
    db.set_owner(&Identity::new(Some(&owner.user_id), Some(&owner.email)))
        .unwrap();
    engine.use_local_state_for_test(path);
    (dir, db)
}

/// The first account signed in, its local state, and the session of a press made now.
fn ready(tag: &str) -> (Fake, Arc<TestEngine>, (tempfile::TempDir, StateDb), SessionGeneration) {
    let fake = fake(tag);
    let engine = engine_at(&fake.api.base);
    signed_in(&engine, &fake.first);
    let state = local_state(&engine, &fake.first);
    let turn = engine.session_turn();
    (fake, engine, state, turn)
}

fn refusal(result: Result<MadeLink, ShareLinkError>) -> ShareLinkError {
    result.expect_err("no link is handed over")
}

/// The part of a link after `#key=`.
fn key_part(link: &str) -> &str {
    link.split_once("#key=").expect("a link with its key part").1
}

fn text<'a>(row: &'a Value, field: &str) -> &'a str {
    row.get(field).and_then(Value::as_str).unwrap_or_default()
}

/// A call on a thread of its own, held at the fake on `route`: returns once the request arrived there.
fn held_call(
    fake: &Fake,
    engine: &Arc<TestEngine>,
    turn: SessionGeneration,
    route: &'static str,
) -> std::thread::JoinHandle<Result<MadeLink, ShareLinkError>> {
    let arrived = fake.hold.on(route);
    let on_worker = engine.clone();
    let call = std::thread::spawn(move || on_worker.share_link_for(turn, NOTE, None));
    arrived
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the request arrived at the fake");
    call
}

#[test]
fn a_plain_copy_link_makes_one_link_that_opens_to_the_files_own_key() {
    let (fake, engine, _state, turn) = ready("makes");
    let made = engine.share_link_for(turn, NOTE, None).expect("a link");
    assert_eq!(
        fake.api.routes(),
        [LIST, POST],
        "the account's links are read, then one link is made"
    );
    assert_eq!(made.session, turn);

    let posted = fake.first.links();
    assert_eq!(posted.len(), 1);
    assert_eq!(text(&posted[0], "file_id"), NOTE);
    assert_eq!(posted[0]["expires_in_hours"], Value::Null, "it never expires");
    assert_eq!(posted[0]["max_opens"], Value::Null);
    assert_eq!(posted[0].get("passphrase"), None, "it asks for no password");

    // The link is the web app's, carries the posted token, and its key part opens the posted wrapped key to the
    // key the file is encrypted with: core's derivation for this id under this account's vault key.
    let link = made.expose();
    assert!(
        link.starts_with(&format!("https://app.beebeeb.io/s/{}#key=", text(&posted[0], "token"))),
        "the link's shape"
    );
    let opened = share_flow::link::open_link_file_key(key_part(link), text(&posted[0], "wrapped_file_key"))
        .expect("the link's key opens what was posted");
    let master = MasterKey::from_bytes(fake.first.master_key);
    let files = beebeeb_core::kdf::derive_file_key(&master, NOTE.as_bytes());
    assert_eq!(opened.as_bytes(), files.as_bytes(), "the file's own key, and no other");
    // The server was sent neither the link's key nor the file's key.
    let sent = String::from_utf8_lossy(&fake.api.requests()[1].body).to_string();
    assert!(
        !sent.contains(key_part(link)),
        "the link's key never leaves this computer"
    );

    // What may be drawn or printed holds nothing of the key part or the token.
    assert_eq!(made.display(), link.split('#').next().unwrap());
    let printed = format!("{made:?}");
    assert!(
        !printed.contains(key_part(link)) && !printed.contains(text(&posted[0], "token")),
        "{printed}"
    );
    assert!(!printed.contains("/s/"), "{printed}");
}

#[test]
fn a_file_that_has_a_live_link_never_gets_a_second_one_behind_it() {
    let (fake, engine, _state, turn) = ready("once");
    let first = engine.share_link_for(turn, NOTE, None).expect("the first link");
    assert_eq!(fake.count(POST), 1);

    // As the account's link list is today (no wrapped file key in it): the live link cannot be checked, so it is
    // not handed out, and no second link is made.
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::HasLink
    );
    assert_eq!(fake.count(POST), 1, "no second link");
    assert_eq!(fake.first.links().len(), 1);

    // The list answers every link of the account, whatever was asked: a link to another file is not this file's.
    engine
        .share_link_for(turn, PLAN, None)
        .expect("another file's first link");
    assert_eq!(fake.count(POST), 2);

    // Once the list carries the wrapped file key, the very same link is rebuilt and checked by core: none is made.
    fake.first.list_links_with_their_wrapped_key(true);
    let again = engine.share_link_for(turn, NOTE, None).expect("the live link again");
    assert_eq!(again.expose(), first.expose(), "the same link");
    assert_eq!(fake.count(POST), 2, "none was made");

    // A revoked link is not live: the next Copy link makes a new one.
    fake.first.revoke_links();
    let new = engine.share_link_for(turn, NOTE, None).expect("a new link");
    assert_ne!(new.expose(), first.expose());
    assert_eq!(fake.count(POST), 3);
}

#[test]
fn a_link_past_its_expiry_or_with_its_opens_used_up_is_not_live() {
    let (fake, engine, _state, turn) = ready("dead");
    engine.share_link_for(turn, NOTE, None).expect("the first link");
    // Past its expiry: Copy link makes a new one.
    fake.first.expire_links();
    engine
        .share_link_for(turn, NOTE, None)
        .expect("a link after the first expired");
    assert_eq!(fake.count(POST), 2);
    // The new one is live.
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::HasLink
    );
    // Opened as often as it may be: not live either.
    fake.first.use_up_links();
    engine
        .share_link_for(turn, NOTE, None)
        .expect("a link after the opens were used up");
    assert_eq!(fake.count(POST), 3);
}

#[test]
fn a_listed_link_that_is_another_files_is_never_handed_out_for_this_file() {
    let (fake, engine, _state, turn) = ready("mislabelled");
    let plans = engine.share_link_for(turn, PLAN, None).expect("the other file's link");
    assert_eq!(fake.count(POST), 1);
    // The list carries everything a rebuild needs, and names the wrong file on the row: the link rebuilds and
    // opens, to the OTHER file's key.
    fake.first.list_links_with_their_wrapped_key(true);
    fake.first.list_every_link_as(NOTE);
    let answer = engine.share_link_for(turn, NOTE, None);
    assert_eq!(
        refusal(answer),
        ShareLinkError::HasLink,
        "not handed out, and no second link behind it"
    );
    assert_eq!(fake.count(POST), 1);
    // Named truthfully again, it is its own file's link as before.
    fake.first.list_every_link_as(PLAN);
    let again = engine.share_link_for(turn, PLAN, None).expect("its own link");
    assert_eq!(again.expose(), plans.expose());
}

#[test]
fn when_the_links_cannot_be_read_no_link_is_made() {
    let (fake, engine, _state, turn) = ready("unread");
    fake.list_status.store(500, Ordering::SeqCst);
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::Refused
    );
    fake.list_status.store(401, Ordering::SeqCst);
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::SessionEnded
    );
    // It answers 200 with something that is not a list of links: not "no links", a read that failed.
    fake.list_status.store(200, Ordering::SeqCst);
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::Refused
    );
    assert_eq!(fake.count(LIST), 3);
    assert_eq!(fake.count(POST), 0, "nothing was made");

    // Nothing answers at all: offline, by its kind.
    let nowhere = engine_at("http://127.0.0.1:9");
    signed_in(&nowhere, &fake.first);
    let _state = local_state(&nowhere, &fake.first);
    assert_eq!(
        refusal(nowhere.share_link_for(nowhere.session_turn(), NOTE, None)),
        ShareLinkError::Offline
    );
}

#[test]
fn the_dialogs_choices_are_sent_and_each_asks_for_a_new_link() {
    let (fake, engine, _state, turn) = ready("choices");
    let choices = LinkChoices {
        expires_in_hours: Some(24),
        passphrase: Some(Zeroizing::new("an invented password".into())),
    };
    let made = engine.share_link_for(turn, NOTE, Some(&choices)).expect("a link");
    assert_eq!(
        fake.api.routes(),
        [POST],
        "a link with choices is a new link: nothing is read first"
    );
    let posted = fake.first.links();
    assert_eq!(posted[0]["expires_in_hours"], 24);
    assert_eq!(text(&posted[0], "passphrase"), "an invented password");
    assert!(
        !format!("{choices:?}").contains("invented"),
        "the password is not printed"
    );

    let second = engine
        .share_link_for(turn, NOTE, Some(&choices))
        .expect("a second link");
    assert_ne!(second.expose(), made.expose());
    assert_eq!(fake.count(POST), 2);

    // A choice core refuses is refused here, unsent.
    let short = LinkChoices {
        expires_in_hours: None,
        passphrase: Some(Zeroizing::new("abc".into())),
    };
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, Some(&short))),
        ShareLinkError::Invalid
    );
    // A password of spaces only would be dropped by core, and the link would ask for none: never made.
    let blank = LinkChoices {
        expires_in_hours: None,
        passphrase: Some(Zeroizing::new("   ".into())),
    };
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, Some(&blank))),
        ShareLinkError::Invalid
    );
    let long = LinkChoices {
        expires_in_hours: Some(24 * 366),
        passphrase: None,
    };
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, Some(&long))),
        ShareLinkError::Invalid
    );
    assert_eq!(fake.count(POST), 2, "nothing more was sent");
}

#[test]
fn what_is_not_a_file_of_this_accounts_local_state_is_refused_with_nothing_sent() {
    let (fake, engine, _state, turn) = ready("unsent");
    let link = |id: &str| refusal(engine.share_link_for(turn, id, None));
    // Not an id as the server writes it (never normalized).
    assert_eq!(link(&NOTE.to_uppercase()), ShareLinkError::NotFound);
    assert_eq!(link("../shares/mine"), ShareLinkError::NotFound);
    // No row in this account's local state.
    assert_eq!(link(UNKNOWN), ShareLinkError::NotFound);
    // A folder, before any key is looked up.
    assert_eq!(link(FOLDER), ShareLinkError::NotAFile);
    // An id whose key is never derived, with no key record on this computer.
    assert_eq!(link(NO_KEY_HERE), ShareLinkError::KeyUnavailable);
    // A file on its way to Trash is not shared.
    assert_eq!(link(TRASHING), ShareLinkError::NotFound);
    assert_eq!(fake.api.requests().len(), 0, "nothing was asked of Beebeeb");
}

#[test]
fn a_file_beebeeb_does_not_have_for_this_account_is_not_found() {
    let (fake, engine, _state, turn) = ready("server");
    // A row this computer still holds of a file that is gone.
    assert_eq!(
        refusal(engine.share_link_for(turn, NOT_ON_SERVER, None)),
        ShareLinkError::NotFound
    );
    // Another account's file (a stale row names it): Beebeeb answers as for a file that does not exist.
    assert_eq!(
        refusal(engine.share_link_for(turn, THEIRS, None)),
        ShareLinkError::NotFound
    );
    assert_eq!(fake.count(POST), 2);
    assert_eq!(
        fake.first.links().len() + fake.second.links().len(),
        0,
        "no link exists"
    );
}

#[test]
fn a_press_under_one_session_is_never_sent_under_another() {
    let (fake, engine, _state, turn) = ready("switch");
    // Signed out.
    engine.forget_session(SessionTransition::SignOut);
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::NotSignedIn
    );
    // Another account signed in since the press.
    signed_in(&engine, &fake.second);
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::NotSignedIn
    );
    // The same account again: another session all the same.
    engine.forget_session(SessionTransition::SignOut);
    signed_in(&engine, &fake.first);
    assert_eq!(
        refusal(engine.share_link_for(turn, NOTE, None)),
        ShareLinkError::NotSignedIn
    );
    // A lock and an unlock since the press.
    let pressed = engine.session_turn();
    engine.forget_session(SessionTransition::Lock);
    signed_in(&engine, &fake.first);
    assert_eq!(
        refusal(engine.share_link_for(pressed, NOTE, None)),
        ShareLinkError::NotSignedIn
    );
    assert_eq!(
        fake.api.requests().len(),
        0,
        "requests that reached the fake, under any token"
    );
    // The session that is here makes its own.
    assert!(engine.share_link_for(engine.session_turn(), NOTE, None).is_ok());
}

#[test]
fn a_session_that_ends_while_the_links_are_read_makes_no_link() {
    let (fake, engine, _state, turn) = ready("reading");
    let call = held_call(&fake, &engine, turn, LIST);
    engine.forget_session(SessionTransition::SignOut);
    signed_in(&engine, &fake.second);
    fake.hold.release();
    assert_eq!(refusal(call.join().unwrap()), ShareLinkError::NotSignedIn);
    assert_eq!(fake.count(POST), 0, "no link was asked for, under any token");
}

#[test]
fn a_token_that_is_taken_is_tried_once_more_and_not_at_all_once_the_session_ended() {
    // The session stays: the second try makes the link.
    let (fake, engine, _state, turn) = ready("taken");
    fake.first.take_the_next_link_token();
    assert!(engine.share_link_for(turn, NOTE, None).is_ok());
    assert_eq!(fake.count(POST), 2);
    assert_eq!(fake.first.links().len(), 1);

    // The session ends while the first try is at Beebeeb: there is no second try.
    let (fake, engine, _state, turn) = ready("taken-ended");
    fake.first.take_the_next_link_token();
    let call = held_call(&fake, &engine, turn, POST);
    engine.forget_session(SessionTransition::Lock);
    fake.hold.release();
    assert_eq!(refusal(call.join().unwrap()), ShareLinkError::NotSignedIn);
    assert_eq!(fake.count(POST), 1, "one request, and no more after the session ended");
    assert_eq!(fake.first.links().len(), 0);
}

#[test]
fn a_link_made_for_a_session_that_ended_meanwhile_is_not_handed_over() {
    let (fake, engine, _state, turn) = ready("handover");
    let call = held_call(&fake, &engine, turn, POST);
    engine.forget_session(SessionTransition::SignOut);
    signed_in(&engine, &fake.second);
    fake.hold.release();
    assert_eq!(refusal(call.join().unwrap()), ShareLinkError::NotSignedIn);
    // Beebeeb made it (the request was the first session's, under its token); nobody was handed it.
    assert_eq!(fake.first.links().len(), 1);
    assert_eq!(fake.second.links().len(), 0);
}
