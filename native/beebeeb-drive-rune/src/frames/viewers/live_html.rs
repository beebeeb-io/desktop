//! The live preview of a html file: a small reader view. Our own converter turns the tags into markdown blocks; nothing
//! is fetched or run. The parse is kept per source hash, so typing does not re-parse unchanged text.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rune::El;

use super::super::Ui;
use super::super::markdown::{self, Block, Piece};
use crate::theme::tokens::Theme;

thread_local! {
    static PARSED: RefCell<Option<(u64, Vec<Block>)>> = const { RefCell::new(None) };
}

pub fn view(source: &str, _ui: &Ui, t: &Theme) -> El {
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    let key = h.finish();
    let blocks = PARSED.with(|c| {
        let mut c = c.borrow_mut();
        match &*c {
            Some((k, b)) if *k == key => b.clone(),
            _ => {
                let b = to_blocks(source);
                *c = Some((key, b.clone()));
                b
            }
        }
    });
    markdown::body_in(&blocks, t, "live-preview-html-body")
}

const SKIP: &[&str] = &[
    "script", "style", "head", "iframe", "object", "embed", "svg", "template", "noscript",
];

#[derive(Default)]
struct Conv {
    blocks: Vec<Block>,
    cur: Vec<Piece>,
    items: Vec<Vec<Piece>>,
    heading: Option<u8>,
    lists: u32,
    in_li: bool,
    pre: Option<String>,
    bold: u32,
    italic: u32,
    code: u32,
    link: Option<String>,
}

impl Conv {
    fn piece(&mut self, text: String) {
        self.cur.push(Piece {
            text,
            bold: self.bold > 0,
            italic: self.italic > 0,
            code: self.code > 0,
            link: self.link.clone(),
        });
    }

    fn text(&mut self, raw: &str) {
        let text = decode(raw);
        if let Some(pre) = &mut self.pre {
            pre.push_str(&text);
            return;
        }
        let mut out = String::new();
        let mut space = false;
        for ch in text.chars() {
            if ch.is_whitespace() && ch != '\u{a0}' {
                space = true;
            } else {
                if space {
                    out.push(' ');
                    space = false;
                }
                out.push(ch);
            }
        }
        if space {
            out.push(' ');
        }
        if !out.is_empty() {
            self.piece(out);
        }
    }

    fn flush(&mut self) {
        let mut pieces = std::mem::take(&mut self.cur);
        if let Some(first) = pieces.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = pieces.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        pieces.retain(|p| !p.text.is_empty());
        if pieces.iter().all(|p| p.text.trim().is_empty()) {
            return;
        }
        if let Some(level) = self.heading {
            self.blocks.push(Block::Heading(level, pieces));
        } else if self.in_li {
            self.items.push(pieces);
        } else {
            self.blocks.push(Block::Para(pieces));
        }
    }

    fn end_list(&mut self) {
        self.flush();
        self.in_li = false;
        if self.lists > 0 {
            self.lists -= 1;
        }
        if self.lists == 0 && !self.items.is_empty() {
            let items = std::mem::take(&mut self.items);
            self.blocks.push(Block::List(items));
        }
    }

    fn end_pre(&mut self) {
        if let Some(code) = self.pre.take() {
            let code = code.trim_matches('\n').to_string();
            if !code.trim().is_empty() {
                self.blocks.push(Block::Code(code));
            }
        }
    }

    fn tag(&mut self, close: bool, name: &str, attrs: &str) {
        if self.pre.is_some() && !(close && name == "pre") {
            return;
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                self.heading = if close { None } else { Some(name.as_bytes()[1] - b'0') };
            }
            "p" | "div" | "section" | "article" | "blockquote" | "td" | "th" | "tr" => self.flush(),
            "ul" | "ol" => {
                if close {
                    self.end_list();
                } else {
                    self.flush();
                    self.lists += 1;
                }
            }
            "li" => {
                self.flush();
                self.in_li = !close && self.lists > 0;
            }
            "pre" => {
                self.flush();
                if close {
                    self.end_pre();
                } else {
                    self.pre = Some(String::new());
                }
            }
            "br" => {
                if !close {
                    self.piece("\n".into());
                }
            }
            "strong" | "b" => {
                self.bold = if close {
                    self.bold.saturating_sub(1)
                } else {
                    self.bold + 1
                }
            }
            "em" | "i" => {
                self.italic = if close {
                    self.italic.saturating_sub(1)
                } else {
                    self.italic + 1
                }
            }
            "code" => {
                self.code = if close {
                    self.code.saturating_sub(1)
                } else {
                    self.code + 1
                }
            }
            "a" => self.link = if close { None } else { attr(attrs, "href") },
            "img" if !close => {
                let alt = attr(attrs, "alt").unwrap_or_default();
                self.piece(format!("[{}]", decode(&alt)));
            }
            _ => {}
        }
    }
}

/// The value of attribute `name` in a tag's attribute text.
fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok =
            at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric() && lower.as_bytes()[at - 1] != b'-';
        let rest = attrs[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let v = rest[1..].trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or("").to_string(),
            _ => v
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or("")
                .to_string(),
        });
    }
    None
}

fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.char_indices().take(12).find(|(_, c)| *c == ';').map(|(j, _)| j);
        let decoded = end.and_then(|j| {
            let ent = &rest[1..j];
            let ch = match ent {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                _ => {
                    let n = ent.strip_prefix('#')?;
                    let v = match n.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok()?,
                        None => n.parse().ok()?,
                    };
                    char::from_u32(v)?
                }
            };
            Some((ch, j))
        });
        match decoded {
            Some((ch, j)) => {
                out.push(ch);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The html as blocks, in order. Never panics, whatever the input.
pub fn to_blocks(source: &str) -> Vec<Block> {
    let mut c = Conv::default();
    let mut rest = source;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            c.text(rest);
            break;
        };
        c.text(&rest[..lt]);
        rest = &rest[lt..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |e| &after[e + 3..]);
            continue;
        }
        let next = rest[1..].chars().next();
        if !matches!(next, Some(ch) if ch.is_ascii_alphabetic() || ch == '/' || ch == '!') {
            c.text("<");
            rest = &rest[1..];
            continue;
        }
        // Find the closing '>' outside quotes.
        let mut quote = None;
        let mut end = None;
        for (i, ch) in rest.char_indices().skip(1) {
            match (quote, ch) {
                (None, '"' | '\'') => quote = Some(ch),
                (Some(q), _) if q == ch => quote = None,
                (None, '>') => {
                    end = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            break;
        };
        let inner = &rest[1..end];
        rest = &rest[end + 1..];
        let (close, inner) = match inner.strip_prefix('/') {
            Some(i) => (true, i),
            None => (false, inner),
        };
        let name_len = inner
            .find(|ch: char| !ch.is_ascii_alphanumeric())
            .unwrap_or(inner.len());
        let name = inner[..name_len].to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        if !close && SKIP.contains(&name.as_str()) && !inner.trim_end().ends_with('/') {
            let lower = rest.to_ascii_lowercase();
            let closer = format!("</{name}");
            rest = match lower.find(&closer) {
                Some(i) => rest[i..].find('>').map_or("", |g| &rest[i + g + 1..]),
                None => "",
            };
            continue;
        }
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        c.tag(close, &name, &inner[name_len..]);
    }
    c.end_pre();
    c.flush();
    c.end_list();
    c.blocks
}

#[cfg(test)]
#[path = "live_html_tests.rs"]
mod tests;
