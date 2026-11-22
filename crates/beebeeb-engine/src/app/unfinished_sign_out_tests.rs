//! What a stop records, by what the engine's abort achieved (`record_the_stop`), and which session ends it refuses
//! (`a_task_may_hold_the_keys`). `AbortOutcome::RevokeFailed` happens on Windows only, so it is made here by hand: the
//! start's fence is given as a counter, never the process-wide one (a Windows test process stays unfenced). Only a task that may still run refuses a sign-out; a revoke failure fences the start alone.

use std::cell::Cell;
use std::sync::atomic::Ordering;

use super::*;
use crate::account::AccountId;

fn account() -> AccountRuntime {
    AccountRuntime::new(AccountId("an-invented-slot".into()))
}

fn revoke_failed() -> AbortOutcome {
    AbortOutcome::RevokeFailed {
        stage: "after",
        source: "an invented failure".into(),
    }
}

/// A Windows stop whose task ended but whose File Explorer callbacks were not revoked fences the start (the callbacks
/// may still be registered), and a confirmed stop or an unconfirmed task does not (the record refuses that start).
/// Mutant U-R2: `fence_the_start()` not called: red.
#[test]
fn a_revoke_failure_fences_the_start() {
    let acct = account();
    let fenced = Cell::new(0);
    let fence = || fenced.set(fenced.get() + 1);

    assert_eq!(
        record_the_stop_with(&acct, &revoke_failed(), fence),
        SyncStop::Unconfirmed
    );
    assert_eq!(fenced.get(), 1, "the start is fenced");
    assert_eq!(
        record_the_stop_with(&acct, &AbortOutcome::Stopped, fence),
        SyncStop::Stopped
    );
    assert_eq!(
        record_the_stop_with(&acct, &AbortOutcome::TaskUnconfirmed, fence),
        SyncStop::Unconfirmed
    );
    assert_eq!(fenced.get(), 1, "only the revoke failure fences");
}

/// The same stop records no task: it ended, nothing holds the keys. A later stop on the empty slot says
/// `WasNotRunning`. Mutant U-R1: the record set for it too (the predicate `!is_stopped()` again): red.
#[test]
fn a_revoke_failure_records_no_task() {
    let acct = account();
    assert_eq!(
        record_the_stop_with(&acct, &revoke_failed(), || {}),
        SyncStop::Unconfirmed
    );
    assert!(
        !acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
        "no record: the task ended"
    );
    assert_eq!(stop_of_an_empty_slot(&acct), SyncStop::WasNotRunning);
}

/// A session end that stopped with `Unconfirmed` is refused only while the record says a task may still run: the
/// revoke failure's `Unconfirmed` (no record) lets the sign-out go on and clear the keys. Mutant U-R3: the gate on
/// any `Unconfirmed` (the old rule): red.
#[test]
fn only_a_task_that_may_still_run_refuses_a_sign_out() {
    let acct = account();
    assert!(
        !a_task_may_hold_the_keys(&acct, SyncStop::Unconfirmed),
        "a revoke failure: the sign-out goes on"
    );
    assert!(!a_task_may_hold_the_keys(&acct, SyncStop::WasNotRunning));
    acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
    assert!(
        a_task_may_hold_the_keys(&acct, SyncStop::Unconfirmed),
        "a task may still run: refused"
    );
}

/// A stop whose task's end is not confirmed records it, and every later stop on the empty slot says `Unconfirmed`; a
/// confirmed stop records nothing. Mutant U-A2 (no record): red.
#[test]
fn an_unconfirmed_task_is_recorded() {
    let acct = account();
    assert_eq!(
        record_the_stop_with(&acct, &AbortOutcome::Stopped, || {}),
        SyncStop::Stopped
    );
    assert!(!acct.engine_stop_unconfirmed.load(Ordering::SeqCst));
    assert_eq!(
        record_the_stop_with(&acct, &AbortOutcome::TaskUnconfirmed, || {}),
        SyncStop::Unconfirmed
    );
    assert!(acct.engine_stop_unconfirmed.load(Ordering::SeqCst), "the record");
    assert_eq!(stop_of_an_empty_slot(&acct), SyncStop::Unconfirmed);
}
