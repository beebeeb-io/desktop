//! Find in a PDF (DW-1506), from the PDF's own text: the words are looked for in the text the renderer's interpreter
//! reads off each page of the document, on the worker that holds it. The pages are drawn by the confined helper; this
//! is the one read of the file left in this process, begun at the first find. Child of `viewer_pdf.rs`.
//!
//! - Memory only: the text is read into this call's own vectors and dropped when it returns; nothing is written, and
//!   neither the words asked for nor a page's text is logged.
//! - Case-insensitive, the whole PDF, page order; runs of spaces and line ends count as one space. What is found is
//!   boxes in the page's own points at 100 %, top left origin, one box per line the words are on.
//! - A page the renderer cannot write ([`writes_in_a_numbered_font_not_in_the_file`]) gives no match: its text is
//!   not in the pixels as it was written, and a box on it would point at the wrong place. A page with no text (a
//!   scan) gives none. Text in an OCR layer (drawn invisible) IS found: it is the page's own text.
//! - Limits: at most [`FIND_MATCHES_MAX`] matches, [`FIND_CHARS_MAX`] characters read per page. A page that makes the
//!   interpreter panic is skipped. `cancel` is asked between pages.
//! - Not done: text in a form the page draws is read like any (the interpreter walks into it), a ligature written as
//!   one glyph is matched by its Unicode string, vertical writing gets boxes of the horizontal shape.

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::kurbo::{Affine, BezPath, Rect};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, TransformExt, interpret_page,
};

use super::*;
use crate::frames::viewers::source::Match;

/// The most matches one search returns (the bar counts them; a page of "e" must not fill memory).
pub const FIND_MATCHES_MAX: usize = 2_000;
/// The most characters read off one page.
pub const FIND_CHARS_MAX: usize = 400_000;
/// How wide a glyph is taken to be when the font does not say, in thousandths of an em.
const GLYPH_WIDTH_DEFAULT: f64 = 500.0;
/// The widest a glyph's box is taken to be (a font's own number that is not in thousandths would not stretch a match
/// over the line).
const GLYPH_WIDTH_MAX: f64 = 3_000.0;
/// A glyph's box in thousandths of an em: from below the baseline to above the capitals.
const GLYPH_DESCENT: f64 = -200.0;
const GLYPH_ASCENT: f64 = 800.0;

/// One character of a page and where its glyph is (`None`: a space made up between runs, nowhere).
type Piece = (char, Option<Rect>);

/// The text of one page, in the order the page draws it.
#[derive(Default)]
struct Text {
    pieces: Vec<Piece>,
}

impl Text {
    fn push_space(&mut self) {
        if self.pieces.last().is_some_and(|(c, _)| *c != ' ') {
            self.pieces.push((' ', None));
        }
    }
}

impl<'a> Device<'a> for Text {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        // A run is one string of the page (a Tj / TJ): the next one is a new word unless the text says otherwise.
        self.push_space();
        // A glyph's own transform places it in text space only. The page's transform is the context's current one
        // (`find`'s top-down start, then the page's own `cm`s and forms): it comes in `props`. Both, once, put the box
        // where the page is drawn, as the renderer composes them; without it the box stays in the PDF's bottom-up
        // space and lands far from the words.
        let page = props.transform;
        for glyph in run.glyphs() {
            if self.pieces.len() >= FIND_CHARS_MAX {
                return;
            }
            let Some(unicode) = glyph.as_unicode() else { continue };
            let width = match &**glyph {
                Glyph::Outline(outline) => outline.advance_width(),
                Glyph::Type3(type3) => type3.advance_width(),
            }
            .map_or(GLYPH_WIDTH_DEFAULT, f64::from)
            .clamp(1.0, GLYPH_WIDTH_MAX);
            let place =
                (page * glyph.transform()).transform_rect_bbox(Rect::new(0.0, GLYPH_DESCENT, width, GLYPH_ASCENT));
            let text = match unicode {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            };
            for c in text.chars() {
                if c.is_whitespace() {
                    self.push_space();
                } else {
                    self.pieces.push((c, Some(place)));
                }
            }
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// A character as it is compared: lower case (the first character of its lower case form).
fn folded(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The words as they are searched: lower case, runs of white space as one space, none at the ends.
fn needle(words: &str) -> Vec<char> {
    words
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(folded)
        .collect()
}

/// The boxes of the characters in `pieces` (one match), one per line (characters whose boxes share a line are one box).
fn boxes(pieces: &[Piece], page: u32) -> Vec<Match> {
    let mut lines: Vec<Rect> = Vec::new();
    for rect in pieces.iter().filter_map(|(_, rect)| *rect) {
        match lines.last_mut() {
            Some(line) if overlap(line, &rect) => *line = line.union(rect),
            _ => lines.push(rect),
        }
    }
    lines
        .into_iter()
        .map(|line| Match {
            page,
            x: line.x0 as f32,
            y: line.y0 as f32,
            w: line.width() as f32,
            h: line.height() as f32,
        })
        .collect()
}

/// Whether two boxes are on the same line: they share more than half of the shorter one's height.
fn overlap(a: &Rect, b: &Rect) -> bool {
    let shared = a.y1.min(b.y1) - a.y0.max(b.y0);
    shared > 0.5 * a.height().min(b.height())
}

/// Where `needle` is in `text`, as boxes, in reading order; matches do not overlap.
fn search(text: &Text, needle: &[char], page: u32, out: &mut Vec<Match>) {
    let pieces = &text.pieces;
    if needle.is_empty() || pieces.len() < needle.len() {
        return;
    }
    let mut at = 0;
    while at + needle.len() <= pieces.len() && out.len() < FIND_MATCHES_MAX {
        let same = pieces[at..at + needle.len()]
            .iter()
            .zip(needle)
            .all(|((c, _), n)| folded(*c) == *n);
        if same {
            out.extend(boxes(&pieces[at..at + needle.len()], page));
            at += needle.len();
        } else {
            at += 1;
        }
    }
}

impl Document {
    /// Where `words` are in the document's text: every page in order, the pages that lack text skipped, until `cancel`
    /// is set (then what was found so far is returned and the caller drops it). Empty words find nothing.
    pub fn find(&self, words: &str, cancel: &AtomicBool) -> Vec<Match> {
        let needle = needle(words);
        let mut found = Vec::new();
        if needle.is_empty() {
            return found;
        }
        let cache = InterpreterCache::new();
        let pages = self.pdf.pages();
        for index in 0..pages.len() {
            if cancel.load(Ordering::SeqCst) || found.len() >= FIND_MATCHES_MAX {
                break;
            }
            let Some(page) = pages.get(index) else { continue };
            let number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if writes_in_a_numbered_font_not_in_the_file(page) {
                    return None;
                }
                let (width, height) = page.render_dimensions();
                let target = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
                // Page points at 100 %, the top left corner the origin: where the view draws a match. Every glyph box
                // goes through it once (`draw_glyph_run`).
                let transform: Affine = page.initial_transform(true).to_kurbo();
                let mut context = Context::new(transform, target, &cache, page.xref(), InterpreterSettings::default());
                let mut text = Text::default();
                interpret_page(page, &mut context, &mut text);
                Some(text)
            }));
            if let Ok(Some(text)) = read {
                search(&text, &needle, number, &mut found);
            }
        }
        found
    }
}
