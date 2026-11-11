//! abort tests split out of runner.rs.

use super::*;

use std::sync::atomic::AtomicUsize;

// ── EngineRunner::abort authoritative-stop mechanics (task 1538 Codex P1) ──

/// A task that promptly observes its cancel signal is confirmed stopped
/// well within the graceful window — the common, happy-path case.
#[tokio::test]
async fn stop_task_and_confirm_returns_true_for_a_cooperative_task() {
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = rx.await;
    });

    let confirmed = stop_task_and_confirm(
        Some(tx),
        Some(task),
        Duration::from_millis(200),
        Duration::from_millis(200),
    )
    .await;

    assert!(confirmed, "a task that honors cancel must be confirmed stopped");
}

/// The regression test for the actual bug: a task that never observes
/// its cancel signal (the real-world case is a single tick body stuck
/// mid-network-call, well past its last `tokio::select!` check) must
/// still be confirmed stopped — and, more importantly, must ACTUALLY
/// stop running, not merely be abandoned.
///
/// The pre-fix `EngineRunner::abort` just dropped the `JoinHandle` after
/// its graceful timeout, which detaches rather than cancels it: swap
/// this function's force-abort branch back to a bare `drop(handle)` (no
/// `handle.abort()`, no confirmation wait) to reproduce that — the
/// second assertion here (`ticks` unchanged after the function returns)
/// is what would catch it, since the loop would keep incrementing
/// `ticks` in the background forever. This was reasoned through against
/// the pre-fix code rather than executed as a live mutation in this
/// session (a shared-machine disk-space guard stopped further `cargo`
/// builds partway through gating — see the task report).
#[tokio::test]
async fn stop_task_and_confirm_force_aborts_a_task_that_never_observes_cancel() {
    let ticks = Arc::new(AtomicUsize::new(0));
    let ticks_for_task = ticks.clone();
    // Intentionally unused by the task below — simulates a tick body
    // that never reaches its own `tokio::select!` boundary within this
    // test's short timeouts, so only a REAL forced abort can stop it.
    let (_tx, _rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        loop {
            ticks_for_task.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });

    let confirmed = stop_task_and_confirm(
        Some(_tx),
        Some(task),
        Duration::from_millis(30),
        Duration::from_millis(300),
    )
    .await;

    assert!(
        confirmed,
        "a forced abort of a stuck task must be confirmed, not just given up on"
    );

    let ticks_at_return = ticks.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        ticks.load(Ordering::SeqCst),
        ticks_at_return,
        "the task must have ACTUALLY stopped — a detached task would keep incrementing here"
    );
}

/// `cancel: None` / `task: None` (the idempotent "already aborted"
/// shape `EngineRunner::abort`'s `Option::take()`s produce on a second
/// call) has nothing left to confirm and must report success rather
/// than hang or falsely report failure.
#[tokio::test]
async fn stop_task_and_confirm_is_idempotent_with_nothing_left_to_wait_on() {
    let confirmed = stop_task_and_confirm(None, None, Duration::from_millis(50), Duration::from_millis(50)).await;
    assert!(confirmed);
}

// ── AuthHealth (task 1546 Codex round 2, finding 5) ────────────────────

fn auth_error() -> anyhow::Error {
    anyhow::anyhow!("HTTP 401 Unauthorized: invalid token")
}

fn other_error() -> anyhow::Error {
    anyhow::anyhow!("HTTP 500 Internal Server Error: db unavailable")
}

#[test]
fn auth_health_starts_not_expired() {
    let health = AuthHealth::new();
    assert!(!health.is_expired());
}

#[test]
fn auth_health_flips_expired_only_at_the_threshold_of_consecutive_auth_failures() {
    let health = AuthHealth::new();
    assert_eq!(AUTH_EXPIRED_THRESHOLD, 3, "test assumes the documented threshold");

    health.note_result(Some(&auth_error()));
    assert!(!health.is_expired(), "1 failure must not trip the banner");

    health.note_result(Some(&auth_error()));
    assert!(!health.is_expired(), "2 failures must not trip the banner");

    health.note_result(Some(&auth_error()));
    assert!(health.is_expired(), "the 3rd CONSECUTIVE auth failure must trip it");
}

#[test]
fn auth_health_non_auth_failures_neither_advance_nor_reset_the_streak() {
    // A network blip / 5xx between two real 401s must not erase progress
    // toward the threshold, and must not itself count as progress.
    let health = AuthHealth::new();

    health.note_result(Some(&auth_error()));
    health.note_result(Some(&auth_error()));
    assert!(!health.is_expired());

    health.note_result(Some(&other_error()));
    assert!(
        !health.is_expired(),
        "an unrelated error must not itself trip the banner"
    );

    // The streak must still be at 2 — one more REAL auth failure trips it.
    health.note_result(Some(&auth_error()));
    assert!(
        health.is_expired(),
        "the unrelated error must not have reset the streak back to 0"
    );
}

#[test]
fn auth_health_any_success_resets_the_streak_and_clears_expired() {
    let health = AuthHealth::new();
    health.note_result(Some(&auth_error()));
    health.note_result(Some(&auth_error()));
    health.note_result(Some(&auth_error()));
    assert!(health.is_expired());

    health.note_result(None);
    assert!(!health.is_expired(), "a success must clear an already-tripped banner");

    // And the streak is genuinely back to 0, not just the flag flipped:
    // two more failures alone must not re-trip it.
    health.note_result(Some(&auth_error()));
    health.note_result(Some(&auth_error()));
    assert!(
        !health.is_expired(),
        "the streak must have been reset to 0, not left at 3"
    );
}

#[test]
fn classify_abort_maps_full_success_to_stopped() {
    assert_eq!(
        classify_abort_result(true, None, None),
        AbortOutcome::Stopped,
        "confirmed task + both revocations ok must be Stopped"
    );
}

#[test]
fn classify_abort_maps_a_confirmed_task_with_a_failed_revocation_to_revoke_failed() {
    // The Bug A core case: the engine task stopped fine; the revocation
    // did NOT. Must be RevokeFailed — never TaskUnconfirmed (which would
    // claim the engine itself is still running).
    assert_eq!(
        classify_abort_result(true, None, Some("drain timed out".to_string())),
        AbortOutcome::RevokeFailed {
            stage: "final Cloud Files callback revocation",
            source: "drain timed out".to_string(),
        },
        "revoke failure with a confirmed-stopped task must be RevokeFailed, not TaskUnconfirmed"
    );
}

#[test]
fn classify_abort_reports_the_initial_stage_when_the_first_revocation_failed() {
    assert_eq!(
        classify_abort_result(true, Some("first drain timed out".to_string()), None),
        AbortOutcome::RevokeFailed {
            stage: "initial Cloud Files callback revocation",
            source: "first drain timed out".to_string(),
        },
        "the first failing revocation stage must be the one reported"
    );
}

#[test]
fn classify_abort_maps_unconfirmed_task_to_task_unconfirmed_even_when_revocation_also_failed() {
    // Stage precedence: an unconfirmed task dominates everything — the
    // caller must treat the engine as alive regardless of revocation
    // outcomes.
    assert_eq!(
        classify_abort_result(false, Some("revoke err".to_string()), Some("revoke err 2".to_string())),
        AbortOutcome::TaskUnconfirmed,
        "an unconfirmed task must never be masked by revocation failures"
    );
}

/// A real (not classified) abort of a runner wrapping a task that
/// promptly observes its cancel signal must classify as `Stopped`.
#[tokio::test]
async fn abort_returns_stopped_for_a_cooperative_task() {
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = rx.await;
    });
    let runner = EngineRunner {
        cancel: Some(tx),
        task: Some(task),
        ipc_bind_error: Arc::new(Mutex::new(None)),
        stopping: Arc::new(AtomicBool::new(false)),
        send_phase: Arc::new(SendPhaseLink::default()),
    };

    let outcome = runner.abort().await;

    assert_eq!(
        outcome,
        AbortOutcome::Stopped,
        "a cooperative task must classify as Stopped"
    );
}

/// The pathological case (task 1538): a task with NO await point cannot
/// be force-aborted by tokio, so even the forced path times out — the
/// outcome must be `TaskUnconfirmed`, never `Stopped`. Uses
/// `spawn_blocking` (blocking pool, not a runtime worker) with a
/// wall-clock deadline so the busy loop cannot leak a spinning thread
/// into the rest of the test run. Runs on a multi-thread runtime so the
/// graceful/force timeouts can still fire while one blocking thread
/// spins.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_returns_task_unconfirmed_when_the_task_cannot_be_aborted() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let task = tokio::task::spawn_blocking(move || {
        while std::time::Instant::now() < deadline {
            std::hint::black_box(());
        }
    });
    let (tx, _rx) = oneshot::channel::<()>();
    let runner = EngineRunner {
        cancel: Some(tx),
        task: Some(task),
        ipc_bind_error: Arc::new(Mutex::new(None)),
        stopping: Arc::new(AtomicBool::new(false)),
        send_phase: Arc::new(SendPhaseLink::default()),
    };

    // GRACEFUL_ABORT_TIMEOUT (3s) + FORCE_ABORT_TIMEOUT (2s) — the task
    // outlives both (its own deadline is 10s), so the abort cannot be
    // confirmed within either window.
    let outcome = runner.abort().await;

    assert_eq!(
        outcome,
        AbortOutcome::TaskUnconfirmed,
        "a task that cannot be aborted must classify as TaskUnconfirmed"
    );
}
