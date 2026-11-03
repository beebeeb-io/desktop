//! Account activity, security, session and device decoding tests.
//!
//! Each fixture mirrors a server response body; the session and sync-session
//! tests also cover fields the server may omit.

use super::*;

    #[test]
    fn billing_usage_parses_percentage() {
        let json = r#"{ "used_bytes": 500, "quota_bytes": 1000, "percentage": 0.5 }"#;
        let u: BillingUsage = serde_json::from_str(json).unwrap();
        assert_eq!(u.used_bytes, 500);
        assert!((u.percentage - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn account_activity_parses_events_and_summary() {
        let json = r#"{
            "events": [
                {
                    "id": "e1",
                    "type": "auth.login.success",
                    "description": "Signed in",
                    "category": "auth",
                    "outcome": "success",
                    "device": "Chrome on macOS",
                    "country_code": "NL",
                    "created_at": "2026-06-01T10:00:00Z"
                },
                {
                    "id": "e2",
                    "type": "file.upload",
                    "description": "Uploaded a file",
                    "category": "general",
                    "outcome": "success",
                    "device": null,
                    "country_code": null,
                    "created_at": "2026-06-01T11:00:00Z"
                }
            ],
            "summary": {
                "last_login_at": "2026-06-01T10:00:00Z",
                "last_login_device": "Chrome on macOS",
                "active_sessions": 2,
                "active_shares": 1,
                "security_score": "Strong"
            }
        }"#;
        let a: AccountActivity = serde_json::from_str(json).unwrap();
        assert_eq!(a.events.len(), 2);
        assert_eq!(a.events[0].event_type, "auth.login.success");
        assert_eq!(a.events[1].device, None);
        assert_eq!(a.summary.active_sessions, 2);
        // security_score is the LABEL string, not numeric.
        assert_eq!(a.summary.security_score, "Strong");
    }

    #[test]
    fn security_score_parses() {
        let json = r#"{
            "score": 4,
            "max": 5,
            "label": "Strong",
            "factors": [
                { "key": "email_verified", "satisfied": true },
                { "key": "two_factor_enabled", "satisfied": false }
            ]
        }"#;
        let s: SecurityScore = serde_json::from_str(json).unwrap();
        assert_eq!(s.score, 4);
        assert_eq!(s.max, Some(5));
        assert_eq!(s.label, "Strong");
        assert_eq!(s.factors.len(), 2);
        assert!(s.factors[0].satisfied);
        assert!(!s.factors[1].satisfied);
    }

    #[test]
    fn security_score_db_error_fallback_parses() {
        // The server's DB-error path omits `max` and uses label "Unknown".
        let json = r#"{ "score": 0, "label": "Unknown", "factors": [] }"#;
        let s: SecurityScore = serde_json::from_str(json).unwrap();
        assert_eq!(s.score, 0);
        assert_eq!(s.max, None);
        assert!(s.factors.is_empty());
    }

    #[test]
    fn account_sessions_parse() {
        let json = r#"{
            "sessions": [
                {
                    "id": "s1",
                    "device_name": "MacBook Pro",
                    "device_kind": "desktop",
                    "country_code": "NL",
                    "last_active_at": "2026-06-01T12:00:00Z",
                    "created_at": "2026-05-01T09:00:00Z",
                    "is_current": true
                },
                {
                    "id": "s2",
                    "device_name": "Unknown device",
                    "device_kind": "web",
                    "country_code": null,
                    "last_active_at": null,
                    "created_at": "2026-05-02T09:00:00Z",
                    "is_current": false
                }
            ]
        }"#;
        let list: AccountSessionList = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions.len(), 2);
        assert!(list.sessions[0].is_current);
        assert_eq!(list.sessions[1].country_code, None);
        assert_eq!(list.sessions[1].last_active_at, None);
    }

    #[test]
    fn revoke_all_result_parses() {
        let json = r#"{ "revoked": 3 }"#;
        let r: RevokeAllResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.revoked, 3);
    }

    #[test]
    fn devices_parse() {
        let json = r#"{
            "devices": [
                {
                    "id": "d1",
                    "hostname": "guus-mbp",
                    "platform": "macos",
                    "bb_version": "0.3.1",
                    "last_seen": "2026-06-01T12:00:00Z",
                    "created_at": "2026-05-01T09:00:00Z",
                    "session_count": 2
                },
                {
                    "id": "d2",
                    "hostname": "guus-pc",
                    "platform": "windows",
                    "bb_version": null,
                    "last_seen": "2026-06-02T12:00:00Z",
                    "created_at": "2026-05-10T09:00:00Z",
                    "session_count": 0
                }
            ]
        }"#;
        let list: ClientDeviceList = serde_json::from_str(json).unwrap();
        assert_eq!(list.devices.len(), 2);
        assert_eq!(list.devices[0].session_count, 2);
        assert_eq!(list.devices[1].bb_version, None);
    }

    #[test]
    fn client_sync_sessions_parse() {
        let json = r#"{
            "sessions": [
                {
                    "id": "cs1",
                    "name": "Documents sync",
                    "session_type": "sync",
                    "local_path": "/Users/guus/Beebeeb",
                    "remote_path": "/",
                    "status": "active",
                    "heartbeat_interval_secs": 60,
                    "alert_after_missed": 5,
                    "created_at": "2026-05-01T09:00:00Z",
                    "last_heartbeat": "2026-06-01T12:00:00Z",
                    "files_synced": 100,
                    "files_total": 120,
                    "bytes_synced": 1000,
                    "bytes_total": 2000,
                    "heartbeat_status": "healthy",
                    "current_file": "report.pdf",
                    "device_hostname": "guus-mbp",
                    "device_platform": "macos",
                    "device_id": "d1",
                    "speed_bps": 5000000
                }
            ]
        }"#;
        let list: ClientSyncSessionList = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions.len(), 1);
        let s = &list.sessions[0];
        assert_eq!(s.local_path.as_deref(), Some("/Users/guus/Beebeeb"));
        assert_eq!(s.files_synced, Some(100));
        assert_eq!(s.speed_bps, Some(5000000));
    }

    #[test]
    fn client_sync_session_tolerates_missing_speed_bps() {
        // speed_bps is absent if the heartbeat try_get fails server-side.
        let json = r#"{
            "sessions": [
                {
                    "id": "cs1",
                    "name": "s",
                    "session_type": "sync",
                    "remote_path": "/",
                    "status": "paused",
                    "created_at": "2026-05-01T09:00:00Z",
                    "device_hostname": "h",
                    "device_platform": "linux",
                    "device_id": "d"
                }
            ]
        }"#;
        let list: ClientSyncSessionList = serde_json::from_str(json).unwrap();
        assert_eq!(list.sessions[0].speed_bps, None);
        assert_eq!(list.sessions[0].local_path, None);
    }
