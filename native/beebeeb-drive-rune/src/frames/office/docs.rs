//! Slice OF5 (Office viewer): the read-only documents viewer of the Office engine, DW-1507 (a .docx) and DW-1511 (an
//! older .doc, with its note). DW-1563 and DW-1564 are the same window in two states (`states.rs`, OF6).
//!
//! What is drawn: the window's own toolbar (the viewer's, reused from `viewers::office`), the viewer bar (zoom, where
//! you are, Open in another app…), the outline of the document's headings, the pages on grey as `rune::Pixels` asked
//! of the source by number, and the status line (Page, words, language, Encrypted, About). Nothing here edits: no
//! ribbon, no Save, Undo, Bold, Insert, Layout, Review and no floating toolbar. A page the source has no picture of
//! yet is blank paper, never a sample.
//!
//! Keys: the window's caption, sidebar and toolbar keep the keys every window has. This file's are `vbar` (and
//! `zoom-less`, `zoom`, `zoom-more`, `vbar-page`, `elsewhere`), `banner` / `banner-text` (the .doc's note), `ol` (the
//! outline), `ol-close` (its fold control), `ol-<N>-row` (a heading's row) and `ol-<N>` (its words), `stage`, `page` (page 1) and
//! `page-<N>` (a later page), and `st-<N>` (the status line). `pg-<N>` is the board's key of a LINE of text on the page; a
//! picture of a page has none (D-B3-7), so no page is ever keyed `pg-`.
//!
//! Wiring this file needs from `viewers` (listed in BRANCH-NOTES.md for the checkpoint): `Body::Doc(DocBody)`, the
//! `viewers::doc_mut(&mut State) -> Option<&mut DocBody>` and `viewers::open_doc(&mut State, OpenFile) -> bool` calls,
//! the window arm calling [`window`], and the key route calling [`DocBody::on_key`].

use rune::tok::Family;
use rune::{Color, Cx, El, Icon, Key, Mods, Pixels, SurfaceFit, TextStyle, col, el, row};
use beebeeb_office_protocol::office::EditKind;
use zeroize::Zeroize;

use crate::State;
use super::edit::{EditUi, ime, word};
use crate::frames::viewers::{self, Ui};
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, type_};
use crate::ui::{caption, kit, sidebar};

#[path = "docs_fixture.rs"]
pub mod docs_fixture;

/// The group's board frames and the scene that shows each (`frames::scenes`). DW-1511 is in `viewers::FRAMES`
/// (its row there points at [`scene_older`]).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[("DW-1507", scene_docx)];

/// DW-1507: the .docx open on page 1.
pub fn scene_docx(s: &mut State) {
    viewers::open_doc(s, docs_fixture::file(false));
}

/// DW-1511: the older .doc, with its note.
pub fn scene_older(s: &mut State) {
    viewers::open_doc(s, docs_fixture::file(true));
}

// ---------------------------------------------------------------------------
// What a document is
// ---------------------------------------------------------------------------

/// The document as the viewer shows it. The pages' pixels are the source's, asked by page number.
#[derive(Clone, Debug, PartialEq)]
pub struct DocView {
    pub pages: u32,
    /// Each page's size in points (width, height), in page order.
    pub page_sizes_pt: Vec<(f32, f32)>,
    /// The headings: level (1 is the title), text, and the page it is on (1-based).
    pub outline: Vec<(u8, String, u32)>,
    pub words: u32,
    /// Zoom in per cent.
    pub zoom: u16,
    /// An older Word file (.doc): the note says it opens read-only.
    pub older_format: bool,
}

/// What `viewers` holds for an open document: the view, the page in view, the heading it came from and the language.
#[derive(Clone, PartialEq)]
pub struct DocBody {
    pub view: DocView,
    /// The page at the top of the stage, 1-based.
    pub page: u32,
    /// The outline entry that was last gone to (or the first).
    pub at: usize,
    pub language: String,
    /// The one quiet line the status bar holds after its left words (DW-1564); `None`: no line.
    pub quiet: Option<&'static str>,
    /// The outline pane is open (the board draws it open; its close control folds it to a strip).
    pub outline_open: bool,
    /// Where the stage was last seen scrolled to, in points from the top of its content (`docs_scroll`).
    pub scroll_y: f32,
    /// The editor asked to show the caret once it has moved: where it stood (page, x, y in points) when it was asked
    /// (`edit::word`). `None`: nothing is asked.
    pub reveal: Option<(u32, f32, f32)>,
}

/// Counts only, never a heading or a word.
impl std::fmt::Debug for DocBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Doc({} pages, {} words)", self.view.pages, self.view.words)
    }
}

pub const ZOOM_MIN: u16 = 50;
pub const ZOOM_MAX: u16 = 200;
const ZOOM_STEP: u16 = 10;

impl DocBody {
    pub fn new(view: DocView, language: &str) -> Self {
        DocBody {
            view,
            page: 1,
            at: 0,
            language: language.into(),
            quiet: None,
            outline_open: true,
            scroll_y: 0.0,
            reveal: None,
        }
    }

    /// Show page `n` at the top of the stage (clamped to the document). The stage's viewport is moved there by
    /// [`scroll::show_page`] from the handler that called this.
    pub fn go_to_page(&mut self, n: u32) {
        self.page = n.clamp(1, self.view.pages.max(1));
        self.scroll_y = self.page_top(self.page) - scroll::PAD_TOP;
    }

    /// A press on an outline heading: the stage goes to its page.
    pub fn go_to_heading(&mut self, i: usize) {
        if let Some((_, _, page)) = self.view.outline.get(i) {
            let page = *page;
            self.at = i;
            self.go_to_page(page);
        }
    }

    /// The outline's close control: fold the pane to a strip, or open it again.
    pub fn toggle_outline(&mut self) {
        self.outline_open = !self.outline_open;
    }

    /// A zoom keeps the page in view: it is shown from its top at the new size.
    pub fn zoom_in(&mut self) {
        self.view.zoom = (self.view.zoom + ZOOM_STEP).min(ZOOM_MAX);
        self.go_to_page(self.page);
    }

    pub fn zoom_out(&mut self) {
        self.view.zoom = self.view.zoom.saturating_sub(ZOOM_STEP).max(ZOOM_MIN);
        self.go_to_page(self.page);
    }

    /// Overwrite the decrypted words held here (see `OpenFile::wipe`).
    pub fn wipe(&mut self) {
        for (_, title, _) in &mut self.view.outline {
            title.zeroize();
        }
        self.language.zeroize();
    }

    /// The viewer's keys: Ctrl+Plus and Ctrl+Minus zoom, Ctrl+0 is the actual size, Page Down and Page Up move the stage by a page. Every other
    /// key is not taken here: the editor's keys (Ctrl+S, Ctrl+K and the rest) are `edit`'s, taken while an editor is live on the document.
    /// `true`: taken.
    pub fn on_key(&mut self, key: &Key, m: &Mods) -> bool {
        let command = (m.ctrl || m.cmd) && !m.alt;
        match key {
            Key::Command('+') | Key::Command('=') if command => self.zoom_in(),
            Key::Command('-') if command => self.zoom_out(),
            Key::Command('0') if command => {
                self.view.zoom = 100;
                self.go_to_page(self.page);
            }
            Key::PageDown if !command && !m.alt && !m.shift => self.go_to_page(self.page.saturating_add(1)),
            Key::PageUp if !command && !m.alt && !m.shift => self.go_to_page(self.page.saturating_sub(1)),
            _ => return false,
        }
        true
    }
}

// ---------------------------------------------------------------------------
// The window (DW-1507, DW-1511)
// ---------------------------------------------------------------------------

/// The words the board shows in these frames.
pub const OPEN_ELSEWHERE: &str = "Open in another app\u{2026}";
/// Said when the file is not kept on this PC (decision 2482, Q8: the viewer makes no temporary decrypted copy).
pub const KEEP_FIRST: &str = "Keep this file on this PC first.";
pub const OLDER_FORMAT: &str = "An older Word file (.doc). It opens read-only here.";
pub const ENCRYPTED: &str = "Encrypted";
pub const ABOUT: &str = "About";

const BAR_LABEL: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const BAR_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);
const PANE_LABEL: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.7);
const OUTLINE: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
const OUTLINE_TOP: TextStyle = TextStyle::new(12.0, 600.0).lh(15.0);
const STATUS_ENC: TextStyle = TextStyle::new(11.5, 600.0).lh(17.0);
const BANNER: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);

static LESS: Icon = Icon {
    name: "docs-less",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M5 12h14"></path></g>"#
    ),
};

static MORE: Icon = Icon {
    name: "docs-more",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M12 5v14M5 12h14"></path></g>"#
    ),
};

static CHEVRON_LEFT: Icon = Icon {
    name: "docs-chevron-left",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M15 5l-7 7 7 7"></path></g>"#
    ),
};

static CHEVRON_RIGHT: Icon = Icon {
    name: "docs-chevron-right",
    view: 24.0,
    body: concat!(
        r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
        r#"<path d="M9 5l7 7-7 7"></path></g>"#
    ),
};

/// The outline pane's width and the page's width at 100 %.
const OUTLINE_W: f32 = 210.0;
const PAGE_W: f32 = 520.0;
/// The board's page is at least this tall at 100 %.
const PAGE_MIN_H: f32 = 700.0;
/// The folded outline's strip.
const STRIP_W: f32 = 44.0;

fn with_doc(act: impl Fn(&mut DocBody) + 'static) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| {
        if let Some(doc) = viewers::doc_mut(s) {
            act(doc);
        }
    }
}

/// Open in another app…: the file must be kept on this PC; if it is not, the press says so and does nothing else.
fn open_elsewhere(s: &mut State) {
    if let Some(ui) = viewers::ui(s) {
        if ui.file.details.offline {
            ui.intent(viewers::Intent::OpenInAnotherApp);
        } else {
            ui.toast = Some(KEEP_FIRST.to_string());
        }
    }
}

fn step_button(key: &'static str, label: &'static str, icon: &'static Icon, act: fn(&mut DocBody), t: &Theme) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .size(24.0, 24.0)
        .center()
        .radius(6.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press_tx(scroll::with_doc_tx(act))
        .child(kit::glyph(format!("{key}-icon"), icon, 13.0, t.text_secondary))
}

/// The bar under the window's toolbar: zoom, where you are, and the way to another app. No ribbon.
fn bar(doc: &DocBody, t: &Theme) -> El {
    let zoom = format!("{}%", doc.view.zoom);
    let page = format!("Page {} of {}", doc.page, doc.view.pages.max(1));
    row()
        .key("vbar")
        .role("toolbar")
        .label("Document")
        .w_full()
        .h(38.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .fill(t.card)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("vbar-zoom")
                .role("group")
                .label(format!("Zoom, {} per cent", doc.view.zoom))
                .gap(4.0)
                .center_y()
                .child(step_button("zoom-less", "Zoom out", &LESS, DocBody::zoom_out, t))
                .child(
                    kit::label("zoom", zoom.clone(), BAR_MONO, t.text_secondary)
                        .w(rune::text::advance(&zoom, BAR_MONO)),
                )
                .child(step_button("zoom-more", "Zoom in", &MORE, DocBody::zoom_in, t)),
        )
        .child(el().key("vbar-sep").w(kit::HAIR).h(18.0).fill(t.separator))
        .child(
            kit::label("vbar-page", page.clone(), BAR_LABEL, t.text_secondary).w(rune::text::advance(&page, BAR_LABEL)),
        )
        .child(el().key("vbar-gap").grow(1.0))
        .child(
            row()
                .key("elsewhere")
                .role("button")
                .label(OPEN_ELSEWHERE)
                .h(28.0)
                .px(10.0)
                .center_y()
                .radius(7.0)
                .fill(t.paper)
                .border(1.0, t.card_border)
                .hover(t.hover)
                .press(t.press)
                .focusable(true)
                .on_press(open_elsewhere)
                .child(kit::label(
                    "elsewhere-label",
                    OPEN_ELSEWHERE,
                    BAR_LABEL,
                    t.text_secondary,
                )),
        )
}

/// The editor's banner fill (`viewers::style::banner`): ink at 6 %, light ink on dark.
fn banner_fill(t: &Theme) -> Color {
    let ink = rune::color::hex(if t.dark { "#EEDDDD" } else { "#111111" }).unwrap_or(Color::BLACK);
    Color { a: 15.0 / 255.0, ..ink }
}

/// The note of an older file, under the toolbar: the editor's banner (`viewers::text::banner_el`), same margins,
/// padding, radius and text style.
fn note(t: &Theme) -> El {
    row()
        .key("banner")
        .role("status")
        .label(OLDER_FORMAT)
        .mx(14.0)
        .mt(10.0)
        .pl(12.0)
        .pr(10.0)
        .py(9.0)
        .gap(10.0)
        .center_y()
        .radius(10.0)
        .fill(banner_fill(t))
        .child(kit::glyph("banner-icon", &viewers::icons::INFO, 16.0, t.text_secondary))
        .child(kit::label("banner-text", OLDER_FORMAT, BANNER, t.text_secondary).grow(1.0))
}

/// The current heading's row fill: the board's ink at 9 % (`color-mix(ink 9%, ground)`), light ink on dark.
fn here_fill(t: &Theme) -> Color {
    let ink = rune::color::hex(if t.dark { "#E9E9DE" } else { "#16160B" }).unwrap_or(Color::BLACK);
    Color { a: 23.0 / 255.0, ..ink }
}

/// The outline's fold control: a 28 px square with a chevron (left while open, right while folded).
fn outline_close(open: bool, t: &Theme) -> El {
    row()
        .key("ol-close")
        .role("button")
        .label(if open { "Close the outline" } else { "Open the outline" })
        .size(28.0, 28.0)
        .center()
        .radius(6.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(with_doc(DocBody::toggle_outline))
        .child(kit::glyph(
            "ol-close-icon",
            if open { &CHEVRON_LEFT } else { &CHEVRON_RIGHT },
            13.0,
            t.text_secondary,
        ))
}

fn outline(doc: &DocBody, t: &Theme) -> El {
    if !doc.outline_open {
        return col()
            .key("ol")
            .role("navigation")
            .label("Outline, closed")
            .w(STRIP_W)
            .h_full()
            .py(12.0)
            .px(8.0)
            .fill(t.surface)
            .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
            .border_layout()
            .clip_rounded(0.0)
            .child(outline_close(false, t));
    }
    let mut pane = col()
        .key("ol")
        .role("navigation")
        .label("Outline")
        .w(OUTLINE_W)
        .h_full()
        .py(12.0)
        .px(8.0)
        .gap(4.0)
        .fill(t.surface)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .clip_rounded(0.0)
        .child(
            row()
                .key("ol-head")
                .w_full()
                .px(8.0)
                .pb(4.0)
                .center_y()
                .child(kit::label("ol-title", "OUTLINE", PANE_LABEL, t.text_tertiary))
                .child(el().key("ol-head-gap").grow(1.0))
                .child(outline_close(true, t)),
        );
    for (i, (level, title, page)) in doc.view.outline.iter().enumerate() {
        let here = i == doc.at;
        let (style, ink, indent) = match (*level, here) {
            (1, _) => (OUTLINE_TOP, t.text_secondary, 8.0),
            (_, true) => (OUTLINE, t.text, 20.0),
            (_, false) => (OUTLINE, t.text_secondary, 20.0),
        };
        let at = i;
        let mut line = row()
            .key(format!("ol-{i}-row"))
            .role("link")
            .label(format!("{title}, page {page}"))
            .w_full()
            .h(23.0)
            .pl(indent)
            .center_y()
            .radius(5.0)
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .on_press_tx(scroll::with_doc_tx(move |d| d.go_to_heading(at)))
            .child(
                kit::label(format!("ol-{i}"), title.clone(), style, ink)
                    .w(rune::text::advance(title, style))
                    .truncate()
                    .min_w(0.0),
            );
        if here {
            line = line.fill(here_fill(t));
        }
        pane = pane.child(line);
    }
    pane
}

/// The page's size in points (width, height) as the file says, the board's A4 when it says nothing.
fn size_pt(doc: &DocBody, n: u32) -> (f32, f32) {
    doc.view
        .page_sizes_pt
        .get(n as usize - 1)
        .copied()
        // A size the file states is trusted only as far as a page can be: finite, positive, and at most 20 times as
        // tall as wide or the reverse (an absurd one is the board's A4).
        .filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0 && h / w <= 20.0 && w / h <= 20.0)
        .unwrap_or((595.0, 842.0))
}

/// Page `n`'s box on the stage, in layout points: the board's width at the stage's zoom, in the page's own proportions.
fn box_pt(doc: &DocBody, n: u32) -> (f32, f32) {
    let (w_pt, h_pt) = size_pt(doc, n);
    let w = PAGE_W * f32::from(doc.view.zoom) / 100.0;
    (w, w * h_pt / w_pt)
}

/// The paper of page `n` before anything is drawn on it. Page 1 is keyed `page`, a later page `page-{n}` (the board's
/// `pg-` is a line of text, which a picture has none of).
fn shell(n: u32, doc: &DocBody) -> El {
    let w = PAGE_W * f32::from(doc.view.zoom) / 100.0;
    col()
        .key(scroll::page_key(n))
        .role("image")
        .label(format!("Page {n}"))
        .w(w)
        .items_start()
        .radius(2.0)
        .clip_rounded(2.0)
        .fill(Color::WHITE)
        .box_shadow(kit::drop(4.0, 18.0, Color::BLACK.alpha(0.16)))
}

/// One page of paper: white, the source's picture drawn whole at its own aspect (never cropped) from the top of the
/// page. The page is the board's width at the stage's zoom and at least the board's height; with no picture it is
/// blank paper in the page's own proportions.
fn paper(n: u32, doc: &DocBody, pixels: Option<Pixels>) -> El {
    let (w, blank_h) = box_pt(doc, n);
    let page = shell(n, doc).h(fixture_h(doc, n, pixels.as_ref()));
    match pixels {
        Some(p) => {
            let (pw, ph) = p.size();
            let pic_h = if pw > 0 { w * ph as f32 / pw as f32 } else { blank_h };
            page.child(
                el().key(format!("page-picture-{n}"))
                    .size(w, pic_h)
                    .pixels_fit(&p, SurfaceFit::Fill),
            )
        }
        None => page,
    }
}

/// The height [`paper`] gives page `n`: with a picture the picture's at the page's width, at least the board's height;
/// without, the page's own proportions.
fn fixture_h(doc: &DocBody, n: u32, pixels: Option<&Pixels>) -> f32 {
    let (w, blank_h) = box_pt(doc, n);
    let scale = f32::from(doc.view.zoom) / 100.0;
    match pixels.map(Pixels::size) {
        Some((pw, ph)) if pw > 0 => (w * ph as f32 / pw as f32).max(PAGE_MIN_H * scale),
        Some(_) => blank_h.max(PAGE_MIN_H * scale),
        None => blank_h,
    }
}

/// A page from the renderer: always its box (the file's proportions), with or without its picture, so a picture that
/// comes never moves the pages under it or the offset the stage stands at. The picture is drawn at its own aspect from
/// the top of the page, clipped by the page.
fn paper_box(n: u32, doc: &DocBody, pixels: Option<Pixels>) -> El {
    let (w, h) = box_pt(doc, n);
    let page = shell(n, doc).h(h);
    match pixels {
        Some(p) => {
            let (pw, ph) = p.size();
            let pic_h = if pw > 0 { w * ph as f32 / pw as f32 } else { h };
            page.child(
                el().key(format!("page-picture-{n}"))
                    .size(w, pic_h)
                    .pixels_fit(&p, SurfaceFit::Fill),
            )
        }
        None => page,
    }
}

/// The gap between two pages on the stage.
const PAGE_GAP: f32 = 16.0;

#[path = "docs_live.rs"]
mod live;
use live::paper_live;
pub use live::{Asks, Live, asked_pages};

#[path = "docs_scroll.rs"]
pub mod scroll;
pub use scroll::on_key;

/// Where a document's pictures come from: the fixture's whole pages, or a renderer asked for what is near the screen.
pub enum Pictures<'a> {
    Fixture(&'a dyn Fn(u32) -> Option<Pixels>),
    Live(&'a Live<'a>),
}

/// The pages on grey in the stage's viewport (`docs_scroll`). Every page keeps its box, so the scroll height is right;
/// live, only the pages near the screen ask the renderer for a picture, and the rest are blank paper of their size; the
/// fixture draws its pages as it always did.
fn stage(doc: &DocBody, pics: &Pictures, stage_rect: Option<[f32; 4]>, ed: Option<(&EditUi, Option<El>)>, t: &Theme) -> El {
    let count = doc.view.pages.max(1);
    let mut pages = col()
        .key("pages")
        .role("document")
        .label("Pages")
        .items_center()
        .gap(PAGE_GAP)
        .py(scroll::PAD_TOP);
    let mut heights = Vec::with_capacity(count as usize);
    match pics {
        Pictures::Fixture(page) => {
            for n in 1..=count {
                let pixels = page(n);
                heights.push(fixture_h(doc, n, pixels.as_ref()));
                pages = pages.child(paper(n, doc, pixels));
            }
        }
        Pictures::Live(live) => {
            let (lo, hi) = asked_pages(doc, live.screen.1);
            // Each page's top below the stage's top, from where the stage was last seen scrolled to.
            let view_y = doc.view_y();
            let mut top = scroll::PAD_TOP;
            for n in 1..=count {
                let h = box_pt(doc, n).1;
                heights.push(h);
                pages = pages.child(if (lo..=hi).contains(&n) {
                    paper_live(n, doc, live, top - view_y)
                } else {
                    paper(n, doc, None)
                });
                top += h + PAGE_GAP;
            }
        }
    }
    // A document open for editing: the pages' column is the editor's surface (focus, keys, pointer, caret).
    let content = match ed {
        Some((edit, ime)) => word::surface(pages, doc, edit, stage_rect, ime),
        None => pages,
    };
    scroll::viewport(content, stage_rect.map(|r| r[1]), heights.into(), t)
}

/// `1284` as "1,284".
fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The status line: Page, words, language at the left; Encrypted and About at the right.
fn status(doc: &DocBody, t: &Theme) -> El {
    let parts = [
        format!("Page {} of {}", doc.page, doc.view.pages.max(1)),
        format!("{} words", thousands(doc.view.words)),
        doc.language.clone(),
    ];
    let mut bar: Vec<El> = Vec::new();
    let mut n = 0;
    let mut said_quiet = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            bar.push(kit::label(
                format!("st-{n}"),
                "\u{00B7}",
                type_::STATUS,
                t.text_tertiary,
            ));
            n += 1;
        }
        bar.push(
            kit::label(format!("st-{n}"), part.clone(), type_::STATUS, t.text_tertiary)
                .w(rune::text::advance(part, type_::STATUS)),
        );
        n += 1;
    }
    // The board's OvStatus: after the left words, `·` and the one quiet line.
    if let Some(quiet) = doc.quiet {
        bar.push(kit::label(
            format!("st-{n}"),
            "\u{00B7}",
            type_::STATUS,
            t.text_tertiary,
        ));
        bar.push(
            kit::label(format!("st-{}", n + 1), quiet, type_::STATUS, t.text_tertiary)
                .w(rune::text::advance(quiet, type_::STATUS)),
        );
        n += 2;
        said_quiet = format!(". {quiet}");
    }
    let about_key = format!("st-{}", n + 2);
    let said = format!("{}{said_quiet}. {ENCRYPTED}", parts.join(", "));
    bar.push(el().key("st-gap").grow(1.0));
    bar.push(
        row()
            .key(format!("st-{n}-box"))
            .gap(4.0)
            .center_y()
            .child(kit::glyph(format!("st-{n}-icon"), &icons::LOCK, 11.0, t.accent_deep))
            .child(
                kit::label(format!("st-{n}"), ENCRYPTED, STATUS_ENC, t.accent_deep)
                    .w(rune::text::advance(ENCRYPTED, STATUS_ENC)),
            ),
    );
    bar.push(kit::label(
        format!("st-{}", n + 1),
        "\u{00B7}",
        type_::STATUS,
        t.text_tertiary,
    ));
    bar.push(
        el().key(format!("{about_key}-box"))
            .role("link")
            .label("About this viewer")
            .focusable(true)
            .on_press(|s: &mut State| {
                if let Some(ui) = viewers::ui(s) {
                    ui.office_about();
                }
            })
            .child(
                // The board underlines "About" (offset 2 px): the line is a bar under the words; the top pad
                // keeps the words where the board has them.
                col()
                    .key(format!("{about_key}-under"))
                    .pt(2.0)
                    .child(
                        kit::label(about_key, ABOUT, type_::STATUS, t.text_tertiary)
                            .w(rune::text::advance(ABOUT, type_::STATUS)),
                    )
                    .child(
                        el().key("st-about-line")
                            .mt(1.0)
                            .w(rune::text::advance(ABOUT, type_::STATUS))
                            .h(1.0)
                            .fill(t.text_tertiary),
                    ),
            ),
    );
    crate::ui::status_bar::frame(bar, t).label(said)
}

/// The window of DW-1507 / DW-1511: the caption, the sidebar, and the document beside it. `page(n)`: the source's
/// picture of page `n`.
pub fn window(s: &State, cx: &mut Cx<State>, ui: &Ui, doc: &DocBody, page: &dyn Fn(u32) -> Option<Pixels>) -> El {
    window_with(s, cx, ui, doc, &Pictures::Fixture(page))
}

/// The window of a document the Office engine renders: the same window, its pictures asked as [`paper_live`] says.
pub fn window_live(s: &State, cx: &mut Cx<State>, ui: &Ui, doc: &DocBody, live: &Live) -> El {
    window_with(s, cx, ui, doc, &Pictures::Live(live))
}

fn window_with(s: &State, cx: &mut Cx<State>, ui: &Ui, doc: &DocBody, pics: &Pictures) -> El {
    let t = tokens::theme(cx.system_dark());
    // The editor of a Word document that runs: its surface needs the stage's whole rectangle (to keep the caret in view).
    let editing = ui
        .office
        .edit
        .as_ref()
        .filter(|e| e.kind == EditKind::Docx && !e.stopped)
        .map(|e| (e, e.has_keyboard.then(|| ime::caret_el(s)).flatten()));
    let stage_rect = if editing.is_some() {
        cx.layout_with(scroll::STAGE, |r| r)
    } else {
        cx.layout_with(scroll::STAGE, |r| [0.0, r[1], 0.0, 0.0])
    };
    let mut body = row()
        .key("viewer-row")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .child(outline(doc, &t))
        .child(stage(doc, pics, stage_rect, editing, &t));
    if ui.show_details {
        body = body.child(viewers::details::panel(&ui.file, ui.knows_offline(), &t));
    }
    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(viewers::office::toolbar(s, ui, false, &t));
    if doc.view.older_format {
        main = main.child(note(&t));
    }
    main = match ui.office.edit.as_ref().map(|e| e.kind) {
        Some(kind) => main.child(super::edit::chrome(s, cx, ui, kind, body)),
        None => main.child(bar(doc, &t)).child(body).child(status(doc, &t)),
    };
    let mut under = el().key("under-caption").grow(1.0).min_h(0.0).w_full().child(
        row()
            .key("body")
            .w_full()
            .h_full()
            .child(sidebar::sidebar(s, &t))
            .child(main),
    );
    if let Some(over) = viewers::office_over::over(ui, &t) {
        under = under.child(over);
    }
    if let Some(words) = &ui.toast {
        under = under.child(viewers::dialogs::toast(words, &t));
    }
    col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(under)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use rune::{Env, KeyEvent, Runtime};

    use super::*;
    use crate::frames::viewers::office_over::CommandGroup;
    use crate::frames::viewers::source::{
        FixtureViewerSource, Keep, OpenError, OpenFile as File, SaveOutcome, ViewerSource,
    };
    use crate::{init, view};

    fn runtime_of(state: State, dark: bool) -> Runtime<State> {
        let env = Env {
            system_dark: dark,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
        rt.on_key = Some(crate::shell::host::on_key);
        rt.poll = Some(crate::shell::host::poll);
        rt.frame(0.0);
        rt
    }

    fn scene(f: fn(&mut State), dark: bool) -> Runtime<State> {
        let mut state = init();
        f(&mut state);
        runtime_of(state, dark)
    }

    fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
    }

    fn press_with(rt: &mut Runtime<State>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(0.0);
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::default()
        }
    }

    /// The fixture, with every call that would save, resolve or run an editor command kept where the test reads it.
    /// Opening is the one call the viewer makes, and it is not recorded.
    struct Recording {
        inner: FixtureViewerSource,
        calls: Rc<RefCell<Vec<String>>>,
    }

    impl ViewerSource for Recording {
        fn open(&mut self, id: &str) -> Result<File, OpenError> {
            self.inner.open(id)
        }
        fn poll(&mut self, id: &str) -> Option<crate::frames::viewers::source::Body> {
            self.inner.poll(id)
        }
        fn cancel(&mut self, id: &str) {
            self.inner.cancel(id)
        }
        fn retry(&mut self, id: &str) -> Result<File, OpenError> {
            self.inner.retry(id)
        }
        fn save(&mut self, id: &str, _text: &str, base: u32) -> SaveOutcome {
            self.calls.borrow_mut().push(format!("save {id} {base}"));
            SaveOutcome::Failed
        }
        fn resolve(&mut self, id: &str, _text: &str, keep: Keep) -> SaveOutcome {
            self.calls.borrow_mut().push(format!("resolve {id} {keep:?}"));
            SaveOutcome::Failed
        }
        fn save_document(&mut self, id: &str, base: u32) -> SaveOutcome {
            self.calls.borrow_mut().push(format!("save document {id} {base}"));
            SaveOutcome::Failed
        }
        fn resolve_document(&mut self, id: &str, keep: Keep) -> SaveOutcome {
            self.calls.borrow_mut().push(format!("resolve document {id} {keep:?}"));
            SaveOutcome::Failed
        }
        fn commands(&self, id: &str, query: &str) -> Vec<CommandGroup> {
            self.calls.borrow_mut().push(format!("commands {id} {query}"));
            Vec::new()
        }
    }

    fn doc(rt: &mut Runtime<State>) -> DocBody {
        viewers::doc_mut(&mut rt.state).expect("a document is open").clone()
    }

    /// The scenes leave the state the frames name: DW-1507 a four-page .docx, no note; DW-1511 a one-page .doc with
    /// the note.
    #[test]
    fn office_docs_scenes_set_their_state() {
        let mut rt = scene(scene_docx, false);
        let d = doc(&mut rt);
        assert_eq!(
            (d.page, d.view.pages, d.view.zoom, d.view.older_format),
            (1, 4, 100, false)
        );
        assert!(find(&rt, "banner").is_none() && find(&rt, "ol").is_some() && find(&rt, "page").is_some());
        let mut rt = scene(scene_older, false);
        let d = doc(&mut rt);
        assert_eq!((d.view.pages, d.view.older_format, d.at), (1, true, 1));
        assert_eq!(
            find(&rt, "banner-text").and_then(|e| e.text),
            Some(OLDER_FORMAT.to_string())
        );
        // DW-1507's row is this scene; DW-1511's is in `viewers::FRAMES`.
        assert!(
            FRAMES
                .iter()
                .any(|(id, f)| *id == "DW-1507" && *f as usize == scene_docx as usize)
        );
        assert!(viewers::FRAMES.iter().any(|(id, _)| *id == "DW-1511"));
    }

    /// The viewer has no edit control: nothing keyed or labelled Save, Undo, Redo, Bold, Insert, Layout, Review, and
    /// no ribbon and no floating selection toolbar.
    #[test]
    fn office_docs_viewer_has_no_edit_control() {
        for f in [scene_docx as fn(&mut State), scene_older] {
            let rt = scene(f, false);
            for e in rt.elements() {
                let label = e.label.clone().unwrap_or_default().to_lowercase();
                let text = e.text.clone().unwrap_or_default().to_lowercase();
                let key = e.key.clone().unwrap_or_default();
                // Whole words: "Preview" and "Overview" are not the Review tab.
                for banned in ["save", "undo", "redo", "bold", "insert", "layout", "review", "floating"] {
                    let word = |s: &str| s.split(|c: char| !c.is_alphanumeric()).any(|w| w == banned);
                    assert!(!word(&label) && !word(&text), "{banned} on {key}");
                }
                assert!(
                    !key.starts_with("rib") && !key.starts_with("rt-") && !key.starts_with("float"),
                    "editor key {key}"
                );
            }
        }
    }

    /// Ctrl+S, Ctrl+K and Ctrl+M on a viewer body ask the source for nothing, and change nothing on screen.
    #[test]
    fn office_viewer_keys_reach_no_editor_call() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut state = init();
        let source = Recording {
            inner: FixtureViewerSource::new(),
            calls: calls.clone(),
        };
        assert!(viewers::open_from(&mut state, Box::new(source), docs_fixture::DOCX_ID));
        let mut rt = runtime_of(state, false);
        let before = doc(&mut rt);
        for c in ['s', 'k', 'm'] {
            press_with(&mut rt, Key::Command(c), ctrl());
        }
        assert!(
            calls.borrow().is_empty(),
            "the viewer asked the source: {:?}",
            calls.borrow()
        );
        assert_eq!(doc(&mut rt), before, "the document changed under a key");
        assert!(viewers::ui(&mut rt.state).is_some_and(|ui| ui.office.over.is_none() && ui.toast.is_none()));
    }

    /// A press on a heading in the outline goes to its page; Ctrl+Plus / Ctrl+Minus zoom, within 50 to 200.
    #[test]
    fn office_docs_outline_goes_to_its_page() {
        let mut rt = scene(scene_docx, false);
        let last = doc(&mut rt).view.outline.len() - 1;
        let want = doc(&mut rt).view.outline[last].2;
        assert!(want > 1, "the fixture's last heading is past page 1");
        let [x, y, w, h] = find(&rt, &format!("ol-{last}-row")).expect("the heading is drawn").rect;
        rt.pointer_down(x + w / 2.0, y + h / 2.0);
        rt.pointer_up(x + w / 2.0, y + h / 2.0);
        rt.frame(0.0);
        rt.frame(0.0);
        let d = doc(&mut rt);
        assert_eq!((d.page, d.at), (want, last));
        // The stage scrolled to the heading's page: it stands at the stage's top, under the top padding.
        let stage = find(&rt, scroll::STAGE).expect("the stage").rect;
        let page = find(&rt, &format!("page-{want}")).expect("the heading's page").rect;
        assert!(
            (page[1] - stage[1] - scroll::PAD_TOP).abs() < 0.6,
            "page {want} at the stage's top: {page:?} in {stage:?}"
        );
        assert_eq!(
            find(&rt, "vbar-page").and_then(|e| e.text),
            Some(format!("Page {want} of 4"))
        );
        press_with(&mut rt, Key::Command('+'), ctrl());
        assert_eq!(doc(&mut rt).view.zoom, 110);
        press_with(&mut rt, Key::Command('0'), ctrl());
        assert_eq!(doc(&mut rt).view.zoom, 100, "Ctrl+0 is the actual size");
        press_with(&mut rt, Key::Command('+'), ctrl());
        for _ in 0..20 {
            press_with(&mut rt, Key::Command('-'), ctrl());
        }
        assert_eq!(doc(&mut rt).view.zoom, ZOOM_MIN);
        let mut d = doc(&mut rt);
        for _ in 0..30 {
            d.zoom_in();
        }
        assert_eq!(d.view.zoom, ZOOM_MAX);
    }

    /// "Open in another app…" on a file that is not kept on this PC says to keep it first, and asks nothing else.
    #[test]
    fn office_docs_open_elsewhere_only_when_kept() {
        let mut rt = scene(scene_docx, false);
        let [x, y, w, h] = find(&rt, "elsewhere").expect("the button is drawn").rect;
        rt.pointer_down(x + w / 2.0, y + h / 2.0);
        rt.pointer_up(x + w / 2.0, y + h / 2.0);
        rt.frame(0.0);
        assert_eq!(
            viewers::ui(&mut rt.state).and_then(|ui| ui.toast.clone()),
            Some(KEEP_FIRST.to_string())
        );
        assert!(viewers::ui(&mut rt.state).is_some_and(|ui| ui.take_intents().is_empty()));
    }

    /// The board's words: the status line's Page, words, language, Encrypted, About; the viewer bar's page.
    #[test]
    fn office_docs_words_from_the_board() {
        let rt = scene(scene_docx, false);
        let words = |key: &str| find(&rt, key).unwrap_or_else(|| panic!("no element keyed {key}")).text;
        assert_eq!(words("st-0").as_deref(), Some("Page 1 of 4"));
        assert_eq!(words("st-2").as_deref(), Some("1,284 words"));
        assert_eq!(words("st-4").as_deref(), Some("English (UK)"));
        assert_eq!(words("st-5").as_deref(), Some("Encrypted"));
        assert_eq!(words("st-7").as_deref(), Some("About"));
        assert_eq!(words("zoom").as_deref(), Some("100%"));
        assert_eq!(words("elsewhere-label").as_deref(), Some(OPEN_ELSEWHERE));
        let rt = scene(scene_older, false);
        let text = find(&rt, "st-2").and_then(|e| e.text);
        assert_eq!(text.as_deref(), Some("96 words"));
    }

    /// Amber is the encryption state only: the lock and the word Encrypted in the status line, and the window's own
    /// New button.
    #[test]
    fn office_docs_no_amber_outside_primary() {
        for dark in [false, true] {
            for f in [scene_docx as fn(&mut State), scene_older] {
                let rt = scene(f, dark);
                let t = tokens::theme(dark);
                let amber = [t.accent, t.accent_deep];
                for e in rt.elements() {
                    let key = e.key.clone().unwrap_or_default();
                    let allowed = matches!(key.as_str(), "new-btn" | "plus-icon" | "new-caret" | "new-label")
                        || key.starts_with("st-5")
                        // The Drive sidebar's places: the folder art in its own tint.
                        || crate::ui::sidebar::SidebarSection::all()
                            .iter()
                            .any(|p| key == format!("{}-icon", p.label()));
                    if allowed {
                        continue;
                    }
                    assert!(!amber.contains(&e.fill), "amber fill on {key}");
                    assert!(!e.text_color.is_some_and(|c| amber.contains(&c)), "amber text on {key}");
                }
            }
        }
    }

    /// `DocBody` alone: the clamps (a model test with no window).
    #[test]
    fn office_docs_body_clamps() {
        let mut d = DocBody::new(docs_fixture::view(), "English (UK)");
        d.go_to_page(0);
        assert_eq!(d.page, 1);
        d.go_to_page(99);
        assert_eq!(d.page, 4);
        d.go_to_heading(99);
        assert_eq!(d.page, 4, "a heading that is not there goes nowhere");
        assert!(!d.on_key(&Key::Command('s'), &ctrl()) && !d.on_key(&Key::Command('k'), &ctrl()));
    }

    /// The window as the board draws it: the outline folds with `ol-close` (and its chevron), and "Open in another
    /// app…" is a bordered small button.
    #[test]
    fn office_docs_window_has_the_outline_close_and_a_bordered_open_elsewhere() {
        for f in [scene_docx as fn(&mut State), scene_older] {
            let rt = scene(f, false);
            assert!(find(&rt, "ol-close").is_some(), "the outline's close control is drawn");
            assert!(find(&rt, "ol-close-icon").is_some(), "its chevron is drawn");
            let t = tokens::theme(false);
            let e = find(&rt, "elsewhere").expect("the button is drawn");
            // The element info has no border; the outlined button's paper fill is set with its border, in one place.
            assert_eq!(
                e.fill, t.paper,
                "the button is the outlined kind (paper fill with a border)"
            );
        }
    }

    /// The row of the heading last gone to (`body.at`) has a fill; no other row has one.
    #[test]
    fn office_docs_marks_the_current_heading() {
        for f in [scene_docx as fn(&mut State), scene_older] {
            let mut rt = scene(f, false);
            let d = doc(&mut rt);
            for i in 0..d.view.outline.len() {
                let row = find(&rt, &format!("ol-{i}-row")).expect("the row is drawn");
                let filled = row.fill.a > 0.0;
                assert_eq!(filled, i == d.at, "row {i} (current: {})", d.at);
            }
        }
    }

    /// The page's picture is drawn whole: the first page is keyed `page`, 520 wide, as tall as the canvas's
    /// proportions say (or the board's 700, if taller), and the picture inside it is never cropped; no page is
    /// keyed `pg-`.
    #[test]
    fn office_docs_page_picture_is_never_cropped() {
        for (f, ratio) in [
            (scene_older as fn(&mut State), 1170.0 / 1040.0),
            (scene_docx, 1260.0 / 1040.0),
        ] {
            let rt = scene(f, false);
            let [_, _, w, h] = find(&rt, "page").expect("the first page is keyed page").rect;
            assert!((w - 520.0).abs() < 1.0, "the page is the board's width: {w}");
            assert!(h >= 700.0 - 1.0, "the page is at least the board's height: {h}");
            let [_, _, pw, ph] = find(&rt, "page-picture-1").expect("the picture is drawn").rect;
            assert!(
                (pw - 520.0).abs() < 1.0 && (ph / pw - ratio).abs() * pw < 1.0,
                "{pw} x {ph}, ratio {ratio}"
            );
            assert!(
                rt.elements()
                    .iter()
                    .all(|e| !e.key.clone().unwrap_or_default().starts_with("pg-"))
            );
        }
    }

    /// DW-1564 is the document with one quiet line in its status bar.
    #[test]
    fn office_docs_quiet_scene_draws_the_document_and_one_quiet_line() {
        let rt = scene(super::super::states::scene_quiet, false);
        assert!(
            find(&rt, "page").is_some() && find(&rt, "ol").is_some(),
            "the document is drawn"
        );
        let quiet = rt
            .elements()
            .iter()
            .filter(|e| e.text.as_deref() == Some("Some content isn't shown."))
            .count();
        assert_eq!(quiet, 1, "the quiet line is in the status bar once");
        let bar = find(&rt, "status-bar").and_then(|e| e.label).unwrap_or_default();
        assert!(bar.contains("Some content isn't shown."));
    }
}

#[cfg(test)]
#[path = "docs_live_tests.rs"]
pub(super) mod live_tests;
