//! The code sent again and the mirrored email, on a real handle against the engine's fake account. Every address
//! and code is invented.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use zeroize::Zeroizing;

use super::super::{Engine, EngineConfig};
use super::*;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Reply, Request};

const PASSWORD: &str = "an invented password for the email resend";

struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    /// The status the resend answers with (200: sent; 404, 429 wait, 429 limit as 429 + the code below).
    status: Arc<AtomicU16>,
    limit: Arc<std::sync::atomic::AtomicBool>,
}

fn fake(tag: &str) -> Fake {
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let status = Arc::new(AtomicU16::new(200));
    let limit = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (code, limited, token) = (status.clone(), limit.clone(), account.session_token.clone());
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        if req.route() != "POST /api/v1/me/email/resend" {
            return rest(req);
        }
        if req.bearer() != Some(token.as_str()) {
            return Reply::error(401, "unauthorized");
        }
        match code.load(Ordering::SeqCst) {
            200 => Reply::ok(serde_json::json!({
                "pending_email": "new.address@example.eu", "code_expires_in": 900, "resends_left": 2
            })),
            429 if limited.load(Ordering::SeqCst) => Reply::error(429, "email_code_resend_limit"),
            429 => Reply::error(429, "email_code_resend_wait").retry_after(42),
            404 => Reply::error(404, "no_pending_email_change"),
            status => Reply::error(status, "refused"),
        }
    }));
    Fake {
        api,
        account,
        phrase,
        status,
        limit,
    }
}

fn engine(fake: &Fake, sign_in: bool) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "email-resend-slot".into(),
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
        .filter(|route| route.as_str() == "POST /api/v1/me/email/resend")
        .count()
}

#[test]
fn a_resend_is_one_request_with_the_servers_words_and_each_refusal_is_its_kind() {
    let fake = fake("resend");
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    let sent = engine.email_change_resend(session).expect("sent");
    assert_eq!(sent.pending_email, "new.address@example.eu");
    assert_eq!((sent.code_expires_in, sent.resends_left), (900, 2));
    assert_eq!(asked(&fake), 1);

    fake.status.store(404, Ordering::SeqCst);
    assert_eq!(engine.email_change_resend(session), Err(EmailResendError::Gone));
    // A wait carries Retry-After; the limit is final. Each press is ONE request: a 429 is never ridden out.
    fake.status.store(429, Ordering::SeqCst);
    assert_eq!(engine.email_change_resend(session), Err(EmailResendError::Wait(42)));
    fake.limit.store(true, Ordering::SeqCst);
    assert_eq!(engine.email_change_resend(session), Err(EmailResendError::Limit));
    assert_eq!(asked(&fake), 4, "four presses, four requests, no hidden retry");
    fake.status.store(500, Ordering::SeqCst);
    assert_eq!(engine.email_change_resend(session), Err(EmailResendError::Failed));
}

#[test]
fn another_session_or_no_session_sends_nothing() {
    let fake = fake("resend-session");
    let engine = engine(&fake, true);
    let stale = crate::account::SessionGeneration::for_test(1);
    assert_eq!(engine.email_change_resend(stale), Err(EmailResendError::NotSignedIn));
    let signed_out = self::engine(&fake, false);
    assert_eq!(
        signed_out.email_change_resend(signed_out.session_turn()),
        Err(EmailResendError::NotSignedIn)
    );
    assert_eq!(asked(&fake), 0);
}

#[test]
fn a_confirmed_change_replaces_the_mirrored_email_for_that_session_only() {
    let fake = fake("mirror");
    let engine = engine(&fake, true);
    let session = engine.session_turn();
    let old = engine.account_email();
    assert!(old.is_some());
    // Nothing to store, or another session: the old address stays.
    assert!(engine.email_changed_for(session, "  ").is_none());
    let stale = crate::account::SessionGeneration::for_test(1);
    assert!(engine.email_changed_for(stale, "new.address@example.eu").is_none());
    assert_eq!(engine.account_email(), old);

    assert_eq!(
        engine.email_changed_for(session, "new.address@example.eu"),
        Some(engine.session_turn()),
        "it hands back the generation its own write moved to"
    );
    assert_eq!(engine.account_email().as_deref(), Some("new.address@example.eu"));
    assert_eq!(
        engine.vault().account_email().ok().flatten().as_deref(),
        Some("new.address@example.eu"),
        "the credential store (the Continue-as name) holds it too"
    );
    assert_ne!(
        engine.session_turn(),
        session,
        "a write of the session moves the generation on"
    );
}
