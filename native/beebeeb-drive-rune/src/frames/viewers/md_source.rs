//! A Markdown note as its source: each line's runs, with the marks dimmed (DW-1523 and DW-1513's
//! Source): `#`, `-`, `**`, `` ` ``, `[`, `](` and `)` faint and italic, bold words strong, inline
//! code and a link's target green, the rest plain.

use super::source::{Ink, Run};

fn run(text: &str, ink: Ink, bold: bool, italic: bool) -> Run {
    Run {
        text: text.to_string(),
        ink,
        bold,
        italic,
        underline: false,
    }
}

fn mark(text: &str) -> Run {
    run(text, Ink::Faint, false, true)
}

/// One line's runs.
pub fn line(text: &str) -> Vec<Run> {
    let mut out = Vec::new();
    let mut rest = text;
    // The mark a line starts with: a heading's hashes or a list item's dash.
    let indent = rest.len() - rest.trim_start_matches(' ').len();
    let after = &rest[indent..];
    let hashes = after.len() - after.trim_start_matches('#').len();
    let lead = if (1..=6).contains(&hashes) && after[hashes..].starts_with(' ') {
        indent + hashes + 1
    } else if after.starts_with("- ") || after.starts_with("* ") || after.starts_with("+ ") {
        indent + 2
    } else {
        0
    };
    if lead > 0 {
        out.push(mark(&rest[..lead]));
        rest = &rest[lead..];
    }
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut Vec<Run>| {
        if !plain.is_empty() {
            out.push(run(plain, Ink::Plain, false, false));
            plain.clear();
        }
    };
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix("**") {
            if let Some(end) = inner.find("**").filter(|end| *end > 0) {
                flush(&mut plain, &mut out);
                out.push(mark("**"));
                out.push(run(&inner[..end], Ink::Strong, true, false));
                out.push(mark("**"));
                rest = &inner[end + 2..];
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix('`') {
            if let Some(end) = inner.find('`').filter(|end| *end > 0) {
                flush(&mut plain, &mut out);
                out.push(mark("`"));
                out.push(run(&inner[..end], Ink::Green, false, false));
                out.push(mark("`"));
                rest = &inner[end + 1..];
                continue;
            }
        }
        if let Some(inner) = rest.strip_prefix('[') {
            if let Some((words, target)) = inner.split_once("](") {
                if let Some(end) = target.find(')').filter(|_| !words.contains('[')) {
                    flush(&mut plain, &mut out);
                    out.push(mark("["));
                    out.push(run(words, Ink::Plain, false, false));
                    out.push(mark("]("));
                    out.push(run(&target[..end], Ink::Green, false, false));
                    out.push(mark(")"));
                    rest = &target[end + 1..];
                    continue;
                }
            }
        }
        let ch = rest.chars().next().unwrap_or(' ');
        plain.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    flush(&mut plain, &mut out);
    out
}

/// Every line of the note.
pub fn lines(source: &str) -> Vec<Vec<Run>> {
    source.lines().map(line).collect()
}
