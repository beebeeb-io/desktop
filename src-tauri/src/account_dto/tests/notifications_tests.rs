//! Activity-feed and notification decoding tests.
//!
//! Includes the partial update payload, which must serialize only the fields
//! the caller set.

use super::*;

    #[test]
    fn activity_feed_parses() {
        let json = r#"{
            "events": [
                {
                    "id": "a1",
                    "type": "file.upload",
                    "subject": "report.pdf",
                    "details": "12 MB",
                    "where": "203.0.113.7",
                    "created_at": "2026-06-01T10:00:00Z"
                },
                {
                    "id": "a2",
                    "type": "auth.login.success",
                    "subject": null,
                    "details": null,
                    "where": null,
                    "created_at": "2026-06-01T09:00:00Z"
                }
            ],
            "total": 42,
            "page": 1
        }"#;
        let feed: ActivityFeed = serde_json::from_str(json).unwrap();
        assert_eq!(feed.total, 42);
        assert_eq!(feed.page, Some(1));
        assert_eq!(feed.events[0].event_type, "file.upload");
        assert_eq!(feed.events[0].origin.as_deref(), Some("203.0.113.7"));
        assert_eq!(feed.events[1].subject, None);
    }

    #[test]
    fn notifications_parse() {
        let json = r#"{
            "notifications": [
                {
                    "id": "n1",
                    "type": "welcome",
                    "title": "Welcome to Beebeeb",
                    "body": "Your vault is ready",
                    "data": { "k": "v" },
                    "read": false,
                    "created_at": "2026-06-01T10:00:00Z"
                }
            ],
            "unread_count": 1
        }"#;
        let list: NotificationList = serde_json::from_str(json).unwrap();
        assert_eq!(list.unread_count, 1);
        assert_eq!(list.notifications[0].notification_type, "welcome");
        assert_eq!(list.notifications[0].body.as_deref(), Some("Your vault is ready"));
        assert!(list.notifications[0].data.is_some());
    }

    #[test]
    fn notification_preferences_parse_and_serialize_partial() {
        let json = r#"{
            "preferences": {
                "file_updated": true,
                "share_received": true,
                "storage_warning": true,
                "new_device_login": true,
                "backup_complete": false
            }
        }"#;
        let prefs: NotificationPreferences = serde_json::from_str(json).unwrap();
        assert!(prefs.preferences.file_updated);
        assert!(!prefs.preferences.backup_complete);

        // Partial update only serializes the fields that are Some.
        let update = NotificationPreferencesUpdate {
            backup_complete: Some(true),
            ..Default::default()
        };
        let body = serde_json::to_string(&update).unwrap();
        assert_eq!(body, r#"{"backup_complete":true}"#);
    }
