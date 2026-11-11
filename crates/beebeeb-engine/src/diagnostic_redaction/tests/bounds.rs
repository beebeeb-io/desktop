//! `bounds` tests split out of `diagnostic_redaction.rs`.
//!
//! Bounds and edge-case tests for the filter, split out of
//! `diagnostic_redaction.rs`.

use super::*;

#[test]
fn a_forged_name_marker_in_input_is_stripped() {
    let out = redact_for_export("failed \u{E000}\u{E000} ok", &KnownNames::new());
    assert!(!out.text.contains('\u{E000}'), "{:?}", out.text);
}

/// Pins the documented residual (docs/CAPABILITIES.md, the Account copy): a
/// name the state DB does not know survives when it is a standard word, and
/// is removed the moment the DB does know it.
#[test]
fn residual_unknown_standard_word_name_survives_but_a_known_one_does_not() {
    let unknown = redact_for_export("rename Payment to Letter failed", &KnownNames::new().finish());
    assert!(unknown.text.contains("Letter"), "documented residual: {}", unknown.text);
    let known = redact_for_export("rename Payment to Letter failed", &names(&["Payment", "Letter"]));
    assert!(
        !known.text.contains("Payment") && !known.text.contains("Letter"),
        "{}",
        known.text
    );
    assert!(known.redactions >= 2, "{known:?}");
    // Digits-only names and opaque ids are kept on purpose.
    let id = "0b9d5a52-7f0e-4c9e-8a6f-1f2f3a4b5c6d";
    let kept = redact_for_export(
        &format!("op {id} at http://127.0.0.1:8080/x/y failed"),
        &KnownNames::new().finish(),
    );
    assert!(
        kept.text.contains(id) && kept.text.contains("http://127.0.0.1:8080"),
        "{}",
        kept.text
    );
    assert!(!kept.text.contains("/x/y"), "{}", kept.text);
}

#[test]
fn uuids_and_numbers_survive() {
    let out = redact_for_export(
        "no state.db row for 123e4567-e89b-12d3-a456-426614174000 (chunk 3 of 12)",
        &KnownNames::new(),
    );
    assert_eq!(
        out.text,
        "no state.db row for 123e4567-e89b-12d3-a456-426614174000 (chunk 3 of 12)"
    );
    assert_eq!(out.redactions, 0);
}

#[test]
fn oversized_input_is_bounded() {
    let long = "failed ".repeat(5000);
    let out = redact_for_export(&long, &KnownNames::new());
    assert!(
        out.text.ends_with("[truncated]"),
        "{}",
        &out.text[out.text.len().saturating_sub(40)..]
    );
    assert!(out.text.split_whitespace().count() <= MAX_OUTPUT_TOKENS + 1);
}
