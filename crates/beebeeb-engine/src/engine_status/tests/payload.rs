//! Payload state machine tests split out of engine_status.rs.

use super::*;
use serde_json::{Value, json};

// ── the payload, one test per state ─────────────────────────────────────

#[test]
fn a_running_payload_is_the_old_shape_with_empty_new_keys() {
    let p = tracker().running().to_json();
    assert_eq!(
        p,
        json!({
            "state": "running", "legacy_state": "running", "sync_root": ROOT, "error": null,
            "files_remaining": null, "bytes_total": null, "bytes_done": null,
            "last_tick_ok_at": null, "reason": null
        })
    );
}

#[test]
fn an_idle_payload_after_a_good_tick_carries_the_check_time_and_no_reason() {
    let t = tracker();
    let p = t
        .finish_tick(&Outcome::Synced, 1_760_000_000, Activity::default())
        .to_json();
    assert_eq!(
        p,
        json!({
            "state": "idle", "legacy_state": "idle", "sync_root": ROOT, "error": null,
            "files_remaining": 0, "bytes_total": 0, "bytes_done": 0,
            "last_tick_ok_at": 1_760_000_000_i64, "reason": null
        })
    );
}

#[test]
fn a_syncing_payload_carries_files_and_bytes_and_still_says_idle_to_old_consumers() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 100, Activity::default());
    let p = t
        .pulse(false, true, || busy(14, 1_288_490_189, 489_626_271))
        .expect("work in flight")
        .to_json();
    assert_eq!(
        p,
        json!({
            "state": "syncing", "legacy_state": "idle", "sync_root": ROOT, "error": null,
            "files_remaining": 14, "bytes_total": 1_288_490_189_u64, "bytes_done": 489_626_271_u64,
            "last_tick_ok_at": 100, "reason": null
        })
    );
}

#[test]
fn a_paused_payload_is_paused_with_no_progress() {
    let p = tracker().paused().to_json();
    assert_eq!(p["state"], "paused");
    assert_eq!(p["legacy_state"], "paused");
    assert_eq!(p["files_remaining"], Value::Null);
    assert_eq!(p["reason"], Value::Null);
}

#[test]
fn an_offline_payload_from_a_failed_tick_keeps_the_old_error_state_for_old_consumers() {
    let t = tracker();
    let err = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)).context("sync_ops");
    let outcome = tick_outcome(Err(&err), &link_with(None), 0);
    let p = t.finish_tick(&outcome, 500, Activity::default()).to_json();
    assert_eq!(p["state"], "offline");
    assert_eq!(p["legacy_state"], "error", "the old emitter said error for an Err tick");
    assert_eq!(p["error"], "sync_ops");
    assert_eq!(p["reason"], json!({ "code": "connect", "detail": null }));
    assert_eq!(
        p["last_tick_ok_at"],
        Value::Null,
        "a failed check is not a successful check"
    );
}

#[test]
fn an_offline_payload_from_a_swallowed_failure_keeps_idle_for_old_consumers() {
    // `sync_tick` returned Ok but its `GET /sync/ops` failed at the link.
    let t = tracker();
    let outcome = tick_outcome(Ok(()), &link_with(Some((offline_failure(), 50))), 40);
    let p = t.finish_tick(&outcome, 60, Activity::default()).to_json();
    assert_eq!(p["state"], "offline");
    assert_eq!(p["legacy_state"], "idle", "the old emitter said idle for an Ok tick");
    assert_eq!(p["error"], Value::Null);
    assert_eq!(p["last_tick_ok_at"], Value::Null);
}

#[test]
fn a_server_that_did_not_answer_is_error_with_the_timeout_line() {
    let t = tracker();
    let outcome = tick_outcome(Ok(()), &link_with(Some((timeout_failure(), 50))), 40);
    let p = t.finish_tick(&outcome, 60, Activity::default()).to_json();
    assert_eq!(p["state"], "error");
    assert_eq!(
        p["reason"],
        json!({ "code": "timeout", "detail": "timeout after 30 s" })
    );
}

#[test]
fn a_5xx_carries_the_status_in_the_mono_line() {
    let t = tracker();
    let err = anyhow::anyhow!("HTTP 503 Service Unavailable: down");
    let p = t
        .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 1, Activity::default())
        .to_json();
    assert_eq!(p["state"], "error");
    assert_eq!(p["reason"], json!({ "code": "http_5xx", "detail": "HTTP 503" }));
    assert_eq!(p["legacy_state"], "error");
}

#[test]
fn an_error_that_is_not_the_link_is_an_engine_error() {
    let t = tracker();
    let err = anyhow::anyhow!("sync root vanished");
    let p = t
        .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 1, Activity::default())
        .to_json();
    assert_eq!(p["state"], "error");
    assert_eq!(p["reason"], json!({ "code": "engine_error", "detail": null }));
    assert_eq!(p["error"], "sync root vanished");
}

#[test]
fn a_401_is_the_server_answering_not_a_connectivity_failure() {
    let t = tracker();
    let err = anyhow::anyhow!("HTTP 401 Unauthorized: bad token");
    let p = t
        .finish_tick(&tick_outcome(Err(&err), &link_with(None), 0), 1, Activity::default())
        .to_json();
    assert_eq!(p["state"], "error");
    assert_eq!(p["reason"]["code"], "auth");
    let mut view = EngineStatusView::default();
    view.apply(&p);
    assert_eq!(view.connectivity(), Connectivity::Online);
}

/// 0.8.15: the view keeps the start-up guard's pass while payloads report it running, and drops it when one reports
/// it at rest (the engine lane's `{running: false, done: 0, total: 0}`). The pause toggle's bare `{state}` leaves it
/// as it was. Malformed or empty values are no check.
#[test]
fn the_view_keeps_the_startup_check_only_while_payloads_report_it_running() {
    let running = |done: u64, total: u64| json!({ "running": true, "done": done, "total": total });
    let mut view = EngineStatusView::default();
    view.apply(&json!({ "state": "running", STARTUP_CHECK_KEY: running(12_400, 37_169) }));
    assert_eq!(
        view.startup_check,
        Some(StartupCheck {
            done: 12_400,
            total: 37_169
        })
    );
    view.apply(&json!({ "state": "running", STARTUP_CHECK_KEY: running(40_000, 37_169) }));
    assert_eq!(
        view.startup_check,
        Some(StartupCheck {
            done: 37_169,
            total: 37_169
        }),
        "clamped"
    );
    view.apply(&json!({ "state": "paused" }));
    assert_eq!(
        view.startup_check,
        Some(StartupCheck {
            done: 37_169,
            total: 37_169
        }),
        "a bare {{state}} says nothing"
    );
    view.apply(&json!({ "state": "idle", STARTUP_CHECK_KEY: { "running": false, "done": 0, "total": 0 } }));
    assert_eq!(view.startup_check, None, "at rest");
    view.apply(&json!({ "state": "running", STARTUP_CHECK_KEY: running(1, 9) }));
    view.apply(&json!({ "state": "idle", STARTUP_CHECK_KEY: null }));
    assert_eq!(view.startup_check, None, "null is no pass");
    for malformed in [
        json!({ "running": true, "done": 1 }),
        json!({ "running": true, "done": 0, "total": 0 }),
        json!({ "done": 3, "total": 9 }),
        json!({ "running": "yes", "done": 3, "total": 9 }),
        json!("12/40"),
        json!({ "running": true, "done": -1, "total": 5 }),
    ] {
        view.apply(&json!({ "state": "running", STARTUP_CHECK_KEY: running(1, 9) }));
        view.apply(&json!({ "state": "running", STARTUP_CHECK_KEY: malformed }));
        assert_eq!(view.startup_check, None, "{malformed}");
    }
}

#[test]
fn a_stopped_payload_is_stopped() {
    let p = tracker().stopped().to_json();
    assert_eq!(p["state"], "stopped");
    assert_eq!(p["legacy_state"], "stopped");
}

#[test]
fn a_plain_error_is_a_whole_engine_failure() {
    let p = plain_payload("error", Some(ROOT.into()), Some("open state.db: locked"));
    assert_eq!(p["state"], "error");
    assert_eq!(p["legacy_state"], "error");
    assert_eq!(p["error"], "open state.db: locked");
    assert_eq!(p["reason"]["code"], "engine_error");
    let running = plain_payload("running", Some(ROOT.into()), None);
    assert_eq!(running["reason"], Value::Null);
}

// ── the tick outcome ────────────────────────────────────────────────────

#[test]
fn an_ok_tick_with_no_failure_is_synced() {
    assert_eq!(tick_outcome(Ok(()), &link_with(None), 100), Outcome::Synced);
}

#[test]
fn a_link_failure_from_before_the_tick_does_not_fail_it() {
    // The previous tick's failure, not cleared because this tick made no request.
    assert_eq!(
        tick_outcome(Ok(()), &link_with(Some((offline_failure(), 99))), 100),
        Outcome::Synced
    );
    // At the same second counts (the tick started at 100, the request failed at 100).
    assert!(matches!(
        tick_outcome(Ok(()), &link_with(Some((offline_failure(), 100))), 100),
        Outcome::Down {
            down: Down::Offline(_),
            ..
        }
    ));
}

#[test]
fn the_errors_own_class_beats_a_link_failure_recorded_in_the_tick() {
    let err = anyhow::anyhow!("HTTP 502 Bad Gateway");
    let outcome = tick_outcome(Err(&err), &link_with(Some((offline_failure(), 10))), 0);
    assert!(
        matches!(
            outcome,
            Outcome::Down {
                down: Down::NoAnswer(_),
                ..
            }
        ),
        "{outcome:?}"
    );
}

// ── the tracker: pulse ──────────────────────────────────────────────────

#[test]
fn the_pulse_says_nothing_while_idle_and_nothing_is_in_flight() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 1, Activity::default());
    assert_eq!(t.pulse(false, true, Activity::default), None);
    assert_eq!(t.pulse(false, true, Activity::default), None);
}

#[test]
fn the_pulse_reports_syncing_while_work_is_in_flight_and_idle_once_when_it_ends() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 1, Activity::default());
    let s = t.pulse(false, true, || busy(3, 300, 100)).unwrap();
    assert_eq!(s.state, State::Syncing);
    assert_eq!(s.activity, Some(busy(3, 300, 100)));
    // Progress changes are reported as they happen.
    let s = t.pulse(false, true, || busy(2, 300, 250)).unwrap();
    assert_eq!(s.activity, Some(busy(2, 300, 250)));
    // The way back: one idle, then silence.
    let s = t.pulse(false, true, Activity::default).unwrap();
    assert_eq!(s.state, State::Idle);
    assert_eq!(t.pulse(false, true, Activity::default), None);
}

#[test]
fn offline_error_and_stopped_outrank_syncing() {
    for outcome in [
        Outcome::Down {
            down: Down::Offline(Reason::from(LinkReason::Connect)),
            error: None,
        },
        Outcome::Down {
            down: Down::NoAnswer(Reason::from(LinkReason::Timeout)),
            error: None,
        },
    ] {
        let t = tracker();
        t.finish_tick(&outcome, 1, Activity::default());
        assert_eq!(t.pulse(false, true, || busy(5, 10, 1)), None, "{outcome:?}");
    }
    let t = tracker();
    t.stopped();
    assert_eq!(t.pulse(false, true, || busy(5, 10, 1)), None);
}

#[test]
fn pausing_wins_over_a_syncing_pulse_at_once_and_is_reported_once() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 1, Activity::default());
    assert_eq!(
        t.pulse(false, true, || busy(3, 300, 100)).unwrap().state,
        State::Syncing
    );
    let paused = t
        .pulse(true, true, || busy(3, 300, 150))
        .expect("pause must be reported");
    assert_eq!(paused.state, State::Paused);
    assert_eq!(t.pulse(true, true, || busy(3, 300, 200)), None, "already told");
}

#[test]
fn resuming_lets_the_pulse_report_work_in_flight_before_the_next_tick() {
    let t = tracker();
    t.paused();
    assert_eq!(t.pulse(false, true, Activity::default), None);
    let s = t
        .pulse(false, true, || busy(1, 10, 0))
        .expect("resumed with work in flight");
    assert_eq!(s.state, State::Syncing);
}

#[test]
fn the_end_of_a_tick_overrides_a_syncing_pulse() {
    let t = tracker();
    assert_eq!(t.pulse(false, true, || busy(2, 20, 5)).unwrap().state, State::Syncing);
    let p = t.finish_tick(&Outcome::Synced, 9, Activity::default());
    assert_eq!(p.state, State::Idle);
    assert_eq!(p.last_tick_ok_at, Some(9));
    // The pulse does not repeat an idle the tick already sent.
    assert_eq!(t.pulse(false, true, Activity::default), None);
}

#[test]
fn a_failed_check_keeps_the_time_of_the_last_good_one() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 100, Activity::default());
    let down = Outcome::Down {
        down: Down::Offline(Reason::from(LinkReason::Connect)),
        error: None,
    };
    let p = t.finish_tick(&down, 200, Activity::default());
    assert_eq!(p.last_tick_ok_at, Some(100));
}

#[test]
fn a_late_pulse_never_announces_anything_after_stopped() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 1, Activity::default());
    t.stopped();
    // Sync was paused when the engine stopped, and work is still marked in flight.
    assert_eq!(
        t.pulse(true, true, || busy(3, 30, 1)),
        None,
        "not `paused` after `stopped`"
    );
    assert_eq!(
        t.pulse(false, true, || busy(3, 30, 1)),
        None,
        "not `syncing` after `stopped`"
    );
    // A new session starts from `running` and works again.
    t.running();
    assert_eq!(t.pulse(false, true, || busy(3, 30, 1)).unwrap().state, State::Syncing);
}

#[test]
fn the_pulse_does_not_read_the_activity_when_it_cannot_matter() {
    let t = tracker();
    t.stopped();
    let mut called = false;
    let _ = t.pulse(false, true, || {
        called = true;
        Activity::default()
    });
    assert!(!called);
}

// ── the view ────────────────────────────────────────────────────────────

#[test]
fn the_view_folds_each_payload_and_keeps_the_last_check_across_a_bare_pause() {
    let t = tracker();
    let mut view = EngineStatusView::default();
    view.apply(&t.finish_tick(&Outcome::Synced, 777, Activity::default()).to_json());
    assert_eq!(view.state, "idle");
    assert_eq!(view.last_tick_ok_at, Some(777));
    view.apply(&t.pulse(false, true, || busy(4, 40, 10)).unwrap().to_json());
    assert!(view.is_syncing());
    assert_eq!(
        (view.files_remaining, view.bytes_total, view.bytes_done),
        (Some(4), Some(40), Some(10))
    );
    // The pause toggle in lib.rs sends only `{"state": "paused"}`.
    view.apply(&json!({ "state": "paused" }));
    assert_eq!(view.state, "paused");
    assert_eq!(view.files_remaining, None);
    assert_eq!(view.last_tick_ok_at, Some(777));
    assert_eq!(view.reason_code, None);
}

#[test]
fn a_stopped_or_restarted_engine_has_no_last_check() {
    let t = tracker();
    let mut view = EngineStatusView::default();
    view.apply(&t.finish_tick(&Outcome::Synced, 777, Activity::default()).to_json());
    assert_eq!(view.last_tick_ok_at, Some(777));
    // Lock or sign-out: the engine stops.
    view.apply(&t.stopped().to_json());
    assert_eq!(view.last_tick_ok_at, None, "a stopped engine has no last check");
    // The next session starts: still nothing until its first good tick.
    view.apply(&t.finish_tick(&Outcome::Synced, 900, Activity::default()).to_json());
    assert_eq!(view.last_tick_ok_at, Some(900));
    view.apply(&t.running().to_json());
    assert_eq!(view.last_tick_ok_at, None, "a restarted engine has not checked yet");
}

#[test]
fn the_view_maps_each_state_to_the_phase_connectivity() {
    let cases: [(Value, Connectivity); 6] = [
        (json!({ "state": "idle" }), Connectivity::Online),
        (json!({ "state": "syncing" }), Connectivity::Online),
        (
            json!({ "state": "offline", "reason": { "code": "connect", "detail": null } }),
            Connectivity::Offline,
        ),
        (
            json!({ "state": "error", "reason": { "code": "timeout", "detail": "timeout after 30 s" } }),
            Connectivity::ServerDidNotAnswer,
        ),
        (
            json!({ "state": "error", "reason": { "code": "engine_error", "detail": null } }),
            Connectivity::ServerDidNotAnswer,
        ),
        (
            json!({ "state": "error", "reason": { "code": "auth", "detail": null } }),
            Connectivity::Online,
        ),
    ];
    for (payload, expect) in cases {
        let mut view = EngineStatusView::default();
        view.apply(&payload);
        assert_eq!(view.connectivity(), expect, "{payload}");
    }
}

// ── no work while idle ──────────────────────────────────────────────────

#[test]
fn an_idle_engine_with_nothing_moving_does_not_touch_the_database() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 1, Activity::default());
    let mut reads = 0;
    for _ in 0..50 {
        let _ = t.pulse(false, false, || {
            reads += 1;
            Activity::default()
        });
    }
    assert_eq!(reads, 0, "50 quiet pulses must read the state DB 0 times");
}

#[test]
fn once_syncing_the_pulse_keeps_reading_until_it_has_reported_the_way_back() {
    let t = tracker();
    t.finish_tick(&Outcome::Synced, 1, Activity::default());
    // A file is moving: read and report syncing.
    assert!(t.pulse(false, true, || busy(2, 20, 5)).is_some());
    // Between two files nothing is on the board, but work is still queued.
    let s = t.pulse(false, false, || busy(1, 10, 10)).expect("still syncing");
    assert_eq!(s.state, State::Syncing);
    // The queue drained and nothing moves: one idle, then quiet with no reads.
    assert_eq!(t.pulse(false, false, Activity::default).unwrap().state, State::Idle);
    let mut reads = 0;
    assert_eq!(
        t.pulse(false, false, || {
            reads += 1;
            Activity::default()
        }),
        None
    );
    assert_eq!(reads, 0);
}
