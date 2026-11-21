//! The Office engine behind two small traits, so the loop in [`crate::serve`] can be tested with a fake.
//!
//! [`Office`] is the real one: Word documents through `beebeeb_office::Document`, presentations through `Deck`,
//! workbooks through `Workbook`. Every size comes from the engine's own call (`Document::render_size`,
//! `Deck::render_size`, `Workbook::anchor_size`), never recomputed here.
//!
//! Every error is a code from the protocol's fixed list ([`open_code`], [`render_code`]); the engine's own words
//! never travel.

use beebeeb_office as eng;
use beebeeb_office_protocol::office::{
    Align, Anchor, AnchorKind, CellData, CellRange, CellText, Clip, DocKind, EditKind, EditOp, Edited, ErrorCode,
    Frozen, HitMode, Opened, OutlineEntry, PageSize, Rect, SheetInfo, Template,
};

use crate::edit_map;

// The PDF engine (2501), beside the Office ones: `src/pdf.rs`.
#[path = "pdf.rs"]
mod pdf;

/// What an opened file is: pages (a document, a deck) or sheets (a workbook).
#[derive(Debug, Clone, PartialEq)]
pub enum Opening {
    /// The page or slide count and every page's size, in points.
    Pages(Opened),
    /// Every sheet, in tab order, with its whole name. [`crate::serve`] cuts the names and fits them in one frame.
    Sheets(Vec<SheetInfo>),
}

/// Opens a file from its bytes.
pub trait Engine {
    /// Opens `bytes` as `kind`. The bytes are only borrowed: the open file keeps what it needs.
    fn open(&self, kind: DocKind, bytes: &[u8]) -> Result<Box<dyn OpenDoc>, ErrorCode>;

    /// Opens `bytes` as `kind` for editing. The bytes are only borrowed. An engine that does not edit says
    /// `Unsupported` (the default).
    fn open_edit(&self, kind: EditKind, bytes: &[u8]) -> Result<Box<dyn EditingDoc>, ErrorCode> {
        let _ = (kind, bytes);
        Err(ErrorCode::Unsupported)
    }

    /// A new document from a built-in template, open for editing.
    fn new_doc(&self, kind: EditKind, template: Template) -> Result<Box<dyn EditingDoc>, ErrorCode> {
        let _ = (kind, template);
        Err(ErrorCode::Unsupported)
    }
}

/// A document open for editing: everything a viewer session answers ([`OpenDoc`], on the current state of the edit),
/// plus the edit calls. Every error is a code from the protocol's fixed list; no engine words travel.
pub trait EditingDoc: OpenDoc {
    /// Applies one edit.
    fn apply(&mut self, op: EditOp) -> Result<Edited, ErrorCode>;
    /// Inserts the picture `bytes` (PNG when `png`, else JPEG), which the helper copied out of the host's section.
    fn insert_image(&mut self, bytes: Vec<u8>, png: bool) -> Result<Edited, ErrorCode>;
    /// A click or drag at a point on a page (slide), in points.
    fn hit(&mut self, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> Result<Edited, ErrorCode>;
    /// Serialises the document: its bytes, and the revision they are of.
    fn save(&mut self) -> Result<(Vec<u8>, u64), ErrorCode>;
    /// The selection as text and, where there is one, html.
    fn copy(&self) -> Clip;
    /// The slide layouts' names (empty for anything else).
    fn layouts(&self) -> Vec<String>;
}

/// An opened file. Lives on the helper's worker thread only.
pub trait OpenDoc {
    /// What was opened.
    fn opened(&self) -> Opening;
    /// The pixel size of `page` at `scale` (device pixels per point), the whole page or `region`: the engine's own
    /// rule, so the size the app sized the buffer with.
    fn render_size(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(u32, u32), ErrorCode>;
    /// Renders into `out` (RGBA8, exactly the size [`OpenDoc::render_size`] gives). `Ok(true)`: something on the page
    /// could not be drawn (missing content).
    fn render(&self, page: u32, scale: f32, region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode>;
    /// The headings, in reading order.
    fn outline(&self) -> Vec<OutlineEntry>;
    /// The word count.
    fn words(&self) -> u32;
    /// The cells of `range` on `sheet` (zero-based), with every merge and every chart or picture that touches the
    /// range.
    fn cells(&self, sheet: u32, range: CellRange) -> Result<CellData, ErrorCode>;
    /// The pixel size of chart or picture `anchor` (its id) of `sheet` at `scale`: the engine's own rule, so the size
    /// the app sized the buffer with. Documents and decks have none: `Unsupported`.
    fn anchor_size(&self, sheet: u32, anchor: u32, scale: f32) -> Result<(u32, u32), ErrorCode>;
    /// Renders it into `out` (RGBA8, exactly the size [`OpenDoc::anchor_size`] gives). `Ok(true)`: the picture could
    /// not be drawn as the file means it (missing content).
    fn render_anchor(&self, sheet: u32, anchor: u32, scale: f32, out: &mut [u8]) -> Result<bool, ErrorCode>;
}

/// An open error as a protocol code. Every variant is listed, so a new one is a compile error here.
pub fn open_code(e: eng::OpenError) -> ErrorCode {
    match e {
        eng::OpenError::Damaged => ErrorCode::Damaged,
        eng::OpenError::TooLarge(_) => ErrorCode::TooLarge,
        eng::OpenError::Unsupported => ErrorCode::Unsupported,
        eng::OpenError::Password => ErrorCode::Password,
    }
}

/// A render error as a protocol code. Every variant is listed, so a new one is a compile error here. Only `TooLarge`
/// says something the app can act on; the rest are the helper failing.
pub fn render_code(e: eng::RenderError) -> ErrorCode {
    match e {
        eng::RenderError::NoSuchPage => ErrorCode::Internal,
        eng::RenderError::BadScale => ErrorCode::Internal,
        eng::RenderError::TooLarge => ErrorCode::TooLarge,
        eng::RenderError::BadRegion => ErrorCode::Internal,
        eng::RenderError::BufferSize => ErrorCode::Internal,
        eng::RenderError::Engine => ErrorCode::Internal,
    }
}

/// The Office engine.
pub struct Office;

impl Engine for Office {
    fn open(&self, kind: DocKind, bytes: &[u8]) -> Result<Box<dyn OpenDoc>, ErrorCode> {
        let limits = eng::Limits::default();
        let opened: Box<dyn OpenDoc> = match kind {
            DocKind::Docx => Box::new(Words(
                eng::Document::open(eng::DocKind::Docx, bytes, &limits).map_err(open_code)?,
            )),
            DocKind::Doc => Box::new(Words(
                eng::Document::open(eng::DocKind::Doc, bytes, &limits).map_err(open_code)?,
            )),
            DocKind::Pptx => Box::new(Slides::new(eng::Deck::open(bytes, &limits).map_err(open_code)?)),
            DocKind::Xlsx => sheets(eng::Workbook::open(eng::SheetKind::Xlsx, bytes, &limits).map_err(open_code)?),
            DocKind::Xls => sheets(eng::Workbook::open(eng::SheetKind::Xls, bytes, &limits).map_err(open_code)?),
            DocKind::Pdf => Box::new(pdf::open(bytes)?),
        };
        Ok(opened)
    }

    fn open_edit(&self, kind: EditKind, bytes: &[u8]) -> Result<Box<dyn EditingDoc>, ErrorCode> {
        let doc = eng::edit::EditDoc::open(edit_map::kind(kind), bytes, &eng::Limits::default()).map_err(open_code)?;
        Ok(Box::new(Editing::new(doc, Some(bytes))?))
    }

    fn new_doc(&self, kind: EditKind, template: Template) -> Result<Box<dyn EditingDoc>, ErrorCode> {
        let doc = eng::edit::EditDoc::new_from(edit_map::kind(kind), edit_map::template(template))
            .map_err(edit_map::edit_code)?;
        Ok(Box::new(Editing::new(doc, None)?))
    }
}

fn page_sizes(sizes_pt: &[(f32, f32)]) -> Vec<PageSize> {
    sizes_pt.iter().map(|&(w_pt, h_pt)| PageSize { w_pt, h_pt }).collect()
}

fn engine_rect(r: Rect) -> eng::Rect {
    eng::Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

/// A Word document.
struct Words(eng::Document);

impl OpenDoc for Words {
    fn opened(&self) -> Opening {
        let info = self.0.info();
        Opening::Pages(Opened {
            pages: info.pages,
            sizes: page_sizes(&info.sizes_pt),
        })
    }

    fn render_size(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        // Only the engine sizes a page: never recomputed here.
        self.0
            .render_size(page, scale, region.map(engine_rect))
            .map_err(render_code)
    }

    fn render(&self, page: u32, scale: f32, region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode> {
        self.0
            .render_page(page, scale, region.map(engine_rect), out)
            .map(|drawn| drawn.missing_content)
            .map_err(render_code)
    }

    fn outline(&self) -> Vec<OutlineEntry> {
        self.0
            .outline()
            .into_iter()
            .map(|e| OutlineEntry {
                level: e.level,
                text: e.text,
                page: e.page,
            })
            .collect()
    }

    fn words(&self) -> u32 {
        self.0.word_count()
    }

    fn cells(&self, _sheet: u32, _range: CellRange) -> Result<CellData, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn anchor_size(&self, _sheet: u32, _anchor: u32, _scale: f32) -> Result<(u32, u32), ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn render_anchor(&self, _sheet: u32, _anchor: u32, _scale: f32, _out: &mut [u8]) -> Result<bool, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }
}

/// What [`Slides`] needs of a deck: the engine's `Deck`, or a test's stand-in (no `Deck` can be made without the
/// slides reader, so the sizing is tested through this).
pub(crate) trait SlideSource {
    fn render_size(&self, n: u32, scale: f32) -> Result<(u32, u32), eng::RenderError>;
    fn render_slide(&self, n: u32, scale: f32, out: &mut [u8]) -> Result<eng::RenderInfo, eng::RenderError>;
}

impl SlideSource for eng::Deck {
    fn render_size(&self, n: u32, scale: f32) -> Result<(u32, u32), eng::RenderError> {
        eng::Deck::render_size(self, n, scale)
    }

    fn render_slide(&self, n: u32, scale: f32, out: &mut [u8]) -> Result<eng::RenderInfo, eng::RenderError> {
        eng::Deck::render_slide(self, n, scale, out)
    }
}

/// A presentation. Its slide sizes and titles are read once, at open.
struct Slides<D> {
    deck: D,
    sizes: Vec<(f32, f32)>,
    titles: Vec<String>,
}

impl Slides<eng::Deck> {
    fn new(deck: eng::Deck) -> Slides<eng::Deck> {
        let info = deck.info();
        Slides {
            deck,
            sizes: info.sizes_pt,
            titles: info.titles,
        }
    }
}

impl<D: SlideSource> OpenDoc for Slides<D> {
    fn opened(&self) -> Opening {
        Opening::Pages(Opened {
            pages: u32::try_from(self.sizes.len()).unwrap_or(u32::MAX),
            sizes: page_sizes(&self.sizes),
        })
    }

    fn render_size(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        // A slide is drawn whole: the app asks no tiles of it.
        if region.is_some() {
            return Err(ErrorCode::Unsupported);
        }
        // Only the engine sizes a slide: never recomputed here.
        self.deck.render_size(page, scale).map_err(render_code)
    }

    fn render(&self, page: u32, scale: f32, region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode> {
        if region.is_some() {
            return Err(ErrorCode::Unsupported);
        }
        self.deck
            .render_slide(page, scale, out)
            .map(|drawn| drawn.missing_content)
            .map_err(render_code)
    }

    /// One heading per slide that has a title, at level 1, on its slide.
    fn outline(&self) -> Vec<OutlineEntry> {
        self.titles
            .iter()
            .enumerate()
            .filter(|(_, title)| !title.is_empty())
            .map(|(n, title)| OutlineEntry {
                level: 1,
                text: title.clone(),
                // At most the engine's 2,000 slides.
                page: u32::try_from(n).unwrap_or(u32::MAX),
            })
            .collect()
    }

    fn words(&self) -> u32 {
        0
    }

    fn cells(&self, _sheet: u32, _range: CellRange) -> Result<CellData, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn anchor_size(&self, _sheet: u32, _anchor: u32, _scale: f32) -> Result<(u32, u32), ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn render_anchor(&self, _sheet: u32, _anchor: u32, _scale: f32, _out: &mut [u8]) -> Result<bool, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }
}

/// A workbook. Its sheet count is read once, at open.
struct Sheets {
    book: eng::Workbook,
    count: usize,
}

fn sheets(book: eng::Workbook) -> Box<dyn OpenDoc> {
    let count = book.info().sheets.len();
    Box::new(Sheets { book, count })
}

fn align(a: eng::CellAlign) -> Align {
    match a {
        eng::CellAlign::Left => Align::Left,
        eng::CellAlign::Center => Align::Center,
        eng::CellAlign::Right => Align::Right,
    }
}

impl OpenDoc for Sheets {
    fn opened(&self) -> Opening {
        Opening::Sheets(
            self.book
                .info()
                .sheets
                .into_iter()
                .map(|s| SheetInfo {
                    name: s.name,
                    rows: s.rows,
                    cols: s.cols,
                })
                .collect(),
        )
    }

    fn render_size(&self, _page: u32, _scale: f32, _region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn render(&self, _page: u32, _scale: f32, _region: Option<Rect>, _out: &mut [u8]) -> Result<bool, ErrorCode> {
        Err(ErrorCode::Unsupported)
    }

    fn outline(&self) -> Vec<OutlineEntry> {
        Vec::new()
    }

    fn words(&self) -> u32 {
        0
    }

    fn cells(&self, sheet: u32, range: CellRange) -> Result<CellData, ErrorCode> {
        if sheet as usize >= self.count {
            return Err(ErrorCode::Internal);
        }
        let asked = eng::CellRange {
            row: range.row,
            col: range.col,
            rows: range.rows,
            cols: range.cols,
        };
        Ok(block_cells(self.book.cells(sheet, asked)))
    }

    fn anchor_size(&self, sheet: u32, anchor: u32, scale: f32) -> Result<(u32, u32), ErrorCode> {
        // Only the engine sizes a chart or picture: never recomputed here.
        self.book.anchor_size(sheet, anchor, scale).map_err(render_code)
    }

    fn render_anchor(&self, sheet: u32, anchor: u32, scale: f32, out: &mut [u8]) -> Result<bool, ErrorCode> {
        self.book
            .render_anchor(sheet, anchor, scale, out)
            .map(|drawn| drawn.missing_content)
            .map_err(render_code)
    }
}

/// A document open for editing, through the engine's `EditDoc`. Its viewer calls are the engine's own, with the viewer's
/// contracts, so a page is sized, rendered and outlined the same way in both sessions.
struct Editing {
    doc: eng::edit::EditDoc,
    /// What the file is, as the first answer says it: kept current when an edit changes the pages.
    opening: Opening,
}

impl Editing {
    /// `original`: the bytes a file was opened from. A workbook's sheet list comes from the viewer's reader over them
    /// (the edit API has no sheet list), or, for a new workbook, over its own serialised bytes.
    fn new(doc: eng::edit::EditDoc, original: Option<&[u8]>) -> Result<Editing, ErrorCode> {
        let opening = match doc.kind() {
            eng::edit::EditKind::Docx | eng::edit::EditKind::Pptx => {
                let info = doc.info().map_err(edit_map::edit_code)?;
                Opening::Pages(Opened {
                    pages: info.pages,
                    sizes: page_sizes(&info.sizes_pt),
                })
            }
            eng::edit::EditKind::Xlsx => {
                let saved;
                let bytes = match original {
                    Some(bytes) => bytes,
                    None => {
                        saved = doc.save().map_err(edit_map::save_code)?;
                        &saved
                    }
                };
                let book =
                    eng::Workbook::open(eng::SheetKind::Xlsx, bytes, &eng::Limits::default()).map_err(open_code)?;
                Opening::Sheets(
                    book.info()
                        .sheets
                        .into_iter()
                        .map(|s| SheetInfo {
                            name: s.name,
                            rows: s.rows,
                            cols: s.cols,
                        })
                        .collect(),
                )
            }
        };
        Ok(Editing { doc, opening })
    }

    /// An edit's change as the protocol's `Edited`, with the page list kept current.
    fn edited(&mut self, change: eng::edit::Change) -> Edited {
        let edited = edit_map::change(change);
        if let (Some(pages), Opening::Pages(held)) = (&edited.pages, &mut self.opening) {
            *held = pages.clone();
        }
        edited
    }
}

impl OpenDoc for Editing {
    fn opened(&self) -> Opening {
        self.opening.clone()
    }

    fn render_size(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        self.doc
            .render_size(page, scale, region.map(engine_rect))
            .map_err(render_code)
    }

    fn render(&self, page: u32, scale: f32, region: Option<Rect>, out: &mut [u8]) -> Result<bool, ErrorCode> {
        self.doc
            .render_page(page, scale, region.map(engine_rect), out)
            .map(|drawn| drawn.missing_content)
            .map_err(render_code)
    }

    fn outline(&self) -> Vec<OutlineEntry> {
        self.doc
            .outline()
            .into_iter()
            .map(|e| OutlineEntry {
                level: e.level,
                text: e.text,
                page: e.page,
            })
            .collect()
    }

    fn words(&self) -> u32 {
        self.doc.words()
    }

    fn cells(&self, sheet: u32, range: CellRange) -> Result<CellData, ErrorCode> {
        let asked = eng::CellRange {
            row: range.row,
            col: range.col,
            rows: range.rows,
            cols: range.cols,
        };
        self.doc
            .cells(sheet, asked)
            .map(block_cells)
            .map_err(edit_map::edit_code)
    }

    fn anchor_size(&self, sheet: u32, anchor: u32, scale: f32) -> Result<(u32, u32), ErrorCode> {
        self.doc.anchor_size(sheet, anchor, scale).map_err(render_code)
    }

    fn render_anchor(&self, sheet: u32, anchor: u32, scale: f32, out: &mut [u8]) -> Result<bool, ErrorCode> {
        self.doc
            .render_anchor(sheet, anchor, scale, out)
            .map(|drawn| drawn.missing_content)
            .map_err(render_code)
    }
}

impl EditingDoc for Editing {
    fn apply(&mut self, op: EditOp) -> Result<Edited, ErrorCode> {
        let change = self.doc.apply(&edit_map::op(op)).map_err(edit_map::edit_code)?;
        Ok(self.edited(change))
    }

    fn insert_image(&mut self, bytes: Vec<u8>, png: bool) -> Result<Edited, ErrorCode> {
        let change = self
            .doc
            .apply(&edit_map::image(bytes, png))
            .map_err(edit_map::edit_code)?;
        Ok(self.edited(change))
    }

    fn hit(&mut self, page: u32, x_pt: f32, y_pt: f32, mode: HitMode) -> Result<Edited, ErrorCode> {
        let change = self
            .doc
            .hit(page, x_pt, y_pt, edit_map::hit_mode(mode))
            .map_err(edit_map::edit_code)?;
        Ok(self.edited(change))
    }

    fn save(&mut self) -> Result<(Vec<u8>, u64), ErrorCode> {
        let revision = self.doc.revision();
        let bytes = self.doc.save().map_err(edit_map::save_code)?;
        Ok((bytes, revision))
    }

    fn copy(&self) -> Clip {
        let clip = self.doc.copy();
        Clip {
            text: clip.text,
            html: clip.html,
        }
    }

    fn layouts(&self) -> Vec<String> {
        self.doc.layouts()
    }
}

/// The engine's block of cells as the protocol carries it, field for field: each cell's text, value and formula; the
/// merges (clipped to the asked range by `serve`, for every engine); the charts and pictures, their `from`/`to`
/// cells split into four numbers; `missing_content`. Texts, formulas and anchors are capped by `serve` too.
pub fn block_cells(block: eng::CellBlock) -> CellData {
    CellData {
        cells: block
            .cells
            .into_iter()
            .map(|c| CellText {
                row: c.row,
                col: c.col,
                text: c.text,
                align: align(c.align),
                bold: c.bold,
                value: c.value,
                formula: c.formula,
            })
            .collect(),
        col_widths: block.col_widths,
        row_heights: block.row_heights,
        merges: block
            .merges
            .into_iter()
            .map(|m| CellRange {
                row: m.row,
                col: m.col,
                rows: m.rows,
                cols: m.cols,
            })
            .collect(),
        frozen: Frozen {
            rows: block.frozen.0,
            cols: block.frozen.1,
        },
        anchors: block.anchors.into_iter().map(anchor_of).collect(),
        missing_content: block.missing_content,
    }
}

fn anchor_of(a: eng::Anchor) -> Anchor {
    Anchor {
        id: a.id,
        kind: match a.kind {
            eng::AnchorKind::Chart => AnchorKind::Chart,
            eng::AnchorKind::Picture => AnchorKind::Picture,
        },
        from_row: a.from.0,
        from_col: a.from.1,
        to_row: a.to.0,
        to_col: a.to.1,
        x_pt: a.x_pt,
        y_pt: a.y_pt,
        w_pt: a.w_pt,
        h_pt: a.h_pt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_errors_map_to_the_fixed_codes() {
        assert_eq!(render_code(eng::RenderError::NoSuchPage), ErrorCode::Internal);
        assert_eq!(render_code(eng::RenderError::BadScale), ErrorCode::Internal);
        assert_eq!(render_code(eng::RenderError::TooLarge), ErrorCode::TooLarge);
        assert_eq!(render_code(eng::RenderError::BadRegion), ErrorCode::Internal);
        assert_eq!(render_code(eng::RenderError::BufferSize), ErrorCode::Internal);
        assert_eq!(render_code(eng::RenderError::Engine), ErrorCode::Internal);
    }

    #[test]
    fn open_errors_map_to_the_fixed_codes() {
        assert_eq!(open_code(eng::OpenError::Damaged), ErrorCode::Damaged);
        assert_eq!(
            open_code(eng::OpenError::TooLarge(eng::limits::PAGES)),
            ErrorCode::TooLarge
        );
        assert_eq!(
            open_code(eng::OpenError::TooLarge(eng::limits::READER)),
            ErrorCode::TooLarge
        );
        assert_eq!(open_code(eng::OpenError::Unsupported), ErrorCode::Unsupported);
        assert_eq!(open_code(eng::OpenError::Password), ErrorCode::Password);
    }

    /// A deck that sizes its slides by its own rule: deliberately not the page rule, so a size taken from anywhere
    /// but the deck shows. Counts its renders.
    struct OwnRule {
        renders: std::cell::Cell<usize>,
    }

    /// What `OwnRule` sizes every slide at, whatever its points: no scale gives this from 960 x 540 pt.
    const OWN_SIZE: (u32, u32) = (7, 3);

    impl SlideSource for OwnRule {
        fn render_size(&self, n: u32, scale: f32) -> Result<(u32, u32), eng::RenderError> {
            if n >= 2 {
                return Err(eng::RenderError::NoSuchPage);
            }
            if !(scale.is_finite() && scale > 0.0) {
                return Err(eng::RenderError::BadScale);
            }
            Ok(OWN_SIZE)
        }

        fn render_slide(&self, n: u32, scale: f32, out: &mut [u8]) -> Result<eng::RenderInfo, eng::RenderError> {
            let (w, h) = self.render_size(n, scale)?;
            if out.len() != w as usize * h as usize * 4 {
                return Err(eng::RenderError::BufferSize);
            }
            self.renders.set(self.renders.get() + 1);
            out.fill(0x7f);
            Ok(eng::RenderInfo {
                w,
                h,
                missing_content: true,
            })
        }
    }

    fn two_slides() -> Slides<OwnRule> {
        Slides {
            deck: OwnRule {
                renders: std::cell::Cell::new(0),
            },
            sizes: vec![(960.0, 540.0), (960.0, 540.0)],
            titles: vec!["Q3 plan".to_owned(), String::new()],
        }
    }

    /// Helper decision 6 ends: a slide's pixel size is the deck's own call, never the protocol's copy of the rule.
    #[test]
    fn slides_are_sized_by_the_engine() {
        let slides = two_slides();
        let page = PageSize {
            w_pt: 960.0,
            h_pt: 540.0,
        };
        let by_rule = beebeeb_office_protocol::check::page_pixels(&page, 1.0);
        assert_eq!(
            by_rule,
            Some((960, 540)),
            "the premise: the page rule gives another size"
        );
        assert_eq!(slides.render_size(0, 1.0, None), Ok(OWN_SIZE));
        assert_eq!(slides.render_size(1, 4.0 / 3.0, None), Ok(OWN_SIZE));
        // The deck's refusals, as their codes.
        assert_eq!(slides.render_size(2, 1.0, None), Err(ErrorCode::Internal));
        assert_eq!(slides.render_size(0, f32::NAN, None), Err(ErrorCode::Internal));
        // A slide is drawn whole.
        let tile = Some(Rect { x: 0, y: 0, w: 1, h: 1 });
        assert_eq!(slides.render_size(0, 1.0, tile), Err(ErrorCode::Unsupported));
        // It renders into a buffer of the deck's size, and says what the deck said.
        let mut out = vec![0u8; OWN_SIZE.0 as usize * OWN_SIZE.1 as usize * 4];
        assert_eq!(slides.render(0, 1.0, None, &mut out), Ok(true));
        assert_eq!(slides.deck.renders.get(), 1);
        assert!(out.iter().all(|b| *b == 0x7f));
        // A deck has no charts or pictures of its own to ask for.
        assert_eq!(slides.anchor_size(0, 0, 1.0), Err(ErrorCode::Unsupported));
        assert_eq!(slides.render_anchor(0, 0, 1.0, &mut out), Err(ErrorCode::Unsupported));
    }

    #[test]
    fn slide_titles_are_the_outline() {
        let slides = two_slides();
        assert_eq!(
            slides.outline(),
            vec![OutlineEntry {
                level: 1,
                text: "Q3 plan".to_owned(),
                page: 0,
            }]
        );
        match slides.opened() {
            Opening::Pages(opened) => assert_eq!(opened.pages, 2),
            other => panic!("pages were due: {other:?}"),
        }
    }
}
