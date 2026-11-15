//! Keeps `vocabulary.txt` in step with the log calls: every word of every string literal and field name at a
//! `tracing::{error,warn,info,debug,trace}!` call in the engine's and this crate's sources must be in the list, so a
//! new log call adds its words in the same change. Test files and `#[cfg(test)]` items are not read. A test, not a
//! build step.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::super::redact::vocabulary::{VOCABULARY, words};

const LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String),
    Ident(String),
    Punct(char),
}

/// The sources the log calls live in: this crate's and the engine's.
fn source_roots() -> [PathBuf; 2] {
    let native = Path::new(env!("CARGO_MANIFEST_DIR"));
    [native.join("src"), native.join("../../crates/beebeeb-engine/src")]
}

fn is_test_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let in_test_dir = path.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("tests" | "test_api" | "testing" | "fixtures")
        )
    });
    in_test_dir
        || name == "tests.rs"
        || name.ends_with("_tests.rs")
        || name.starts_with("tests_")
        || name.starts_with("test_")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") && !is_test_file(&path) {
            out.push(path);
        }
    }
}

/// A small Rust lexer: string literals (escapes decoded, raw strings), identifiers and single punctuation characters;
/// comments, char literals, lifetimes and number literals are skipped.
fn lex(src: &str) -> Vec<Tok> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && next == Some('/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && next == Some('*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if let Some((text, end)) = raw_string(&c, i) {
            out.push(Tok::Str(text));
            i = end;
        } else if ch == '"' || (ch == 'b' && next == Some('"')) {
            let (text, end) = quoted(&c, if ch == 'b' { i + 2 } else { i + 1 });
            out.push(Tok::Str(text));
            i = end;
        } else if ch == '\'' {
            // A char literal (`'x'`, `'\n'`, `'\u{201c}'`) is skipped whole; a lifetime or label only its quote.
            if next == Some('\\') {
                i += 2;
                while i < c.len() && c[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if c.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if ch.is_alphanumeric() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            if !ch.is_ascii_digit() {
                out.push(Tok::Ident(c[start..i].iter().collect()));
            }
        } else {
            out.push(Tok::Punct(ch));
            i += 1;
        }
    }
    out
}

/// `r"…"`, `r#"…"#`, `br"…"` at `i`: the text and the index after it.
fn raw_string(c: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = match (c.get(i), c.get(i + 1)) {
        (Some('r'), _) => i + 1,
        (Some('b'), Some('r')) => i + 2,
        _ => return None,
    };
    let hashes = c[j..].iter().take_while(|ch| **ch == '#').count();
    j += hashes;
    if c.get(j) != Some(&'"') {
        return None;
    }
    let start = j + 1;
    let mut k = start;
    while k < c.len() {
        if c[k] == '"' && c[k + 1..].iter().take(hashes).filter(|ch| **ch == '#').count() == hashes {
            return Some((c[start..k].iter().collect(), k + 1 + hashes));
        }
        k += 1;
    }
    Some((c[start..].iter().collect(), c.len()))
}

/// The body of a `"…"` literal starting at `start` (after the quote), escapes decoded: the text and the index after it.
fn quoted(c: &[char], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = start;
    while i < c.len() && c[i] != '"' {
        if c[i] != '\\' {
            text.push(c[i]);
            i += 1;
            continue;
        }
        i += 1;
        match c.get(i) {
            Some('n' | 'r' | 't' | '0') => text.push(' '),
            Some('u') => {
                let close = c[i..].iter().position(|ch| *ch == '}').map_or(c.len(), |p| i + p);
                let hex: String = c[i + 2..close.min(c.len())].iter().collect();
                text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                i = close;
            }
            Some('x') => {
                let hex: String = c[i + 1..(i + 3).min(c.len())].iter().collect();
                text.extend(u8::from_str_radix(&hex, 16).ok().map(char::from));
                i += 2;
            }
            // A line continuation: the newline and the next line's leading spaces are not part of the text.
            Some('\n' | '\r') => {
                while c.get(i + 1).is_some_and(|ch| ch.is_whitespace()) {
                    i += 1;
                }
            }
            Some(other) => text.push(*other),
            None => {}
        }
        i += 1;
    }
    (text, i + 1)
}

/// Drops every item behind `#[cfg(test)]`: up to its first `;` or the end of its first `{ … }` block.
fn without_test_items(toks: Vec<Tok>) -> Vec<Tok> {
    let cfg_test = |t: &[Tok]| {
        t.len() >= 7
            && t[0] == Tok::Punct('#')
            && t[1] == Tok::Punct('[')
            && t[2] == Tok::Ident("cfg".into())
            && t[3] == Tok::Punct('(')
            && t[4] == Tok::Ident("test".into())
            && t[5] == Tok::Punct(')')
            && t[6] == Tok::Punct(']')
    };
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if !cfg_test(&toks[i..]) {
            i += 1;
            out.push(i - 1);
            continue;
        }
        i += 7;
        let mut depth = 0;
        while i < toks.len() {
            match toks[i] {
                Tok::Punct('{') => depth += 1,
                Tok::Punct('}') => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                Tok::Punct(';') if depth == 0 => {
                    i += 1;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
    }
    let keep: BTreeSet<usize> = out.into_iter().collect();
    toks.into_iter()
        .enumerate()
        .filter(|(at, _)| keep.contains(at))
        .map(|(_, t)| t)
        .collect()
}

/// The words of every log call in `src`: its string literals (format placeholders left out) and its field names.
fn log_call_words(src: &str) -> BTreeSet<String> {
    let toks = without_test_items(lex(src));
    let mut found = BTreeSet::new();
    let mut i = 0;
    while i + 5 < toks.len() {
        let call = toks[i] == Tok::Ident("tracing".into())
            && toks[i + 1] == Tok::Punct(':')
            && toks[i + 2] == Tok::Punct(':')
            && matches!(&toks[i + 3], Tok::Ident(level) if LEVELS.contains(&level.as_str()))
            && toks[i + 4] == Tok::Punct('!')
            && matches!(toks[i + 5], Tok::Punct('(' | '{' | '['));
        if !call {
            i += 1;
            continue;
        }
        let mut depth = 0;
        // Field names come before the message: `name = value`, or the `%name` / `?name` / `name` shorthand.
        let mut in_fields = true;
        let mut j = i + 5;
        while j < toks.len() {
            match &toks[j] {
                Tok::Punct('(' | '{' | '[') => depth += 1,
                Tok::Punct(')' | '}' | ']') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Tok::Str(text) => {
                    found.extend(literal_words(text));
                    if depth == 1 && !matches!(toks[j - 1], Tok::Punct('=' | ':')) {
                        in_fields = false;
                    }
                }
                Tok::Ident(name) if depth == 1 && in_fields => {
                    let starts_arg = |at: usize| matches!(toks[at], Tok::Punct('(' | ','));
                    let shorthand = matches!(toks[j - 1], Tok::Punct('%' | '?')) && starts_arg(j - 2);
                    let assigned = matches!(toks.get(j + 1), Some(Tok::Punct('=')))
                        && !matches!(toks.get(j + 2), Some(Tok::Punct('=')));
                    let bare = matches!(toks.get(j + 1), Some(Tok::Punct(',' | ')' | '.')));
                    if (starts_arg(j - 1) && (assigned || bare)) || shorthand {
                        found.extend(words(name).map(str::to_lowercase));
                    }
                }
                _ => {}
            }
            j += 1;
        }
        i = j;
    }
    found
}

/// A literal's words, its `{…}` format placeholders left out, numbers left out (every number is written anyway).
fn literal_words(text: &str) -> Vec<String> {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() != Some(&'{') {
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }
            plain.push(' ');
        } else {
            plain.push(ch);
        }
    }
    words(&plain)
        .filter(|w| !w.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_lowercase)
        .collect()
}

fn listed() -> BTreeSet<String> {
    VOCABULARY
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_word_of_every_log_call_is_in_the_vocabulary() {
    let mut files = Vec::new();
    for root in source_roots() {
        assert!(
            root.is_dir(),
            "the sources are where the scan expects them: {}",
            root.display()
        );
        rust_files(&root, &mut files);
    }
    assert!(
        files.len() > 100,
        "the scan reads the crates' sources: {} files",
        files.len()
    );
    let mut scanned = BTreeSet::new();
    for file in &files {
        scanned.extend(log_call_words(&std::fs::read_to_string(file).unwrap()));
    }
    assert!(
        scanned.len() > 300,
        "the scan finds the log calls' words: {}",
        scanned.len()
    );
    let listed = listed();
    let missing: Vec<&String> = scanned.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "{} log-call words are missing from log_file/vocabulary.txt (add them, one per line, sorted): {missing:?}",
        missing.len()
    );
    // The list holds no word that no log call has any more: a larger list writes more of a person's words.
    let stale: Vec<&String> = listed.difference(&scanned).collect();
    assert!(
        stale.is_empty(),
        "{} words of log_file/vocabulary.txt are in no log call any more (remove them): {stale:?}",
        stale.len()
    );
}

#[test]
fn the_vocabulary_is_sorted_lowercase_and_one_word_per_line() {
    let lines: Vec<&str> = VOCABULARY
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let unsorted = lines.windows(2).find(|pair| pair[0] >= pair[1]);
    assert!(unsorted.is_none(), "sorted (byte order), no duplicates: {unsorted:?}");
    for line in lines {
        assert_eq!(words(line).collect::<Vec<_>>(), [line], "one word per line: {line:?}");
        assert_eq!(line, line.to_lowercase(), "lowercase: {line:?}");
    }
}

#[test]
fn the_scan_reads_messages_fields_and_skips_tests() {
    let src = r##"
        fn f() { tracing::warn!(file_id = %id, ?kind, count, "upload parked: {e} at \u{201c}{name}\u{201d}"); }
        fn g() { tracing::info!(target: "beebeeb_engine::runner", r#"raw words"#); }
        // tracing::error!("commented out");
        #[cfg(test)]
        mod tests { fn t() { tracing::error!("anna smith"); } }
        fn h() { tracing::error!("after the tests"); }
    "##;
    let found: Vec<String> = log_call_words(src).into_iter().collect();
    let want = [
        "after",
        "at",
        "beebeeb_engine",
        "count",
        "file_id",
        "kind",
        "parked",
        "raw",
        "runner",
        "tests",
        "the",
        "upload",
        "words",
    ];
    assert_eq!(found, want);
}
