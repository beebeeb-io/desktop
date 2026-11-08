//! Crypto review rule 10 for B11: every DTO a B11 command returns is free of secret-shaped field names. Seen red on a
//! planted field (`planted_secret_field_is_caught`). Allow-listed: `auth_signup_phrase_words` returns a bare list of
//! the 12 words for DW-A39, the one screen that shows them once (rules 1 and 8: B11's one-time display).
use super::{PasswordView, PolicyView};

const FORBIDDEN: [&str; 10] = [
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
];

fn offending(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(obj) = value.as_object() {
        for (key, v) in obj {
            if FORBIDDEN.iter().any(|f| key.to_lowercase().contains(f)) {
                out.push(key.clone());
            }
            out.extend(offending(v));
        }
    }
    out
}

#[test]
fn no_b11_dto_carries_a_secret_shaped_field() {
    let all = [
        serde_json::to_value(PolicyView {
            code_length: 8,
            resend_after_secs: 60,
            min_length: 12,
            verify_word_count: 3,
            terms_url: Some("https://beebeeb.io/terms".into()),
            privacy_url: None,
        })
        .unwrap(),
        serde_json::to_value(PasswordView {
            length: 10,
            min_length: 12,
            missing: 2,
            strength: "too_short",
            hint: "more",
        })
        .unwrap(),
    ];
    assert_eq!(all.len(), 2);
    for dto in &all {
        assert_eq!(offending(dto), Vec::<String>::new(), "{dto}");
    }
}

#[test]
fn planted_secret_field_is_caught() {
    for word in FORBIDDEN {
        let planted = serde_json::json!({ "min_length": 12, "nested": { format!("x_{word}"): "x" } });
        assert_eq!(offending(&planted).len(), 1, "{word}");
    }
}
