//! `account_session_list`'s decode: the shape the server's `list_sessions` builds.
//!
//! The server's `GET /api/v1/account/sessions` (docs/plans/023-settings-billing.md, Step 2)
//! selects `token AS id, device_name, device_kind, country_code, last_active_at,
//! first_seen_recognized` and adds `is_current`; it does not send `created_at`. A required
//! `created_at` made the whole list fail to decode, so the pane showed no devices at all.

use crate::account_dto::AccountSessionList;

/// A captured-shaped answer: this PC (the session that made the request) and one other.
/// Device names are invented; no token, response body or email is logged.
const THIS_PC_AND_ONE_OTHER: &str = r#"{
    "sessions": [
        {
            "id": "sess-this-pc-0001",
            "device_name": "Studio PC",
            "device_kind": "windows",
            "country_code": "NL",
            "last_active_at": "2026-11-09T22:14:00Z",
            "is_current": true,
            "is_recognized": true
        },
        {
            "id": "sess-other-0002",
            "device_name": "Pocket Phone",
            "device_kind": "ios",
            "country_code": null,
            "last_active_at": null,
            "is_current": false,
            "is_recognized": false
        }
    ]
}"#;

#[test]
fn account_session_list_decodes_this_pc() {
    let list: AccountSessionList =
        serde_json::from_str(THIS_PC_AND_ONE_OTHER).expect("the server's session list decodes");

    assert_eq!(list.sessions.len(), 2);

    let this_pc = &list.sessions[0];
    assert_eq!(this_pc.id, "sess-this-pc-0001");
    assert_eq!(this_pc.device_name, "Studio PC");
    assert_eq!(this_pc.device_kind, "windows");
    assert_eq!(this_pc.last_active_at.as_deref(), Some("2026-11-09T22:14:00Z"));
    assert_eq!(this_pc.country_code.as_deref(), Some("NL"));
    assert!(this_pc.is_current, "the session that made the request is current");

    let other = &list.sessions[1];
    assert_eq!(other.id, "sess-other-0002");
    assert_eq!(other.device_name, "Pocket Phone");
    assert_eq!(other.device_kind, "ios");
    assert_eq!(other.last_active_at, None);
    assert_eq!(other.country_code, None);
    assert!(!other.is_current);
}

#[test]
fn account_session_list_empty_is_a_success_not_an_error() {
    let list: AccountSessionList = serde_json::from_str(r#"{"sessions":[]}"#).expect("an empty list is a valid answer");
    assert_eq!(list.sessions.len(), 0);
}
