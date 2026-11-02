//! `credential_shapes` items split out of `diagnostic_redaction.rs`.

use super::*;

/// Words that introduce a credential value (compared lowercase, punctuation and
/// the trailing `:`/`=` removed).
pub(super) const CREDENTIAL_LABELS: &[&str] = &[
    "bearer",
    "token",
    "session",
    "authorization",
    "session_token",
    "session-token",
    "x-session-token",
    "access_token",
    "refresh_token",
    "x-auth-token",
    "api_key",
    "x-api-key",
    "cookie",
    "set-cookie",
];

pub(super) enum CredentialShape<'a> {
    /// A bare label: `Bearer`, `Authorization:`, `"token":`, `Bearer,`.
    Label,
    /// Only punctuation (`-`, `:`, `"`): carries no value.
    Punctuation,
    /// `key=value` / `key:value` glued in one whitespace part, key a label.
    LabelledValue {
        key: &'a str,
        delim: char,
        /// The value is itself a label (`authorization=Bearer`), so the real
        /// secret is the NEXT part.
        value_is_label: bool,
    },
    Other,
}

pub(super) fn label_core(text: &str) -> String {
    text.chars()
        .filter(|c| *c != NAME_MARK)
        .collect::<String>()
        .trim_matches(|c| EDGE_PUNCT.contains(&c) || c == '=')
        .to_ascii_lowercase()
}

pub(super) fn is_credential_label(text: &str) -> bool {
    let core = label_core(text);
    CREDENTIAL_LABELS.contains(&core.as_str())
}

pub(super) fn credential_shape(part: &str) -> CredentialShape<'_> {
    if is_credential_label(part) {
        return CredentialShape::Label;
    }
    if label_core(part).is_empty() {
        return CredentialShape::Punctuation;
    }
    if let Some(idx) = part.find(['=', ':']) {
        let (key, rest) = part.split_at(idx);
        let delim = rest.chars().next().unwrap_or('=');
        let value = &rest[delim.len_utf8()..];
        if is_credential_label(key) {
            if label_core(value).is_empty() {
                // `Authorization:` / `"token":` with the delimiter attached.
                return CredentialShape::Label;
            }
            return CredentialShape::LabelledValue {
                key,
                delim,
                value_is_label: is_credential_label(value),
            };
        }
    }
    CredentialShape::Other
}

/// Replacement for a credential value, keeping only edge punctuation so
/// `"Bearer <tok>"` stays well-formed.
pub(super) fn redact_value(part: &str) -> String {
    let part = part.replace(NAME_MARK, "");
    // Already a placeholder (the persisted copy is redacted once at write and
    // again at export): stay idempotent rather than nesting `[[redacted]]`.
    if part.contains("[redacted]") {
        return part;
    }
    let trimmed_start = part.trim_start_matches(|c| EDGE_PUNCT.contains(&c));
    let lead = &part[..part.len() - trimmed_start.len()];
    let trail = &part[part.trim_end_matches(|c| EDGE_PUNCT.contains(&c)).len()..];
    format!("{lead}[redacted]{trail}")
}
