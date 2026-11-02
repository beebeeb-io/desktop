//! `credentials` tests split out of `diagnostic_redaction.rs`.
//!
//! Credential-token redaction tests, split out of `diagnostic_redaction.rs`.

use super::*;


    #[test]
    fn credential_tokens_are_still_redacted_and_counted() {
        let out = redact_for_export(
            "401 unauthorized Bearer abc.def.ghi session_token=super-secret",
            &KnownNames::new(),
        );
        assert!(out.text.contains("Bearer [redacted]"), "{}", out.text);
        assert!(out.text.contains("session_token=[redacted]"), "{}", out.text);
        assert!(!out.text.contains("abc.def.ghi"));
        assert!(!out.text.contains("super-secret"));
        assert_eq!(out.redactions, 2, "{out:?}");
    }

    // ---- task 1685 P1: stacked / variant credential label shapes ----------
    //
    // A credential label must keep the skip armed until an actual VALUE has
    // been redacted, and that value is redacted even when it looks like a UUID,
    // a number or a standard word.

    const UUID_TOKEN: &str = "123e4567-e89b-12d3-a456-426614174000";

    /// Runs both passes. The token must not appear in either output, the export
    /// pass must equal `expected` exactly and count `redactions` placeholders.
    fn assert_credential_shape(input: &str, token: &str, expected: &str, redactions: u32) {
        let export = redact_for_export(input, &KnownNames::new());
        assert!(!export.text.contains(token), "export leaked {token:?}: {}", export.text);
        assert_eq!(export.text, expected, "export text for {input:?}");
        assert_eq!(export.redactions, redactions, "export count for {input:?}: {export:?}");
        let secrets = redact_secrets_only(input);
        assert!(!secrets.contains(token), "secrets-only leaked {token:?}: {secrets}");
        assert_eq!(secrets, expected, "secrets-only text for {input:?}");
    }

    #[test]
    fn credential_authorization_colon_bearer_uuid() {
        let input = format!("Authorization: Bearer {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Authorization: Bearer [redacted]", 1);
    }

    #[test]
    fn credential_authorization_bearer_uuid_keeps_following_words() {
        let input = format!("401 Authorization: Bearer {UUID_TOKEN} retry failed");
        assert_credential_shape(&input, UUID_TOKEN, "401 Authorization: Bearer [redacted] retry failed", 1);
    }

    #[test]
    fn credential_authorization_equals_bearer() {
        let input = format!("authorization=Bearer {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "authorization=Bearer [redacted]", 1);
    }

    #[test]
    fn credential_bearer_colon() {
        let input = format!("Bearer: {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Bearer: [redacted]", 1);
    }

    #[test]
    fn credential_token_colon() {
        let input = format!("token: {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "token: [redacted]", 1);
    }

    #[test]
    fn credential_session_token_space() {
        let input = format!("session_token {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "session_token [redacted]", 1);
    }

    #[test]
    fn credential_x_session_token_header() {
        let input = format!("X-Session-Token: {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "X-Session-Token: [redacted]", 1);
    }

    #[test]
    fn credential_cookie_session_equals() {
        let input = format!("Cookie: session={UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Cookie: session=[redacted]", 1);
    }

    #[test]
    fn credential_bearer_comma_then_value() {
        let input = format!("Bearer, {UUID_TOKEN}");
        assert_credential_shape(&input, UUID_TOKEN, "Bearer, [redacted]", 1);
    }

    #[test]
    fn credential_quoted_bearer_value() {
        let input = format!("\"Bearer {UUID_TOKEN}\"");
        assert_credential_shape(&input, UUID_TOKEN, "\"Bearer [redacted]\"", 1);
    }

    #[test]
    fn credential_json_shaped_token() {
        let input = format!("{{\"token\": \"{UUID_TOKEN}\"}}");
        assert_credential_shape(&input, UUID_TOKEN, "{\"token\": \"[redacted]\"}", 1);
    }

    #[test]
    fn credential_value_that_looks_like_a_number_or_standard_word() {
        assert_credential_shape("Authorization: Bearer 4815162342 ok", "4815162342", "Authorization: Bearer [redacted] ok", 1);
        assert_credential_shape("token: failed ok", "failed", "token: [redacted] ok", 1);
    }

    #[test]
    fn credential_label_at_end_of_string_is_kept_and_nothing_else_is_touched() {
        for input in ["failed with Authorization:", "failed with Authorization: Bearer", "failed with token"] {
            let export = redact_for_export(input, &KnownNames::new());
            assert_eq!(export.text, input);
            assert_eq!(export.redactions, 0, "{export:?}");
            assert_eq!(redact_secrets_only(input), input);
        }
    }

    #[test]
    fn credential_redaction_is_idempotent_across_the_persist_and_export_passes() {
        let input = format!("Authorization: Bearer {UUID_TOKEN} \"token\": \"{UUID_TOKEN}\"");
        let once = redact_secrets_only(&input);
        assert_eq!(redact_secrets_only(&once), once);
        let export = redact_for_export(&once, &KnownNames::new());
        assert!(!export.text.contains("[["), "{}", export.text);
        assert!(!export.text.contains(UUID_TOKEN), "{}", export.text);
    }

    #[test]
    fn credential_labels_do_not_arm_beyond_the_value() {
        // Only the value after the label run is consumed; later words survive.
        let input = format!("token: {UUID_TOKEN} token: other failed");
        let export = redact_for_export(&input, &KnownNames::new());
        assert_eq!(export.text, "token: [redacted] token: [redacted] failed");
        assert_eq!(export.redactions, 2);
    }

    #[test]
    fn secrets_only_mode_does_not_touch_paths() {
        // The persisted copy keeps only credential stripping; the export pass
        // is the privacy boundary.
        let s = redact_secrets_only("write /a/b/c failed Bearer zzz");
        assert_eq!(s, "write /a/b/c failed Bearer [redacted]");
    }
