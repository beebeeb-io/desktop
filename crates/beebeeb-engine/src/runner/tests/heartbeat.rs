//! heartbeat tests split out of runner.rs.

use super::*;

#[test]
fn sync_complete_notification_state_fires_once_on_sync_to_idle_transition() {
    let mut state = SyncCompleteNotificationState::default();

    assert!(!state.observe(false, 0, 0), "idle no-op ticks are ignored");
    assert!(!state.observe(true, 2, 0), "active sync does not fire until caught up");
    assert!(state.observe(false, 0, 0), "transition back to caught up fires once");
    assert!(!state.observe(false, 0, 0), "dedupes repeated idle ticks");
}

#[test]
fn sync_complete_notification_state_waits_for_conflicts_to_clear() {
    let mut state = SyncCompleteNotificationState::default();

    assert!(!state.observe(true, 0, 1), "conflicted sync is not caught up");
    assert!(state.observe(false, 0, 0), "clearing conflicts completes the run");
}

#[test]
fn quota_warning_notification_state_fires_once_per_threshold_crossing() {
    let mut state = QuotaWarningNotificationState::default();

    assert!(!state.observe(None, 10_000), "unlimited cache caps never warn");
    assert!(!state.observe(Some(1_000), 899), "below 90 percent is quiet");
    assert!(state.observe(Some(1_000), 900), "90 percent crossing warns");
    assert!(!state.observe(Some(1_000), 950), "remaining over threshold dedupes");
    assert!(!state.observe(Some(1_000), 100), "dropping below threshold resets");
    assert!(state.observe(Some(1_000), 901), "a later crossing warns again");
}

// ── Heartbeat telemetry mapping ───────────────────────────────────────────

#[test]
fn engine_state_maps_to_lowercase_heartbeat_status() {
    // The lowercase vocabulary the read side (web devices.tsx + the desktop
    // Bandwidth view) maps to a display label. Must match the CLI exactly.
    assert_eq!(engine_state_to_heartbeat_status("paused", 0), "paused");
    assert_eq!(engine_state_to_heartbeat_status("error", 0), "error");
    assert_eq!(engine_state_to_heartbeat_status("running", 0), "watching");
    assert_eq!(engine_state_to_heartbeat_status("idle", 0), "watching");
    assert_eq!(engine_state_to_heartbeat_status("syncing", 0), "syncing");
    // Unknown / stopped degrade to a flat idle rather than panicking.
    assert_eq!(engine_state_to_heartbeat_status("stopped", 0), "idle");
    assert_eq!(engine_state_to_heartbeat_status("totally-unknown", 0), "idle");
}

#[test]
fn files_in_flight_promotes_status_to_syncing() {
    // Even when the loop's last emitted state was the post-tick "idle",
    // active transfers must read as "syncing" — the live signal wins.
    assert_eq!(engine_state_to_heartbeat_status("idle", 3), "syncing");
    assert_eq!(engine_state_to_heartbeat_status("running", 1), "syncing");
    // ...but a user PAUSE always wins over in-flight (we don't report
    // "syncing" while paused — the producer also hard-overrides this).
    assert_eq!(engine_state_to_heartbeat_status("paused", 5), "paused");
    // ...and an error state is preserved over in-flight, so a failing
    // session doesn't masquerade as healthily syncing.
    assert_eq!(engine_state_to_heartbeat_status("error", 5), "error");
}

#[test]
fn build_heartbeat_carries_counts_and_status() {
    let telem = TelemetryState {
        engine_state: "idle".to_string(),
        current_file: Some("active.bin".to_string()),
    };
    let snap = SyncSnapshot {
        files_synced: 8,
        files_total: 10,
        files_in_flight: 2,
        bytes_synced: 5_000,
        bytes_total: 9_000,
    };
    // P1 wire counters: 3000 bytes upload + 1000 bytes download over 20 s = 200 B/s.
    let body = build_heartbeat(&telem, snap, 3_000, 1_000, 20);
    // 2 in flight → syncing, and current_file surfaces.
    assert_eq!(body.status, "syncing");
    assert_eq!(body.files_synced, Some(8));
    assert_eq!(body.files_total, Some(10));
    assert_eq!(body.bytes_synced, Some(5_000));
    assert_eq!(body.bytes_total, Some(9_000));
    assert_eq!(body.current_file.as_deref(), Some("active.bin"));
    // Wire speed: (3000 + 1000) / 20 = 200 B/s.
    assert_eq!(body.speed_bps, Some(200));
}

#[test]
fn build_heartbeat_omits_current_file_when_idle() {
    // Nothing in flight → status is "watching" and current_file is dropped
    // even though the telemetry slot still holds a stale name.
    let telem = TelemetryState {
        engine_state: "idle".to_string(),
        current_file: Some("stale.bin".to_string()),
    };
    let snap = SyncSnapshot {
        files_synced: 10,
        files_total: 10,
        files_in_flight: 0,
        bytes_synced: 9_000,
        bytes_total: 9_000,
    };
    // Zero wire bytes → idle beat, 0 B/s.
    let body = build_heartbeat(&telem, snap, 0, 0, 20);
    assert_eq!(body.status, "watching");
    assert_eq!(body.current_file, None);
    // No wire bytes → 0 B/s.
    assert_eq!(body.speed_bps, Some(0));
}

#[test]
fn build_heartbeat_speed_never_negative_on_regression() {
    // With wire counters there is no concept of "regression" — the counters are
    // drained per beat so they can never go negative.  Zero wire bytes = 0 B/s.
    let telem = TelemetryState {
        engine_state: "idle".to_string(),
        current_file: None,
    };
    let snap = SyncSnapshot {
        files_synced: 1,
        files_total: 5,
        files_in_flight: 1,
        bytes_synced: 100,
        bytes_total: 9_000,
    };
    // No wire bytes were counted this beat → 0 B/s.
    let body = build_heartbeat(&telem, snap, 0, 0, 20);
    assert_eq!(body.speed_bps, Some(0));
}

#[test]
fn build_heartbeat_wire_speed_from_counters() {
    // Confirm that the new wire-counter-based speed is correct.
    // 10 000 upload + 5 000 download = 15 000 bytes over 20 s = 750 B/s.
    let telem = TelemetryState {
        engine_state: "syncing".to_string(),
        current_file: None,
    };
    let snap = SyncSnapshot {
        files_synced: 3,
        files_total: 10,
        files_in_flight: 2,
        bytes_synced: 30_000,
        bytes_total: 100_000,
    };
    let body = build_heartbeat(&telem, snap, 10_000, 5_000, 20);
    assert_eq!(body.speed_bps, Some(750));
    assert_eq!(body.status, "syncing");
}

#[test]
fn platform_str_is_a_known_family() {
    assert!(matches!(platform_str(), "windows" | "macos" | "linux"));
}

#[test]
fn beat_elapsed_uses_real_wall_clock_with_a_floor() {
    // Normal case: real elapsed time is used verbatim.
    assert_eq!(beat_elapsed_secs(1_020, 1_000), 20);
    // A long stall (suspended laptop) reports the FULL gap, not the nominal
    // interval — so speed isn't overstated by dividing by a small interval.
    assert_eq!(beat_elapsed_secs(1_300, 1_000), 300);
    // Two beats in the same second floor to 1 (never divide by zero).
    assert_eq!(beat_elapsed_secs(1_000, 1_000), 1);
    // Clock stepped backwards floors to 1 (never divide by a negative).
    assert_eq!(beat_elapsed_secs(990, 1_000), 1);
}

#[test]
fn build_heartbeat_speed_uses_real_elapsed_not_nominal_interval() {
    // 4000 wire bytes over a REAL 40s gap (e.g. a delayed loop) is
    // 100 B/s — NOT 200 B/s (which dividing by the nominal 20s would give).
    let telem = TelemetryState {
        engine_state: "idle".to_string(),
        current_file: None,
    };
    let snap = SyncSnapshot {
        files_synced: 2,
        files_total: 4,
        files_in_flight: 1,
        bytes_synced: 5_000,
        bytes_total: 9_000,
    };
    let elapsed = beat_elapsed_secs(1_040, 1_000); // 40s real gap
    // 4000 total wire bytes over 40 s = 100 B/s.
    let body = build_heartbeat(&telem, snap, 3_000, 1_000, elapsed);
    assert_eq!(body.speed_bps, Some(100));
}

// ── api_base_url() env-override ───────────────────────────────────────────

// The value is passed in: the real env var is process-wide and a parallel test sets it.
#[test]
fn api_base_url_trims_trailing_slash() {
    assert_eq!(
        api_base_url_from(Some("http://localhost:3001/")),
        "http://localhost:3001"
    );
}

#[test]
fn api_base_url_trims_multiple_trailing_slashes() {
    assert_eq!(
        api_base_url_from(Some("http://localhost:3001///")),
        "http://localhost:3001"
    );
}

#[test]
fn api_base_url_trims_whitespace() {
    assert_eq!(
        api_base_url_from(Some("  http://10.100.0.239:3001  ")),
        "http://10.100.0.239:3001"
    );
}

#[test]
fn a_unit_test_build_never_defaults_to_the_production_api() {
    for unset in [None, Some(""), Some("   ")] {
        let base = api_base_url_from(unset);
        assert_eq!(base, "http://127.0.0.1:9", "{unset:?}: a closed loopback port");
        assert!(!base.contains("beebeeb.io"), "{unset:?}: {base}");
    }
}

#[test]
fn every_other_build_defaults_to_the_production_api() {
    // A test build cannot run the production branch, so its source is pinned: the default
    // without BB_API_BASE is the production constant, and that constant is production.
    assert_eq!(PRODUCTION_API_BASE, "https://api.beebeeb.io");
    // A Windows checkout has CRLF line endings, so the source is read with them normalized. The
    // pin also runs on a CRLF copy, so a Mac shows what Windows CI would.
    let checkout = normalized_line_endings(include_str!("../../runner.rs"));
    for source in [checkout.clone(), checkout.replace('\n', "\r\n")] {
        let source = normalized_line_endings(&source);
        let body = &source[source.find("fn api_base_url_from(").expect("api_base_url_from")..];
        let body = &body[..body.find("\n}\n").expect("its end")];
        assert!(
            body.contains(
                "#[cfg(not(any(test, feature = \"test-support\")))]\n        return PRODUCTION_API_BASE.to_string();"
            ),
            "the non-test default is the production API:\n{body}"
        );
    }
}

/// Source text with CRLF line endings (a Windows checkout) as LF.
fn normalized_line_endings(source: &str) -> String {
    source.replace("\r\n", "\n")
}

// ── Engine status: the tick's payload and the pulse (task 1683 slice 2) ────
