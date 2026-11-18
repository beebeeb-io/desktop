//! The protocol's edit types to the Office engine's `edit` types and back, one match per type.
//!
//! Every enum is matched by variant with no wildcard, so a variant added on either side is a compile error here
//! and not a silent default. The engine's error words never travel: [`edit_code`] and [`save_code`] give the
//! protocol's fixed codes only. `Debug` of every type here already shows lengths, never text.

use beebeeb_office::CellRange as EngineRange;
use beebeeb_office::edit as e;
use beebeeb_office_protocol::office::{
    CellRange, Dirty, EditKind, EditOp, Edited, ErrorCode, HitMode, ListKind, Mark, Marks, Motion, NumberFormat,
    Opened, PageRect, PageSize, ParaStyle, RectPt, Rgb, Template, TextAlign,
};

/// The engine's kind of document for the protocol's.
pub fn kind(k: EditKind) -> e::EditKind {
    match k {
        EditKind::Docx => e::EditKind::Docx,
        EditKind::Xlsx => e::EditKind::Xlsx,
        EditKind::Pptx => e::EditKind::Pptx,
    }
}

/// The engine's template for the protocol's.
pub fn template(t: Template) -> e::Template {
    match t {
        Template::Blank => e::Template::Blank,
        Template::Letter => e::Template::Letter,
        Template::Invoice => e::Template::Invoice,
        Template::MeetingNotes => e::Template::MeetingNotes,
        Template::Report => e::Template::Report,
    }
}

/// The engine's click mode for the protocol's.
pub fn hit_mode(m: HitMode) -> e::HitMode {
    match m {
        HitMode::Place => e::HitMode::Place,
        HitMode::Extend => e::HitMode::Extend,
        HitMode::Word => e::HitMode::Word,
        HitMode::Paragraph => e::HitMode::Paragraph,
    }
}

fn motion(m: Motion) -> e::Motion {
    match m {
        Motion::Left => e::Motion::Left,
        Motion::Right => e::Motion::Right,
        Motion::Up => e::Motion::Up,
        Motion::Down => e::Motion::Down,
        Motion::WordLeft => e::Motion::WordLeft,
        Motion::WordRight => e::Motion::WordRight,
        Motion::LineStart => e::Motion::LineStart,
        Motion::LineEnd => e::Motion::LineEnd,
        Motion::DocStart => e::Motion::DocStart,
        Motion::DocEnd => e::Motion::DocEnd,
        Motion::PageUp => e::Motion::PageUp,
        Motion::PageDown => e::Motion::PageDown,
    }
}

fn mark(m: Mark) -> e::Mark {
    match m {
        Mark::Bold => e::Mark::Bold,
        Mark::Italic => e::Mark::Italic,
        Mark::Underline => e::Mark::Underline,
        Mark::Strike => e::Mark::Strike,
    }
}

fn para_style(s: ParaStyle) -> e::ParaStyle {
    match s {
        ParaStyle::Normal => e::ParaStyle::Normal,
        ParaStyle::Title => e::ParaStyle::Title,
        ParaStyle::Heading1 => e::ParaStyle::Heading1,
        ParaStyle::Heading2 => e::ParaStyle::Heading2,
        ParaStyle::Heading3 => e::ParaStyle::Heading3,
    }
}

fn list_kind(k: ListKind) -> e::ListKind {
    match k {
        ListKind::None => e::ListKind::None,
        ListKind::Bullet => e::ListKind::Bullet,
        ListKind::Number => e::ListKind::Number,
    }
}

fn text_align(a: TextAlign) -> e::TextAlign {
    match a {
        TextAlign::Left => e::TextAlign::Left,
        TextAlign::Center => e::TextAlign::Center,
        TextAlign::Right => e::TextAlign::Right,
        TextAlign::Justify => e::TextAlign::Justify,
    }
}

fn number_format(n: NumberFormat) -> e::NumberFormat {
    match n {
        NumberFormat::General => e::NumberFormat::General,
        NumberFormat::Currency => e::NumberFormat::Currency,
        NumberFormat::Percent => e::NumberFormat::Percent,
        NumberFormat::MoreDecimals => e::NumberFormat::MoreDecimals,
        NumberFormat::FewerDecimals => e::NumberFormat::FewerDecimals,
    }
}

fn rgb(c: Rgb) -> e::Rgb {
    e::Rgb { r: c.r, g: c.g, b: c.b }
}

fn range(r: CellRange) -> EngineRange {
    EngineRange {
        row: r.row,
        col: r.col,
        rows: r.rows,
        cols: r.cols,
    }
}

fn range_back(r: EngineRange) -> CellRange {
    CellRange {
        row: r.row,
        col: r.col,
        rows: r.rows,
        cols: r.cols,
    }
}

/// The engine's op for the protocol's. An image is not an op of the protocol: see [`image`].
pub fn op(op: EditOp) -> e::Op {
    match op {
        EditOp::InsertText(s) => e::Op::InsertText(s),
        EditOp::InsertParagraph => e::Op::InsertParagraph,
        EditOp::InsertLineBreak => e::Op::InsertLineBreak,
        EditOp::Delete { back, word } => e::Op::Delete { back, word },
        EditOp::Move { to, extend } => e::Op::Move { to: motion(to), extend },
        EditOp::SelectAll => e::Op::SelectAll,
        EditOp::Mark { mark: m, on } => e::Op::Mark { mark: mark(m), on },
        EditOp::FontSize(v) => e::Op::FontSize(v),
        EditOp::FontFamily(s) => e::Op::FontFamily(s),
        EditOp::TextColour(c) => e::Op::TextColour(rgb(c)),
        EditOp::Highlight(c) => e::Op::Highlight(c.map(rgb)),
        EditOp::ParaStyle(s) => e::Op::ParaStyle(para_style(s)),
        EditOp::List(k) => e::Op::List(list_kind(k)),
        EditOp::Align(a) => e::Op::Align(text_align(a)),
        EditOp::Indent(n) => e::Op::Indent(n),
        EditOp::Table { rows, cols } => e::Op::Table { rows, cols },
        EditOp::Link { text, url } => e::Op::Link { text, url },
        EditOp::PageBreak => e::Op::PageBreak,
        EditOp::SetCell { sheet, row, col, text } => e::Op::SetCell { sheet, row, col, text },
        EditOp::SelectCells { sheet, range: r } => e::Op::SelectCells { sheet, range: range(r) },
        EditOp::NumberFormat(n) => e::Op::NumberFormat(number_format(n)),
        EditOp::FillColour(c) => e::Op::FillColour(c.map(rgb)),
        EditOp::AutoSum => e::Op::AutoSum,
        EditOp::Sort { ascending } => e::Op::Sort { ascending },
        EditOp::Filter => e::Op::Filter,
        EditOp::MakeTable => e::Op::MakeTable,
        EditOp::RenameSheet { sheet, name } => e::Op::RenameSheet { sheet, name },
        EditOp::SelectShape { slide, x_pt, y_pt } => e::Op::SelectShape { slide, x_pt, y_pt },
        EditOp::ClearShape => e::Op::ClearShape,
        EditOp::NewSlide { layout } => e::Op::NewSlide { layout },
        EditOp::DuplicateSlide(n) => e::Op::DuplicateSlide(n),
        EditOp::DeleteSlide(n) => e::Op::DeleteSlide(n),
        EditOp::MoveSlide { from, to } => e::Op::MoveSlide { from, to },
        EditOp::SetLayout { slide, layout } => e::Op::SetLayout { slide, layout },
        EditOp::Paste { text } => e::Op::Paste { text },
        EditOp::Undo => e::Op::Undo,
        EditOp::Redo => e::Op::Redo,
    }
}

/// The engine's op for an `InsertImage`: the bytes the helper copied out of the host's read-only section.
pub fn image(bytes: Vec<u8>, png: bool) -> e::Op {
    e::Op::Image { bytes, png }
}

fn para_style_back(s: e::ParaStyle) -> ParaStyle {
    match s {
        e::ParaStyle::Normal => ParaStyle::Normal,
        e::ParaStyle::Title => ParaStyle::Title,
        e::ParaStyle::Heading1 => ParaStyle::Heading1,
        e::ParaStyle::Heading2 => ParaStyle::Heading2,
        e::ParaStyle::Heading3 => ParaStyle::Heading3,
    }
}

fn list_kind_back(k: e::ListKind) -> ListKind {
    match k {
        e::ListKind::None => ListKind::None,
        e::ListKind::Bullet => ListKind::Bullet,
        e::ListKind::Number => ListKind::Number,
    }
}

fn text_align_back(a: e::TextAlign) -> TextAlign {
    match a {
        e::TextAlign::Left => TextAlign::Left,
        e::TextAlign::Center => TextAlign::Center,
        e::TextAlign::Right => TextAlign::Right,
        e::TextAlign::Justify => TextAlign::Justify,
    }
}

fn page_rect(r: e::PageRect) -> PageRect {
    PageRect {
        page: r.page,
        rect: RectPt {
            x: r.rect.x,
            y: r.rect.y,
            w: r.rect.w,
            h: r.rect.h,
        },
    }
}

/// The page sizes the protocol carries, from the engine's points.
pub fn page_sizes(sizes_pt: &[(f32, f32)]) -> Vec<PageSize> {
    sizes_pt.iter().map(|&(w_pt, h_pt)| PageSize { w_pt, h_pt }).collect()
}

/// What an edit changed, as the protocol says it. Nothing is cut here: [`crate::fit::edited`] holds the caps.
pub fn change(c: e::Change) -> Edited {
    Edited {
        revision: c.revision,
        caret: c.caret.map(page_rect),
        selection: c.selection.into_iter().map(page_rect).collect(),
        can_undo: c.can_undo,
        can_redo: c.can_redo,
        dirty: match c.dirty {
            e::Dirty::All => Dirty::All,
            e::Dirty::Rects(r) => Dirty::Rects(r.into_iter().map(page_rect).collect()),
        },
        pages: c.pages.map(|p| Opened {
            // At most the engine's 2,000 pages.
            pages: u32::try_from(p.sizes_pt.len()).unwrap_or(u32::MAX),
            sizes: page_sizes(&p.sizes_pt),
        }),
        cells: c.cells.into_iter().map(|(sheet, r)| (sheet, range_back(r))).collect(),
        slides: c.slides,
        marks: Marks {
            bold: c.marks.bold,
            italic: c.marks.italic,
            underline: c.marks.underline,
            strike: c.marks.strike,
            style: para_style_back(c.marks.style),
            align: text_align_back(c.marks.align),
            list: list_kind_back(c.marks.list),
        },
    }
}

/// An edit error as a protocol code. Every variant is listed. `NotApplicable` is "this cannot be done here", the
/// same to the app as a thing this build does not do.
pub fn edit_code(err: e::EditError) -> ErrorCode {
    match err {
        e::EditError::Unsupported => ErrorCode::Unsupported,
        e::EditError::NotApplicable => ErrorCode::Unsupported,
        e::EditError::TooLarge => ErrorCode::TooLarge,
        e::EditError::Engine => ErrorCode::Internal,
    }
}

/// A save error as a protocol code. Every variant is listed.
pub fn save_code(err: e::SaveError) -> ErrorCode {
    match err {
        e::SaveError::Unsupported => ErrorCode::Unsupported,
        e::SaveError::TooLarge => ErrorCode::TooLarge,
        e::SaveError::Engine => ErrorCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_the_fixed_codes() {
        assert_eq!(edit_code(e::EditError::Unsupported), ErrorCode::Unsupported);
        assert_eq!(edit_code(e::EditError::NotApplicable), ErrorCode::Unsupported);
        assert_eq!(edit_code(e::EditError::TooLarge), ErrorCode::TooLarge);
        assert_eq!(edit_code(e::EditError::Engine), ErrorCode::Internal);
        assert_eq!(save_code(e::SaveError::Unsupported), ErrorCode::Unsupported);
        assert_eq!(save_code(e::SaveError::TooLarge), ErrorCode::TooLarge);
        assert_eq!(save_code(e::SaveError::Engine), ErrorCode::Internal);
    }

    #[test]
    fn ops_and_images_map_field_for_field() {
        assert_eq!(
            op(EditOp::Delete {
                back: true,
                word: false
            }),
            e::Op::Delete {
                back: true,
                word: false
            }
        );
        assert_eq!(
            op(EditOp::Highlight(Some(Rgb { r: 1, g: 2, b: 3 }))),
            e::Op::Highlight(Some(e::Rgb { r: 1, g: 2, b: 3 }))
        );
        assert_eq!(
            op(EditOp::SelectCells {
                sheet: 2,
                range: CellRange {
                    row: 1,
                    col: 2,
                    rows: 3,
                    cols: 4
                }
            }),
            e::Op::SelectCells {
                sheet: 2,
                range: EngineRange {
                    row: 1,
                    col: 2,
                    rows: 3,
                    cols: 4
                }
            }
        );
        assert_eq!(
            image(vec![9], true),
            e::Op::Image {
                bytes: vec![9],
                png: true
            }
        );
    }
}
