//! `tokens` items split out of `diagnostic_redaction.rs`.

use super::*;

pub(super) enum Filtered {
    Keep,
    /// Replacement text and whether it counts as a redaction.
    Replace(String, bool),
}

pub(super) const EDGE_PUNCT: &[char] = &[
    '(', ')', '"', '\'', '`', '[', ']', ',', ';', ':', '<', '>', '{', '}', '!', '?',
];

pub(super) fn filter_token(token: &str) -> Filtered {
    let core = token.trim_matches(|c| EDGE_PUNCT.contains(&c));
    let lead = &token[..token.len() - token.trim_start_matches(|c| EDGE_PUNCT.contains(&c)).len()];
    let trail = &token[token.trim_end_matches(|c| EDGE_PUNCT.contains(&c)).len()..];

    if let Some(url) = summarise_url(core) {
        let changed = url.1;
        return Filtered::Replace(format!("{lead}{}{trail}", url.0), changed);
    }
    if is_path_like(core) {
        return Filtered::Replace("[path]".into(), true);
    }
    if core.contains(NAME_MARK) {
        return Filtered::Replace("[name]".into(), true);
    }
    if core.is_empty() || is_standard_word(core) {
        return Filtered::Keep;
    }
    Filtered::Replace("[name]".into(), true)
}

/// `http(s)://host[:port]/anything` becomes `http(s)://host/[path]`. Returns
/// the text plus whether anything was dropped. A host containing anything but
/// host characters (userinfo, odd bytes) is not a URL we trust: `None`, so the
/// caller treats it as a path.
pub(super) fn summarise_url(core: &str) -> Option<(String, bool)> {
    let (scheme, rest) = if let Some(rest) = core.strip_prefix("https://") {
        ("https://", rest)
    } else if let Some(rest) = core.strip_prefix("http://") {
        ("http://", rest)
    } else {
        return None;
    };
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = &rest[..host_end];
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if !host_ok {
        return None;
    }
    let tail = &rest[host_end..];
    if tail.is_empty() {
        Some((format!("{scheme}{host}"), false))
    } else {
        Some((format!("{scheme}{host}/[path]"), true))
    }
}

pub(super) fn is_path_like(core: &str) -> bool {
    if core.contains('/') || core.contains('\\') || core.starts_with('~') {
        return true;
    }
    let bytes = core.as_bytes();
    if bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    core.to_ascii_lowercase().contains("cloudstorage")
}

pub(super) fn is_standard_word(core: &str) -> bool {
    if is_uuid(core) {
        return true;
    }
    for piece in core.split(|c: char| !c.is_alphanumeric()) {
        if piece.is_empty() {
            continue;
        }
        if piece.len() <= 12 && piece.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if !piece.is_ascii() || !STANDARD_WORDS.contains(&piece.to_ascii_lowercase().as_str()) {
            return false;
        }
    }
    // A token with no alphanumerics at all (`-`, `...`) carries nothing.
    true
}

pub(super) fn is_uuid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && [8usize, 4, 4, 4, 12]
            .iter()
            .zip(&groups)
            .all(|(len, g)| g.len() == *len && g.bytes().all(|b| b.is_ascii_hexdigit()))
}
