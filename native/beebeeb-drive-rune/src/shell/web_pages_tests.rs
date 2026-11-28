//! Tests of the web pages the frames open (`shell::web_pages`): every name resolves or its control is not drawn, every
//! address is allowed, and the support draft is percent-encoded.

use super::*;

#[test]
fn every_name_the_frames_ask_for_resolves_or_its_control_is_not_drawn() {
    // The names the frames queue as `Intent::Web(..)` (grep `Intent::Web(` in `frames/`).
    for name in [
        "terms-and-privacy-policy",
        "encryption-core-on-github",
        "beebeeb.io",
        "desktop-source",
        "plan",
        "delete-account",
        "privacy",
        "export-my-data",
        "support",
    ] {
        let address = url(name).unwrap_or_else(|| panic!("no address for {name}"));
        assert!(
            crate::engine_host::signin::is_allowed_address(address),
            "{name}: {address}"
        );
    }
    // The one name with no address is hidden, not drawn as a dead link.
    assert_eq!(url("about-audits"), None);
    assert_eq!(url("nothing"), None);
}

#[test]
fn every_address_in_the_table_is_allowed_and_names_are_unique() {
    for (name, address) in NAMES {
        assert!(crate::engine_host::signin::is_allowed_address(address), "{name}");
        assert_eq!(NAMES.iter().filter(|(n, _)| n == name).count(), 1, "{name} twice");
    }
}

#[test]
fn the_support_draft_percent_encodes_its_subject() {
    assert_eq!(
        support_mailto("Help signing in"),
        "mailto:support@beebeeb.io?subject=Help%20signing%20in"
    );
    assert_eq!(support_mailto("a&b\nc"), "mailto:support@beebeeb.io?subject=a%26b%0Ac");
}

#[test]
fn the_opener_refuses_file_javascript_and_a_bare_path() {
    use crate::engine_host::signin::{OPENED, open_web_page};
    OPENED.with(|o| o.borrow_mut().clear());
    for bad in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "/etc/passwd",
        "C:\\Windows\\notepad.exe",
        "",
    ] {
        assert!(open_web_page(bad).is_err(), "{bad} must be refused");
    }
    assert!(OPENED.with(|o| o.borrow().is_empty()), "nothing was opened");
    for good in [
        "https://beebeeb.io",
        "http://beebeeb.io",
        "mailto:support@beebeeb.io",
        "ms-settings:notifications",
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension",
    ] {
        assert!(open_web_page(good).is_ok(), "{good}");
    }
    assert_eq!(OPENED.with(|o| o.borrow().len()), 5);
}

#[test]
fn legal_links_open_the_live_legal_pages_from_one_constant() {
    assert_eq!(TERMS_PAGE, "https://beebeeb.io/legal/terms/");
    assert_eq!(PRIVACY_PAGE, "https://beebeeb.io/legal/privacy/");
    assert_eq!(LEGAL_PAGE, "https://beebeeb.io/legal/");
    assert_eq!(url("terms-and-privacy-policy"), Some(LEGAL_PAGE));
    assert!(crate::engine_host::signin::is_allowed_address(TERMS_PAGE));
    assert!(crate::engine_host::signin::is_allowed_address(PRIVACY_PAGE));
    // No legal text points at the web app, and the sign-up rows name the real addresses.
    let signup = include_str!("../frames/signin/view_create.rs");
    assert!(signup.contains("Opens beebeeb.io/legal/terms"));
    assert!(signup.contains("Opens beebeeb.io/legal/privacy"));
}
