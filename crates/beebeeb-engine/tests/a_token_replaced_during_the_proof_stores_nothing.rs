//! S1, invariant I5 (contract §6): the key is stored only if the token the proof was made for is still the stored
//! one. The fake's check replaces the stored token (another sign-in, as it would land meanwhile) before it answers
//! "valid": nothing is stored and the gate does not open. Mutant: drop the "still the stored token" check.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_token_replaced_during_the_proof_stores_nothing() {
    use std::sync::Arc;

    use beebeeb_engine::keychain::{AuthSecretStore, SessionToken};
    use beebeeb_engine::signin::gate::GateState;
    use beebeeb_engine::test_api::Handler;
    use zeroize::Zeroizing;

    let s = common::setup_with(|real, stores, slot| {
        let handler: Handler = Arc::new(move |request| {
            if request.route() == "POST /api/v1/auth/verify-recovery-check" {
                let replaced = SessionToken::new("an-invented-token-of-another-sign-in".to_string()).unwrap();
                stores.store_for(&slot).save_session_token(&replaced).unwrap();
            }
            real(request)
        });
        handler
    });
    s.sign_in();

    let result = s.engine.unlock_with_phrase(Zeroizing::new(s.phrase.clone()));

    assert!(result.is_err(), "the proof was for a token that is no longer stored");
    assert!(!s.entries().contains(&"vault key"), "no key entry");
    assert_ne!(s.engine.gate(), GateState::Ready);
    assert_ne!(s.engine.view().gate, GateState::Ready);
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
