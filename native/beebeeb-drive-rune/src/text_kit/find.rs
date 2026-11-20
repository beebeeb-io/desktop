//! Find and replace over a byte-indexed text.

use std::ops::Range;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    pub needle: String,
    pub match_case: bool,
    pub whole_word: bool,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn same(a: char, b: char, match_case: bool) -> bool {
    a == b || (!match_case && a.to_lowercase().eq(b.to_lowercase()))
}

/// Match ranges: ascending, non-overlapping, on char boundaries. An empty needle matches nothing.
pub fn find_all(text: &str, q: &FindQuery) -> Vec<Range<usize>> {
    let needle: Vec<char> = q.needle.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut skip_to = 0;
    for (start, _) in text.char_indices() {
        if start < skip_to {
            continue;
        }
        let mut it = text[start..].char_indices();
        let mut end = start;
        let mut ok = true;
        for &n in &needle {
            match it.next() {
                Some((i, c)) if same(c, n, q.match_case) => end = start + i + c.len_utf8(),
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        if q.whole_word {
            let before = text[..start].chars().next_back().is_some_and(is_word);
            let after = text[end..].chars().next().is_some_and(is_word);
            if before || after {
                continue;
            }
        }
        out.push(start..end);
        skip_to = end;
    }
    out
}

/// Index of the next match from `caret`, wrapping: forwards the first match starting at or after the caret,
/// backwards the last one starting before it.
pub fn next_match(matches: &[Range<usize>], caret: usize, backwards: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    if backwards {
        let n = matches.partition_point(|m| m.start < caret);
        Some(if n == 0 { matches.len() - 1 } else { n - 1 })
    } else {
        let n = matches.partition_point(|m| m.start < caret);
        Some(if n == matches.len() { 0 } else { n })
    }
}

pub fn replace_range(text: &str, r: Range<usize>, with: &str) -> String {
    let mut s = String::with_capacity(text.len() + with.len());
    s.push_str(&text[..r.start]);
    s.push_str(with);
    s.push_str(&text[r.end..]);
    s
}

/// Replace every match; returns the new text and the count.
pub fn replace_all(text: &str, q: &FindQuery, with: &str) -> (String, usize) {
    let ms = find_all(text, q);
    let mut s = String::with_capacity(text.len());
    let mut at = 0;
    for m in &ms {
        s.push_str(&text[at..m.start]);
        s.push_str(with);
        at = m.end;
    }
    s.push_str(&text[at..]);
    (s, ms.len())
}

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
