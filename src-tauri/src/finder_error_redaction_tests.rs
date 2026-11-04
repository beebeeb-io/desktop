    use super::*;
    use crate::finder_setup::error::{COCOA_DOMAIN, FpError, POSIX_DOMAIN};

    const PATH_MESSAGE: &str = "The folder \u{201c}/Users/sam/Secret Folder\u{201d} already exists.";

    fn shim_body(signature: &str) -> String {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let marker = format!("#[cfg(target_os = \"macos\")]\n{signature} {{\n");
        let start = source.find(&marker).unwrap_or_else(|| panic!("{signature} exists")) + marker.len();
        let end = start + source[start..].find("\n}\n").expect("the shim ends");
        source[start..end].to_string()
    }

    #[test]
    fn the_bridge_error_text_a_person_can_see_is_redacted() {
        let error = || FpError::new(COCOA_DOMAIN, 516, PATH_MESSAGE).with_underlying(POSIX_DOMAIN, 17);
        let removed = user_facing_fp::<()>(Err(error())).unwrap_err();
        let located = user_facing_fp::<Option<String>>(Err(error())).unwrap_err();
        for text in [&removed, &located] {
            assert_eq!(text, "NSCocoaErrorDomain 516 (underlying NSPOSIXErrorDomain 17)");
            assert!(
                !text.contains("Secret Folder") && !text.contains("/Users/sam"),
                "{text}"
            );
        }
        assert_eq!(
            user_facing_fp(Ok::<_, FpError>(Some("/p".to_string()))),
            Ok(Some("/p".to_string()))
        );
    }

    /// Lead ruling T1-4 / 11 (task 8): `finder_domain_user_enabled` returns this to the frontend.
    /// The plan's snippet used `FpError`'s `Display` here, which carries the OS's message.
    #[test]
    fn the_domain_user_enabled_read_goes_through_the_redacting_conversion() {
        let body = shim_body("fn file_provider_domain_user_enabled() -> Result<Option<bool>, String>");
        assert!(
            body.contains("user_facing_fp(finder_setup::macos_ports::domain_state())"),
            "{body}"
        );
        assert!(
            !body.contains("to_string") && !body.contains("{e}") && !body.contains("{error}"),
            "builds text from Display: {body}"
        );
    }

    /// Lead ruling T8-gate-all (4) moved the removal into the reconciler and the other reads behind the
    /// gate: no macOS `remove_file_provider_domain` is left to redact, and (task 1885) the open and the
    /// reveal go through their gated wrappers and `finder_open_outcome`, which is the one redacting
    /// conversion for them (`finder_open`'s tests pin that it carries domain and code only).
    #[test]
    fn the_open_and_the_reveal_go_through_the_gate_and_the_one_redacting_conversion() {
        use crate::source_pin::squeeze;
        let open = shim_body("fn file_provider_open_location() -> Result<(), String>");
        assert_eq!(
            squeeze(&open),
            squeeze(
                "finder_open::finder_open_outcome(finder_setup::macos_ports::open_location(), finder_open::NOTHING_TO_OPEN,)"
            )
        );
        let reveal = shim_body("fn file_provider_reveal_item(item_id: &str) -> Result<(), String>");
        assert_eq!(
            squeeze(&reveal),
            squeeze(
                "finder_open::finder_open_outcome(finder_setup::macos_ports::reveal_item(item_id), finder_open::NOTHING_TO_SHOW,)"
            )
        );
        for body in [&open, &reveal] {
            assert!(!body.contains("to_string"), "builds text from Display: {body}");
        }
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        assert!(
            !source.contains("#[cfg(target_os = \"macos\")]\nfn remove_file_provider_domain()"),
            "a Mac has no direct removal: the reconciler owns it"
        );
    }
