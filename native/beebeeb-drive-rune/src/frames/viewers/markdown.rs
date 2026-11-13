//! Markdown, rendered (DW-1513): headings, paragraphs, lists, inline code, bold, italic and links.
//!
//! The note is parsed by `pulldown-cmark` into [`Block`]s; nothing is turned into HTML and raw
//! HTML in the note is shown as the text it is, never run (the board's rule). A paragraph or a
//! list item is a wrapping row of words, so inline code keeps its own box and a link is a link.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use rune::{El, Span, TextStyle, col, row};

use super::{style, ui};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::kit;

/// A run of a line in one manner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// Where a link goes: a file in Drive, or the web.
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// The level (1 to 6) and the words.
    Heading(u8, Vec<Piece>),
    Para(Vec<Piece>),
    /// One entry per item.
    List(Vec<Vec<Piece>>),
    /// A fenced or indented block of code, as written.
    Code(String),
}

/// The note as blocks, in order.
pub fn parse(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut line: Vec<Piece> = Vec::new();
    let mut items: Vec<Vec<Vec<Piece>>> = Vec::new();
    let mut heading = None;
    let mut code: Option<String> = None;
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut link: Option<String> = None;
    let push = |line: &mut Vec<Piece>, text: &str, bold: u32, italic: u32, is_code: bool, link: &Option<String>| {
        if text.is_empty() {
            return;
        }
        line.push(Piece {
            text: text.to_string(),
            bold: bold > 0,
            italic: italic > 0,
            code: is_code,
            link: link.clone(),
        });
    };
    let source = lenient(source);
    for event in Parser::new_ext(&source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                blocks.push(Block::Heading(heading.take().unwrap_or(1), std::mem::take(&mut line)));
            }
            Event::End(TagEnd::Paragraph) => {
                // A paragraph inside a list item is that item's words.
                if items.is_empty() && !line.is_empty() {
                    blocks.push(Block::Para(std::mem::take(&mut line)));
                }
            }
            Event::Start(Tag::List(_)) => {
                // A nested list: what the parent item said so far stays its own line.
                if let Some(list) = items.last_mut() {
                    if !line.is_empty() {
                        list.push(std::mem::take(&mut line));
                    }
                }
                items.push(Vec::new());
            }
            Event::End(TagEnd::Item) => {
                if let Some(list) = items.last_mut() {
                    if !line.is_empty() {
                        list.push(std::mem::take(&mut line));
                    }
                }
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(list) = items.pop() {
                    match items.last_mut() {
                        // A nested list's items follow their parent's (one level is drawn).
                        Some(parent) => parent.extend(list),
                        None => blocks.push(Block::List(list)),
                    }
                }
            }
            Event::Start(Tag::CodeBlock(_)) => code = Some(String::new()),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(text) = code.take() {
                    blocks.push(Block::Code(text.trim_end_matches('\n').to_string()));
                }
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => link = None,
            Event::Text(text) => match code.as_mut() {
                Some(block) => block.push_str(&text),
                None => push(&mut line, &text, bold, italic, false, &link),
            },
            Event::Code(text) => push(&mut line, &text, bold, italic, true, &link),
            // Raw HTML is shown as the text it is.
            Event::Html(text) | Event::InlineHtml(text) => push(&mut line, &text, bold, italic, false, &link),
            Event::SoftBreak | Event::HardBreak => push(&mut line, " ", bold, italic, false, &link),
            _ => {}
        }
    }
    if !line.is_empty() {
        blocks.push(Block::Para(line));
    }
    blocks
}

/// A link whose target has spaces (`[Service agreement](Service agreement.docx)`, as people
/// write a file's name) is a link here too: its target is put in the angle brackets CommonMark asks for.
fn lenient(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find("](") {
        let (head, tail) = rest.split_at(at + 2);
        out.push_str(head);
        let line_end = tail.find('\n').unwrap_or(tail.len());
        match tail[..line_end].find(')') {
            Some(end) if tail[..end].contains(' ') && !tail[..end].contains(['<', '"', '(']) => {
                out.push('<');
                out.push_str(&tail[..end]);
                out.push('>');
                rest = &tail[end..];
            }
            _ => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

fn plain(pieces: &[Piece]) -> String {
    pieces.iter().map(|p| p.text.as_str()).collect()
}

/// Is this where a link goes on the web (opened in the browser), not a file in Drive?
pub fn is_web(dest: &str) -> bool {
    let lower = dest.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

/// Where a link's line sits in the 17-point line box.
const UNDERLINE_Y: f32 = 15.0;

/// Where a link's line is drawn under `word`: everywhere but under a letter that descends
/// through it (`text-decoration-skip-ink`), as from-to pairs in points.
fn underline_runs(word: &str, text_style: TextStyle) -> Vec<(f32, f32)> {
    let mut runs: Vec<(f32, f32)> = Vec::new();
    let mut open: Option<f32> = None;
    for (at, ch) in word.char_indices() {
        // (Rune measures an empty text as one space wide.)
        let from = if at == 0 {
            0.0
        } else {
            rune::text::advance(&word[..at], text_style)
        };
        if "gjpqy,;".contains(ch) {
            if let Some(start) = open.take() {
                runs.push((start, from));
            }
        } else if open.is_none() {
            open = Some(from);
        }
    }
    if let Some(start) = open {
        runs.push((start, rune::text::advance(word, text_style)));
    }
    runs
}

/// A paragraph or a list item: a wrapping row of words (`key`), with the gate's note of the
/// whole line. Inline code is `<key>-code` (the first of the line; later ones `<key>-code-<n>`).
fn flow(key: String, pieces: &[Piece], t: &Theme) -> El {
    kit::note_text(&key, &key, &plain(pieces), style::MD_BODY, t.text_secondary);
    let mut line = row()
        .key(key.clone())
        .role("paragraph")
        .label(plain(pieces))
        .w_full()
        .flex_wrap()
        .items_start();
    let (mut n, mut codes) = (0usize, 0usize);
    for piece in pieces {
        if piece.code {
            let code_key = if codes == 0 {
                format!("{key}-code")
            } else {
                format!("{key}-code-{codes}")
            };
            codes += 1;
            line = line.child(
                row()
                    .key(code_key.clone())
                    .h(15.5)
                    .my(1.0)
                    .px(4.0)
                    .radius(4.0)
                    .fill(t.field)
                    .center_y()
                    .child(style::fitted(
                        format!("{code_key}-text"),
                        &piece.text,
                        style::MD_CODE,
                        t.text_secondary,
                    )),
            );
            continue;
        }
        let text_style: TextStyle = if piece.bold { style::MD_BOLD } else { style::MD_BODY };
        let ink = if piece.bold || piece.link.is_some() {
            t.text
        } else {
            t.text_secondary
        };
        // A word and the space after it stay together, so a line breaks between words.
        let words: Vec<&str> = piece.text.split_inclusive(' ').collect();
        for (at, word) in words.iter().enumerate() {
            let mut label = style::fitted(format!("{key}-w{n}"), word, text_style, ink);
            n += 1;
            // A link's line runs under the spaces between its words, not after its last one.
            let shown = if at + 1 < words.len() {
                word.len()
            } else {
                word.trim_end().len()
            };
            if piece.italic {
                label = label.spans(vec![Span::new(0, shown).italic()]);
            }
            if piece.link.is_some() {
                // The link's line, 2 below the baseline (`underline-offset-2`): Rune draws a
                // span's underline at the font's own offset and has no setting for it ("Rune
                // limitations", PR body), so the line is a box of its own.
                let mut linked = rune::el()
                    .key(format!("{key}-w{}-link", n - 1))
                    .size(rune::text::advance(word, text_style), 17.0)
                    .child(label);
                for (from, to) in underline_runs(&word[..shown], text_style) {
                    linked = linked.child(rune::el().absolute(from, UNDERLINE_Y).size(to - from, 1.0).fill(ink));
                }
                label = linked;
            }
            if let Some(dest) = &piece.link {
                let dest = dest.clone();
                label = label
                    .role("link")
                    .label(piece.text.clone())
                    .focusable(true)
                    .focus_ring(2.0, 1.0, t.accent_deep)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |s: &mut State| {
                        if let Some(ui) = ui(s) {
                            ui.follow_link(&dest);
                        }
                    });
            }
            line = line.child(label);
        }
    }
    line
}

/// The note, rendered: a column at most 620 wide on the content ground.
pub fn body(blocks: &[Block], t: &Theme) -> El {
    let mut md = col().key("md").w_full().max_w(620.0).gap(12.0);
    let (mut h1, mut h2, mut p, mut li, mut pre) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for block in blocks {
        match block {
            Block::Heading(1, words) => {
                let key = if h1 == 0 {
                    "md-h1".to_string()
                } else {
                    format!("md-h1-{h1}")
                };
                h1 += 1;
                md = md.child(
                    kit::label(key, plain(words), style::MD_H1, t.text)
                        .role("heading")
                        .level(1)
                        .wrap()
                        .w_full(),
                );
            }
            Block::Heading(level, words) => {
                md = md.child(
                    kit::label(format!("md-h2-{h2}"), plain(words), style::TITLE, t.text)
                        .role("heading")
                        .level(*level as u32)
                        .wrap()
                        .w_full()
                        .mt(4.0),
                );
                h2 += 1;
            }
            Block::Para(pieces) => {
                md = md.child(flow(format!("md-p-{p}"), pieces, t));
                p += 1;
            }
            Block::List(items) => {
                let mut list = col()
                    .key(format!("md-list-{li}"))
                    .role("list")
                    .w_full()
                    .pl(20.0)
                    .gap(4.0);
                for item in items {
                    list = list.child(flow(format!("md-li-{li}"), item, t).role("listitem"));
                    li += 1;
                }
                md = md.child(list);
            }
            Block::Code(text) => {
                md = md.child(
                    col()
                        .key(format!("md-pre-{pre}"))
                        .w_full()
                        .px(12.0)
                        .py(8.0)
                        .radius(6.0)
                        .fill(t.field)
                        .child(
                            kit::label(
                                format!("md-pre-{pre}-text"),
                                text.clone(),
                                style::CODE,
                                t.text_secondary,
                            )
                            .wrap()
                            .w_full(),
                        ),
                );
                pre += 1;
            }
        }
    }
    col()
        .key("viewer-body")
        .role("document")
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .h_full()
        .px(48.0)
        .py(32.0)
        .fill(t.bg)
        .clip_rounded(0.0)
        // A note longer than the window is read by scrolling.
        .min_h(0.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(md)
}
