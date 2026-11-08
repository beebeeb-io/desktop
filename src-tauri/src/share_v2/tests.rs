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
