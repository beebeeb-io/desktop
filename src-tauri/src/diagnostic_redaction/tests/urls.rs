//! `urls` tests split out of `diagnostic_redaction.rs`.
//!
//! URL summarisation tests, split out of `diagnostic_redaction.rs`.

use super::*;

#[test]
fn url_keeps_host_and_drops_path_and_query() {
    let out = redact_for_export(
        "HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:8080/api/v1/files/Tax%202025?cursor=abc)",
        &KnownNames::new(),
    );
    assert!(out.text.contains("401 Unauthorized"), "{}", out.text);
    assert!(out.text.contains("http://127.0.0.1:8080/[path]"), "{}", out.text);
    assert!(!out.text.contains("Tax"), "{}", out.text);
    assert!(!out.text.contains("cursor"), "{}", out.text);
}

#[test]
fn url_with_userinfo_is_not_trusted() {
    let out = redact_for_export("request to https://guus:pw@evil.example/x failed", &KnownNames::new());
    assert!(!out.text.contains("guus"), "{}", out.text);
    assert!(!out.text.contains("evil"), "{}", out.text);
}
