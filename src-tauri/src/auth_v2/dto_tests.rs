//! Crypto review rule 10: every DTO a B10 command returns is free of secret-shaped field names. Seen red on a planted
//! field (`planted_secret_field_is_caught`).
use super::approve::{ApprovalResult, ApprovalReview};
use super::device::DeviceRequestView;
use super::passkey::PasskeySignInOutcome;

const FORBIDDEN: [&str; 10] =
    ["password", "phrase", "mnemonic", "secret", "prf", "master", "backup_codes", "otpauth", "fragment", "token"];

/// Field names of a serialized DTO that contain a forbidden word. `prf_unlocked` is the one allow-listed flag: a
/// boolean saying whether the passkey could unlock the vault, never PRF output (rule 6's `LoginOutcome` flags).
fn offending(value: &serde_json::Value) -> Vec<String> {
    const ALLOW: [&str; 1] = ["prf_unlocked"];
    let mut out = Vec::new();
    if let Some(obj) = value.as_object() {
        for (key, v) in obj {
            if !ALLOW.contains(&key.as_str()) && FORBIDDEN.iter().any(|f| key.to_lowercase().contains(f)) {
                out.push(key.clone());
            }
            out.extend(offending(v));
        }
    }
    out
}

fn samples() -> Vec<serde_json::Value> {
    vec![
        serde_json::to_value(DeviceRequestView {
            status: "waiting".into(),
            user_code: Some("K7QM-4WTD".into()),
            seconds_left: 252,
            device_name: Some("Anna-Laptop".into()),
            qr_svg: Some("<svg/>".into()),
        })
        .unwrap(),
        serde_json::to_value(ApprovalReview {
            code: "K7QM-4WTD".into(),
            country: Some("DE".into()),
            ip: "203.0.113.7".into(),
            requested_at: "2026-11-08T10:00:00Z".into(),
            expires_at: "2026-11-08T10:05:00Z".into(),
            seconds_left: 240,
            device_name: Some("Anna's iPhone".into()),
            client: Some("ios".into()),
            os: Some("iOS".into()),
        })
        .unwrap(),
        serde_json::to_value(ApprovalResult { status: "approved".into() }).unwrap(),
        serde_json::to_value(PasskeySignInOutcome { signed_in: true, prf_unlocked: false }).unwrap(),
    ]
}

#[test]
fn no_b10_dto_carries_a_secret_shaped_field() {
    let all = samples();
    assert_eq!(all.len(), 4);
    for dto in &all {
        assert_eq!(offending(dto), Vec::<String>::new(), "{dto}");
    }
}

#[test]
fn planted_secret_field_is_caught() {
    for word in FORBIDDEN {
        let planted = serde_json::json!({ "status": "approved", "nested": { format!("session_{word}"): "x" } });
        assert_eq!(offending(&planted).len(), 1, "{word}");
    }
}
