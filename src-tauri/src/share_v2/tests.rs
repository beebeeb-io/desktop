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

/// P3 (review r4508300968): an ended session and a rate limit on the permanent DELETE answer the
/// window's signed-out and rate-limit words, never "Something went wrong".
#[test]
fn the_permanent_delete_answers_an_ended_session_and_a_rate_limit_by_name() {
    let mut checked = 0;
    for (status, want) in [
        (200, Ok(())),
        (204, Ok(())),
        (401, Err("signed-out")),
        (403, Err("signed-out")),
        (404, Err("not-found")),
        (429, Err("rate_limited")),
        (500, Err("failed")),
        (409, Err("failed")),
    ] {
        assert_eq!(delete_answer(status), want.map_err(str::to_string), "{status}");
        checked += 1;
    }
    assert_eq!(checked, 8);
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
        // Core v0.2.8 items 10 and 13: another spelling of the same UUID is refused, never normalized.
        "6F1C2B9E-1D1A-4C55-9A51-3F7A2B8C9D01",
        "6f1c2b9e-1d1a-4c55-9a51-3F7A2B8C9D01",
        "{6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01}",
        "urn:uuid:6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01",
    ] {
        assert_eq!(flows::item_id(bad).unwrap_err(), "not-found", "{bad}");
        assert!(!valid_ids(&[bad.to_string()]), "{bad}");
    }
}

#[test]
fn a_permanent_delete_carries_no_token_until_core_attaches_one() {
    let id = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";
    let req = permanent_delete_request(id);
    assert_eq!(req.method, beebeeb_core::auth_flow::Method::Delete);
    assert_eq!(req.path, format!("/api/v1/files/{id}/permanent"));
    assert_eq!(req.body, None);
    assert_eq!(req.auth, beebeeb_core::auth_flow::RequestAuth::Session);
    assert_eq!(req.confirm_token_header(), None);
    // The GET and POST builders the share commands use start without a token too.
    assert_eq!(flows::get("/x".into()).confirm_token_header(), None);
    assert_eq!(flows::get("/x".into()).body, None);
    assert_eq!(reads::post("/x".into()).method, beebeeb_core::auth_flow::Method::Post);
}

#[test]
fn a_live_link_is_rebuilt_only_with_a_stored_wrapped_key_that_opens() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    use beebeeb_core::share_flow::link::{CreateLink, LinkOptions};
    use serde_json::json;
    let master = MasterKey::from_bytes([3; 32]);
    let id = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";
    let options = LinkOptions {
        expires_in_hours: None,
        max_opens: None,
        passphrase: None,
    };
    // A real link's blobs, as core makes them and the server stores them.
    let (_, req) = CreateLink::start(
        "https://app.example",
        id,
        &master,
        &FileKey::from_bytes([5; 32]),
        &options,
    )
    .unwrap();
    let body = req.body.unwrap();
    let field = |k: &str| body[k].as_str().unwrap().to_string();
    let row = |wrapped: Option<String>| {
        let mut row = json!({ "owner_wrapped_key": field("owner_wrapped_key"), "owner_wrapped_token": field("owner_wrapped_token") });
        if let Some(w) = wrapped {
            row["wrapped_file_key"] = json!(w);
        }
        row
    };
    let rebuilt = flows::rebuilt_link("https://app.example", &master, &row(Some(field("wrapped_file_key")))).unwrap();
    assert!(rebuilt.starts_with(&format!("https://app.example/s/{}", field("token"))));
    // Core v0.2.8 item 9: without the stored wrapped key (today's owner listing) or with one that
    // does not open under the rebuilt key, nothing is handed out.
    assert!(flows::rebuilt_link("https://app.example", &master, &row(None)).is_none());
    assert!(flows::rebuilt_link("https://app.example", &master, &row(Some(String::new()))).is_none());
    let (_, other) = CreateLink::start(
        "https://app.example",
        id,
        &master,
        &FileKey::from_bytes([6; 32]),
        &options,
    )
    .unwrap();
    let foreign = other.body.unwrap()["wrapped_file_key"].as_str().unwrap().to_string();
    assert!(flows::rebuilt_link("https://app.example", &master, &row(Some(foreign))).is_none());
    assert!(flows::LINK_NOT_REBUILT.starts_with("unsupported"));
}
