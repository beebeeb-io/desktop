//! The live preview of a json file: a flat tree, one row per node, its parse kept per source hash.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rune::{El, col, row};
use serde_json::Value;

use super::super::{Ui, style};
use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::Tint;
use crate::theme::tokens::Theme;
use crate::ui::kit;

/// Rows shown before the "... N more" line.
pub(super) const MAX_ROWS: usize = 2000;
const INDENT: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Container,
    Str,
    Num,
    Lit,
    More,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub depth: usize,
    /// `key:` (or `[i]:` is not shown; array items have no key).
    pub key: Option<String>,
    pub text: String,
    pub kind: Kind,
}

type Parsed = Result<Vec<Row>, String>;

thread_local! {
    static CACHE: RefCell<Option<(u64, Parsed)>> = const { RefCell::new(None) };
}

fn count(v: &Value) -> usize {
    match v {
        Value::Array(a) => 1 + a.iter().map(count).sum::<usize>(),
        Value::Object(o) => 1 + o.values().map(count).sum::<usize>(),
        _ => 1,
    }
}

fn push(rows: &mut Vec<Row>, depth: usize, key: Option<&str>, v: &Value) {
    if rows.len() >= MAX_ROWS {
        return;
    }
    let (text, kind) = match v {
        Value::Object(o) => (format!("{{{}}}", o.len()), Kind::Container),
        Value::Array(a) => (format!("[{}]", a.len()), Kind::Container),
        Value::String(s) => (format!("{s:?}"), Kind::Str),
        Value::Number(n) => (n.to_string(), Kind::Num),
        Value::Bool(b) => (b.to_string(), Kind::Lit),
        Value::Null => ("null".to_string(), Kind::Lit),
    };
    rows.push(Row { depth, key: key.map(str::to_string), text, kind });
    match v {
        Value::Object(o) => o.iter().for_each(|(k, c)| push(rows, depth + 1, Some(k), c)),
        Value::Array(a) => a.iter().for_each(|c| push(rows, depth + 1, None, c)),
        _ => {}
    }
}

pub(super) fn parse(source: &str) -> Parsed {
    match serde_json::from_str::<Value>(source) {
        Ok(v) => {
            let mut rows = Vec::new();
            push(&mut rows, 0, None, &v);
            let total = count(&v);
            if total > rows.len() {
                rows.push(Row { depth: 0, key: None, text: format!("... {} more", total - rows.len()), kind: Kind::More });
            }
            Ok(rows)
        }
        Err(e) => Err(format!("Not valid JSON: line {}, column {}", e.line(), e.column())),
    }
}

fn cached(source: &str) -> Parsed {
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    let id = h.finish();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((k, p)) = c.as_ref()
            && *k == id
        {
            return p.clone();
        }
        let p = parse(source);
        *c = Some((id, p.clone()));
        p
    })
}

pub fn view(source: &str, _ui: &Ui, t: &Theme) -> El {
    let body = match cached(source) {
        Err(msg) => col().child(kit::label("live-json-error", msg, style::CODE, t.text_secondary)),
        Ok(rows) => {
            let mut c = col();
            for (i, r) in rows.iter().enumerate() {
                let color = match r.kind {
                    Kind::Container => t.text,
                    Kind::Str => tint_color(Tint::Markdown, t.dark),
                    Kind::Num => tint_color(Tint::Code, t.dark),
                    Kind::Lit => tint_color(Tint::Data, t.dark),
                    Kind::More => t.text_secondary,
                };
                let mut line = row().key(format!("live-json-row-{i}")).padding_left(r.depth as f32 * INDENT);
                if let Some(k) = &r.key {
                    line = line.child(kit::label(format!("live-json-key-{i}"), format!("{k}: "), style::CODE, t.text_secondary));
                }
                c = c.child(line.child(kit::label(format!("live-json-val-{i}"), r.text.clone(), style::CODE, color)));
            }
            c
        }
    };
    body.key("live-preview-json")
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .h_full()
        .padding(16.0)
        .fill(t.bg)
        .min_h(0.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
}

#[cfg(test)]
#[path = "live_json_tests.rs"]
mod tests;
