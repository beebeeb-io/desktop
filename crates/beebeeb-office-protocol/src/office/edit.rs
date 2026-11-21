//! The editor's messages (W1 of the editor epic): what the app asks the helper to change and what it answers. Same
//! rules as the viewer's messages: fixed little-endian layout, length-prefixed UTF-8, one frame per message, and
//! `Debug` never shows document text. The viewer's messages and their bytes are not touched.
//!
//! **Closed set.** [`EditOp`] names what the app may ask; nothing in it names an engine command. An image is not an
//! op: it is the [`InsertImage`] message (its bytes travel in a read-only section the host made).
//!
//! **Save handshake.** `SaveSize` -> `SaveReady{token, len, revision}` -> the app checks
//! [`crate::check::check_save_ready`], makes a section of exactly `len`, sends `SaveInto{token, out}` -> `SavedInto{len}`
//! ([`crate::check::check_saved_into`] holds `len` equal to the granted length).

use std::fmt;

use super::{CellRange, DecodeError, EncodeError, GrantRef, Opened};
use crate::check::MAX_OP_TEXT_BYTES;
use crate::wire::{In, Out};

/// App -> helper: open the document in the read-only section `doc` for editing.
pub const KIND_OPEN_EDIT: u16 = 0x0108;
/// App -> helper: start a new document from a template.
pub const KIND_NEW_DOC: u16 = 0x0109;
/// App -> helper: one edit.
pub const KIND_EDIT: u16 = 0x010A;
/// App -> helper: what is under a point of a page.
pub const KIND_HIT: u16 = 0x010B;
/// App -> helper: serialise and say how many bytes.
pub const KIND_SAVE_SIZE: u16 = 0x010C;
/// App -> helper: copy the serialised bytes into the granted section.
pub const KIND_SAVE_INTO: u16 = 0x010D;
/// App -> helper: send the selection as text and html.
pub const KIND_COPY: u16 = 0x010E;
/// App -> helper: send the names of the slide layouts.
pub const KIND_LAYOUTS: u16 = 0x010F;
/// App -> helper: insert the image in a read-only section. Answered by `Edited`.
pub const KIND_INSERT_IMAGE: u16 = 0x0110;
/// Helper -> app: what an edit changed.
pub const KIND_EDITED: u16 = 0x0207;
/// Helper -> app: the serialised document is ready: its size, under a token.
pub const KIND_SAVE_READY: u16 = 0x0208;
/// Helper -> app: the bytes are in the section.
pub const KIND_SAVED_INTO: u16 = 0x0209;
/// Helper -> app: the selection as text and, when it fits, html.
pub const KIND_CLIP: u16 = 0x020A;
/// Helper -> app: the answer to `OpenEdit` when the file holds parts the editor would drop on save; the helper ends.
pub const KIND_EDIT_GUARDED: u16 = 0x020C;
/// Helper -> app: the slide layouts' names.
pub const KIND_LAYOUT_LIST: u16 = 0x020B;

/// Declares a fieldless enum with a one-byte wire tag per variant; any other tag is [`DecodeError::BadTag`].
macro_rules! tag_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $t:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($(#[$vm])* $v),+ }

        impl $name {
            fn tag(self) -> u8 {
                match self { $($name::$v => $t),+ }
            }

            fn from_tag(tag: u8) -> Result<$name, DecodeError> {
                match tag {
                    $($t => Ok($name::$v),)+
                    _ => Err(DecodeError::BadTag),
                }
            }
        }
    };
}

tag_enum!(
    /// What the editor edits.
    EditKind { Docx = 1, Xlsx = 2, Pptx = 3 }
);
tag_enum!(
    /// A kind of package part the editor's engines do not keep on save (office-editor-090 look-back): a file that
    /// holds one opens in the viewer instead, so a save can never remove it.
    DroppedPart { Chart = 1, Ole = 2, Vml = 3, SmartArt = 4, Other = 5 }
);
tag_enum!(
    /// The starting point of a new document.
    Template { Blank = 1, Letter = 2, Invoice = 3, MeetingNotes = 4, Report = 5 }
);
tag_enum!(
    /// Why the app asks for the bytes: a save, or a snapshot (the app drives snapshot timing).
    Purpose { Save = 1, Snapshot = 2 }
);
tag_enum!(
    /// What a click does to the selection.
    HitMode { Place = 1, Extend = 2, Word = 3, Paragraph = 4 }
);
tag_enum!(
    /// Where a caret move goes.
    Motion {
        Left = 1, Right = 2, Up = 3, Down = 4, WordLeft = 5, WordRight = 6, LineStart = 7, LineEnd = 8,
        DocStart = 9, DocEnd = 10, PageUp = 11, PageDown = 12,
    }
);
tag_enum!(
    /// A character mark.
    Mark { Bold = 1, Italic = 2, Underline = 3, Strike = 4 }
);
tag_enum!(
    /// A paragraph style.
    ParaStyle { Normal = 1, Title = 2, Heading1 = 3, Heading2 = 4, Heading3 = 5 }
);
tag_enum!(
    /// A list kind (`None` = not a list item).
    ListKind { None = 1, Bullet = 2, Number = 3 }
);
tag_enum!(
    /// Paragraph alignment (the viewer's cell [`super::Align`] has no `Justify`).
    TextAlign { Left = 1, Center = 2, Right = 3, Justify = 4 }
);
tag_enum!(
    /// A cell number format.
    NumberFormat { General = 1, Currency = 2, Percent = 3, MoreDecimals = 4, FewerDecimals = 5 }
);

/// A colour, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    fn put(&self, o: &mut Out) {
        o.u8(self.r);
        o.u8(self.g);
        o.u8(self.b);
    }

    fn get(i: &mut In<'_>) -> Result<Rgb, DecodeError> {
        Ok(Rgb {
            r: i.u8()?,
            g: i.u8()?,
            b: i.u8()?,
        })
    }
}

fn put_opt_rgb(o: &mut Out, v: &Option<Rgb>) {
    match v {
        None => o.u8(0),
        Some(c) => {
            o.u8(1);
            c.put(o);
        }
    }
}

fn get_opt_rgb(i: &mut In<'_>) -> Result<Option<Rgb>, DecodeError> {
    match i.u8()? {
        0 => Ok(None),
        1 => Ok(Some(Rgb::get(i)?)),
        _ => Err(DecodeError::BadTag),
    }
}

fn put_opt_u32(o: &mut Out, v: Option<u32>) {
    match v {
        None => o.u8(0),
        Some(n) => {
            o.u8(1);
            o.u32(n);
        }
    }
}

fn get_opt_u32(i: &mut In<'_>) -> Result<Option<u32>, DecodeError> {
    match i.u8()? {
        0 => Ok(None),
        1 => Ok(Some(i.u32()?)),
        _ => Err(DecodeError::BadTag),
    }
}

/// An op's text, refused over [`MAX_OP_TEXT_BYTES`] before anything is written.
fn put_op_text(o: &mut Out, s: &str) -> Result<(), EncodeError> {
    if s.len() > MAX_OP_TEXT_BYTES {
        return Err(EncodeError::TextTooLong);
    }
    o.str(s);
    Ok(())
}

fn get_op_text(i: &mut In<'_>) -> Result<String, DecodeError> {
    let s = i.str()?;
    if s.len() > MAX_OP_TEXT_BYTES {
        return Err(DecodeError::TextTooLong);
    }
    Ok(s)
}

/// A rectangle in points on a page (the viewer's [`Rect`] is in pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectPt {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A rectangle on a zero-based page, in points: the caret, a piece of the selection, a dirty area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageRect {
    pub page: u32,
    pub rect: RectPt,
}

impl PageRect {
    /// Bytes of one on the wire: the page and four floats.
    pub(crate) const BYTES: usize = 4 + 4 * 4;

    fn put(&self, o: &mut Out) {
        o.u32(self.page);
        o.f32(self.rect.x);
        o.f32(self.rect.y);
        o.f32(self.rect.w);
        o.f32(self.rect.h);
    }

    fn get(i: &mut In<'_>) -> Result<PageRect, DecodeError> {
        Ok(PageRect {
            page: i.u32()?,
            rect: RectPt {
                x: i.f32()?,
                y: i.f32()?,
                w: i.f32()?,
                h: i.f32()?,
            },
        })
    }
}

fn put_rects(o: &mut Out, rects: &[PageRect]) {
    let n = o.count(rects.len());
    for r in rects.iter().take(n) {
        r.put(o);
    }
}

fn get_rects(i: &mut In<'_>) -> Result<Vec<PageRect>, DecodeError> {
    let n = i.count(PageRect::BYTES)?;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(PageRect::get(i)?);
    }
    Ok(v)
}

/// Open the document in the read-only section `doc` for editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenEdit {
    pub doc: GrantRef,
    pub kind: EditKind,
}

impl OpenEdit {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u64(self.doc.wire);
        o.u64(self.doc.len);
        o.u8(self.kind.tag());
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<OpenEdit, DecodeError> {
        let mut i = In::new(payload);
        let doc = GrantRef {
            wire: i.u64()?,
            len: i.u64()?,
        };
        let kind = EditKind::from_tag(i.u8()?)?;
        i.finish()?;
        Ok(OpenEdit { doc, kind })
    }
}

/// Start a new document of `kind` from `template`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewDoc {
    pub kind: EditKind,
    pub template: Template,
}

impl NewDoc {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u8(self.kind.tag());
        o.u8(self.template.tag());
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<NewDoc, DecodeError> {
        let mut i = In::new(payload);
        let kind = EditKind::from_tag(i.u8()?)?;
        let template = Template::from_tag(i.u8()?)?;
        i.finish()?;
        Ok(NewDoc { kind, template })
    }
}

/// One edit. Closed: nothing here names an engine command.
#[derive(Clone, PartialEq)]
pub enum EditOp {
    // Text.
    InsertText(String),
    InsertParagraph,
    InsertLineBreak,
    Delete { back: bool, word: bool },
    Move { to: Motion, extend: bool },
    SelectAll,
    // Format.
    Mark { mark: Mark, on: bool },
    FontSize(f32),
    FontFamily(String),
    TextColour(Rgb),
    Highlight(Option<Rgb>),
    ParaStyle(ParaStyle),
    List(ListKind),
    Align(TextAlign),
    Indent(i8),
    // Insert (an image is the `InsertImage` message).
    Table { rows: u32, cols: u32 },
    Link { text: String, url: String },
    PageBreak,
    // Sheets.
    SetCell { sheet: u32, row: u32, col: u32, text: String },
    SelectCells { sheet: u32, range: CellRange },
    NumberFormat(NumberFormat),
    FillColour(Option<Rgb>),
    AutoSum,
    Sort { ascending: bool },
    Filter,
    MakeTable,
    RenameSheet { sheet: u32, name: String },
    // Slides.
    SelectShape { slide: u32, x_pt: f32, y_pt: f32 },
    ClearShape,
    NewSlide { layout: Option<u32> },
    DuplicateSlide(u32),
    DeleteSlide(u32),
    MoveSlide { from: u32, to: u32 },
    SetLayout { slide: u32, layout: u32 },
    /// Plain text only in 0.9.0. Over [`MAX_OP_TEXT_BYTES`] it does not encode.
    Paste { text: String },
    Undo,
    Redo,
}

/// The text of an op is document content, so `Debug` shows its length only.
impl fmt::Debug for EditOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = |s: &String| s.len();
        match self {
            EditOp::InsertText(s) => f.debug_struct("InsertText").field("text_len", &len(s)).finish(),
            EditOp::InsertParagraph => f.write_str("InsertParagraph"),
            EditOp::InsertLineBreak => f.write_str("InsertLineBreak"),
            EditOp::Delete { back, word } => f.debug_struct("Delete").field("back", back).field("word", word).finish(),
            EditOp::Move { to, extend } => f.debug_struct("Move").field("to", to).field("extend", extend).finish(),
            EditOp::SelectAll => f.write_str("SelectAll"),
            EditOp::Mark { mark, on } => f.debug_struct("Mark").field("mark", mark).field("on", on).finish(),
            EditOp::FontSize(v) => f.debug_tuple("FontSize").field(v).finish(),
            EditOp::FontFamily(s) => f.debug_struct("FontFamily").field("name_len", &len(s)).finish(),
            EditOp::TextColour(c) => f.debug_tuple("TextColour").field(c).finish(),
            EditOp::Highlight(c) => f.debug_tuple("Highlight").field(c).finish(),
            EditOp::ParaStyle(s) => f.debug_tuple("ParaStyle").field(s).finish(),
            EditOp::List(k) => f.debug_tuple("List").field(k).finish(),
            EditOp::Align(a) => f.debug_tuple("Align").field(a).finish(),
            EditOp::Indent(n) => f.debug_tuple("Indent").field(n).finish(),
            EditOp::Table { rows, cols } => f.debug_struct("Table").field("rows", rows).field("cols", cols).finish(),
            EditOp::Link { text, url } => f
                .debug_struct("Link")
                .field("text_len", &len(text))
                .field("url_len", &len(url))
                .finish(),
            EditOp::PageBreak => f.write_str("PageBreak"),
            EditOp::SetCell { sheet, row, col, text } => f
                .debug_struct("SetCell")
                .field("sheet", sheet)
                .field("row", row)
                .field("col", col)
                .field("text_len", &len(text))
                .finish(),
            EditOp::SelectCells { sheet, range } => {
                f.debug_struct("SelectCells").field("sheet", sheet).field("range", range).finish()
            }
            EditOp::NumberFormat(n) => f.debug_tuple("NumberFormat").field(n).finish(),
            EditOp::FillColour(c) => f.debug_tuple("FillColour").field(c).finish(),
            EditOp::AutoSum => f.write_str("AutoSum"),
            EditOp::Sort { ascending } => f.debug_struct("Sort").field("ascending", ascending).finish(),
            EditOp::Filter => f.write_str("Filter"),
            EditOp::MakeTable => f.write_str("MakeTable"),
            EditOp::RenameSheet { sheet, name } => f
                .debug_struct("RenameSheet")
                .field("sheet", sheet)
                .field("name_len", &len(name))
                .finish(),
            EditOp::SelectShape { slide, x_pt, y_pt } => f
                .debug_struct("SelectShape")
                .field("slide", slide)
                .field("x_pt", x_pt)
                .field("y_pt", y_pt)
                .finish(),
            EditOp::ClearShape => f.write_str("ClearShape"),
            EditOp::NewSlide { layout } => f.debug_struct("NewSlide").field("layout", layout).finish(),
            EditOp::DuplicateSlide(n) => f.debug_tuple("DuplicateSlide").field(n).finish(),
            EditOp::DeleteSlide(n) => f.debug_tuple("DeleteSlide").field(n).finish(),
            EditOp::MoveSlide { from, to } => f.debug_struct("MoveSlide").field("from", from).field("to", to).finish(),
            EditOp::SetLayout { slide, layout } => {
                f.debug_struct("SetLayout").field("slide", slide).field("layout", layout).finish()
            }
            EditOp::Paste { text } => f.debug_struct("Paste").field("text_len", &len(text)).finish(),
            EditOp::Undo => f.write_str("Undo"),
            EditOp::Redo => f.write_str("Redo"),
        }
    }
}

impl EditOp {
    fn put(&self, o: &mut Out) -> Result<(), EncodeError> {
        match self {
            EditOp::InsertText(s) => {
                o.u8(1);
                put_op_text(o, s)?;
            }
            EditOp::InsertParagraph => o.u8(2),
            EditOp::InsertLineBreak => o.u8(3),
            EditOp::Delete { back, word } => {
                o.u8(4);
                o.bool(*back);
                o.bool(*word);
            }
            EditOp::Move { to, extend } => {
                o.u8(5);
                o.u8(to.tag());
                o.bool(*extend);
            }
            EditOp::SelectAll => o.u8(6),
            EditOp::Mark { mark, on } => {
                o.u8(7);
                o.u8(mark.tag());
                o.bool(*on);
            }
            EditOp::FontSize(v) => {
                o.u8(8);
                o.f32(*v);
            }
            EditOp::FontFamily(s) => {
                o.u8(9);
                put_op_text(o, s)?;
            }
            EditOp::TextColour(c) => {
                o.u8(10);
                c.put(o);
            }
            EditOp::Highlight(c) => {
                o.u8(11);
                put_opt_rgb(o, c);
            }
            EditOp::ParaStyle(s) => {
                o.u8(12);
                o.u8(s.tag());
            }
            EditOp::List(k) => {
                o.u8(13);
                o.u8(k.tag());
            }
            EditOp::Align(a) => {
                o.u8(14);
                o.u8(a.tag());
            }
            EditOp::Indent(n) => {
                o.u8(15);
                o.u8(*n as u8);
            }
            EditOp::Table { rows, cols } => {
                o.u8(16);
                o.u32(*rows);
                o.u32(*cols);
            }
            EditOp::Link { text, url } => {
                o.u8(17);
                put_op_text(o, text)?;
                put_op_text(o, url)?;
            }
            EditOp::PageBreak => o.u8(18),
            EditOp::SetCell { sheet, row, col, text } => {
                o.u8(19);
                o.u32(*sheet);
                o.u32(*row);
                o.u32(*col);
                put_op_text(o, text)?;
            }
            EditOp::SelectCells { sheet, range } => {
                o.u8(20);
                o.u32(*sheet);
                range.put(o);
            }
            EditOp::NumberFormat(n) => {
                o.u8(21);
                o.u8(n.tag());
            }
            EditOp::FillColour(c) => {
                o.u8(22);
                put_opt_rgb(o, c);
            }
            EditOp::AutoSum => o.u8(23),
            EditOp::Sort { ascending } => {
                o.u8(24);
                o.bool(*ascending);
            }
            EditOp::Filter => o.u8(25),
            EditOp::MakeTable => o.u8(26),
            EditOp::RenameSheet { sheet, name } => {
                o.u8(27);
                o.u32(*sheet);
                put_op_text(o, name)?;
            }
            EditOp::SelectShape { slide, x_pt, y_pt } => {
                o.u8(28);
                o.u32(*slide);
                o.f32(*x_pt);
                o.f32(*y_pt);
            }
            EditOp::ClearShape => o.u8(29),
            EditOp::NewSlide { layout } => {
                o.u8(30);
                put_opt_u32(o, *layout);
            }
            EditOp::DuplicateSlide(n) => {
                o.u8(31);
                o.u32(*n);
            }
            EditOp::DeleteSlide(n) => {
                o.u8(32);
                o.u32(*n);
            }
            EditOp::MoveSlide { from, to } => {
                o.u8(33);
                o.u32(*from);
                o.u32(*to);
            }
            EditOp::SetLayout { slide, layout } => {
                o.u8(34);
                o.u32(*slide);
                o.u32(*layout);
            }
            EditOp::Paste { text } => {
                o.u8(35);
                put_op_text(o, text)?;
            }
            EditOp::Undo => o.u8(36),
            EditOp::Redo => o.u8(37),
        }
        Ok(())
    }

    fn get(i: &mut In<'_>) -> Result<EditOp, DecodeError> {
        Ok(match i.u8()? {
            1 => EditOp::InsertText(get_op_text(i)?),
            2 => EditOp::InsertParagraph,
            3 => EditOp::InsertLineBreak,
            4 => EditOp::Delete {
                back: i.bool()?,
                word: i.bool()?,
            },
            5 => EditOp::Move {
                to: Motion::from_tag(i.u8()?)?,
                extend: i.bool()?,
            },
            6 => EditOp::SelectAll,
            7 => EditOp::Mark {
                mark: Mark::from_tag(i.u8()?)?,
                on: i.bool()?,
            },
            8 => EditOp::FontSize(i.f32()?),
            9 => EditOp::FontFamily(get_op_text(i)?),
            10 => EditOp::TextColour(Rgb::get(i)?),
            11 => EditOp::Highlight(get_opt_rgb(i)?),
            12 => EditOp::ParaStyle(ParaStyle::from_tag(i.u8()?)?),
            13 => EditOp::List(ListKind::from_tag(i.u8()?)?),
            14 => EditOp::Align(TextAlign::from_tag(i.u8()?)?),
            15 => EditOp::Indent(i.u8()? as i8),
            16 => EditOp::Table {
                rows: i.u32()?,
                cols: i.u32()?,
            },
            17 => EditOp::Link {
                text: get_op_text(i)?,
                url: get_op_text(i)?,
            },
            18 => EditOp::PageBreak,
            19 => EditOp::SetCell {
                sheet: i.u32()?,
                row: i.u32()?,
                col: i.u32()?,
                text: get_op_text(i)?,
            },
            20 => EditOp::SelectCells {
                sheet: i.u32()?,
                range: CellRange::get(i)?,
            },
            21 => EditOp::NumberFormat(NumberFormat::from_tag(i.u8()?)?),
            22 => EditOp::FillColour(get_opt_rgb(i)?),
            23 => EditOp::AutoSum,
            24 => EditOp::Sort { ascending: i.bool()? },
            25 => EditOp::Filter,
            26 => EditOp::MakeTable,
            27 => EditOp::RenameSheet {
                sheet: i.u32()?,
                name: get_op_text(i)?,
            },
            28 => EditOp::SelectShape {
                slide: i.u32()?,
                x_pt: i.f32()?,
                y_pt: i.f32()?,
            },
            29 => EditOp::ClearShape,
            30 => EditOp::NewSlide { layout: get_opt_u32(i)? },
            31 => EditOp::DuplicateSlide(i.u32()?),
            32 => EditOp::DeleteSlide(i.u32()?),
            33 => EditOp::MoveSlide {
                from: i.u32()?,
                to: i.u32()?,
            },
            34 => EditOp::SetLayout {
                slide: i.u32()?,
                layout: i.u32()?,
            },
            35 => EditOp::Paste { text: get_op_text(i)? },
            36 => EditOp::Undo,
            37 => EditOp::Redo,
            _ => return Err(DecodeError::BadTag),
        })
    }
}

/// One edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub op: EditOp,
}

impl Edit {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        self.op.put(&mut o)?;
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<Edit, DecodeError> {
        let mut i = In::new(payload);
        let op = EditOp::get(&mut i)?;
        i.finish()?;
        Ok(Edit { op })
    }
}

/// What is under (`x_pt`, `y_pt`) on `page` (points, finite); `mode` says what the click does to the selection.
/// Answered by `Edited`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub page: u32,
    pub x_pt: f32,
    pub y_pt: f32,
    pub mode: HitMode,
}

impl Hit {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u32(self.page);
        o.f32(self.x_pt);
        o.f32(self.y_pt);
        o.u8(self.mode.tag());
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<Hit, DecodeError> {
        let mut i = In::new(payload);
        let got = Hit {
            page: i.u32()?,
            x_pt: i.f32()?,
            y_pt: i.f32()?,
            mode: HitMode::from_tag(i.u8()?)?,
        };
        i.finish()?;
        Ok(got)
    }
}

/// Serialise the document into the helper's own memory and answer `SaveReady`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveSize {
    pub purpose: Purpose,
}

impl SaveSize {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u8(self.purpose.tag());
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<SaveSize, DecodeError> {
        let mut i = In::new(payload);
        let purpose = Purpose::from_tag(i.u8()?)?;
        i.finish()?;
        Ok(SaveSize { purpose })
    }
}

/// Copy the bytes serialised under `token` into the read-write section `out` (exactly `SaveReady.len` bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveInto {
    pub token: u32,
    pub out: GrantRef,
}

impl SaveInto {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u32(self.token);
        o.u64(self.out.wire);
        o.u64(self.out.len);
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<SaveInto, DecodeError> {
        let mut i = In::new(payload);
        let token = i.u32()?;
        let out = GrantRef {
            wire: i.u64()?,
            len: i.u64()?,
        };
        i.finish()?;
        Ok(SaveInto { token, out })
    }
}

/// Send the selection as text and html (answered by `Clip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Copy;

/// Send the names of the slide layouts (answered by `LayoutList`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layouts;

macro_rules! empty_edit_message {
    ($($t:ident),*) => {$(
        impl $t {
            pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
                Out::new().finish()
            }

            pub fn decode(payload: &[u8]) -> Result<$t, DecodeError> {
                In::new(payload).finish()?;
                Ok($t)
            }
        }
    )*};
}

empty_edit_message!(Copy, Layouts);

/// Insert the image whose bytes are in the read-only section `bytes` (made by the host). Answered by `Edited`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertImage {
    pub bytes: GrantRef,
    pub png: bool,
}

impl InsertImage {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u64(self.bytes.wire);
        o.u64(self.bytes.len);
        o.bool(self.png);
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<InsertImage, DecodeError> {
        let mut i = In::new(payload);
        let bytes = GrantRef {
            wire: i.u64()?,
            len: i.u64()?,
        };
        let png = i.bool()?;
        i.finish()?;
        Ok(InsertImage { bytes, png })
    }
}

/// What to redraw: everything, or these areas.
#[derive(Debug, Clone, PartialEq)]
pub enum Dirty {
    All,
    Rects(Vec<PageRect>),
}

/// The marks and styles at the caret or over the selection, for the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub style: ParaStyle,
    pub align: TextAlign,
    pub list: ListKind,
}

/// What an edit changed. `revision` rises with every change; `caret` and `selection` are where they are now;
/// `dirty` is what to redraw; `pages` is set when the page count or sizes changed; `cells` are the changed ranges
/// (sheet, range); `slides` is set when the slide count changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Edited {
    pub revision: u64,
    pub caret: Option<PageRect>,
    pub selection: Vec<PageRect>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dirty: Dirty,
    pub pages: Option<Opened>,
    pub cells: Vec<(u32, CellRange)>,
    pub slides: Option<u32>,
    pub marks: Marks,
}

impl Edited {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u64(self.revision);
        match &self.caret {
            None => o.u8(0),
            Some(c) => {
                o.u8(1);
                c.put(&mut o);
            }
        }
        put_rects(&mut o, &self.selection);
        o.bool(self.can_undo);
        o.bool(self.can_redo);
        match &self.dirty {
            Dirty::All => o.u8(0),
            Dirty::Rects(rects) => {
                o.u8(1);
                put_rects(&mut o, rects);
            }
        }
        match &self.pages {
            None => o.u8(0),
            Some(p) => {
                o.u8(1);
                p.put(&mut o);
            }
        }
        let n = o.count(self.cells.len());
        for (sheet, range) in self.cells.iter().take(n) {
            o.u32(*sheet);
            range.put(&mut o);
        }
        put_opt_u32(&mut o, self.slides);
        let m = &self.marks;
        o.bool(m.bold);
        o.bool(m.italic);
        o.bool(m.underline);
        o.bool(m.strike);
        o.u8(m.style.tag());
        o.u8(m.align.tag());
        o.u8(m.list.tag());
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<Edited, DecodeError> {
        let mut i = In::new(payload);
        let revision = i.u64()?;
        let caret = match i.u8()? {
            0 => None,
            1 => Some(PageRect::get(&mut i)?),
            _ => return Err(DecodeError::BadTag),
        };
        let selection = get_rects(&mut i)?;
        let can_undo = i.bool()?;
        let can_redo = i.bool()?;
        let dirty = match i.u8()? {
            0 => Dirty::All,
            1 => Dirty::Rects(get_rects(&mut i)?),
            _ => return Err(DecodeError::BadTag),
        };
        let pages = match i.u8()? {
            0 => None,
            1 => Some(Opened::get(&mut i)?),
            _ => return Err(DecodeError::BadTag),
        };
        // Each: a sheet and a range.
        let n = i.count(4 + 16)?;
        let mut cells = Vec::with_capacity(n);
        for _ in 0..n {
            cells.push((i.u32()?, CellRange::get(&mut i)?));
        }
        let slides = get_opt_u32(&mut i)?;
        let marks = Marks {
            bold: i.bool()?,
            italic: i.bool()?,
            underline: i.bool()?,
            strike: i.bool()?,
            style: ParaStyle::from_tag(i.u8()?)?,
            align: TextAlign::from_tag(i.u8()?)?,
            list: ListKind::from_tag(i.u8()?)?,
        };
        i.finish()?;
        Ok(Edited {
            revision,
            caret,
            selection,
            can_undo,
            can_redo,
            dirty,
            pages,
            cells,
            slides,
            marks,
        })
    }
}

/// The serialised document is ready under `token`: `len` bytes, as of `revision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveReady {
    pub token: u32,
    pub len: u64,
    pub revision: u64,
}

impl SaveReady {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u32(self.token);
        o.u64(self.len);
        o.u64(self.revision);
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<SaveReady, DecodeError> {
        let mut i = In::new(payload);
        let got = SaveReady {
            token: i.u32()?,
            len: i.u64()?,
            revision: i.u64()?,
        };
        i.finish()?;
        Ok(got)
    }
}

/// The bytes are in the section: `len` of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedInto {
    pub len: u64,
}

impl SavedInto {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.u64(self.len);
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<SavedInto, DecodeError> {
        let mut i = In::new(payload);
        let len = i.u64()?;
        i.finish()?;
        Ok(SavedInto { len })
    }
}

/// The selection as plain text and, when it fits one frame with the text, as html.
#[derive(Clone, PartialEq, Eq)]
pub struct Clip {
    pub text: String,
    pub html: Option<String>,
}

/// Both are document content, so `Debug` shows lengths only.
impl fmt::Debug for Clip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Clip")
            .field("text_len", &self.text.len())
            .field("html_len", &self.html.as_ref().map(String::len))
            .finish()
    }
}

impl Clip {
    fn put(&self, html: Option<&str>) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        o.str(&self.text);
        match html {
            None => o.u8(0),
            Some(h) => {
                o.u8(1);
                o.str(h);
            }
        }
        o.finish()
    }

    /// With the html when the whole fits one frame; without it when only the text does; [`EncodeError::OverFrame`]
    /// when the text alone does not (the helper then answers `Error{TooLarge}`).
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        match self.put(self.html.as_deref()) {
            Err(EncodeError::OverFrame) if self.html.is_some() => self.put(None),
            other => other,
        }
    }

    pub fn decode(payload: &[u8]) -> Result<Clip, DecodeError> {
        let mut i = In::new(payload);
        let text = i.str()?;
        let html = match i.u8()? {
            0 => None,
            1 => Some(i.str()?),
            _ => return Err(DecodeError::BadTag),
        };
        i.finish()?;
        Ok(Clip { text, html })
    }
}

/// The slide layouts' names, in order (index = the layout number the slide ops take).
#[derive(Clone, PartialEq, Eq)]
pub struct LayoutList {
    pub names: Vec<String>,
}

/// The names are the document's, so `Debug` shows the count only.
impl fmt::Debug for LayoutList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayoutList").field("names", &self.names.len()).finish()
    }
}

impl LayoutList {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        let n = o.count(self.names.len());
        for name in self.names.iter().take(n) {
            o.str(name);
        }
        o.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<LayoutList, DecodeError> {
        let mut i = In::new(payload);
        let n = i.count(4)?;
        let mut names = Vec::with_capacity(n);
        for _ in 0..n {
            names.push(i.str()?);
        }
        i.finish()?;
        Ok(LayoutList { names })
    }
}


/// The answer to `OpenEdit` for a file the editor would damage on save: the kinds of part it would drop (one of
/// each, in [`DroppedPart`] order, never empty). No editing session exists; the helper ends after sending it, and the
/// app opens the file in the viewer and says which parts editing would remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditGuarded {
    pub dropped: Vec<DroppedPart>,
}

/// The most kinds an [`EditGuarded`] names: one of each [`DroppedPart`].
const MAX_DROPPED: usize = 5;

impl EditGuarded {
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut o = Out::new();
        let n = o.count(self.dropped.len());
        for part in self.dropped.iter().take(n) {
            o.u8(part.tag());
        }
        o.finish()
    }

    /// Refuses an empty list, a repeated kind or more kinds than exist (`BadTag`): a guard that names nothing is not
    /// a guard.
    pub fn decode(payload: &[u8]) -> Result<EditGuarded, DecodeError> {
        let mut i = In::new(payload);
        let n = i.count(1)?;
        if n == 0 || n > MAX_DROPPED {
            return Err(DecodeError::BadTag);
        }
        let mut dropped = Vec::with_capacity(n);
        for _ in 0..n {
            let part = DroppedPart::from_tag(i.u8()?)?;
            if dropped.contains(&part) {
                return Err(DecodeError::BadTag);
            }
            dropped.push(part);
        }
        i.finish()?;
        Ok(EditGuarded { dropped })
    }
}

#[cfg(test)]
mod guarded_tests {
    use super::*;

    #[test]
    fn edit_guarded_round_trips_and_refuses_nonsense() {
        let all = EditGuarded {
            dropped: vec![DroppedPart::Chart, DroppedPart::Ole, DroppedPart::Vml, DroppedPart::SmartArt, DroppedPart::Other],
        };
        assert_eq!(EditGuarded::decode(&all.encode().unwrap()), Ok(all));
        let one = EditGuarded { dropped: vec![DroppedPart::Vml] };
        assert_eq!(EditGuarded::decode(&one.encode().unwrap()), Ok(one));
        let empty = EditGuarded { dropped: vec![] }.encode().unwrap();
        assert_eq!(EditGuarded::decode(&empty), Err(DecodeError::BadTag));
        let twice = EditGuarded { dropped: vec![DroppedPart::Chart, DroppedPart::Chart] }.encode().unwrap();
        assert_eq!(EditGuarded::decode(&twice), Err(DecodeError::BadTag));
        assert_eq!(EditGuarded::decode(&[1, 0, 0, 0, 9]), Err(DecodeError::BadTag));
        assert_eq!(EditGuarded::decode(&[1, 0, 0, 0, 1, 0]), Err(DecodeError::TrailingBytes));
    }
}
