//! Change password on the real engine's side of the source: strength is core's, a typed password is kept for ONE
//! change under the session and account of its press, and each way the engine refuses is the form's own word. The
//! engine's flow itself (policy, breach check, OPAQUE legs, the stored token) is proved in
//! `app/account_password_api_tests.rs`. Every name is invented; the fake is on loopback.

use std::sync::Arc;

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::test_account;
use zeroize::Zeroizing;

use super::*;

type TestEngine = Arc<Engine<MemoryCredentials>>;

fn engine_on(base: &str) -> TestEngine {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "account-password-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

/// Signed in and unlocked on the fake of `test_account`.
fn signed_in() -> (TestEngine, beebeeb_engine::test_api::FakeApi) {
    let (_account, phrase, fake) = test_account();
    let engine = engine_on(&fake.base);
    engine
        .sign_in_password("test@example.eu", Zeroizing::new("correct horse battery".into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    (engine, fake)
}

#[test]
fn strength_is_cores_evaluator() {
    let calls = PasswordCalls::new(engine_on("http://127.0.0.1:9"));
    let of = |text: &str| calls.strength(&Password::from_field(text));
    assert_eq!(of(""), Strength::TooShort);
    assert_eq!(of("short"), Strength::TooShort);
    assert_eq!(of("alllowercaseletters"), Strength::Fair);
    assert_eq!(of("Mixed Case Letters"), Strength::Good);
    assert_eq!(of("Mixed Case Letters 42"), Strength::Strong);
    assert!(of("Mixed Case Letters").accepted() && !of("alllowercaseletters").accepted());
}

#[test]
fn nothing_is_kept_or_asked_without_a_session() {
    let mut calls = PasswordCalls::new(engine_on("http://127.0.0.1:9"));
    assert_eq!(calls.pane_state(false), PaneState::SignedOut);
    assert_eq!(
        calls.keep(Password::from_field("a password")).err(),
        Some(Refused::Locked)
    );
    // Nothing was kept, so no confirmation exists for a change.
    let forged = StepUp::granted(Purpose::ChangePassword, 1);
    assert_eq!(
        calls.change(forged, Password::from_field("A New Password 42")).err(),
        Some(Refused::NotConfirmed)
    );
}

#[test]
fn an_empty_password_is_refused_as_wrong() {
    let (engine, _fake) = signed_in();
    let mut calls = PasswordCalls::new(engine);
    assert_eq!(calls.keep(Password::from_field("")).err(), Some(Refused::WrongPassword));
}

#[test]
fn a_kept_password_serves_one_change_for_its_own_confirmation() {
    let (engine, fake) = signed_in();
    let mut calls = PasswordCalls::new(engine);
    assert_eq!(calls.pane_state(false), PaneState::Ready);
    assert_eq!(calls.pane_state(true), PaneState::SessionEnded);
    let asked = fake.routes().len();

    // A confirmation of another purpose, and a number nobody made: nothing is sent, and the kept password goes.
    let first = calls.keep(Password::from_field("correct horse battery")).expect("kept");
    let other = StepUp::granted(Purpose::TwoFactor, first.handle());
    assert_eq!(
        calls.change(other, Password::from_field("A New Password 42")).err(),
        Some(Refused::NotConfirmed)
    );
    let first_again = StepUp::granted(Purpose::ChangePassword, first.handle());
    assert_eq!(
        calls
            .change(first_again, Password::from_field("A New Password 42"))
            .err(),
        Some(Refused::NotConfirmed),
        "the first refusal used it up"
    );

    // A second step-up replaces the first one: the first confirmation is no longer this one's.
    let stale = calls.keep(Password::from_field("correct horse battery")).expect("kept");
    let fresh = calls.keep(Password::from_field("correct horse battery")).expect("kept");
    assert_ne!(stale.handle(), fresh.handle());
    assert_eq!(
        calls.change(stale, Password::from_field("A New Password 42")).err(),
        Some(Refused::NotConfirmed)
    );
    assert_eq!(fake.routes().len(), asked, "none of these sent anything");
}

#[test]
fn a_kept_password_is_dropped_when_the_session_moves_on() {
    let (engine, fake) = signed_in();
    let mut calls = PasswordCalls::new(engine.clone());
    let confirmation = calls.keep(Password::from_field("correct horse battery")).expect("kept");
    let asked = fake.routes().len();
    // The vault locks: another session than the one the password was typed under.
    engine.lock();
    assert_eq!(
        calls
            .change(confirmation, Password::from_field("A New Password 42"))
            .err(),
        Some(Refused::NotConfirmed)
    );
    assert_eq!(fake.routes().len(), asked, "a session that moved on sends nothing");
    assert_eq!(calls.pane_state(false), PaneState::SignedOut);
}

#[test]
fn each_refusal_of_the_engine_is_the_forms_own_word() {
    use PasswordChangeError as E;
    assert_eq!(refusal(E::WrongPassword), Refused::WrongPassword);
    assert_eq!(refusal(E::SamePassword), Refused::Weak);
    assert_eq!(refusal(E::TooWeak), Refused::Weak);
    assert_eq!(refusal(E::Breached), Refused::Weak);
    assert_eq!(refusal(E::Locked), Refused::Locked);
    assert_eq!(refusal(E::NotSignedIn), Refused::NotConfirmed);
    for nothing_changed in [
        E::SessionEnded,
        E::BreachCheckUnavailable,
        E::RateLimited,
        E::Offline,
        E::Failed("protocol"),
    ] {
        assert_eq!(refusal(nothing_changed), Refused::Offline);
    }
}

#[test]
fn a_change_the_server_cannot_take_is_refused_and_the_session_stays() {
    // The fake of `test_account` has no policy or password-change route: the engine refuses. (The wrong-password
    // case against real OPAQUE legs is the engine's own test.)
    let (engine, _fake) = signed_in();
    let turn = engine.session_turn();
    let mut calls = PasswordCalls::new(engine.clone());
    let confirmation = calls.keep(Password::from_field("not the password")).expect("kept");
    let answer = calls.change(confirmation, Password::from_field("A New Password 42"));
    assert!(answer.is_err(), "nothing went through");
    assert_eq!(engine.session_turn(), turn, "the session did not move");
}
