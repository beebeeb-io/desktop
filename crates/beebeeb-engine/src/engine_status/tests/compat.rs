//! Compatibility and activity tests split out of engine_status.rs.

use super::*;
use serde_json::{Value, json};

// ── the numbers ─────────────────────────────────────────────────────────

fn backlog(up_files: u32, up_bytes: u64, down_files: u32, down_bytes: u64) -> crate::state_db::TransferBacklog {
    crate::state_db::TransferBacklog {
        upload_files: up_files,
        upload_bytes: up_bytes,
        download_files: down_files,
        download_bytes: down_bytes,
        ..Default::default()
    }
}

#[test]
fn the_overall_numbers_add_finished_bytes_to_what_is_left() {
    use crate::transfer_progress::{Direction, TransferBoard};
    let board = TransferBoard::new();
    let a = board.begin("a", Direction::Up, 400);
    a.update(400);
    a.finish();
    let b = board.begin("b", Direction::Up, 600);
    b.update(150);
    // 400 finished; b (600) and c (1000) still queued.
    let a = compute_activity(&backlog(2, 1_600, 0, 0), &board);
    assert_eq!(a.files_remaining, 2);
    assert_eq!(a.bytes_total, 400 + 1_600);
    assert_eq!(a.bytes_done, 400 + 150);
}

#[test]
fn the_overall_bar_never_runs_backwards_when_a_file_finishes() {
    use crate::transfer_progress::{Direction, TransferBoard};
    let board = TransferBoard::new();
    let b = board.begin("b", Direction::Up, 500);
    b.update(400);
    let before = compute_activity(&backlog(2, 1_000, 0, 0), &board);
    b.update(500);
    b.finish();
    // b left the queue: 1 file and 500 bytes remain.
    let after = compute_activity(&backlog(1, 500, 0, 0), &board);
    assert_eq!(
        before.bytes_total, after.bytes_total,
        "the total is the batch, not what is left"
    );
    assert!(after.bytes_done > before.bytes_done);
}

#[test]
fn a_new_file_whose_row_says_zero_bytes_uses_the_size_the_board_knows() {
    use crate::transfer_progress::{Direction, TransferBoard};
    let board = TransferBoard::new();
    let g = board.begin("new", Direction::Up, 800);
    g.update(300);
    let a = compute_activity(&backlog(1, 0, 0, 0), &board);
    assert_eq!(a.bytes_total, 800);
    assert_eq!(a.bytes_done, 300);
    assert!(a.bytes_done <= a.bytes_total);
}

#[test]
fn a_moving_upload_with_no_queue_row_still_counts_as_a_file_left() {
    use crate::transfer_progress::{Direction, TransferBoard};
    // Keep Mine uploads directly, without enqueuing an operation.
    let board = TransferBoard::new();
    let g = board.begin("keep-mine", Direction::Up, 700);
    g.update(200);
    let a = compute_activity(&backlog(0, 0, 0, 0), &board);
    assert_eq!(a.files_remaining, 1);
    assert_eq!((a.bytes_total, a.bytes_done), (700, 200));
    // A queued file that is also on the board is one file, not two.
    let a = compute_activity(&backlog(1, 700, 0, 0), &board);
    assert_eq!(a.files_remaining, 1);
}

#[test]
fn downloads_count_as_files_left() {
    use crate::transfer_progress::TransferBoard;
    let board = TransferBoard::new();
    let a = compute_activity(&backlog(3, 30, 2, 20), &board);
    assert_eq!(a.files_remaining, 5);
    assert_eq!(a.bytes_total, 50);
    assert_eq!(a.bytes_done, 0);
}

// ── nothing a user sees today changes ───────────────────────────────────

/// What the pre-slice-2 `emit_status` built, for the same situations.
fn old_payload(state: &str, error: Option<&str>) -> Value {
    json!({ "state": state, "sync_root": ROOT, "error": error, "files_remaining": null })
}

fn assert_same_to_a_user(new: &Value, old: &Value) {
    assert_eq!(tray_tooltip(new), tray_tooltip(old), "tooltip: new {new} vs old {old}");
    assert_eq!(
        sync_status_engine(legacy_state(new)),
        sync_status_engine(legacy_state(old)),
        "sync_status engine: new {new} vs old {old}"
    );
}

#[test]
fn every_new_state_looks_to_old_consumers_like_the_old_emitter_said() {
    let t = tracker();
    // A good tick: old said idle.
    let good = t.finish_tick(&Outcome::Synced, 5, Activity::default()).to_json();
    assert_same_to_a_user(&good, &old_payload("idle", None));
    // Work in flight: old said idle (it never said syncing).
    let syncing = t.pulse(false, true, || busy(14, 100, 10)).unwrap().to_json();
    assert_eq!(syncing["state"], "syncing");
    assert_same_to_a_user(&syncing, &old_payload("idle", None));
    // A tick that returned Ok while the link was down: old said idle.
    let quiet_offline = tick_outcome(Ok(()), &link_with(Some((offline_failure(), 10))), 0);
    let p = t.finish_tick(&quiet_offline, 6, Activity::default()).to_json();
    assert_eq!(p["state"], "offline");
    assert_same_to_a_user(&p, &old_payload("idle", None));
    // A tick that returned Err: old said error with the text, whatever the class.
    for err in [
        "connection refused",
        "HTTP 503 Service Unavailable",
        "decrypt failed",
        "HTTP 401 Unauthorized",
    ] {
        let e = anyhow::anyhow!(err);
        let p = t
            .finish_tick(&tick_outcome(Err(&e), &link_with(None), 0), 7, Activity::default())
            .to_json();
        assert_same_to_a_user(&p, &old_payload("error", Some(err)));
    }
    // Paused, stopped, running: unchanged.
    assert_same_to_a_user(&t.paused().to_json(), &old_payload("paused", None));
    assert_same_to_a_user(&t.stopped().to_json(), &old_payload("stopped", None));
    assert_same_to_a_user(&t.running().to_json(), &old_payload("running", None));
    // The pause toggle in lib.rs: a bare `{state}` with no legacy_state.
    assert_eq!(tray_tooltip(&json!({ "state": "paused" })), "Beebeeb · Paused");
    assert_eq!(legacy_state(&json!({ "state": "idle" })), "idle");
    assert_eq!(legacy_state(&json!({})), "");
}

#[test]
fn a_file_moving_before_the_first_tick_ends_keeps_the_old_running_tooltip() {
    // Before this slice the tray said `Beebeeb · running` until the first tick ended, while
    // the initial upload was still going. The pulse must not turn that into `Synced`.
    let t = tracker();
    let started = t.running().to_json();
    assert_eq!(tray_tooltip(&started), "Beebeeb · running");

    let moving = t.pulse(false, true, || busy(3, 30, 1)).unwrap().to_json();
    assert_eq!(moving["state"], "syncing", "the popover still sees the new state");
    assert_eq!(moving["legacy_state"], "running");
    assert_eq!(tray_tooltip(&moving), "Beebeeb · running");
    assert_same_to_a_user(&moving, &old_payload("running", None));

    // The last file lands before the tick ends: still not `Synced`, the tick has not said so.
    let landed = t.pulse(false, true, Activity::default).unwrap().to_json();
    assert_eq!(landed["state"], "idle");
    assert_eq!(tray_tooltip(&landed), "Beebeeb · running");

    // The tick ends: now it is `Synced`, and later pulses keep saying so.
    let done = t.finish_tick(&Outcome::Synced, 9, Activity::default()).to_json();
    assert_eq!(tray_tooltip(&done), "Beebeeb · Synced");
    let again = t.pulse(false, true, || busy(2, 20, 1)).unwrap().to_json();
    assert_eq!(tray_tooltip(&again), "Beebeeb · Synced");
}

#[test]
fn a_pulse_after_a_resume_says_idle_like_the_pause_toggle_did() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 1, Activity::default());
    assert_eq!(t.pulse(true, false, Activity::default).unwrap().legacy_state, "paused");
    let resumed = t.pulse(false, true, || busy(2, 20, 1)).unwrap().to_json();
    assert_eq!(resumed["state"], "syncing");
    assert_same_to_a_user(&resumed, &old_payload("idle", None));
    // Resumed before the first tick ever ended (paused straight after start).
    let t = tracker();
    t.running();
    assert_eq!(t.pulse(true, false, Activity::default).unwrap().legacy_state, "paused");
    let resumed = t.pulse(false, true, || busy(2, 20, 1)).unwrap().to_json();
    assert_same_to_a_user(&resumed, &old_payload("idle", None));
}

#[test]
fn the_new_states_would_change_the_tooltip_if_the_listener_read_them() {
    // Why `legacy_state` exists: read `state` and two tooltips change today.
    assert_eq!(
        tray_tooltip(&json!({ "state": "syncing", "files_remaining": 14 })),
        "Beebeeb · Syncing 14 files…"
    );
    assert_eq!(tray_tooltip(&json!({ "state": "offline" })), "Beebeeb · Offline");
}

// ── the JSON the TypeScript side validates ──────────────────────────────

#[test]
fn the_syncing_and_offline_events_are_exactly_the_shared_fixtures() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 100, Activity::default());
    let syncing = t
        .pulse(false, true, || busy(14, 1_288_490_189, 489_626_271))
        .unwrap()
        .to_json();
    let expected: Value =
        serde_json::from_str(include_str!("../../../../../tests/fixtures/engine-status.syncing.json")).unwrap();
    assert_eq!(syncing, expected);

    let t = tracker();
    let err = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)).context("sync_ops");
    let offline = t
        .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 500, Activity::default())
        .to_json();
    let expected: Value =
        serde_json::from_str(include_str!("../../../../../tests/fixtures/engine-status.offline.json")).unwrap();
    assert_eq!(offline, expected);
}
