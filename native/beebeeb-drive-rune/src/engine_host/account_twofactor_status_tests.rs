//! The two-factor status on the real engine: Beebeeb's values shown, a read that failed showing none, an answer that
//! lands after the session moved on dropped, and the dates in the list's own words. Every value is invented.

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Reply};
use chrono::{TimeZone, Timelike, Utc};
use serde_json::json;
use zeroize::Zeroizing;

use super::*;

const PASSWORD: &str = "an invented password for two-factor status";

struct Fake {
    api: FakeApi,
    status: Arc<AtomicU16>,
    delay_ms: Arc<AtomicU64>,
    account: Arc<FakeAccount>,
    phrase: String,
}

fn fake() -> Fake {
    let (account, phrase) = FakeAccount::register("two-factor-status@example.eu", PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let (status, delay_ms) = (Arc::new(AtomicU16::new(200)), Arc::new(AtomicU64::new(0)));
    let body = Arc::new(Mutex::new(json!({
        "enabled": true, "added_at": "2026-10-28T09:00:00Z", "last_used_at": "2026-11-14T08:12:00Z",
        "backup_codes_left": 2, "backup_codes_total": 8
    })));
    let (code, delay, answer) = (status.clone(), delay_ms.clone(), body.clone());
    let api = FakeApi::start(Arc::new(move |req| {
        if req.route() != "GET /api/v1/me/two-factor" {
            return rest(req);
        }
        std::thread::sleep(Duration::from_millis(delay.load(Ordering::SeqCst)));
        match code.load(Ordering::SeqCst) {
            200 => Reply::ok(answer.lock().unwrap().clone()),
            status => Reply::error(status, "refused"),
        }
    }));
    Fake {
        api,
        status,
        delay_ms,
        account,
        phrase,
    }
}

fn signed_in(fake: &Fake) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "two-factor-status-host-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);
    engine
        .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    engine
}

fn asked(fake: &Fake) -> usize {
    fake.api
        .routes()
        .iter()
        .filter(|route| route.as_str() == "GET /api/v1/me/two-factor")
        .count()
}

fn settle(part: &mut StatusPart<MemoryCredentials>) {
    let began = Instant::now();
    loop {
        let (_, busy) = part.pump();
        if !busy {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the status did not answer");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_status_is_beebeebs_own_values_read_when_a_pane_opens_and_not_again_while_fresh() {
    let fake = fake();
    let mut part = StatusPart::new(signed_in(&fake));
    assert_eq!(part.known(), None, "nothing is drawn before Beebeeb answered");
    part.opened(Pane::TwoFactor);
    assert_eq!(part.known(), None, "asked, not answered");
    settle(&mut part);
    let two = part.known().expect("read");
    assert!(two.on);
    assert_eq!((two.codes_left, two.codes_total), (2, 8));
    assert!(!two.added.is_empty() && !two.last_used.is_empty());
    assert_eq!(asked(&fake), 1);
    // Opened again while fresh: nothing is asked again; a change makes it stale and it is read again.
    part.opened(Pane::TwoFactor);
    settle(&mut part);
    assert_eq!(asked(&fake), 1);
    part.stale();
    settle(&mut part);
    assert_eq!(asked(&fake), 2);
}

#[test]
fn a_read_that_failed_draws_no_value_and_is_not_asked_again_by_a_poll() {
    let fake = fake();
    fake.status.store(500, Ordering::SeqCst);
    let mut part = StatusPart::new(signed_in(&fake));
    part.opened(Pane::Security);
    settle(&mut part);
    assert_eq!(part.known(), None, "never a made-up value");
    for _ in 0..5 {
        part.pump();
    }
    assert_eq!(asked(&fake), 1, "a poll does not ask a failed read again");
    // Opened again: asked again, and the values show.
    fake.status.store(200, Ordering::SeqCst);
    part.opened(Pane::TwoFactor);
    settle(&mut part);
    assert!(part.known().is_some());
}

#[test]
fn an_answer_after_the_session_moved_on_is_dropped() {
    let fake = fake();
    fake.delay_ms.store(300, Ordering::SeqCst);
    let engine = signed_in(&fake);
    let mut part = StatusPart::new(engine.clone());
    part.opened(Pane::TwoFactor);
    engine.lock();
    std::thread::sleep(Duration::from_millis(600));
    part.pump();
    assert_eq!(part.known(), None);
    // A new session of the same account starts with nothing of the first one's read.
    fake.delay_ms.store(0, Ordering::SeqCst);
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(part.known(), None);
}

#[test]
fn what_beebeeb_does_not_give_stays_empty_and_dates_use_the_lists_words() {
    let now = Local.with_ymd_and_hms(2026, 11, 14, 18, 0, 0).unwrap();
    let on = |last: Option<&str>, left: Option<u32>, total: Option<u32>| TwoFactorStatus {
        enabled: true,
        added_at: Some("2026-10-28T12:00:00Z".into()),
        last_used_at: last.map(str::to_string),
        backup_codes_left: left,
        backup_codes_total: total,
    };
    let at = |hour| now.with_hour(hour).unwrap().with_timezone(&Utc).to_rfc3339();
    let today = two_factor_of(&on(Some(&at(9)), Some(8), Some(8)), now);
    assert_eq!(today.last_used, "Today, 09:00");
    assert_eq!((today.codes_left, today.codes_total), (8, 8));
    let yesterday = (now - chrono::Duration::days(1)).with_timezone(&Utc).to_rfc3339();
    assert_eq!(
        two_factor_of(&on(Some(&yesterday), None, None), now).last_used,
        "Yesterday, 18:00"
    );
    // The words come from the local clock: 40 days back can cross a change of summer time, so the expectation is
    // read from the same local time, never a fixed hour.
    let long_ago_local = now - chrono::Duration::days(40);
    let long_ago = long_ago_local.with_timezone(&Utc).to_rfc3339();
    assert_eq!(
        two_factor_of(&on(Some(&long_ago), None, None), now).last_used,
        long_ago_local.format("%-d %b, %H:%M").to_string()
    );
    // Never used, no counts: those rows are empty (the page hides them); a count above its total is clamped.
    let bare = two_factor_of(&on(None, None, None), now);
    assert_eq!((bare.last_used.as_str(), bare.codes_total), ("", 0));
    assert_eq!(two_factor_of(&on(None, Some(9), Some(8)), now).codes_left, 8);
    // Off: nothing but "off".
    let mut off = on(Some(&at(9)), Some(8), Some(8));
    off.enabled = false;
    let off = two_factor_of(&off, now);
    assert!(!off.on && off.added.is_empty() && off.last_used.is_empty() && off.codes_total == 0);
}
