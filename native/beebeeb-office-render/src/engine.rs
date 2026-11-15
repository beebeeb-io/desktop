//! The Office engine behind two small traits, so the loop in [`crate::serve`] can be tested with a fake.
//!
//! [`Office`] is the real one: Word documents through `beebeeb_office::Document`, presentations through `Deck`,
//! workbooks through `Workbook`. Today `Deck::open` and `Workbook::open` answer `Unsupported` for every file; the
//! paths below exist so the sheets and slides slices fill in the Office engine without touching this crate.
//!
//! Every error is a code from the protocol's fixed list ([`open_code`], [`render_code`]); the engine's own words
//! never travel.

use beebeeb_office as eng;
use beebeeb_office_protocol::check;
use beebeeb_office_protocol::office::{
    Align, CellData, CellRange, CellText, DocKind, ErrorCode, Frozen, Opened, OutlineEntry, PageSize, Rect, SheetInfo,
};

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
    /// The cells of `range` on `sheet` (zero-based), with every merge that touches the range.
    fn cells(&self, sheet: u32, range: CellRange) -> Result<CellData, ErrorCode>;
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
            DocKind::Pptx => {
                let deck = eng::Deck::open(bytes, &limits).map_err(open_code)?;
                let sizes = deck.info().sizes_pt;
                Box::new(Slides { deck, sizes })
            }
            DocKind::Xlsx => sheets(eng::Workbook::open(eng::SheetKind::Xlsx, bytes, &limits).map_err(open_code)?),
            DocKind::Xls => sheets(eng::Workbook::open(eng::SheetKind::Xls, bytes, &limits).map_err(open_code)?),
        };
        Ok(opened)
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
}

/// A presentation. Its slide sizes are read once, at open.
struct Slides {
    deck: eng::Deck,
    sizes: Vec<(f32, f32)>,
}

impl OpenDoc for Slides {
    fn opened(&self) -> Opening {
        Opening::Pages(Opened {
            pages: u32::try_from(self.sizes.len()).unwrap_or(u32::MAX),
            sizes: page_sizes(&self.sizes),
        })
    }

    fn render_size(&self, page: u32, scale: f32, region: Option<Rect>) -> Result<(u32, u32), ErrorCode> {
        if region.is_some() {
            return Err(ErrorCode::Unsupported);
        }
        let &(w_pt, h_pt) = self.sizes.get(page as usize).ok_or(ErrorCode::Internal)?;
        // The deck has no size call of its own; `page_pixels` is the engine's rule, pinned to it by the shared
        // page-pixel vector, and the slide renderer refuses any other buffer size.
        let (w, h) = check::page_pixels(&PageSize { w_pt, h_pt }, scale).ok_or(ErrorCode::Internal)?;
        if w > eng::limits::MAX_RENDER_SIDE_PX || h > eng::limits::MAX_RENDER_SIDE_PX {
            return Err(ErrorCode::TooLarge);
        }
        Ok((w, h))
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

    fn outline(&self) -> Vec<OutlineEntry> {
        Vec::new()
    }

    fn words(&self) -> u32 {
        0
    }

    fn cells(&self, _sheet: u32, _range: CellRange) -> Result<CellData, ErrorCode> {
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
        let block = self.book.cells(sheet, asked);
        Ok(CellData {
            cells: block
                .cells
                .into_iter()
                .map(|c| CellText {
                    row: c.row,
                    col: c.col,
                    text: c.text,
                    align: align(c.align),
                    bold: c.bold,
                })
                .collect(),
            col_widths: block.col_widths,
            row_heights: block.row_heights,
            // Clipped to the asked range by `serve`, for every engine.
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
        })
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
}
