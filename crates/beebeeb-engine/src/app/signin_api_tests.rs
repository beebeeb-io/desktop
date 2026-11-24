//! The wipe of a half-done password sign-in: what the engine holds between the password
//! and the end of the sign-in goes when the sign-in ends, however it ends. Two copies are held: the proven password's
//! code challenge (`pending_2fa`: core's partial token, and the email beside it) and the password's stretch for the
//! screen-lock check (`lock_password`: core's check key). Each wipes as it drops, so "gone" here is "dropped".
//!
//! A real handle against the engine's fake account on loopback, with a code step and a profile read that can fail put
//! in front of it. Every name, address and code is invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use beebeeb_core::auth_flow::sign_in::PendingSecondFactor;
use serde_json::json;
use zeroize::Zeroizing;

use super::super::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::signin::gate::NextAfterSignIn;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{FakeApi, Handler, Reply, Request};

const PASSWORD: &str = "an invented password for the wipe";
const CODE: &str = "246810";

/// The fake: the account's own routes, with a code step after the password when `two_factor` is on, and a profile read
/// that answers 500 while `profile_fails` is on.
struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    profile_fails: Arc<AtomicBool>,
}

fn fake(tag: &str, two_factor: bool) -> Fake {
    let (account, _) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let profile_fails = Arc::new(AtomicBool::new(false));
    let partials: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (fails, token) = (profile_fails.clone(), account.session_token.clone());
    let handler: Handler = Arc::new(move |req: &Request| match req.route().as_str() {
        "POST /api/v1/opaque/login-finish" if two_factor => {
            let proven = rest(req);
            if proven.status != 200 {
                return proven;
            }
            let partial = format!("partial-{}", uuid::Uuid::new_v4());
            partials.lock().unwrap().push(partial.clone());
            Reply::ok(json!({ "requires_2fa": true, "partial_token": partial }))
        }
        "POST /api/v1/auth/2fa/verify" => {
            let body = req.json();
            let known = body["partial_token"]
                .as_str()
                .is_some_and(|partial| partials.lock().unwrap().iter().any(|p| p == partial));
            if known && body["code"] == CODE {
                Reply::ok(json!({ "session_token": token }))
            } else {
                Reply::error(401, "invalid_code")
            }
        }
        "GET /api/v1/auth/me" if fails.load(Ordering::SeqCst) => Reply::error(500, "server_error"),
        _ => rest(req),
    });
    Fake {
        api: FakeApi::start(handler),
        account,
        profile_fails,
    }
}

fn opened(fake: &Fake, slot: &str) -> Engine<MemoryCredentials> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("signin-wipe-{slot}"),
        state_dir: crate::state_paths::init_for_test(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    engine
}

fn secret(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_string())
}

/// What a half-done sign-in holds here: (the code challenge, the password's stretch).
fn half_done(engine: &Engine<MemoryCredentials>) -> (bool, bool) {
    // The slot's type is pinned: the email beside the challenge is a wiping buffer, and core's challenge holds its
    // partial token in one (`PendingSecondFactor`).
    let challenge: &Mutex<Option<(PendingSecondFactor, Zeroizing<String>)>> = &engine.pending_2fa;
    (
        challenge.lock().unwrap().is_some(),
        engine.lock_password.holds_a_stretch(),
    )
}

/// The password was proven and the code is due: both copies are held (the precondition every test below starts from,
/// so a test cannot pass because nothing was ever held).
fn at_the_code(engine: &Engine<MemoryCredentials>, fake: &Fake) {
    assert_eq!(
        engine.sign_in_password(&fake.account.email, secret(PASSWORD)),
        Ok(NextAfterSignIn::TwoFactor)
    );
    assert_eq!(half_done(engine), (true, true), "a proven password waits for its code");
}

/// Cancel: the person goes back from the code. The challenge AND the stretch go.
#[test]
fn signin_wipe_leaving_the_code_step_wipes_the_challenge_and_the_stretch() {
    let fake = fake("wipe-cancel", true);
    let engine = opened(&fake, "cancel");
    at_the_code(&engine, &fake);
    engine.discard_partial_sign_in().expect("nothing is signed in to end");
    assert_eq!(half_done(&engine), (false, false), "nothing of the left sign-in stays");
}

/// Failure after the code: the server accepted the code (the challenge is spent) but the session could not be
/// installed (the profile read failed, the new session was ended). Nothing of the sign-in stays.
#[test]
fn signin_wipe_a_code_accepted_then_a_failed_install_wipes_the_challenge_and_the_stretch() {
    let fake = fake("wipe-failed-code", true);
    let engine = opened(&fake, "failed-code");
    at_the_code(&engine, &fake);
    fake.profile_fails.store(true, Ordering::SeqCst);
    assert!(engine.two_factor(secret(CODE)).is_err(), "the install failed");
    assert_eq!(
        half_done(&engine),
        (false, false),
        "nothing of the failed sign-in stays"
    );
}

/// Failure without a code step: the password was proven (the stretch was made) but the install failed.
#[test]
fn signin_wipe_a_failed_install_after_the_password_wipes_the_stretch() {
    let fake = fake("wipe-failed-password", false);
    let engine = opened(&fake, "failed-password");
    fake.profile_fails.store(true, Ordering::SeqCst);
    assert!(
        engine.sign_in_password(&fake.account.email, secret(PASSWORD)).is_err(),
        "the install failed"
    );
    assert_eq!(
        half_done(&engine),
        (false, false),
        "the stretch of a failed sign-in is gone"
    );
}

/// Replaced: a new sign-in while a proven password waits for its code. The older one's challenge and stretch go,
/// whatever the new one's answer (here a wrong password).
#[test]
fn signin_wipe_a_new_sign_in_replaces_the_half_done_one() {
    let fake = fake("wipe-replaced", true);
    let engine = opened(&fake, "replaced");
    at_the_code(&engine, &fake);
    assert!(
        engine
            .sign_in_password(&fake.account.email, secret("not the password at all"))
            .is_err()
    );
    assert_eq!(half_done(&engine), (false, false), "the replaced sign-in left nothing");
}

/// Success: the code completes the sign-in, and neither copy outlives it.
#[test]
fn signin_wipe_a_completed_sign_in_keeps_no_challenge_and_no_stretch() {
    let fake = fake("wipe-success", true);
    let engine = opened(&fake, "success");
    at_the_code(&engine, &fake);
    assert!(
        engine.two_factor(secret(CODE)).is_ok(),
        "the code completes the sign-in"
    );
    assert_eq!(half_done(&engine), (false, false), "nothing of the finished step stays");
}

/// A passkey or device sign-in (and a recovery) installs through `store_first_sign_in`, not through the password
/// step's own start: a proven password waiting for its code must not outlive the session that was stored instead.
#[test]
fn signin_wipe_a_stored_first_sign_in_forgets_the_half_done_password_step() {
    let fake = fake("wipe-stored", true);
    let engine = opened(&fake, "stored");
    at_the_code(&engine, &fake);
    let token = fake.account.session_token.clone();
    let profile = engine
        .runtime
        .block_on(super::super::fetch_profile(&engine.client, &fake.api.base, &token))
        .expect("the fake answers the profile");
    let turn = engine.session_turn();
    engine
        .store_first_sign_in(turn, &token, &fake.account.email, &profile)
        .expect("the session is stored");
    assert_eq!(
        half_done(&engine),
        (false, false),
        "the stored session ended the half-done step"
    );
}
