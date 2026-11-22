//! The live preview of a table file: our own CSV/TSV parse, kept per source hash.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rune::El;

use super::super::{
    Ui,
    kinds::{Column, TableDoc},
    table,
};
use crate::theme::tokens::Theme;

const MAX_ROWS: usize = 1_000;

thread_local! {
    static CACHE: RefCell<Option<(u64, TableDoc)>> = const { RefCell::new(None) };
}

pub fn view(source: &str, ui: &Ui, t: &Theme) -> El {
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    let key = h.finish();
    let doc = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        match &*c {
            Some((k, d)) if *k == key => d.clone(),
            _ => {
                let d = parse(source);
                *c = Some((key, d.clone()));
                d
            }
        }
    });
    // The rows are the previewed text's own, not the editor file's (`ui.table_rows()` reads that one).
    table::body_in(ui, &doc, t, "live-preview-table", doc.rows.iter().collect())
}

/// The separator from the first line: tab, else ',' or ';' (whichever there is more of; ',' on a tie).
fn detect(source: &str) -> (char, &'static str) {
    let first = source.lines().next().unwrap_or("");
    if first.contains('\t') {
        return ('\t', "Tab-separated");
    }
    let commas = first.matches(',').count();
    let semis = first.matches(';').count();
    if semis > commas {
        (';', "Semicolon-separated")
    } else {
        (',', "Comma-separated")
    }
}

/// RFC 4180 records. An unterminated quote runs to the end of the text.
fn records(source: &str, sep: char) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        any = true;
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(ch);
            }
        } else if ch == '"' && cell.is_empty() {
            quoted = true;
        } else if ch == sep {
            row.push(std::mem::take(&mut cell));
        } else if ch == '\r' || ch == '\n' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut cell));
            out.push(std::mem::take(&mut row));
            any = false;
        } else {
            cell.push(ch);
        }
    }
    if any {
        row.push(cell);
        out.push(row);
    }
    out
}

pub(crate) fn parse(source: &str) -> TableDoc {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let (sep, label) = detect(source);
    let mut recs = records(source, sep);
    // Blank lines are not rows.
    recs.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    let mut it = recs.into_iter();
    let head = it.next().unwrap_or_default();
    let body: Vec<Vec<String>> = it.collect();
    let total_rows = body.len() as u32;
    let width = head.len();
    let columns = head
        .into_iter()
        .enumerate()
        .map(|(c, name)| Column {
            name,
            numeric: body
                .iter()
                .filter_map(|r| r.get(c))
                .filter(|v| !v.trim().is_empty())
                .all(|v| v.trim().parse::<f64>().is_ok()),
        })
        .collect();
    let rows = body
        .into_iter()
        .take(MAX_ROWS)
        .map(|mut r| {
            r.resize(width, String::new());
            r
        })
        .collect();
    TableDoc {
        separator: label.to_string(),
        columns,
        rows,
        total_rows,
    }
}

#[cfg(test)]
#[path = "live_table_tests.rs"]
mod tests;
