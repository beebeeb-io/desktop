//! Indent detection and block indent / outdent over a selection.

use super::lines::{line_of, line_starts};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentUnit {
    Tab,
    Spaces(u8),
}

/// `Tab` when more lines start with a tab than a space; else the most common indent step out of 2 and 4
/// (default 4).
pub fn detect(text: &str) -> IndentUnit {
    let (mut tabs, mut spaces) = (0usize, 0usize);
    let (mut two, mut four) = (0usize, 0usize);
    let mut prev = 0usize;
    for line in text.lines() {
        if line.starts_with('\t') {
            tabs += 1;
        } else if line.starts_with(' ') {
            spaces += 1;
        }
        if line.trim().is_empty() {
            continue;
        }
        let ind = line.len() - line.trim_start_matches(' ').len();
        if ind > prev {
            match ind - prev {
                2 => two += 1,
                4 => four += 1,
                _ => {}
            }
        }
        prev = ind;
    }
    if tabs > spaces {
        IndentUnit::Tab
    } else if two > four {
        IndentUnit::Spaces(2)
    } else {
        IndentUnit::Spaces(4)
    }
}

pub fn unit_str(u: IndentUnit) -> String {
    match u {
        IndentUnit::Tab => "\t".into(),
        IndentUnit::Spaces(n) => " ".repeat(n as usize),
    }
}

/// Byte range of the lines a selection touches (a selection ending at a line's first byte leaves that line out).
fn touched(text: &str, sel: &Range<usize>) -> Range<usize> {
    let starts = line_starts(text);
    let first = line_of(&starts, sel.start);
    let mut last = line_of(&starts, sel.end);
    if sel.end > sel.start && starts[last] == sel.end && last > first {
        last -= 1;
    }
    first..last + 1
}

/// Put one unit at the start of every non-empty selected line. Returns the new text and selection.
pub fn indent_lines(text: &str, sel: Range<usize>, u: IndentUnit) -> (String, Range<usize>) {
    let starts = line_starts(text);
    let lines = touched(text, &sel);
    let unit = unit_str(u);
    let mut out = String::with_capacity(text.len() + unit.len() * lines.len());
    let (mut start, mut end) = (sel.start, sel.end);
    let mut at = 0;
    for l in lines {
        let ls = starts[l];
        let le = starts.get(l + 1).map_or(text.len(), |&n| n - 1);
        if ls == le {
            continue;
        }
        out.push_str(&text[at..ls]);
        out.push_str(&unit);
        at = ls;
        if ls <= sel.start {
            start += unit.len();
        }
        end += unit.len();
    }
    out.push_str(&text[at..]);
    (out, start..end.max(start))
}

/// Take one unit (a tab, or up to the unit's width in spaces) off every selected line.
pub fn outdent_lines(text: &str, sel: Range<usize>, u: IndentUnit) -> (String, Range<usize>) {
    let starts = line_starts(text);
    let lines = touched(text, &sel);
    let width = match u {
        IndentUnit::Tab => 4,
        IndentUnit::Spaces(n) => n as usize,
    };
    let mut out = String::with_capacity(text.len());
    let (mut start, mut end) = (sel.start, sel.end);
    let mut at = 0;
    for l in lines {
        let ls = starts[l];
        let rest = &text[ls..];
        let n = if rest.starts_with('\t') {
            1
        } else {
            rest.bytes().take(width).take_while(|&b| b == b' ').count()
        };
        if n == 0 {
            continue;
        }
        out.push_str(&text[at..ls]);
        at = ls + n;
        // The part of the removed run that lies before a position moves it back.
        let before = |p: usize| n.min(p.saturating_sub(ls));
        start -= before(sel.start);
        end -= before(sel.end);
    }
    out.push_str(&text[at..]);
    (out, start..end.max(start))
}

#[cfg(test)]
#[path = "indent_tests.rs"]
mod tests;
