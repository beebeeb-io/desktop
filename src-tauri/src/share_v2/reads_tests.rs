use super::reads::*;
use beebeeb_core::auth_flow::{HttpResponse, Method};
use beebeeb_core::share_flow::ShareFlowError;
use serde_json::json;

const ID: &str = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";

#[test]
fn request_shapes() {
    let claim = post(invite_path(ID, "/claim").unwrap());
    assert_eq!(claim.method, Method::Post);
    assert_eq!(claim.path, format!("/api/v1/shares/invites/{ID}/claim"));
    assert_eq!(invite_path(ID, "").unwrap(), format!("/api/v1/shares/invites/{ID}"));
    assert_eq!(invite_path("../x", "/hide"), Err("not-found".to_string()));
}

#[test]
fn errors_map_to_the_windows_reasons() {
    let reason = |status: u16| {
        super::flows::flow_reason(ShareFlowError::from_status(&HttpResponse::from_text(
            status, "{}", None,
        )))
    };
    assert_eq!(reason(404), "not-found");
    assert_eq!(reason(401), "signed-out");
    assert_eq!(reason(500), "server");
}

#[test]
fn only_an_invited_invite_is_a_card_and_a_member_id_is_the_invite_id() {
    let inv = json!({"id": ID, "status": "invited", "sender_email": "lotte@x.nl", "is_folder": true});
    let row = invitation_row(&inv, "Autumn shoot".into()).unwrap();
    assert_eq!(row["from"], "lotte@x.nl");
    assert_eq!(row["isFolder"], true);
    assert!(invitation_row(&json!({"id": ID, "status": "approved"}), "x".into()).is_none());
    let m = member_row(&json!({"id": ID, "recipient_email": "mia@x.nl", "status": "claimed", "permission": "read"}))
        .unwrap();
    assert_eq!(
        (m["id"].as_str(), m["access"].as_str(), m["invited"].as_bool()),
        (Some(ID), Some("Can view"), Some(true))
    );
    let m = member_row(&json!({"id": ID, "recipient_email": "j@x.nl", "status": "approved", "permission": "write"}))
        .unwrap();
    assert_eq!(
        (m["access"].as_str(), m["invited"].as_bool()),
        (Some("Can edit"), Some(false))
    );
    assert!(member_row(&json!({"id": ID, "recipient_email": "j@x.nl", "status": "denied"})).is_none());
}

#[test]
fn rows_carry_no_secret_field() {
    let link = link_row(
        &json!({"file_id": ID, "token": "t", "url": "u#key=k", "owner_wrapped_key": "w", "owner_wrapped_token": "w", "has_passphrase": true, "revoked": false}),
        "Budget.xlsx".into(),
    )
    .unwrap();
    assert_eq!(link["password"], true, "a flag, never the value");
    assert!(link_row(&json!({"file_id": ID, "revoked": true}), "x".into()).is_none());
    let req = request_row(&json!({"id": ID, "title": "T", "token": "t", "wrapped_private_key": "w", "wrap_nonce": "n", "files_received": 3, "max_files": 0}), None).unwrap();
    assert!(req["maxFiles"].is_null());
    let member =
        member_row(&json!({"id": ID, "recipient_email": "a@b.c", "status": "invited", "recipient_public_key": "pk"}))
            .unwrap();
    let inv = invitation_row(
        &json!({"id": ID, "status": "invited", "encrypted_file_key": "k", "sender_public_key": "p"}),
        "n".into(),
    )
    .unwrap();
    for json in [link, req, member, inv].map(|v| v.to_string()) {
        for bad in [
            "phrase", "mnemonic", "secret", "prf", "master", "otpauth", "fragment", "token", "wrapped", "key", "nonce",
            "#",
        ] {
            assert!(!json.contains(bad), "{bad} in {json}");
        }
    }
}
