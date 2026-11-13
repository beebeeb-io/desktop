//! Trash on the real engine (`EngineTrashSource`), against the engine's fake accounts on loopback with the Trash
//! listing answered in the server's shape here: the rows, every state the view draws (loading, empty, offline, a
//! server error, a session that ended, signed out), what is read when (only on opening and after a change, never from
//! a draw call), restores (optimistic by item id, refused, a failed re-read, a read already on its way), the session
//! rule (a lock, another account on the same handle), and the window's element tree after a lock and after another
//! account signed in. Engine handles are on the process's state folder, which nothing here writes in. Every name is
//! invented.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use chrono::TimeZone;
use rune::{Env, Runtime};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::State;
use crate::shell::app::Section;

mod session;

const PASSWORD: &str = "an invented password for trash";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;

const DOCS: &str = "a0000000-0000-4000-8000-000000000201";
const CONTRACT: &str = "a0000000-0000-4000-8000-000000000202";
const PHOTO: &str = "a0000000-0000-4000-8000-000000000203";
const KEPT: &str = "a0000000-0000-4000-8000-000000000204";
const OTHERS: &str = "a0000000-0000-4000-8000-000000000301";

/// One or two accounts on one fake: a sign-in leg goes to the account its email names, anything else to the account
/// whose token it carries.
struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    first_phrase: String,
    second: Arc<FakeAccount>,
    second_phrase: String,
    /// How the Trash listing answers.
    list_mode: Arc<AtomicU8>,
    /// How a restore answers.
    restore_mode: Arc<AtomicU8>,
    /// How long the listing waits before it answers (with what Beebeeb held when it was asked), in ms.
    list_delay_ms: Arc<AtomicU64>,
    /// The next restore request says on the first that it reached the fake, then waits on the second before
    /// anything is done with it.
    restore_hold: RestoreHold,
}

type RestoreHold = Arc<Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>>;

fn is_listing(req: &Request) -> bool {
    req.route() == "GET /api/v1/files" && req.query("trashed") == Some("true")
}

fn names_email(req: &Request, email: &str) -> bool {
    req.json()
        .get("email")
        .and_then(Value::as_str)
        .is_some_and(|sent| sent.eq_ignore_ascii_case(email))
}

fn fake(tag: &str) -> Fake {
    let (first, first_phrase) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, second_phrase) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (second_email, second_token) = (second.email.clone(), second.session_token.clone());
    let (list_mode, restore_mode, list_delay_ms) = (
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU64::new(0)),
    );
    let (list_in, restore_in, delay_in) = (list_mode.clone(), restore_mode.clone(), list_delay_ms.clone());
    let restore_hold = RestoreHold::default();
    let hold_in = restore_hold.clone();
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let sign_in = route.starts_with("POST /api/v1/opaque/");
        let theirs = if sign_in {
            names_email(req, &second_email)
        } else {
            req.bearer() == Some(second_token.as_str())
        };
        let rest = if theirs { &two } else { &one };
        let restoring = route.starts_with("POST /api/v1/files/") && route.ends_with("/restore");
        if !is_listing(req) && !restoring {
            return rest(req);
        }
        let held = if restoring {
            hold_in.lock().unwrap().take()
        } else {
            None
        };
        if let Some((reached, release)) = held {
            let _ = reached.send(());
            let _ = release.recv_timeout(Duration::from_secs(30));
        }
        let mode = if restoring { &restore_in } else { &list_in };
        match mode.load(Ordering::SeqCst) {
            SERVER_ERROR => return Reply::json(500, json!({ "error": "a server body" })),
            SESSION_ENDED => return Reply::json(401, json!({ "error": "a server body" })),
            _ => {}
        }
        if restoring {
            return rest(req);
        }
        // The fake answers `items`; the server (and the engine's client) say `files` and `next_cursor`.
        let body: Value = serde_json::from_slice(&rest(req).body).expect("the fake's listing");
        std::thread::sleep(Duration::from_millis(delay_in.load(Ordering::SeqCst)));
        Reply::ok(json!({ "files": body["items"], "next_cursor": null }))
    }));
    // The first account: a folder, a PDF and a photo in Trash, one file not. The second: one item in Trash.
    first.add_item(DOCS, None, "Invented old invoices", true, 0);
    first.add_item(CONTRACT, None, "Invented contract.pdf", false, 1_153_434);
    first.add_item(PHOTO, None, "IMG_0001.HEIC", false, 2_097_152);
    first.add_item(KEPT, None, "Not deleted.txt", false, 10);
    for id in [DOCS, CONTRACT, PHOTO] {
        assert!(first.trash_item(id));
    }
    second.add_item(OTHERS, None, "Second account budget.xlsx", false, 94_208);
    assert!(second.trash_item(OTHERS));
    Fake {
        fake,
        first,
        first_phrase,
        second,
        second_phrase,
        list_mode,
        restore_mode,
        list_delay_ms,
        restore_hold,
    }
}

fn engine_at(base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "trash-test-slot".into(),
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

/// Pump until no read and no restore is on its way (at most 10 s); every notice on the way, in order.
fn settle(source: &mut EngineTrashSource<MemoryCredentials>) -> Vec<Notice> {
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

fn opened(source: &mut EngineTrashSource<MemoryCredentials>) -> Vec<Notice> {
    source.opened();
    settle(source)
}

fn listings(fake: &FakeApi) -> usize {
    fake.requests().iter().filter(|r| is_listing(r)).count()
}

fn names(source: &EngineTrashSource<MemoryCredentials>) -> Vec<String> {
    source.items().into_iter().map(|row| row.name).collect()
}

#[test]
fn the_trash_is_the_accounts_newest_deleted_first_with_names_decrypted_in_the_engine() {
    let fake = fake("rows");
    let mut source = EngineTrashSource::new(signed_in(&fake));
    // Drawing never reads: the window may draw a thousand times before Trash opens.
    for _ in 0..20 {
        let _ = (source.items(), source.totals(), source.loaded(), source.count());
    }
    assert_eq!(source.loaded(), Loaded::Loading);
    assert_eq!(
        listings(&fake.fake),
        0,
        "nothing is read before the Trash section opens"
    );

    assert!(opened(&mut source).is_empty());
    assert_eq!(source.loaded(), Loaded::Ready);
    let rows = source.items();
    let seen: Vec<_> = rows
        .iter()
        .map(|row| (row.id.as_str(), row.name.as_str(), row.kind, row.size.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            (PHOTO, "IMG_0001.HEIC", Kind::Photo(None), "2.0 MB"),
            (CONTRACT, "Invented contract.pdf", Kind::Document, "1.1 MB"),
            (DOCS, "Invented old invoices", Kind::Folder, ""),
        ],
        "only what is in Trash, newest deleted first, each name decrypted"
    );
    for row in &rows {
        assert_eq!(
            row.extra,
            Extra::Words(String::new()),
            "where it was: not bound (local state)"
        );
        assert!(!row.date.is_empty());
    }
    assert_eq!(
        source.totals(),
        TrashTotals {
            items: 3,
            size: crate::source::format_size(1_153_434 + 2_097_152),
        }
    );
    assert_eq!(source.count(), None, "no badge: the engine has no count call");
    assert!(!source.actions_available());
    assert_eq!(source.emptying(), None);

    // Opening again while the list is fresh reads nothing.
    assert!(opened(&mut source).is_empty());
    assert_eq!(listings(&fake.fake), 1);
}

#[test]
fn an_empty_trash_is_read_as_empty() {
    let fake = fake("empty");
    let engine = engine_at(&fake.fake.base);
    sign_in(&engine, &fake.second, &fake.second_phrase);
    fake.second.restore_item(OTHERS);
    let mut source = EngineTrashSource::new(engine);
    opened(&mut source);
    assert_eq!(
        source.loaded(),
        Loaded::Ready,
        "an empty Trash it has read is an answer"
    );
    assert!(source.items().is_empty());
}

#[test]
fn a_failed_first_read_is_drawn_and_a_failed_re_read_keeps_the_list_and_says_so() {
    let fake = fake("error");
    fake.list_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let mut source = EngineTrashSource::new(signed_in(&fake));
    opened(&mut source);
    assert_eq!(source.loaded(), Loaded::Failed);
    assert!(source.items().is_empty());

    fake.list_mode.store(ANSWER, Ordering::SeqCst);
    source.expire();
    opened(&mut source);
    assert_eq!(source.items().len(), 3);

    fake.list_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.expire();
    assert_eq!(opened(&mut source), [Notice::RefreshFailed]);
    assert_eq!(source.items().len(), 3, "the rows already read stay");
    assert_eq!(source.loaded(), Loaded::Ready);
}

#[test]
fn offline_is_drawn_as_offline() {
    let fake = fake("offline");
    let engine = signed_in(&fake);
    let Fake { fake: server, .. } = fake;
    drop(server);
    let mut source = EngineTrashSource::new(engine);
    opened(&mut source);
    assert_eq!(source.loaded(), Loaded::Offline);
}

#[test]
fn a_session_that_ended_shows_nothing_and_drawing_asks_nothing() {
    let fake = fake("ended");
    let mut source = EngineTrashSource::new(signed_in(&fake));
    opened(&mut source);
    assert_eq!(source.items().len(), 3);

    fake.list_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.expire();
    opened(&mut source);
    assert_eq!(source.loaded(), Loaded::SessionEnded);
    assert!(source.items().is_empty());
    let asked = listings(&fake.fake);
    for _ in 0..20 {
        let _ = (source.items(), source.totals(), source.pump());
    }
    assert_eq!(listings(&fake.fake), asked, "no retry from draw calls or polls");
}

#[test]
fn signed_out_is_drawn_as_signed_out_and_asks_nothing() {
    let fake = fake("out");
    let mut source = EngineTrashSource::new(engine_at(&fake.fake.base));
    opened(&mut source);
    assert_eq!(source.loaded(), Loaded::SignedOut);
    assert!(source.items().is_empty());
    assert_eq!(listings(&fake.fake), 0);
}

#[test]
fn a_restored_item_leaves_at_once_and_does_not_come_back() {
    let fake = fake("restore");
    let mut source = EngineTrashSource::new(signed_in(&fake));
    opened(&mut source);

    assert!(!source.restore("an-item-not-in-trash"), "no such row");
    assert!(source.restore(CONTRACT));
    assert_eq!(
        names(&source),
        ["IMG_0001.HEIC", "Invented old invoices"],
        "the row leaves at once"
    );
    assert_eq!(source.totals().items, 2);
    // Every poll on the way: never drawn again.
    let began = Instant::now();
    loop {
        let pumped = source.pump();
        assert!(
            !names(&source).contains(&"Invented contract.pdf".to_string()),
            "drawn again on the way"
        );
        if !pumped.busy && !source.busy() {
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    let restores = fake
        .fake
        .requests()
        .iter()
        .filter(|r| r.route() == format!("POST /api/v1/files/{CONTRACT}/restore"))
        .count();
    assert_eq!(restores, 1, "sent once");
    assert_eq!(listings(&fake.fake), 2, "read again once Beebeeb answered");
}

#[test]
fn a_read_on_its_way_when_a_restore_lands_does_not_bring_the_row_back() {
    let fake = fake("race");
    let mut source = EngineTrashSource::new(signed_in(&fake));
    opened(&mut source);
    // A slow read that saw the item still in Trash, and the restore answered before it lands.
    fake.list_delay_ms.store(600, Ordering::SeqCst);
    source.expire();
    source.opened();
    std::thread::sleep(Duration::from_millis(100));
    assert!(source.restore(CONTRACT));
    fake.list_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
    assert_eq!(names(&source), ["IMG_0001.HEIC", "Invented old invoices"]);
    assert_eq!(listings(&fake.fake), 3, "read once more after the read on its way");
}

#[test]
fn a_restore_then_a_failed_re_read_does_not_bring_the_row_back() {
    let fake = fake("reread");
    let mut source = EngineTrashSource::new(signed_in(&fake));
    opened(&mut source);
    fake.list_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.restore(PHOTO));
    assert_eq!(
        settle(&mut source),
        [Notice::RefreshFailed],
        "the failed re-read is drawn"
    );
    assert_eq!(names(&source), ["Invented contract.pdf", "Invented old invoices"]);
}

#[test]
fn a_refused_restore_brings_the_row_back_and_says_so() {
    let fake = fake("refused");
    fake.restore_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let mut source = EngineTrashSource::new(signed_in(&fake));
    opened(&mut source);
    assert!(source.restore(PHOTO));
    assert_eq!(source.items().len(), 2);
    assert_eq!(
        settle(&mut source),
        [Notice::RestoreRefused("IMG_0001.HEIC".into())],
        "the view draws it"
    );
    assert_eq!(
        names(&source),
        ["IMG_0001.HEIC", "Invented contract.pdf", "Invented old invoices"]
    );
}

#[test]
fn a_lock_drops_every_name_at_once_and_another_account_never_sees_them() {
    let fake = fake("session");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    opened(&mut source);
    assert_eq!(source.items().len(), 3);

    let _ = engine.lock();
    // Before any poll: the accessors already hand out nothing of the locked session.
    assert!(source.items().is_empty());
    assert_eq!(source.totals().items, 0);
    assert_eq!(source.loaded(), Loaded::SignedOut);
    assert!(source.pump().changed, "the poll clears the list it held");

    // The same handle, signed out, then the second account: only its own Trash.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    assert!(source.items().is_empty(), "nothing of the first account");
    opened(&mut source);
    assert_eq!(names(&source), ["Second account budget.xlsx"]);
}

#[test]
fn a_listing_made_for_a_session_that_ended_is_dropped() {
    let fake = fake("late");
    let engine = signed_in(&fake);
    let mut source = EngineTrashSource::new(engine.clone());
    fake.list_delay_ms.store(500, Ordering::SeqCst);
    source.opened();
    std::thread::sleep(Duration::from_millis(100));
    let _ = engine.lock();
    settle(&mut source);
    assert!(
        source.items().is_empty(),
        "an answer that outlived its session is never drawn"
    );
    assert_eq!(source.loaded(), Loaded::SignedOut);
}

/// Every text and label in the window's element tree.
fn words(rt: &Runtime<State>) -> String {
    rt.elements()
        .into_iter()
        .filter(|e| !e.exiting)
        .flat_map(|e| [e.label.unwrap_or_default(), e.text.unwrap_or_default()])
        .collect::<Vec<_>>()
        .join("\n")
}

/// Draw, and let what moves come to rest.
fn draw(rt: &mut Runtime<State>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

/// Poll as the window does, then draw.
fn poll(rt: &mut Runtime<State>) {
    let began = Instant::now();
    loop {
        rt.update(|s, tx| {
            crate::frames::trash::poll(s, tx);
        });
        draw(rt);
        let busy = rt.state.frames.trash.source.pump().busy;
        if !busy {
            rt.update(|s, tx| {
                crate::frames::trash::poll(s, tx);
            });
            draw(rt);
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_window_shows_no_name_of_a_locked_session_or_of_the_account_before() {
    let fake = fake("window");
    let engine = signed_in(&fake);
    let mut state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine.clone());
    crate::frames::go(&mut state, Section::Trash);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (1278.0, 798.0), env);
    poll(&mut rt);
    let seen = words(&rt);
    assert!(seen.contains("Invented contract.pdf"), "the Trash is drawn: {seen}");
    assert!(
        !rt.elements().iter().any(|e| e.key.as_deref() == Some("Trash-badge")),
        "no badge from the engine"
    );

    let _ = engine.lock();
    poll(&mut rt);
    let seen = words(&rt);
    for name in ["Invented contract.pdf", "IMG_0001.HEIC", "Invented old invoices"] {
        assert!(!seen.contains(name), "{name} is still drawn after the lock");
    }
    assert!(seen.contains("Signed out"), "the signed-out state is drawn: {seen}");

    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    // The section is re-opened by the person (the window's gate went round): leave and come back.
    crate::frames::go(&mut rt.state, Section::Files);
    poll(&mut rt);
    crate::frames::go(&mut rt.state, Section::Trash);
    poll(&mut rt);
    let seen = words(&rt);
    for name in ["Invented contract.pdf", "IMG_0001.HEIC", "Invented old invoices"] {
        assert!(
            !seen.contains(name),
            "{name} of the first account is drawn for the second"
        );
    }
    assert!(
        seen.contains("Second account budget.xlsx"),
        "the second account's Trash: {seen}"
    );
}

#[test]
fn a_time_reads_as_today_yesterday_or_its_date_on_this_computers_clock() {
    let now = Local.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).single().expect("a time");
    let at = |d: u32, h: u32, m: u32| {
        Local
            .with_ymd_and_hms(2026, 11, d, h, m, 0)
            .single()
            .unwrap()
            .timestamp()
    };
    assert_eq!(when(at(5, 7, 2), now), "Today, 07:02");
    assert_eq!(when(at(4, 16, 42), now), "Yesterday, 16:42");
    assert_eq!(when(at(2, 9, 0), now), "2 Nov 2026");
    assert_eq!(seconds_of("2026-11-02T10:00:00Z"), Some(1_793_613_600));
    assert_eq!(seconds_of("1700000001"), Some(1_700_000_001));
    assert_eq!(seconds_of("not a time"), None);
}
