//! P1c: one in-place install at a time, across every Beebeeb window.
//!
//! The three tests share one process-wide flag, and cargo runs them in parallel threads, so each takes
//! the same lock. A poisoned lock (an earlier assertion failed) is taken anyway: a panic in one test
//! must not hide the next one's result.

use std::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Red on the mutant "a claim is never refused": the second window would start a second install.
#[test]
fn held_claim_refuses_the_next_with_the_in_progress_code() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let first = match super::claim_install() {
        Ok(claim) => claim,
        Err(reason) => panic!("the first claim must succeed, got: {reason}"),
    };

    let reason = match super::claim_install() {
        Ok(_) => panic!("a second claim must be refused while the first is held"),
        Err(reason) => reason,
    };
    assert!(
        reason.starts_with(super::UPDATE_INSTALL_IN_PROGRESS_CODE),
        "refusal must lead with the in-progress code, got: {reason}"
    );

    drop(first);
}

/// Red on the mutant "the flag is never released": after the first install ends, every later one is refused.
#[test]
fn a_dropped_claim_lets_the_next_one_through() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let first = match super::claim_install() {
        Ok(claim) => claim,
        Err(reason) => panic!("the first claim must succeed, got: {reason}"),
    };
    drop(first);

    match super::claim_install() {
        Ok(claim) => drop(claim),
        Err(reason) => panic!("a claim after the first was dropped must succeed, got: {reason}"),
    }
}

/// Red on the mutant "the flag is sticky": a second install in the same process would be refused forever.
#[test]
fn two_claims_one_after_the_other_both_succeed() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let first = match super::claim_install() {
        Ok(claim) => claim,
        Err(reason) => panic!("the first claim must succeed, got: {reason}"),
    };
    drop(first);

    let second = match super::claim_install() {
        Ok(claim) => claim,
        Err(reason) => panic!("the second claim must succeed, got: {reason}"),
    };
    drop(second);
}
