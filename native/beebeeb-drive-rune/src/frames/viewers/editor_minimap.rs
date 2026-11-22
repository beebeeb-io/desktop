//! The editor's minimap (ux-21): an 80 px wide picture of the whole file, one pixel row per line (at most
//! 2,000 rows), drawn at the editor's right edge with the visible part as a box. Pressing it scrolls the editor.
//! The picture is cached by a hash of the text, so a frame without an edit only hashes and reuses it.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use rune::{Color, El, Pixels, ScrollPos, col, el};

use crate::theme::tokens::Theme;

/// The map's width in pixels (and points).
pub const WIDTH: u32 = 80;
/// The most rows the map has; longer files share rows.
pub const MAX_ROWS: usize = 2_000;
/// The editor's line height in points, to turn a line into a scroll offset.
const LINE_H: f32 = 21.0;
/// The viewport height before the editor has said it, in points.
const DEFAULT_VIEW: f32 = 600.0;
/// The scroll key of the editor body (`text::editor`'s `viewer-body`).
const SCROLL_KEY: &str = "viewer-body";

thread_local! {
    static SCROLL: Cell<[f32; 2]> = const { Cell::new([0.0, DEFAULT_VIEW]) };
    static CACHE: RefCell<Option<(u64, Pixels)>> = const { RefCell::new(None) };
}

/// The editor body's scroll, as its viewport reports it: the offset and the viewport height.
pub fn note_scroll(at: ScrollPos) {
    SCROLL.with(|s| s.set([at.offset[1], at.viewport[1]]));
}

/// The scroll the editor last reported.
pub fn current_scroll() -> f32 {
    SCROLL.with(|s| s.get()[0])
}

/// The rows a file of `lines` lines gets.
pub fn rows_for(lines: usize) -> usize {
    lines.clamp(1, MAX_ROWS)
}

/// The line a press at `y` of a map `h` tall lands on, in a file of `lines` lines.
pub fn line_at(y: f32, h: f32, lines: usize) -> usize {
    if lines == 0 || h <= 0.0 {
        return 0;
    }
    let rows = rows_for(lines);
    let row = ((y / h) * rows as f32).floor().clamp(0.0, (rows - 1) as f32) as usize;
    (row * lines / rows).min(lines - 1)
}

/// The editor's scroll offset that puts `line` at the top.
pub fn scroll_for(line: usize) -> f32 {
    line as f32 * LINE_H
}

fn rgba(c: Color) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(c.r), q(c.g), q(c.b), q(c.a)]
}

/// The map's pixels: `rows` rows of [`WIDTH`] pixels, transparent where the line is empty.
pub fn build(text: &str, file_name: &str, t: &Theme) -> (usize, Vec<u8>) {
    let lines = text.split('\n').count();
    let rows = rows_for(lines);
    let mut buf = vec![0u8; rows * WIDTH as usize * 4];
    let mut spans = crate::text_kit::highlight::spans(file_name, text, t.dark);
    spans.sort_by_key(|s| s.start);
    let plain = rgba(t.text_tertiary);
    let (mut next, mut at) = (0usize, 0usize);
    for (i, line) in text.split('\n').enumerate() {
        let row = i * rows / lines;
        let base = row * WIDTH as usize * 4;
        let mut col = 0usize;
        for (off, ch) in line.char_indices() {
            if col >= WIDTH as usize {
                break;
            }
            let p = at + off;
            let w = if ch == '\t' { 4 } else { 1 };
            if !ch.is_whitespace() {
                while next < spans.len() && spans[next].end as usize <= p {
                    next += 1;
                }
                let colour = match spans.get(next) {
                    Some(s) if s.start as usize <= p => s.color.map(rgba).unwrap_or(plain),
                    _ => plain,
                };
                let px = base + col * 4;
                buf[px..px + 4].copy_from_slice(&colour);
            }
            col += w;
        }
        at += line.len() + 1;
    }
    (rows, buf)
}

/// The cached picture of `text`, built when the text (or the theme's darkness) changed.
fn picture(text: &str, file_name: &str, t: &Theme) -> Pixels {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    file_name.hash(&mut h);
    t.dark.hash(&mut h);
    let key = h.finish();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((k, p)) = c.as_ref()
            && *k == key
        {
            return p.clone();
        }
        let (rows, buf) = build(text, file_name, t);
        let p = Pixels::new();
        p.set(WIDTH, rows as u32, buf);
        *c = Some((key, p.clone()));
        p
    })
}

/// The minimap: the picture with the visible part as a `t.hover` box. `scroll` is the editor's offset in points.
pub fn minimap(edit: &rune::TextEdit, file_name: &str, scroll: f32, t: &Theme) -> El {
    let text = &edit.text;
    let lines = text.split('\n').count();
    let rows = rows_for(lines);
    let view = SCROLL.with(|s| s.get()[1]);
    // Where the visible part is, as a share of the map's height.
    let total = lines as f32 * LINE_H;
    let (top, size) = if total > 0.0 {
        ((scroll / total).clamp(0.0, 1.0), (view / total).clamp(0.0, 1.0))
    } else {
        (0.0, 1.0)
    };
    let pixels = picture(text, file_name, t);
    let h = rows as f32;
    col()
        .key("editor-minimap")
        .w(WIDTH as f32)
        .h_full()
        .cursor(rune::Cursor::Pointer)
        .a11y_hidden()
        .on_drag_any(move |d, tx| {
            if matches!(d.phase, rune::Phase::Down | rune::Phase::Move) {
                let line = line_at(d.y, d.h, lines);
                tx.scroll_to_y(SCROLL_KEY, scroll_for(line));
            }
        })
        .child(el().key("minimap-picture").w_full().h_full().pixels(&pixels))
        .child(
            el().key("minimap-view")
                .absolute(0.0, top * h)
                .size(WIDTH as f32, (size * h).max(4.0))
                .fill(t.hover),
        )
}

#[cfg(test)]
#[path = "editor_minimap_tests.rs"]
mod tests;
