use super::*;

#[test]
fn display_form_drops_the_key_part() {
    let link = beebeeb_core::share_link::build("https://app.example", "tok123", "a+b/c=");
    assert!(link.contains('#'));
    let shown = display_form(&link);
    assert!(!shown.contains('#'));
    assert!(!shown.contains("key="));
    assert!(shown.ends_with("/s/tok123"));
}

#[test]
fn ids_are_checked_before_anything_is_sent() {
    let a = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01".to_string();
    let b = "0b6c1d2e-3f40-4a51-8b62-7c8d9e0f1a2b".to_string();
    assert!(valid_ids(&[a.clone(), b]));
    assert!(!valid_ids(&[]));
    assert!(!valid_ids(&["../etc".to_string()]));
    assert!(!valid_ids(&[a.clone(), a]));
}

#[test]
fn dtos_carry_no_secret_field() {
    let lists = serde_json::to_string(&ShareLists::default()).unwrap();
    let link = serde_json::to_string(&CopiedLink {
        display: "https://app.example/s/t".into(),
    })
    .unwrap();
    for json in [lists, link] {
        for bad in [
            "password",
            "phrase",
            "mnemonic",
            "secret",
            "prf",
            "master",
            "backup_codes",
            "otpauth",
            "fragment",
            "token",
            "key",
        ] {
            assert!(!json.contains(bad), "{bad} in {json}");
        }
    }
}

#[test]
fn only_a_live_file_link_is_rebuilt() {
    use serde_json::json;
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z")
        .unwrap()
        .to_utc();
    assert!(flows::live_link(&json!({"id": "a", "expires_at": null}), now));
    assert!(flows::live_link(
        &json!({"expires_at": "2026-11-09T00:00:00Z", "share_type": "file"}),
        now
    ));
    assert!(
        !flows::live_link(&json!({"expires_at": "2026-11-08T11:59:59Z"}), now),
        "expired"
    );
    assert!(!flows::live_link(&json!({"revoked": true}), now), "revoked");
    assert!(
        !flows::live_link(&json!({"share_type": "bundle"}), now),
        "a bundle is not this item's link"
    );
}

#[test]
fn ids_must_be_canonical_uuids() {
    let id = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";
    assert_eq!(flows::item_id(id).unwrap(), id);
    for bad in [
        "",
        "../x",
        "6f1c2b9e1d1a4c559a513f7a2b8c9d01",
        "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01?x=1",
    ] {
        assert_eq!(flows::item_id(bad).unwrap_err(), "not-found", "{bad}");
    }
}

#[test]
fn folder_refusals_keep_their_code_and_server_codes_never_leak_text() {
    use beebeeb_core::share_flow::ShareFlowError;
    assert_eq!(
        flows::flow_reason(ShareFlowError::FolderKeyUnverified),
        "folder_key_unverified"
    );
    assert_eq!(
        flows::flow_reason(ShareFlowError::FolderNeedsUpgrade),
        "folder_needs_upgrade"
    );
    assert_eq!(flows::flow_reason(ShareFlowError::NotFound), "not-found");
    assert_eq!(
        flows::flow_reason(ShareFlowError::Refused("plan_required")),
        "plan_required"
    );
    assert_eq!(flows::flow_reason(ShareFlowError::Server { status: 500 }), "server");
}
