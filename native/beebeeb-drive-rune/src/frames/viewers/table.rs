//! A CSV or TSV as a table (DW-1514): the first row as the columns' names, numbers right-aligned
//! in tabular figures, one cell selected (outlined, neutral). A press on a column's name sorts by
//! it; the rows scroll under the names. Edit opens it in Sheets (DW-1508).

use rune::tok::Family;
use rune::{Align, El, TextStyle, col, row};

use super::chrome::{Dot, Status};
use super::media::TABLE_KEY;
use super::source::TableDoc;
use super::{Intent, Ui, details, icons, style, text, ui};
use crate::State;
use crate::theme::tokens::Theme;
use crate::ui::kit;

/// A cell: 12.5px. Its line is as high as the font's own (15.25 for Inter at this size).
const CELL: TextStyle = TextStyle::new(12.5, 400.0).lh(15.25);
const CELL_MONO: TextStyle = TextStyle::new(12.5, 400.0).family(Family::Mono).lh(15.25);
const HEAD: TextStyle = TextStyle::new(12.5, 600.0).lh(15.25);
const HEAD_H: f32 = 31.25;
const ROW_H: f32 = 29.5;
/// A cell's padding left and right (`px-3`).
const PAD: f32 = 12.0;

pub fn tools(t: &Theme) -> Vec<El> {
    vec![
        text::tool_action("tb-btn-0", "Edit", Some(&icons::EDIT), Some("Ctrl+E"), t).on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.intent(Intent::EditInSheets);
            }
        }),
    ]
}

pub fn status() -> Status {
    Status {
        dot: Dot::Green,
        text: details::DECRYPTED_HERE.into(),
        details: "Space for Preview \u{00B7} Ctrl+E to edit".into(),
    }
}

/// The columns' shares of the table's width: each as wide as its widest value with its padding,
/// and what is left shared out in the same proportion (how a browser lays an automatic table out).
pub fn shares(doc: &TableDoc) -> Vec<f32> {
    let content: Vec<f32> = doc
        .columns
        .iter()
        .enumerate()
        .map(|(c, column)| {
            let style = if column.numeric { CELL_MONO } else { CELL };
            let cells = doc
                .rows
                .iter()
                .filter_map(|row| row.get(c))
                .map(|v| rune::text::advance(v, style))
                .fold(0.0_f32, f32::max);
            cells.max(rune::text::advance(&column.name, HEAD)) + 2.0 * PAD
        })
        .collect();
    let sum: f32 = content.iter().sum();
    content.iter().map(|w| if sum > 0.0 { w / sum } else { 0.0 }).collect()
}

pub fn body(ui: &Ui, doc: &TableDoc, t: &Theme) -> El {
    let shares = shares(doc);
    let mut head = row().key("csv-head").role("row").w_full().h(HEAD_H);
    for (c, column) in doc.columns.iter().enumerate() {
        let sorted = ui.media.sort.filter(|(by, _)| *by == c).map(|(_, ascending)| ascending);
        head = head.child(
            kit::label(format!("csv-th-{c}"), column.name.clone(), HEAD, t.text_secondary)
                .role("columnheader")
                .sorted(sorted)
                .w_frac(shares[c])
                .h(HEAD_H)
                .px(PAD)
                .pt(8.0)
                .align(if column.numeric { Align::End } else { Align::Start })
                .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
                .focusable(true)
                .focus_ring(2.0, -2.0, t.accent_deep)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    if let Some(ui) = super::ui(s) {
                        ui.sort_by(c);
                    }
                }),
        );
    }
    let mut rows = col()
        .key("csv-rows")
        .w_full()
        .shrink()
        .min_h(0.0)
        .scroll(rune::ScrollSpec { stick_bottom: false });
    for (r, values) in ui.table_rows().into_iter().enumerate() {
        let mut line = row().key(format!("csv-row-{r}")).role("row").w_full().h(ROW_H);
        for (c, column) in doc.columns.iter().enumerate() {
            let selected = ui.media.cell == Some((r, c));
            let value = values.get(c).cloned().unwrap_or_default();
            let mut cell = kit::label(
                format!("csv-{r}-{c}"),
                value,
                if column.numeric { CELL_MONO } else { CELL },
                t.text,
            )
            .role("cell")
            .checked(selected)
            .w_frac(shares[c])
            .h(ROW_H)
            .px(PAD)
            .pt(6.875)
            .align(if column.numeric { Align::End } else { Align::Start })
            .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                if let Some(ui) = super::ui(s) {
                    ui.select_cell(r, c);
                }
            });
            if selected {
                // Outlined, neutral: never Amber.
                cell = cell.box_shadow(rune::Shadow {
                    x: 0.0,
                    y: 0.0,
                    blur: 0.0,
                    spread: 2.0,
                    color: t.text,
                    inset: true,
                });
            }
            line = line.child(cell);
        }
        rows = rows.child(line);
    }
    let table = col()
        .key(TABLE_KEY)
        .role("grid")
        .label(format!("{} columns, {} rows", doc.columns.len(), doc.total_rows))
        .w_full()
        .shrink()
        .min_h(0.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        // The ring is outside the table's box, so what clips the corners is a box inside it.
        .child(
            col()
                .key("csv-clip")
                .w_full()
                .shrink()
                .min_h(0.0)
                .clip_rounded(8.0)
                .child(head)
                .child(rows),
        );
    col()
        .key("viewer-body")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .padding(16.0)
        .fill(style::stage(t))
        .clip_rounded(0.0)
        .child(table)
}
