//! An archive's list (DW-1515): what is in it, without unpacking. A note says how much, the
//! list shows the folders and files with what they hold and their sizes, and the way to the
//! files themselves is "Open in another app…". A folder's row opens that folder's list (Enter,
//! a double press); Backspace and Back step out again.

use rune::{Align, Color, El, col, el, row};

use super::chrome::{Dot, Status};
use super::media::LIST_KEY;
use super::source::{ArchiveDoc, Entry, EntryKind};
use super::{Intent, Ui, details, media_icons, style, text, ui};
use crate::State;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::kit;

const COL_CONTAINS_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;
const COL_GAP: f32 = 12.0;
/// The two columns the Files list has and an archive's has not, at no width: their gaps stay.
const COL_REST: f32 = 2.0 * COL_GAP;

pub fn tools(t: &Theme) -> Vec<El> {
    vec![
        text::tool_action("tb-btn-0", "Open in another app\u{2026}", None, None, t).on_press(|s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.intent(Intent::OpenInAnotherApp);
            }
        }),
    ]
}

pub fn banner(doc: &ArchiveDoc, t: &Theme) -> El {
    text::banner_el(&media_icons::FILE_ARCHIVE, &doc.note, None, t)
}

pub fn status() -> Status {
    Status {
        dot: Dot::Green,
        text: details::DECRYPTED_HERE.into(),
        details: "Space for Preview".into(),
    }
}

fn head(key: &'static str, words: &'static str, t: &Theme) -> El {
    style::fitted(key, words, type_::COLUMN_HEADER.lh(13.0), t.text_tertiary).role("columnheader")
}

fn headers(t: &Theme) -> El {
    row()
        .key("column-headers")
        .role("row")
        .h(28.0)
        .w_full()
        .mt(4.0)
        .px(20.0)
        .gap(COL_GAP)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            el().key("col-name-cell")
                .grow(1.0)
                .min_w(0.0)
                .child(head("col-name", "Name", t)),
        )
        .child(
            el().key("col-date-cell")
                .w(COL_CONTAINS_W)
                .child(head("col-date", "Contains", t)),
        )
        .child(
            row()
                .key("col-size-cell")
                .w(COL_SIZE_W)
                .justify_end()
                .child(head("col-size", "Size", t)),
        )
        .child(el().key("col-rest").w(COL_REST - COL_GAP))
}

fn dash(words: &str) -> String {
    if words.is_empty() {
        "\u{2014}".into()
    } else {
        words.to_string()
    }
}

fn entry_row(i: usize, entry: &Entry, selected: bool, t: &Theme) -> El {
    let k = format!("row-{i}");
    let icon = match entry.kind {
        EntryKind::Folder => &media_icons::FOLDER,
        EntryKind::Text => &media_icons::FILE_TEXT,
        EntryKind::Sheet => &media_icons::FILE_SHEET,
    };
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    row()
        .key(k.clone())
        .role("listitem")
        .label(format!(
            "{}, {}",
            entry.path,
            if entry.contains.is_empty() {
                &entry.size
            } else {
                &entry.contains
            }
        ))
        .checked(selected)
        .h(30.0)
        .w_full()
        .px(14.0)
        .gap(COL_GAP)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if selected { t.selection_row } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .on_press(move |s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.select_entry(i);
            }
        })
        .on_double_press(move |s: &mut State| {
            if let Some(ui) = ui(s) {
                ui.enter_entry(i);
            }
        })
        .child(
            row()
                .key(format!("{k}-name-cell"))
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .gap(8.0)
                .child(
                    el().key(format!("{k}-tile"))
                        .size(22.0, 22.0)
                        .radius(6.0)
                        .center()
                        .fill(style::tile_neutral(t))
                        .child(kit::glyph(format!("{k}-icon"), icon, 14.0, t.text_secondary)),
                )
                .child(
                    kit::label(format!("{k}-name"), entry.path.clone(), type_::LIST_ITEM, t.text)
                        .truncate()
                        .min_w(0.0),
                ),
        )
        .child(
            kit::label(
                format!("{k}-date"),
                dash(&entry.contains),
                type_::META_MONO.lh(15.0),
                meta,
            )
            .w(COL_CONTAINS_W)
            .truncate(),
        )
        .child(
            kit::label(format!("{k}-size"), dash(&entry.size), type_::META_MONO.lh(15.0), meta)
                .w(COL_SIZE_W)
                .align(Align::End),
        )
        .child(el().key(format!("{k}-rest")).w(COL_REST - COL_GAP))
}

pub fn body(ui: &Ui, t: &Theme) -> El {
    let mut rows = col()
        .key(LIST_KEY)
        .role("list")
        .label(if ui.media.dir.is_empty() {
            ui.file.name.clone()
        } else {
            ui.media.dir.clone()
        })
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(6.0)
        .py(4.0)
        .focusable(true)
        .no_focus_ring()
        .scroll(rune::ScrollSpec { stick_bottom: false });
    for (i, entry) in ui.entries().into_iter().enumerate() {
        rows = rows.child(entry_row(i, entry, ui.media.entry == Some(i), t));
    }
    col()
        .key("viewer-body")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(headers(t))
        .child(rows)
}
