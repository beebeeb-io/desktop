//! Wire-shape tests: retry hints, provenance headers, metadata bodies.

use super::*;

#[test]
fn heartbeat_body_minimal_serializes_status_only() {
    // A status-only beat (no counts/bytes yet) must put `status` on the
    // wire and OMIT every optional field — the server's HeartbeatBody reads
    // them as `Option`, so absence is the same as null but smaller.
    let body = HeartbeatBody {
        status: "watching".into(),
        ..Default::default()
    };
    let json = serde_json::to_string(&body).unwrap();
    assert_eq!(json, r#"{"status":"watching"}"#);
}

#[test]
fn heartbeat_body_full_serializes_all_fields() {
    let body = HeartbeatBody {
        status: "syncing".into(),
        files_synced: Some(10),
        files_total: Some(12),
        bytes_synced: Some(1000),
        bytes_total: Some(2000),
        current_file: Some("report.pdf".into()),
        speed_bps: Some(500_000),
        detail: Some("uploading".into()),
    };
    let v: serde_json::Value = serde_json::to_value(&body).unwrap();
    assert_eq!(v["status"], "syncing");
    assert_eq!(v["files_synced"], 10);
    assert_eq!(v["files_total"], 12);
    assert_eq!(v["bytes_synced"], 1000);
    assert_eq!(v["bytes_total"], 2000);
    assert_eq!(v["current_file"], "report.pdf");
    assert_eq!(v["speed_bps"], 500_000);
    assert_eq!(v["detail"], "uploading");
}

#[test]
fn heartbeat_body_skips_none_optionals_but_keeps_zero() {
    // A zero count is a meaningful value (0 files synced) and MUST be sent;
    // only `None` is skipped. Guards against a `skip_serializing_if` that
    // accidentally drops legitimate zeros.
    let body = HeartbeatBody {
        status: "idle".into(),
        files_synced: Some(0),
        bytes_synced: Some(0),
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&body).unwrap();
    assert_eq!(v["files_synced"], 0);
    assert_eq!(v["bytes_synced"], 0);
    assert!(v.get("files_total").is_none());
    assert!(v.get("current_file").is_none());
    assert!(v.get("speed_bps").is_none());
}

#[test]
fn registered_device_parses_and_tolerates_extra_fields() {
    let json = r#"{
            "id": "11111111-1111-1111-1111-111111111111",
            "hostname": "guus-pc",
            "platform": "windows",
            "bb_version": "0.1.0",
            "last_seen": "2026-06-15T00:00:00Z",
            "created_at": "2026-06-01T00:00:00Z"
        }"#;
    let d: RegisteredDevice = serde_json::from_str(json).unwrap();
    assert_eq!(d.id, "11111111-1111-1111-1111-111111111111");
    assert_eq!(d.platform.as_deref(), Some("windows"));
}

#[test]
fn created_session_parses_minimal() {
    let json = r#"{ "id": "cs1", "heartbeat_interval_secs": 20 }"#;
    let s: CreatedSession = serde_json::from_str(json).unwrap();
    assert_eq!(s.id, "cs1");
    assert_eq!(s.heartbeat_interval_secs, Some(20));
}
/// Spec 2026-10-09 §7.2 (final review): the three parents a metadata update sends, as the
/// server reads them: omitted (leave it alone), `null` (the top level), an id (that folder).
#[test]
fn a_metadata_update_body_names_each_parent_state() {
    let body = |name, parent| serde_json::Value::Object(metadata_update_body(name, parent));
    assert_eq!(
        body(Some("name"), ParentChange::Unchanged),
        serde_json::json!({ "name_encrypted": "name" }),
        "unchanged: no parent_id key"
    );
    assert_eq!(
        body(Some("name"), ParentChange::TopLevel),
        serde_json::json!({ "name_encrypted": "name", "parent_id": null }),
        "the top level: an explicit null"
    );
    assert_eq!(
        body(None, ParentChange::Folder("folder-1")),
        serde_json::json!({ "parent_id": "folder-1" }),
        "a folder: its id"
    );
    assert_eq!(ParentChange::to(None), ParentChange::TopLevel);
    assert_eq!(ParentChange::to(Some("folder-1")), ParentChange::Folder("folder-1"));
}
