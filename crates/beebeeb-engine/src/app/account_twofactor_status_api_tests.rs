//! The two-factor status read on a real handle against the engine's fake account on loopback. Every value is invented.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Reply, Request};

const PASSWORD: &str = "an invented password for two-factor status";

struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    status: Arc<AtomicU16>,
    body: Arc<std::sync::Mutex<Value>>,
}

fn fake(tag: &str) -> Fake {
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let status = Arc::new(AtomicU16::new(200));
    let body = Arc::new(std::sync::Mutex::new(json!({
        "enabled": true, "added_at": "2026-10-28T09:00:00Z", "last_used_at": "2026-11-14T08:12:00Z",
        "backup_codes_left": 6, "backup_codes_total": 8
    })));
    let (code, answer, token) = (status.clone(), body.clone(), account.session_token.clone());
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        if req.route() != "GET /api/v1/me/two-factor" {
            return rest(req);
        }
        if req.bearer() != Some(token.as_str()) {
            return Reply::error(401, "unauthorized");
        }
        match code.load(Ordering::SeqCst) {
            200 => Reply::ok(answer.lock().unwrap().clone()),
            status => Reply::error(status, "refused"),
        }
    }));
    Fake {
        api,
        account,
        phrase,
        status,
        body,
    }
}

fn engine(fake: &Fake, sign_in: bool) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "two-factor-status-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    if sign_in {
        engine
            .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
            .expect("the fake accepts the password");
        engine
            .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
            .expect("the phrase unlocks");
    }
    engine
}

fn asked(fake: &Fake) -> usize {
    fake.api
        .routes()
        .iter()
        .filter(|route| route.as_str() == "GET /api/v1/me/two-factor")
        .count()
}

#[test]
fn the_status_is_beebeebs_own_values_for_the_session_in_memory() {
    let fake = fake("status");
    let engine = engine(&fake, true);
    let read = engine.two_factor_status().expect("the status reads");
    assert_eq!(read.session, engine.session_turn());
    let status = read.value;
    assert!(status.enabled);
    assert_eq!(status.added_at.as_deref(), Some("2026-10-28T09:00:00Z"));
    assert_eq!(status.last_used_at.as_deref(), Some("2026-11-14T08:12:00Z"));
    assert_eq!(
        (status.backup_codes_left, status.backup_codes_total),
        (Some(6), Some(8))
    );
    assert_eq!(asked(&fake), 1);

    // Off and never used: the values Beebeeb does not give are `None`, never made up.
    *fake.body.lock().unwrap() = json!({ "enabled": false, "added_at": null, "last_used_at": null });
    let off = engine.two_factor_status().expect("reads").value;
    assert!(!off.enabled);
    assert_eq!(
        (off.added_at, off.last_used_at, off.backup_codes_left),
        (None, None, None)
    );
}

#[test]
fn a_failed_read_says_its_kind_and_signed_out_asks_nothing() {
    let fake = fake("failed");
    let engine = engine(&fake, true);
    fake.status.store(500, Ordering::SeqCst);
    assert_eq!(engine.two_factor_status().err(), Some(AccountCallError::ServerError));
    fake.status.store(200, Ordering::SeqCst);
    *fake.body.lock().unwrap() = json!("not an object");
    assert!(
        engine.two_factor_status().is_err(),
        "an answer that does not read is an error"
    );

    let signed_out = self::engine(&fake, false);
    let before = asked(&fake);
    assert_eq!(
        signed_out.two_factor_status().err(),
        Some(AccountCallError::NotSignedIn)
    );
    assert_eq!(asked(&fake), before);
}
