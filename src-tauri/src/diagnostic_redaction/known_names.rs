//! `known_names` items split out of `diagnostic_redaction.rs`.

use super::*;

/// Plaintext names the local daemon knows about. Built from the state DB
/// (file paths, queued targets, activity rows) plus the sync-root path.
#[derive(Debug, Default, Clone)]
pub struct KnownNames {
    pub(super) names: Vec<String>,
}

impl KnownNames {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a single name verbatim (also usable for a whole relative path).
    pub fn add_name(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." {
            return;
        }
        self.names.push(name.to_ascii_lowercase());
    }

    /// Add a path as a whole plus each of its components, so both the full
    /// `Tax 2025/aangifte.pdf` and the bare `Tax 2025` are known.
    pub fn add_path(&mut self, path: &str) {
        self.add_name(path);
        for component in path.split(['/', '\\']) {
            self.add_name(component);
        }
    }

    /// Sort longest first (so `Tax 2025` is replaced before `Tax`) and dedupe.
    pub fn finish(mut self) -> Self {
        self.names.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        self.names.dedup();
        self
    }
}

/// Replace every occurrence of a known name (ASCII case-insensitive, at word
/// boundaries) with [`NAME_MARK`]. Names are pre-sorted longest first.
pub(super) fn replace_known_names(input: &str, names: &KnownNames) -> String {
    let mut current = input.to_string();
    let mut hay = current.to_ascii_lowercase();
    for name in &names.names {
        if name.is_empty() || name.len() > hay.len() || !hay.contains(name.as_str()) {
            continue;
        }
        let mut out = String::with_capacity(current.len());
        let mut last = 0usize;
        let mut from = 0usize;
        let mut hits = 0usize;
        while let Some(rel) = hay[from..].find(name.as_str()) {
            let start = from + rel;
            let end = start + name.len();
            if boundary_ok(&current, start, end) {
                out.push_str(&current[last..start]);
                out.push(NAME_MARK);
                last = end;
                from = end;
                hits += 1;
            } else {
                from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
            }
        }
        if hits > 0 {
            out.push_str(&current[last..]);
            current = out;
            hay = current.to_ascii_lowercase();
        }
    }
    current
}

pub(super) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub(super) fn boundary_ok(s: &str, start: usize, end: usize) -> bool {
    let before_ok = s[..start].chars().next_back().map_or(true, |c| !is_word_char(c));
    let after_ok = s[end..].chars().next().map_or(true, |c| !is_word_char(c));
    before_ok && after_ok
}
