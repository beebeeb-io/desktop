//! `names` tests split out of `diagnostic_redaction.rs`.
//!
//! Known-name, path and standard-word allow-list tests, split out of
//! `diagnostic_redaction.rs`. The `names()` harness and `ISSUE_ERROR` stay in the hub.

use super::*;

#[test]
fn standard_words_are_lowercase_ascii() {
    for w in STANDARD_WORDS {
        assert!(
            w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "vocabulary entry {w:?} must be lowercase ascii"
        );
    }
}

#[test]
fn issue_example_path_and_known_multiword_name_are_gone() {
    let out = redact_for_export(ISSUE_ERROR, &names(&["Tax 2025/aangifte.pdf"]));
    for leaked in [
        "/Users/guus",
        "CloudStorage",
        "Beebeeb-Drive",
        "Tax 2025",
        "Tax",
        "2025",
        "aangifte",
        ".pdf",
    ] {
        assert!(!out.text.contains(leaked), "{leaked:?} leaked: {}", out.text);
    }
    assert!(
        out.text.contains("failed"),
        "the failure word must survive: {}",
        out.text
    );
    assert!(out.redactions >= 1, "redactions must be counted: {out:?}");
}

#[test]
fn multiword_name_is_replaced_even_when_the_path_is_unknown() {
    // The name is known (from the DB) but the error embeds it with spaces
    // and no slash: substring scan must catch it before tokenising.
    let out = redact_for_export("upload of Tax 2025 folder failed", &names(&["Tax 2025"]));
    assert!(!out.text.contains("Tax"), "{}", out.text);
    assert!(!out.text.contains("2025"), "{}", out.text);
    assert!(out.text.contains("failed"));
    assert_eq!(out.redactions, 1, "{out:?}");
}

#[test]
fn longest_known_name_wins_over_its_prefix() {
    let out = redact_for_export("x Tax 2025 y Tax z", &names(&["Tax", "Tax 2025"]));
    assert!(!out.text.contains("2025"), "{}", out.text);
    assert!(!out.text.contains("Tax"), "{}", out.text);
}

#[test]
fn names_match_case_insensitively_at_word_boundaries_only() {
    let out = redact_for_export("upload TAX 2025 failed, taxi failed", &names(&["tax 2025"]));
    assert!(!out.text.to_lowercase().contains("tax 2025"), "{}", out.text);
    // `taxi` is not the known name `tax`/`tax 2025`; it is also not a
    // standard word, so the allow-list replaces it rather than the scan.
    assert!(!out.text.contains("taxi"), "{}", out.text);
}

#[test]
fn unknown_names_are_removed_by_the_allow_list() {
    // No DB knowledge at all: a never-seen name cannot survive.
    let out = redact_for_export(
        "cannot read Holiday photos Zoë.heic: Permission denied (os error 13)",
        &KnownNames::new(),
    );
    for leaked in ["Holiday", "photos", "Zoë", "heic"] {
        assert!(!out.text.contains(leaked), "{leaked} leaked: {}", out.text);
    }
    assert!(out.text.contains("Permission denied (os error 13)"), "{}", out.text);
}

#[test]
fn path_shapes_are_all_redacted() {
    for p in [
        "/Users/guus/Documents/x.txt",
        "~/Documents/x.txt",
        "C:\\home\\guus\\Documents\\x.txt",
        "D:\\x.txt",
        "\\\\server\\share\\x.txt",
        "Library/CloudStorage/Beebeeb-Drive",
        "relative/dir/file",
    ] {
        let out = redact_for_export(&format!("write {p}: denied"), &KnownNames::new());
        assert!(out.text.contains("[path]"), "no [path] for {p}: {}", out.text);
        assert!(!out.text.contains("Users"), "{p}: {}", out.text);
        assert!(!out.text.contains("Documents"), "{p}: {}", out.text);
        assert!(!out.text.contains("Beebeeb-Drive"), "{p}: {}", out.text);
        assert!(!out.text.contains("x.txt"), "{p}: {}", out.text);
    }
}
