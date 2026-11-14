//! Shared on the real engine (`EngineSharedSource`), against the engine's fake accounts on loopback with the share
//! routes answered here in the server's shape: the lists, every state the view draws, what is read when, the three
//! bound changes and the unbound ones, the session rule, and the window's element tree after a lock and after
//! another account signed in. Every name and address is invented.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use rune::{Env, Runtime};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::State;
use crate::frames::shared::Tab;
use crate::shell::app::Section;

const PASSWORD: &str = "an invented password for shared";
const TRIP: &str = "a0000000-0000-4000-8000-000000000501";
const THEIRS: &str = "a0000000-0000-4000-8000-000000000502";
const OTHERS: &str = "a0000000-0000-4000-8000-000000000503";
const INV_A: &str = "b0000000-0000-4000-8000-000000000101";
const INV_B: &str = "b0000000-0000-4000-8000-000000000102";
const INV_IN: &str = "b0000000-0000-4000-8000-000000000103";
const INV_GOT: &str = "b0000000-0000-4000-8000-000000000104";
const REQ: &str = "b0000000-0000-4000-8000-000000000105";

struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    first_phrase: String,
    second: Arc<FakeAccount>,
    second_phrase: String,
    /// The status the share routes answer with (200: the lists).
    status: Arc<AtomicU16>,
    delay_ms: Arc<AtomicU64>,
    /// The status the four list reads answer with instead of `status` (0: `status`).
    read_status: Arc<AtomicU16>,
}

fn encrypted(account: &FakeAccount, id: &str, name: &str) -> String {
    account.encrypted_name(id, name)
}

/// What the four lists answer for an account: `[incoming, sent, mine, file-requests]`.
fn lists(account: &FakeAccount, first: bool) -> [Value; 4] {
    let sent = |id: &str, item: &str, name: &str, email: &str, status: &str, permission: &str| {
        json!({ "id": id, "file_id": item, "recipient_email": email, "status": status, "permission": permission,
            "file_name_encrypted": encrypted(account, item, name), "is_folder_share": true })
    };
    if !first {
        return [
            json!({ "invites": [] }),
            json!({ "invites": [sent(INV_A, OTHERS, "Second account plans", "third@example.eu", "approved", "read")] }),
            json!({ "shares": [] }),
            json!({ "file_requests": [] }),
        ];
    }
    [
        json!({ "invites": [
            { "id": INV_IN, "file_id": THEIRS, "status": "invited", "sender_email": "sender@example.eu",
              "file_name_encrypted": "a plaintext field", "is_folder": true, "created_at": "2026-11-01T12:00:00Z" },
            { "id": INV_GOT, "file_id": OTHERS, "status": "approved", "sender_email": "giver@example.eu",
              "file_name_encrypted": "{}", "size_bytes": 2048, "approved_at": "2026-11-02T12:00:00Z" },
        ] }),
        json!({ "invites": [
            sent(INV_A, TRIP, "Invented trip", "one@example.eu", "approved", "write"),
            sent(INV_B, TRIP, "Invented trip", "two@example.eu", "invited", "read"),
        ] }),
        json!({ "shares": [
            { "id": REQ, "file_id": TRIP, "token": "A-LINK-TOKEN", "has_passphrase": true,
              "expires_at": "2099-01-01T12:00:00Z" },
            { "id": INV_A, "file_id": THEIRS, "token": "A-LINK-TOKEN", "expires_at": "2026-10-03T12:00:00Z" },
        ] }),
        json!({ "file_requests": [
            { "id": REQ, "title": "Invented request", "files_received": 12, "max_files": 50, "closed": false },
            { "id": INV_B, "title": "Closed request", "files_received": 1, "closed": true },
        ] }),
    ]
}

const PATHS: [&str; 4] = [
    "GET /api/v1/shares/invites/incoming",
    "GET /api/v1/shares/invites/sent",
    "GET /api/v1/shares/mine",
    "GET /api/v1/file-requests",
];

fn fake(tag: &str) -> Fake {
    let (first, first_phrase) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, second_phrase) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let (ours, theirs_lists) = (lists(&first, true), lists(&second, false));
    let (status, delay_ms) = (Arc::new(AtomicU16::new(200)), Arc::new(AtomicU64::new(0)));
    let (code, delay) = (status.clone(), delay_ms.clone());
    let read_status = Arc::new(AtomicU16::new(0));
    let read_code = read_status.clone();
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let theirs = if route.starts_with("POST /api/v1/opaque/") {
            req.json()["email"]
                .as_str()
                .is_some_and(|sent| sent.eq_ignore_ascii_case(&second_email))
        } else {
            req.bearer() == Some(second_token.as_str())
        };
        let at = PATHS.iter().position(|path| route == *path);
        if at.is_none() && !route.contains("/api/v1/shares/") && !route.contains("/api/v1/file-requests/") {
            return if theirs { two(req) } else { one(req) };
        }
        std::thread::sleep(Duration::from_millis(delay.load(Ordering::SeqCst)));
        let code = match (read_code.load(Ordering::SeqCst), at) {
            (0, _) | (_, None) => code.load(Ordering::SeqCst),
            (read, Some(_)) => read,
        };
        match (code, at) {
            (200, Some(at)) => Reply::ok(if theirs { &theirs_lists } else { &ours }[at].clone()),
            (200, None) => Reply::ok(json!({})),
            (status, _) => Reply::json(status, json!({ "error": "a server body" })),
        }
    }));
    Fake {
        fake,
        first,
        first_phrase,
        second,
        second_phrase,
        status,
        delay_ms,
        read_status,
    }
}

fn engine_at(base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "shared-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

fn sign_in(engine: &Engine<MemoryCredentials>, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

fn signed_in(fake: &Fake) -> Arc<Engine<MemoryCredentials>> {
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.first, &fake.first_phrase);
    engine
}

/// Pump until nothing is on its way (at most 10 s); every notice on the way, in order.
fn settle(source: &mut EngineSharedSource<MemoryCredentials>) -> Vec<Notice> {
    let began = Instant::now();
    let mut notices = Vec::new();
    loop {
        let pumped = source.pump();
        notices.extend(pumped.notice);
        if !pumped.busy && !source.busy() {
            return notices;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn opened(source: &mut EngineSharedSource<MemoryCredentials>) -> Vec<Notice> {
    source.opened();
    settle(source)
}

/// The share routes asked so far.
fn asked(fake: &FakeApi) -> Vec<String> {
    fake.routes()
        .into_iter()
        .filter(|route| route.contains("/shares/") || route.contains("/file-requests"))
        .collect()
}

fn changes(fake: &FakeApi) -> Vec<String> {
    asked(fake)
        .into_iter()
        .filter(|route| !route.starts_with("GET "))
        .collect()
}

#[test]
fn the_lists_are_read_when_shared_opens_and_drawn_from_what_beebeeb_answered() {
    let fake = fake("lists");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    assert!(source.wired());
    // Drawing never reads.
    for _ in 0..20 {
        let _ = (
            source.invitations(),
            source.with_me(),
            source.people_shares(),
            source.links(),
        );
        let _ = (source.requests(), source.members(TRIP), source.loaded());
    }
    assert_eq!(source.loaded(), Loaded::Loading);
    assert!(asked(&fake.fake).is_empty(), "nothing is read before Shared opens");

    assert!(opened(&mut source).is_empty());
    assert_eq!(source.loaded(), Loaded::Ready);
    assert_eq!(asked(&fake.fake), PATHS);

    let invitations = source.invitations();
    assert_eq!(invitations.len(), 1);
    assert_eq!(
        (invitations[0].id.as_str(), invitations[0].name.as_str()),
        (INV_IN, "Encrypted item")
    );
    assert_eq!(invitations[0].kind, Kind::Folder);
    assert_eq!(invitations[0].from_line, "sender@example.eu wants to share this folder");

    let with_me = source.with_me();
    assert_eq!(with_me.len(), 1);
    assert_eq!((with_me[0].id.as_str(), with_me[0].size.as_str()), (OTHERS, "2 KB"));
    assert_eq!(with_me[0].date, "2 Nov 2026");
    assert_eq!(
        with_me[0].extra,
        Extra::Person {
            initials: "G".into(),
            name: "giver@example.eu".into(),
            hue: 'e'
        }
    );
    assert_eq!(with_me[0].pc, None);

    let people = source.people_shares();
    assert_eq!(people.len(), 1);
    assert_eq!(
        (people[0].id.as_str(), people[0].name.as_str()),
        (TRIP, "Invented trip")
    );
    assert_eq!(people[0].names, "one@example.eu, two@example.eu");
    assert_eq!(
        (people[0].people.len(), people[0].pending, people[0].kind),
        (2, false, Kind::Folder)
    );

    let members = source.members(TRIP);
    let seen: Vec<_> = members
        .iter()
        .map(|m| (m.id.as_str(), m.email.as_str(), m.access, m.you, m.invited, m.compared))
        .collect();
    assert_eq!(
        seen,
        [
            (OWNER, fake.first.email.as_str(), None, true, false, false),
            (INV_A, "one@example.eu", Some(Access::Edit), false, false, false),
            (INV_B, "two@example.eu", Some(Access::View), false, true, false),
        ]
    );
    assert!(source.members(OTHERS).is_empty(), "no such share: no people");

    let links: Vec<_> = source.links().into_iter().map(|l| (l.id, l.name, l.expired)).collect();
    assert_eq!(
        links,
        [
            (REQ.to_string(), "Invented trip".to_string(), false),
            (INV_A.to_string(), "Shared item".to_string(), true)
        ]
    );
    let lines: Vec<_> = source.links().into_iter().map(|l| l.line).collect();
    assert!(lines[0].starts_with("Expires in ") && lines[0].ends_with(" days, with password"));
    assert_eq!(lines[1], "Expired 3 Oct");

    let requests: Vec<_> = source.requests().into_iter().map(|r| (r.id, r.line, r.state)).collect();
    assert_eq!(
        requests,
        [
            (REQ.to_string(), "12 of 50 files".to_string(), RequestState::Open),
            (INV_B.to_string(), "1 file".to_string(), RequestState::Closed)
        ]
    );

    // Opening again while the lists are fresh reads nothing.
    assert!(opened(&mut source).is_empty());
    assert_eq!(asked(&fake.fake).len(), 4);
}

#[test]
fn an_account_that_shares_nothing_and_every_way_the_lists_do_not_come() {
    let fake = fake("states");
    // Nobody signed in: the signed-out state, and nothing is asked.
    let mut nobody = EngineSharedSource::new(engine_at(&fake.fake.base));
    assert!(opened(&mut nobody).is_empty());
    assert_eq!(nobody.loaded(), Loaded::SignedOut);
    assert!(asked(&fake.fake).is_empty());

    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.second, &fake.second_phrase);
    let mut source = EngineSharedSource::new(engine);
    fake.status.store(500, Ordering::SeqCst);
    opened(&mut source);
    assert_eq!(source.loaded(), Loaded::Failed);
    fake.status.store(401, Ordering::SeqCst);
    opened(&mut source);
    assert_eq!(source.loaded(), Loaded::SessionEnded);
    fake.status.store(200, Ordering::SeqCst);
    opened(&mut source);
    assert_eq!(source.loaded(), Loaded::Ready, "empty lists it has read are an answer");
    assert!(source.invitations().is_empty() && source.with_me().is_empty());
    assert!(source.links().is_empty() && source.requests().is_empty());
    assert_eq!(source.people_shares().len(), 1);

    // A refresh that fails keeps what is shown and says so.
    source.forget();
    opened(&mut source);
    source.read_at = None;
    fake.status.store(500, Ordering::SeqCst);
    assert_eq!(opened(&mut source), [Notice::RefreshFailed]);
    assert!(source.read_at.is_none(), "a failed read leaves the lists old");
    assert_eq!(source.people_shares().len(), 1);

    // Beebeeb cannot be reached at all.
    let dead = engine_at(&fake.fake.base);
    sign_in(&dead, &fake.first, &fake.first_phrase);
    let mut offline = EngineSharedSource::new(dead);
    fake.status.store(200, Ordering::SeqCst);
    fake.delay_ms.store(0, Ordering::SeqCst);
    drop(fake);
    opened(&mut offline);
    assert_eq!(offline.loaded(), Loaded::Offline);
}

#[test]
fn the_three_bound_changes_go_out_once_by_id_and_nothing_is_drawn_before_beebeeb_answered() {
    let fake = fake("changes");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    // Before the lists are here nothing can be changed.
    assert!(!source.accept(INV_IN) && !source.remove_access(TRIP, INV_A));
    opened(&mut source);
    let before = source.held.clone();

    // Not an invitation, the owner, a person of another share, an unknown share: nothing is sent.
    assert!(!source.accept(INV_A));
    assert!(!source.set_access(TRIP, OWNER, Access::Edit));
    assert!(!source.remove_access(TRIP, OWNER));
    assert!(!source.remove_access(OTHERS, INV_A));
    assert!(!source.set_access(TRIP, INV_IN, Access::Edit));
    assert!(settle(&mut source).is_empty());
    assert!(changes(&fake.fake).is_empty());

    // One press, one request; a second press while the answer is on its way sends nothing. Until Beebeeb answered,
    // every row is what Beebeeb last said.
    fake.delay_ms.store(150, Ordering::SeqCst);
    assert!(source.accept(INV_IN));
    assert!(source.set_access(TRIP, INV_B, Access::Edit));
    assert!(source.remove_access(TRIP, INV_A));
    assert!(!source.accept(INV_IN), "an answer is on its way");
    assert!(!source.set_access(TRIP, INV_B, Access::View));
    assert!(!source.set_access(TRIP, INV_B, Access::Edit));
    assert!(!source.remove_access(TRIP, INV_A));
    assert_eq!(source.held, before, "nothing is drawn as made before Beebeeb said so");
    assert!(settle(&mut source).is_empty());
    let mut sent = changes(&fake.fake);
    sent.sort();
    assert_eq!(
        sent,
        [
            format!("DELETE /api/v1/shares/invites/{INV_A}"),
            format!("PATCH /api/v1/shares/invites/{INV_B}"),
            format!("POST /api/v1/shares/invites/{INV_IN}/claim"),
        ]
    );
    // The lists were read again after the answers: they are Beebeeb's (this fake still lists everything).
    assert!(asked(&fake.fake).len() >= 4 + 3 + 4);
    assert_eq!(source.held, before);
    // Answered: the same person can be changed again.
    fake.delay_ms.store(0, Ordering::SeqCst);
    assert!(source.set_access(TRIP, INV_B, Access::Edit));
    assert!(settle(&mut source).is_empty());
    assert_eq!(changes(&fake.fake).len(), 4);
}

#[test]
fn a_change_beebeeb_never_made_is_never_drawn_as_made_also_when_the_refresh_fails() {
    let fake = fake("refused");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    opened(&mut source);
    let servers = source.held.clone();
    assert!(servers.is_some());

    // Beebeeb refuses every change, and the read after it fails too.
    fake.status.store(500, Ordering::SeqCst);
    assert!(source.remove_access(TRIP, INV_A));
    assert!(source.set_access(TRIP, INV_B, Access::Edit));
    assert!(source.accept(INV_IN));
    let notices = settle(&mut source);
    assert!(
        !notices.is_empty() && notices.iter().all(|n| *n == Notice::ChangeRefused),
        "the refusal keeps its own words, never only \"couldn't refresh\": {notices:?}"
    );
    assert_eq!(
        source.held, servers,
        "the rows are the server's: nobody was removed, nothing changed"
    );
    let people: Vec<_> = source.members(TRIP).into_iter().map(|m| (m.id, m.access)).collect();
    assert_eq!(
        people,
        [
            (OWNER.to_string(), None),
            (INV_A.to_string(), Some(Access::Edit)),
            (INV_B.to_string(), Some(Access::View))
        ]
    );
    assert_eq!(source.people_shares()[0].names, "one@example.eu, two@example.eu");
    assert_eq!(source.invitations().len(), 1);

    // The failed read did not make the lists fresh: the next opening reads again at once.
    fake.status.store(200, Ordering::SeqCst);
    let reads = asked(&fake.fake)
        .iter()
        .filter(|route| route.as_str() == PATHS[0])
        .count();
    assert!(opened(&mut source).is_empty());
    assert_eq!(
        asked(&fake.fake)
            .iter()
            .filter(|route| route.as_str() == PATHS[0])
            .count(),
        reads + 1
    );

    // A change Beebeeb MADE after a refused one, with the read after it failing: the words are about the refresh,
    // never "couldn't make that change" for a change that was made.
    fake.status.store(500, Ordering::SeqCst);
    assert!(source.set_access(TRIP, INV_B, Access::Edit));
    assert!(settle(&mut source).contains(&Notice::ChangeRefused));
    fake.status.store(200, Ordering::SeqCst);
    fake.read_status.store(500, Ordering::SeqCst);
    assert!(source.set_access(TRIP, INV_B, Access::Edit));
    assert_eq!(settle(&mut source), [Notice::RefreshFailed]);
    fake.read_status.store(0, Ordering::SeqCst);

    // Beebeeb cannot be reached at all (the change and the read both get no answer).
    drop(fake);
    assert!(source.remove_access(TRIP, INV_A));
    let notices = settle(&mut source);
    assert!(notices.contains(&Notice::ChangeRefused), "{notices:?}");
    assert_eq!(source.held, servers);
}

#[test]
fn what_is_not_bound_sends_nothing_and_says_not_available() {
    let fake = fake("unbound");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    opened(&mut source);
    let unbound: [&dyn Fn(&mut EngineSharedSource<MemoryCredentials>) -> bool; 3] =
        [&|s| s.keep_offline(OTHERS), &|s| s.remove_request(INV_B), &|s| {
            s.invite(TRIP, "new@example.eu", Access::View)
        }];
    for act in unbound {
        assert!(!act(&mut source));
        assert_eq!(settle(&mut source), [Notice::NotAvailable]);
    }
    assert!(changes(&fake.fake).is_empty());
    assert_eq!(source.invitations().len(), 1);
    assert_eq!(source.people_shares().len(), 1);
}

#[test]
fn decline_stop_sharing_and_close_request_go_out_once_by_id_and_nothing_is_drawn_before_beebeeb_answered() {
    let fake = fake("ends");
    let mut source = EngineSharedSource::new(signed_in(&fake));
    // Before the lists are here nothing can be ended.
    assert!(!source.decline(INV_IN) && !source.stop_sharing(TRIP) && !source.close_request(REQ));
    opened(&mut source);
    let before = source.held.clone();

    // Not an invitation, not a row, a request that is closed already: nothing is sent, nothing is said.
    assert!(!source.decline(INV_A) && !source.decline("owner"));
    assert!(
        !source.stop_sharing(OTHERS) && !source.stop_sharing(THEIRS),
        "an item's id names no link row"
    );
    assert!(!source.close_request(INV_B), "closed already");
    assert!(!source.close_request(INV_A));
    assert!(settle(&mut source).is_empty());
    assert!(changes(&fake.fake).is_empty());

    fake.delay_ms.store(150, Ordering::SeqCst);
    assert!(source.decline(INV_IN));
    assert!(source.stop_sharing(TRIP), "a share with people: every invite of it");
    assert!(source.close_request(REQ));
    assert!(!source.decline(INV_IN), "an answer is on its way");
    assert!(!source.stop_sharing(TRIP) && !source.close_request(REQ));
    assert_eq!(source.held, before, "nothing is drawn as ended before Beebeeb said so");
    assert!(settle(&mut source).is_empty());
    // (This fake gives a link and a request one id: a link row is named by its link's id.)
    assert!(source.stop_sharing(REQ), "a link row is named by its link");
    assert!(settle(&mut source).is_empty());
    let mut sent = changes(&fake.fake);
    sent.sort();
    let mut expected = vec![
        format!("POST /api/v1/shares/invites/{INV_IN}/hide"),
        format!("DELETE /api/v1/shares/invites/{INV_A}"),
        format!("DELETE /api/v1/shares/invites/{INV_B}"),
        format!("DELETE /api/v1/shares/{REQ}"),
        format!("POST /api/v1/file-requests/{REQ}/close"),
    ];
    expected.sort();
    assert_eq!(sent, expected);
    assert_eq!(
        source.held, before,
        "the lists are Beebeeb's, read again (this fake still lists everything)"
    );

    // Refused: the words say it was not made, and the row stays.
    fake.status.store(500, Ordering::SeqCst);
    assert!(source.decline(INV_IN));
    let notices = settle(&mut source);
    assert!(
        !notices.is_empty() && notices.iter().all(|n| *n == Notice::ChangeRefused),
        "{notices:?}"
    );
    assert_eq!(source.held, before);
}

#[test]
fn lists_of_a_session_that_ended_are_never_handed_out() {
    let fake = fake("session");
    let engine = signed_in(&fake);
    let mut source = EngineSharedSource::new(engine.clone());
    opened(&mut source);
    assert_eq!(source.people_shares().len(), 1);
    let _ = engine.lock();
    // Before any pump: every accessor is already empty.
    assert!(source.people_shares().is_empty() && source.members(TRIP).is_empty());
    assert!(source.invitations().is_empty() && source.links().is_empty());
    assert!(
        !source.remove_access(TRIP, INV_A),
        "no change for a session that is gone"
    );
    assert_eq!(source.loaded(), Loaded::SignedOut);
    assert!(source.pump().changed);

    // An answer that outlives its session is dropped.
    engine
        .unlock_with_phrase(Zeroizing::new(fake.first_phrase.clone()))
        .expect("unlocks");
    fake.delay_ms.store(120, Ordering::SeqCst);
    source.opened();
    std::thread::sleep(Duration::from_millis(60));
    let _ = engine.lock();
    settle(&mut source);
    assert!(source.people_shares().is_empty(), "never drawn");
    assert!(changes(&fake.fake).is_empty());
}

fn words(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.label.unwrap_or_default(), e.text.unwrap_or_default()])
        .collect::<Vec<_>>()
        .join("\n")
}

/// Poll as the window does (`shell::host` calls `frames::shared::poll`), then draw, until nothing is on its way.
fn poll(rt: &mut Runtime<State>) {
    let began = Instant::now();
    loop {
        rt.update(|s, tx| {
            crate::frames::shared::poll(s, tx);
        });
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
        if !rt.state.frames.shared.source.pump().busy {
            rt.update(|s, tx| {
                crate::frames::shared::poll(s, tx);
            });
            rt.frame(0.0);
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_window_draws_the_lists_its_states_a_notice_and_nothing_of_the_account_before() {
    let fake = fake("window");
    let engine = signed_in(&fake);
    let mut state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine.clone());
    assert!(
        state.frames.shared.source.wired(),
        "the running app hands Shared the engine's source"
    );
    crate::frames::go(&mut state, Section::Shared);
    state.frames.shared.show(Tab::ByMe);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    poll(&mut rt);
    let seen = words(&rt);
    assert!(
        seen.contains("Invented trip") && seen.contains("one@example.eu"),
        "{seen}"
    );
    assert!(crate::ui::sidebar::badge_for(&rt.state, crate::ui::sidebar::SidebarSection::Shared).is_some());

    // A control that is not bound says so in the window.
    rt.state.frames.shared.selected = Some(0);
    rt.update(|s, _| {
        s.frames.shared.source.keep_offline(OTHERS);
    });
    poll(&mut rt);
    assert!(words(&rt).contains("Not available yet in this version"));
    assert!(words(&rt).contains("Invented trip"));

    let _ = engine.lock();
    poll(&mut rt);
    let seen = words(&rt);
    for gone in ["Invented trip", "one@example.eu", "two@example.eu", "sender@example.eu"] {
        assert!(!seen.contains(gone), "{gone} is still drawn after the lock");
    }
    assert!(seen.contains("Signed out"), "{seen}");
    assert!(crate::ui::sidebar::badge_for(&rt.state, crate::ui::sidebar::SidebarSection::Shared).is_none());

    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    crate::frames::go(&mut rt.state, Section::Files);
    poll(&mut rt);
    crate::frames::go(&mut rt.state, Section::Shared);
    poll(&mut rt);
    let seen = words(&rt);
    for gone in [
        "Invented trip",
        "one@example.eu",
        "two@example.eu",
        fake.first.email.as_str(),
    ] {
        assert!(
            !seen.contains(gone),
            "{gone} of the first account is drawn for the second"
        );
    }
    assert!(
        seen.contains("Second account plans"),
        "the second account's shares: {seen}"
    );

    // Loading is drawn, never an empty list, while the first answer is on its way.
    rt.update(|s, _| s.frames.shared.source.forget());
    fake.delay_ms.store(150, Ordering::SeqCst);
    rt.update(|s, _| s.frames.shared.source.opened());
    rt.frame(0.0);
    assert!(words(&rt).contains("Loading Shared"));
    assert!(!words(&rt).contains("You haven\u{2019}t shared anything"));
    poll(&mut rt);
}
