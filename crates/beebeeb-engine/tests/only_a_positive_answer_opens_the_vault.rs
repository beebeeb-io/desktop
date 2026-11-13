//! S1, invariant I4 (contract §6): the phrase is accepted only on the server's positive answer. The fake's check
//! answers, in turn: no answer (the connection closes), 401, 500, a 400 with another code, a 200 without `valid`.
//! Each is a refusal with no key stored and the gate unchanged; then the true answer opens the vault. Mutant: read any
//! other failure than a 400 as a match (`recovery_check_answer`).

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn only_a_positive_answer_opens_the_vault() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use beebeeb_engine::signin::gate::GateState;
    use beebeeb_engine::test_api::{Handler, Reply};
    use serde_json::json;
    use zeroize::Zeroizing;

    const CHECK: &str = "POST /api/v1/auth/verify-recovery-check";
    let s = common::setup_with(|real, _, _| {
        let asked = Arc::new(AtomicUsize::new(0));
        let handler: Handler = Arc::new(move |request| {
            if request.route() != CHECK {
                return real(request);
            }
            match asked.fetch_add(1, Ordering::SeqCst) {
                0 => Reply::hang_up(),
                1 => Reply::error(401, "unauthorized"),
                2 => Reply::error(500, "internal"),
                3 => Reply::error(400, "something_else"),
                4 => Reply::ok(json!({ "checked": true })),
                _ => real(request),
            }
        });
        handler
    });
    s.sign_in();
    let entries = s.entries();
    let from = s.fake.requests().len();

    for answer in ["no answer", "401", "500", "400 with another code", "200 without valid"] {
        let refused = s.engine.unlock_with_phrase(Zeroizing::new(s.phrase.clone()));
        assert!(refused.is_err(), "{answer}: refused");
        assert_eq!(s.entries(), entries, "{answer}: no key stored");
        assert_eq!(
            s.engine.gate(),
            GateState::NeedsRecoveryPhrase,
            "{answer}: the gate is unchanged"
        );
    }
    let checks = s.routes_from(from).iter().filter(|route| *route == CHECK).count();
    assert_eq!(checks, 5, "5 checks were answered, 5 refusals");

    s.unlock();
    assert_eq!(s.engine.gate(), GateState::Ready, "the true answer opens the vault");
    assert!(s.entries().contains(&"vault key"));
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
