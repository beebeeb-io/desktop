//! The Office messages: what the app asks the helper and what the helper answers, each with a fixed little-endian
//! layout and length-prefixed UTF-8 text. No serde: every byte is read by hand and checked.
//!
//! **Units.** Page sizes are in points (1/72 inch). A `Render` asks for a page at a `scale`, where 1.0 is 100 % zoom
//! on a 96 DPI monitor (the viewer contract §7: scale = zoom x the monitor's DPI factor), so a page `w_pt` wide is
//! `round(w_pt * scale * 96 / 72)` pixels wide ([`crate::check::page_pixels`]). A `region` is a rectangle in those
//! pixels (tiles of 512 px at large zooms) and lies inside the page. Pixels are RGBA8, `w * h * 4` bytes, in the
//! section `out`.
//!
//! **Ids** are in the frame, not here: an answer (`Opened`, `Rendered`, `Error`, …) carries the id of its request in
//! the frame header, and `Cancel` carries the id of the request it cancels.
//!
//! Encoding a message larger than a frame (an outline or cell block over 64 KiB) succeeds here; `write_frame` then
//! refuses it, so the helper must keep its answers under the cap.

use std::fmt;

use crate::wire::{In, Out};

/// The app asks the helper to open the document in a granted section.
pub const KIND_OPEN: u16 = 0x0101;
/// The app asks for a page (or a region of it) as pixels.
pub const KIND_RENDER: u16 = 0x0102;
/// The app asks for the cells of a range of one sheet.
pub const KIND_CELLS: u16 = 0x0103;
/// The app asks for the outline (headings) of a document.
pub const KIND_OUTLINE: u16 = 0x0104;
/// The app asks for the document's counts.
pub const KIND_STATS: u16 = 0x0105;
/// The app cancels the request whose id the frame carries.
pub const KIND_CANCEL: u16 = 0x0106;
/// The helper opened a document or a deck: its pages and their sizes.
pub const KIND_OPENED: u16 = 0x0201;
/// The helper opened a workbook: its sheets.
pub const KIND_OPENED_SHEETS: u16 = 0x0202;
/// The helper wrote the asked pixels into the section.
pub const KIND_RENDERED: u16 = 0x0203;
/// The helper's cells for an asked range.
pub const KIND_CELL_DATA: u16 = 0x0204;
/// The helper's outline.
pub const KIND_OUTLINE_DATA: u16 = 0x0205;
/// The helper's counts.
pub const KIND_STATS_DATA: u16 = 0x0206;
/// The helper could not do what was asked: a code from a fixed list.
pub const KIND_ERROR: u16 = 0x02FF;

/// Why a payload was refused. Fixed reasons; nothing from the payload is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The payload ended inside a fixed field.
    Truncated,
    /// A declared vector or text length is larger than the bytes that remain.
    LengthPastEnd,
    /// The frame's kind is not an Office message.
    UnknownKind,
    /// An error code outside [`ErrorCode`]'s list.
    BadCode,
    /// A tag, a flag bit or a bool outside its list.
    BadTag,
    /// Text that is not UTF-8.
    BadUtf8,
    /// A NaN or an infinity.
    NotFinite,
    /// Bytes left over after the message.
    TrailingBytes,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            DecodeError::Truncated => "the message ended early",
            DecodeError::LengthPastEnd => "a length past the end of the message",
            DecodeError::UnknownKind => "an unknown message kind",
            DecodeError::BadCode => "an error code off the list",
            DecodeError::BadTag => "a tag off its list",
            DecodeError::BadUtf8 => "text that is not UTF-8",
            DecodeError::NotFinite => "a number that is not finite",
            DecodeError::TrailingBytes => "bytes after the message",
        };
        f.write_str(text)
    }
}

impl std::error::Error for DecodeError {}

/// The Office formats the helper may be asked to open. `.ppt` is not on the list: it has no preview (2482 Q5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Docx,
    Doc,
    Xlsx,
    Xls,
    Pptx,
}

impl DocKind {
    fn tag(self) -> u8 {
        match self {
            DocKind::Docx => 1,
            DocKind::Doc => 2,
            DocKind::Xlsx => 3,
            DocKind::Xls => 4,
            DocKind::Pptx => 5,
        }
    }

    fn from_tag(tag: u8) -> Result<DocKind, DecodeError> {
        match tag {
            1 => Ok(DocKind::Docx),
            2 => Ok(DocKind::Doc),
            3 => Ok(DocKind::Xlsx),
            4 => Ok(DocKind::Xls),
            5 => Ok(DocKind::Pptx),
            _ => Err(DecodeError::BadTag),
        }
    }
}

/// A shared section handed to the helper: the handle value as the helper sees it, and the section's length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantRef {
    pub wire: u64,
    pub len: u64,
}

impl GrantRef {
    fn put(&self, o: &mut Out) {
        o.u64(self.wire);
        o.u64(self.len);
    }

    fn get(i: &mut In<'_>) -> Result<GrantRef, DecodeError> {
        Ok(GrantRef {
            wire: i.u64()?,
            len: i.u64()?,
        })
    }
}

/// Open the document whose bytes are in the read-only section `doc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Open {
    pub kind: DocKind,
    pub doc: GrantRef,
}

impl Open {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.u8(self.kind.tag());
        self.doc.put(&mut o);
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<Open, DecodeError> {
        let mut i = In::new(payload);
        let kind = DocKind::from_tag(i.u8()?)?;
        let doc = GrantRef::get(&mut i)?;
        i.finish()?;
        Ok(Open { kind, doc })
    }
}

/// One page's size in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub w_pt: f32,
    pub h_pt: f32,
}

/// A document or a deck opened: its page (or slide) count and each page's size.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub pages: u32,
    pub sizes: Vec<PageSize>,
}

impl Opened {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.u32(self.pages);
        let n = o.count(self.sizes.len());
        for size in self.sizes.iter().take(n) {
            o.f32(size.w_pt);
            o.f32(size.h_pt);
        }
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<Opened, DecodeError> {
        let mut i = In::new(payload);
        let pages = i.u32()?;
        let n = i.count(8)?;
        let mut sizes = Vec::with_capacity(n);
        for _ in 0..n {
            sizes.push(PageSize {
                w_pt: i.f32()?,
                h_pt: i.f32()?,
            });
        }
        i.finish()?;
        Ok(Opened { pages, sizes })
    }
}

/// One sheet of a workbook. The name is the helper's text: shown as text, never used as anything else.
#[derive(Clone, PartialEq, Eq)]
pub struct SheetInfo {
    pub name: String,
    pub rows: u32,
    pub cols: u32,
}

/// The name is document content, so `Debug` shows its length only.
impl fmt::Debug for SheetInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetInfo")
            .field("name_len", &self.name.len())
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .finish()
    }
}

/// A workbook opened: its sheets in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedSheets {
    pub sheets: Vec<SheetInfo>,
}

impl OpenedSheets {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        let n = o.count(self.sheets.len());
        for sheet in self.sheets.iter().take(n) {
            o.str(&sheet.name);
            o.u32(sheet.rows);
            o.u32(sheet.cols);
        }
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<OpenedSheets, DecodeError> {
        let mut i = In::new(payload);
        // Each sheet: a 4-byte name length (its text may be empty), rows, cols.
        let n = i.count(12)?;
        let mut sheets = Vec::with_capacity(n);
        for _ in 0..n {
            let name = i.str()?;
            sheets.push(SheetInfo {
                name,
                rows: i.u32()?,
                cols: i.u32()?,
            });
        }
        i.finish()?;
        Ok(OpenedSheets { sheets })
    }
}

/// A rectangle in pixels of the page rendered at the request's scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Render page `page` at `scale` (the whole page, or only `region`) as RGBA8 into the writable section `out`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Render {
    pub page: u32,
    pub scale: f32,
    pub region: Option<Rect>,
    pub out: GrantRef,
}

impl Render {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.u32(self.page);
        o.f32(self.scale);
        match self.region {
            None => o.u8(0),
            Some(r) => {
                o.u8(1);
                o.u32(r.x);
                o.u32(r.y);
                o.u32(r.w);
                o.u32(r.h);
            }
        }
        self.out.put(&mut o);
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<Render, DecodeError> {
        let mut i = In::new(payload);
        let page = i.u32()?;
        let scale = i.f32()?;
        let region = match i.u8()? {
            0 => None,
            1 => Some(Rect {
                x: i.u32()?,
                y: i.u32()?,
                w: i.u32()?,
                h: i.u32()?,
            }),
            _ => return Err(DecodeError::BadTag),
        };
        let out = GrantRef::get(&mut i)?;
        i.finish()?;
        Ok(Render {
            page,
            scale,
            region,
            out,
        })
    }
}

/// The pixels are in the section: `w` x `h` RGBA8. `missing_content`: a picture or a font could not be drawn
/// (the status line "Some content isn't shown.").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rendered {
    pub w: u32,
    pub h: u32,
    pub missing_content: bool,
}

impl Rendered {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.u32(self.w);
        o.u32(self.h);
        o.bool(self.missing_content);
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<Rendered, DecodeError> {
        let mut i = In::new(payload);
        let got = Rendered {
            w: i.u32()?,
            h: i.u32()?,
            missing_content: i.bool()?,
        };
        i.finish()?;
        Ok(got)
    }
}

/// A block of cells: `rows` x `cols` starting at (`row`, `col`), zero-based. Also the shape of a merged area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRange {
    pub row: u32,
    pub col: u32,
    pub rows: u32,
    pub cols: u32,
}

impl CellRange {
    /// The number of cells in the block.
    pub fn area(&self) -> u64 {
        u64::from(self.rows) * u64::from(self.cols)
    }

    /// Whether (`row`, `col`) lies in the block.
    pub fn contains(&self, row: u32, col: u32) -> bool {
        row >= self.row
            && col >= self.col
            && u64::from(row) < u64::from(self.row) + u64::from(self.rows)
            && u64::from(col) < u64::from(self.col) + u64::from(self.cols)
    }

    /// Whether `inner` is non-empty and lies wholly in the block.
    pub fn holds(&self, inner: &CellRange) -> bool {
        inner.rows > 0
            && inner.cols > 0
            && inner.row >= self.row
            && inner.col >= self.col
            && u64::from(inner.row) + u64::from(inner.rows) <= u64::from(self.row) + u64::from(self.rows)
            && u64::from(inner.col) + u64::from(inner.cols) <= u64::from(self.col) + u64::from(self.cols)
    }

    fn put(&self, o: &mut Out) {
        o.u32(self.row);
        o.u32(self.col);
        o.u32(self.rows);
        o.u32(self.cols);
    }

    fn get(i: &mut In<'_>) -> Result<CellRange, DecodeError> {
        Ok(CellRange {
            row: i.u32()?,
            col: i.u32()?,
            rows: i.u32()?,
            cols: i.u32()?,
        })
    }
}

/// Send the cells of `range` on sheet `sheet` (zero-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cells {
    pub sheet: u32,
    pub range: CellRange,
}

impl Cells {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.u32(self.sheet);
        self.range.put(&mut o);
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<Cells, DecodeError> {
        let mut i = In::new(payload);
        let sheet = i.u32()?;
        let range = CellRange::get(&mut i)?;
        i.finish()?;
        Ok(Cells { sheet, range })
    }
}

/// How a cell's text sits in its cell. The helper resolves "general" (numbers right, text left) before sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    fn tag(self) -> u8 {
        match self {
            Align::Left => 1,
            Align::Center => 2,
            Align::Right => 3,
        }
    }

    fn from_tag(tag: u8) -> Result<Align, DecodeError> {
        match tag {
            1 => Ok(Align::Left),
            2 => Ok(Align::Center),
            3 => Ok(Align::Right),
            _ => Err(DecodeError::BadTag),
        }
    }
}

/// The flag bit for bold text. Every other bit is reserved and refused on decode.
const FLAG_BOLD: u8 = 1;

/// One non-empty cell: its text as shown (the number format already applied by the helper), its alignment, bold.
#[derive(Clone, PartialEq, Eq)]
pub struct CellText {
    pub row: u32,
    pub col: u32,
    pub text: String,
    pub align: Align,
    pub bold: bool,
}

/// The text is document content, so `Debug` shows its length only.
impl fmt::Debug for CellText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CellText")
            .field("row", &self.row)
            .field("col", &self.col)
            .field("text_len", &self.text.len())
            .field("align", &self.align)
            .field("bold", &self.bold)
            .finish()
    }
}

/// Frozen panes: the first `rows` rows and `cols` columns of the sheet stay in place when it scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frozen {
    pub rows: u32,
    pub cols: u32,
}

/// The cells of an asked range. `col_widths` has one width per column of the range and `row_heights` one height per
/// row, in points (0 = hidden); `merges` are the merged areas inside the range; `frozen` is the sheet's frozen panes.
#[derive(Debug, Clone, PartialEq)]
pub struct CellData {
    pub cells: Vec<CellText>,
    pub col_widths: Vec<f32>,
    pub row_heights: Vec<f32>,
    pub merges: Vec<CellRange>,
    pub frozen: Frozen,
}

impl CellData {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        let n = o.count(self.cells.len());
        for cell in self.cells.iter().take(n) {
            o.u32(cell.row);
            o.u32(cell.col);
            o.u8(cell.align.tag());
            o.u8(if cell.bold { FLAG_BOLD } else { 0 });
            o.str(&cell.text);
        }
        let n = o.count(self.col_widths.len());
        for w in self.col_widths.iter().take(n) {
            o.f32(*w);
        }
        let n = o.count(self.row_heights.len());
        for h in self.row_heights.iter().take(n) {
            o.f32(*h);
        }
        let n = o.count(self.merges.len());
        for m in self.merges.iter().take(n) {
            m.put(&mut o);
        }
        o.u32(self.frozen.rows);
        o.u32(self.frozen.cols);
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<CellData, DecodeError> {
        let mut i = In::new(payload);
        // Each cell: row, col, align, flags, a 4-byte text length.
        let n = i.count(14)?;
        let mut cells = Vec::with_capacity(n);
        for _ in 0..n {
            let row = i.u32()?;
            let col = i.u32()?;
            let align = Align::from_tag(i.u8()?)?;
            let flags = i.u8()?;
            if flags & !FLAG_BOLD != 0 {
                return Err(DecodeError::BadTag);
            }
            let text = i.str()?;
            cells.push(CellText {
                row,
                col,
                text,
                align,
                bold: flags & FLAG_BOLD != 0,
            });
        }
        let n = i.count(4)?;
        let mut col_widths = Vec::with_capacity(n);
        for _ in 0..n {
            col_widths.push(i.f32()?);
        }
        let n = i.count(4)?;
        let mut row_heights = Vec::with_capacity(n);
        for _ in 0..n {
            row_heights.push(i.f32()?);
        }
        let n = i.count(16)?;
        let mut merges = Vec::with_capacity(n);
        for _ in 0..n {
            merges.push(CellRange::get(&mut i)?);
        }
        let frozen = Frozen {
            rows: i.u32()?,
            cols: i.u32()?,
        };
        i.finish()?;
        Ok(CellData {
            cells,
            col_widths,
            row_heights,
            merges,
            frozen,
        })
    }
}

/// Send the outline (headings) of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outline;

/// Send the document's counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats;

/// Stop working on the request whose id the frame carries; its answer may still arrive, or an `Error`, or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancel;

macro_rules! empty_message {
    ($($t:ident),*) => {$(
        impl $t {
            pub fn encode(&self) -> Vec<u8> {
                Vec::new()
            }

            pub fn decode(payload: &[u8]) -> Result<$t, DecodeError> {
                In::new(payload).finish()?;
                Ok($t)
            }
        }
    )*};
}

empty_message!(Outline, Stats, Cancel);

/// One heading: its level (1 = top), its text as shown, and the zero-based page it starts on.
#[derive(Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    pub level: u8,
    pub text: String,
    pub page: u32,
}

/// The text is document content, so `Debug` shows its length only.
impl fmt::Debug for OutlineEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutlineEntry")
            .field("level", &self.level)
            .field("text_len", &self.text.len())
            .field("page", &self.page)
            .finish()
    }
}

/// The document's outline in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineData {
    pub entries: Vec<OutlineEntry>,
}

impl OutlineData {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        let n = o.count(self.entries.len());
        for e in self.entries.iter().take(n) {
            o.u8(e.level);
            o.str(&e.text);
            o.u32(e.page);
        }
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<OutlineData, DecodeError> {
        let mut i = In::new(payload);
        // Each entry: level, a 4-byte text length, page.
        let n = i.count(9)?;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let level = i.u8()?;
            let text = i.str()?;
            entries.push(OutlineEntry {
                level,
                text,
                page: i.u32()?,
            });
        }
        i.finish()?;
        Ok(OutlineData { entries })
    }
}

/// The document's counts (the status bar's word count).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatsData {
    pub words: u32,
}

impl StatsData {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.u32(self.words);
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<StatsData, DecodeError> {
        let mut i = In::new(payload);
        let words = i.u32()?;
        i.finish()?;
        Ok(StatsData { words })
    }
}

/// Why the helper could not do what was asked. The only words an error carries are the app's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The file is damaged or not what its kind says.
    Damaged,
    /// The file is over a limit of the viewer contract §5.2.
    TooLarge,
    /// The format (or this file's variant of it) has no preview here.
    Unsupported,
    /// The file is password-protected.
    Password,
    /// Anything else went wrong inside the helper.
    Internal,
}

impl ErrorCode {
    fn code(self) -> u16 {
        match self {
            ErrorCode::Damaged => 1,
            ErrorCode::TooLarge => 2,
            ErrorCode::Unsupported => 3,
            ErrorCode::Password => 4,
            ErrorCode::Internal => 5,
        }
    }

    fn from_code(code: u16) -> Result<ErrorCode, DecodeError> {
        match code {
            1 => Ok(ErrorCode::Damaged),
            2 => Ok(ErrorCode::TooLarge),
            3 => Ok(ErrorCode::Unsupported),
            4 => Ok(ErrorCode::Password),
            5 => Ok(ErrorCode::Internal),
            _ => Err(DecodeError::BadCode),
        }
    }
}

/// The helper's refusal of the request whose id the frame carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    pub code: ErrorCode,
}

impl Error {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out::new();
        o.u16(self.code.code());
        o.into_vec()
    }

    pub fn decode(payload: &[u8]) -> Result<Error, DecodeError> {
        let mut i = In::new(payload);
        let code = ErrorCode::from_code(i.u16()?)?;
        i.finish()?;
        Ok(Error { code })
    }
}

/// Any Office message, chosen by the frame's kind.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Open(Open),
    Opened(Opened),
    OpenedSheets(OpenedSheets),
    Render(Render),
    Rendered(Rendered),
    Cells(Cells),
    CellData(CellData),
    Outline(Outline),
    OutlineData(OutlineData),
    Stats(Stats),
    StatsData(StatsData),
    Cancel(Cancel),
    Error(Error),
}

impl Message {
    /// The frame kind this message travels under.
    pub fn kind(&self) -> u16 {
        match self {
            Message::Open(_) => KIND_OPEN,
            Message::Opened(_) => KIND_OPENED,
            Message::OpenedSheets(_) => KIND_OPENED_SHEETS,
            Message::Render(_) => KIND_RENDER,
            Message::Rendered(_) => KIND_RENDERED,
            Message::Cells(_) => KIND_CELLS,
            Message::CellData(_) => KIND_CELL_DATA,
            Message::Outline(_) => KIND_OUTLINE,
            Message::OutlineData(_) => KIND_OUTLINE_DATA,
            Message::Stats(_) => KIND_STATS,
            Message::StatsData(_) => KIND_STATS_DATA,
            Message::Cancel(_) => KIND_CANCEL,
            Message::Error(_) => KIND_ERROR,
        }
    }

    /// The payload for a frame of [`Message::kind`].
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Message::Open(m) => m.encode(),
            Message::Opened(m) => m.encode(),
            Message::OpenedSheets(m) => m.encode(),
            Message::Render(m) => m.encode(),
            Message::Rendered(m) => m.encode(),
            Message::Cells(m) => m.encode(),
            Message::CellData(m) => m.encode(),
            Message::Outline(m) => m.encode(),
            Message::OutlineData(m) => m.encode(),
            Message::Stats(m) => m.encode(),
            Message::StatsData(m) => m.encode(),
            Message::Cancel(m) => m.encode(),
            Message::Error(m) => m.encode(),
        }
    }

    /// Decodes a frame's payload by its kind. An unknown kind is refused.
    pub fn decode(kind: u16, payload: &[u8]) -> Result<Message, DecodeError> {
        Ok(match kind {
            KIND_OPEN => Message::Open(Open::decode(payload)?),
            KIND_OPENED => Message::Opened(Opened::decode(payload)?),
            KIND_OPENED_SHEETS => Message::OpenedSheets(OpenedSheets::decode(payload)?),
            KIND_RENDER => Message::Render(Render::decode(payload)?),
            KIND_RENDERED => Message::Rendered(Rendered::decode(payload)?),
            KIND_CELLS => Message::Cells(Cells::decode(payload)?),
            KIND_CELL_DATA => Message::CellData(CellData::decode(payload)?),
            KIND_OUTLINE => Message::Outline(Outline::decode(payload)?),
            KIND_OUTLINE_DATA => Message::OutlineData(OutlineData::decode(payload)?),
            KIND_STATS => Message::Stats(Stats::decode(payload)?),
            KIND_STATS_DATA => Message::StatsData(StatsData::decode(payload)?),
            KIND_CANCEL => Message::Cancel(Cancel::decode(payload)?),
            KIND_ERROR => Message::Error(Error::decode(payload)?),
            _ => return Err(DecodeError::UnknownKind),
        })
    }
}

#[cfg(test)]
mod tests;
