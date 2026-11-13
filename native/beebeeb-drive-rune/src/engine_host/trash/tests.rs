//! Trash on the real engine (`EngineTrashSource`), against the engine's fake account on loopback with the Trash
//! listing answered in the server's shape here: the rows the view draws, newest deleted first with their names
//! decrypted in the engine; an empty Trash; a server error; a session that ended (not asked again at once); signed
//! out (nothing asked); a restore, a refused restore and an unknown item. An engine handle is on the process's state
//! folder (the engine refuses another), which nothing here writes in. Every name is invented.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Reply, Request};
use chrono::TimeZone;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;

const PASSWORD: &str = "an invented password for trash";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;

const DOCS: &str = "a0000000-0000-4000-8000-000000000201";
const CONTRACT: &str = "a0000000-0000-4000-8000-000000000202";
const PHOTO: &str = "a0000000-0000-4000-8000-000000000203";
const KEPT: &str = "a0000000-0000-4000-8000-000000000204";

struct Account {
    fake: FakeApi,
    vault: Arc<FakeAccount>,
    /// How the Trash listing answers.
    list_mode: Arc<AtomicU8>,
    /// How a restore answers.
    restore_mode: Arc<AtomicU8>,
    email: String,
    phrase: String,
}

fn is_listing(req: &Request) -> bool {
    req.route() == "GET /api/v1/files" && req.query("trashed") == Some("true")
}

fn account(email: &str) -> Account {
    let (vault, phrase) = FakeAccount::register(email, PASSWORD, TEST_PHRASE);
    let vault = Arc::new(vault);
    let token = vault.session_token.clone();
    let rest = account_handler(vault.clone());
    let (list_mode, restore_mode) = (Arc::new(AtomicU8::new(ANSWER)), Arc::new(AtomicU8::new(ANSWER)));
    let (list_in, restore_in) = (list_mode.clone(), restore_mode.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let restoring = route.starts_with("POST /api/v1/files/") && route.ends_with("/restore");
        if !is_listing(req) && !restoring {
            return rest(req);
        }
        let mode = if restoring { &restore_in } else { &list_in };
        match mode.load(Ordering::SeqCst) {
            SERVER_ERROR => return Reply::json(500, json!({ "error": "a server body" })),
            SESSION_ENDED => return Reply::json(401, json!({ "error": "a server body" })),
            _ => {}
        }
        if restoring || req.bearer() != Some(token.as_str()) {
            return rest(req);
        }
        // The fake answers `items`; the server (and the engine's client) say `files` and `next_cursor`.
        let body: Value = serde_json::from_slice(&rest(req).body).expect("the fake's listing");
        Reply::ok(json!({ "files": body["items"], "next_cursor": null }))
    }));
    Account {
        fake,
        vault,
        list_mode,
        restore_mode,
        email: email.to_string(),
        phrase,
    }
}

/// The account's vault: a folder, a PDF and a photo in Trash, one file not.
fn fill(account: &Account) {
    let vault = &account.vault;
    vault.add_item(DOCS, None, "Invented old invoices", true, 0);
    vault.add_item(CONTRACT, None, "Invented contract.pdf", false, 1_153_434);
    vault.add_item(PHOTO, None, "IMG_0001.HEIC", false, 2_097_152);
    vault.add_item(KEPT, None, "Not deleted.txt", false, 10);
    for id in [DOCS, CONTRACT, PHOTO] {
        assert!(vault.trash_item(id));
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

fn signed_in(account: &Account) -> Arc<Engine<MemoryCredentials>> {
    let engine = engine_at(&account.fake.base);
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(account.phrase.clone()))
        .expect("the phrase unlocks");
    engine
}

/// Wait until no read and no restore is on its way (at most 10 s).
fn settle(source: &EngineTrashSource<MemoryCredentials>) {
    let began = Instant::now();
    while source.busy() {
        assert!(began.elapsed() < Duration::from_secs(10), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn listings(fake: &FakeApi) -> usize {
    fake.requests().iter().filter(|r| is_listing(r)).count()
}

fn names(source: &EngineTrashSource<MemoryCredentials>) -> Vec<String> {
    source.items().into_iter().map(|row| row.name).collect()
}

#[test]
fn the_trash_is_the_accounts_newest_deleted_first_with_names_decrypted_in_the_engine() {
    let account = account("trash-rows@example.eu");
    fill(&account);
    let source = EngineTrashSource::new(signed_in(&account));
    assert_eq!(
        listings(&account.fake),
        0,
        "nothing is asked before the view draws Trash"
    );
    assert!(!source.wired(), "not read yet: the shell's frame, never an empty Trash");
    settle(&source);
    assert!(source.wired());

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
        assert_eq!(row.extra, Extra::Words("From Files".into()), "at the top of Files");
        assert!(!row.date.is_empty(), "the deleted date is drawn");
        assert_eq!(row.pc, None);
    }
    assert_eq!(
        source.totals(),
        TrashTotals {
            items: 3,
            size: crate::source::format_size(1_153_434 + 2_097_152),
        }
    );
    assert_eq!(source.emptying(), None, "not bound: no empty-Trash call in the engine");
    assert_eq!(listings(&account.fake), 1, "read once while fresh");
}

#[test]
fn an_empty_trash_is_read_as_empty() {
    let account = account("trash-empty@example.eu");
    account.vault.add_item(KEPT, None, "Not deleted.txt", false, 10);
    let source = EngineTrashSource::new(signed_in(&account));
    source.items();
    settle(&source);
    assert!(source.wired(), "an empty Trash it has read is an answer");
    assert!(source.items().is_empty());
    assert_eq!(source.totals().items, 0);
}

#[test]
fn a_server_error_draws_no_rows_it_has_not_read_and_keeps_those_it_has() {
    let account = account("trash-error@example.eu");
    fill(&account);
    account.list_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let source = EngineTrashSource::new(signed_in(&account));
    source.items();
    settle(&source);
    assert!(
        !source.wired(),
        "a failed first read: the shell's frame, never an empty Trash"
    );
    assert!(source.items().is_empty());
    assert_eq!(source.totals().items, 0);

    account.list_mode.store(ANSWER, Ordering::SeqCst);
    source.expire();
    source.items();
    settle(&source);
    assert_eq!(source.items().len(), 3);

    account.list_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.expire();
    source.items();
    settle(&source);
    assert_eq!(source.items().len(), 3, "a failed re-read keeps the rows already read");
}

#[test]
fn a_session_that_ended_shows_nothing_of_the_account_and_is_not_asked_again_at_once() {
    let account = account("trash-ended@example.eu");
    fill(&account);
    let source = EngineTrashSource::new(signed_in(&account));
    source.items();
    settle(&source);
    assert_eq!(source.items().len(), 3);

    account.list_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.expire();
    source.items();
    settle(&source);
    assert!(!source.wired(), "nothing of the account stays on screen");
    assert!(source.items().is_empty());
    let asked = listings(&account.fake);
    for _ in 0..20 {
        source.items();
        source.totals();
    }
    settle(&source);
    assert_eq!(
        listings(&account.fake),
        asked,
        "no retry loop: not asked again within FRESH_FOR"
    );
}

#[test]
fn signed_out_asks_nothing_and_draws_nothing() {
    let account = account("trash-out@example.eu");
    fill(&account);
    let source = EngineTrashSource::new(engine_at(&account.fake.base));
    assert!(!source.wired());
    assert!(source.items().is_empty());
    assert_eq!(source.totals().items, 0);
    settle(&source);
    assert_eq!(listings(&account.fake), 0, "nothing is asked without a session");
}

#[test]
fn a_restored_item_leaves_at_once_and_beebeeb_has_it_back() {
    let account = account("trash-restore@example.eu");
    fill(&account);
    let mut source = EngineTrashSource::new(signed_in(&account));
    source.items();
    settle(&source);

    assert!(!source.restore("an-item-not-in-trash"), "no such row");
    assert!(source.restore(CONTRACT));
    assert_eq!(
        names(&source),
        ["IMG_0001.HEIC", "Invented old invoices"],
        "the row leaves at once"
    );
    assert_eq!(source.totals().items, 2);
    settle(&source);
    assert_eq!(names(&source), ["IMG_0001.HEIC", "Invented old invoices"]);
    let restores = account
        .fake
        .requests()
        .iter()
        .filter(|r| r.route() == format!("POST /api/v1/files/{CONTRACT}/restore"))
        .count();
    assert_eq!(restores, 1, "sent once");
    assert_eq!(listings(&account.fake), 2, "read again once Beebeeb answered");
}

#[test]
fn a_refused_restore_brings_the_row_back() {
    let account = account("trash-refused@example.eu");
    fill(&account);
    account.restore_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let mut source = EngineTrashSource::new(signed_in(&account));
    source.items();
    settle(&source);

    assert!(source.restore(PHOTO));
    assert_eq!(source.items().len(), 2);
    settle(&source);
    assert_eq!(
        names(&source),
        ["IMG_0001.HEIC", "Invented contract.pdf", "Invented old invoices"],
        "Beebeeb still has it in Trash: it is drawn again"
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
