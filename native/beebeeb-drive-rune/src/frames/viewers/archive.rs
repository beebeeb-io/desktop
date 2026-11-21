//! An archive's list (DW-1515): what is in it, without unpacking. A note says how much, the
//! list shows the folders and files with what they hold and their sizes, and the way to the
//! files themselves is "Open in another app…". A folder's row opens that folder's list (Enter,
//! a double press); Backspace and Back step out again.

use rune::{Align, El, col};

use super::chrome::{Dot, Status};
use super::media::LIST_KEY;
use super::source::{ArchiveDoc, Entry, EntryKind};
use super::{Intent, Ui, details, media_icons, text, ui};
use crate::State;
use crate::theme::file_icons::icon_for;
use crate::theme::tokens::{Theme, type_};
use crate::ui::file_row::{self, Column, RowLook, Width};
use crate::ui::kit;
use crate::ui::view_state::{ViewState, view_state};

const COL_CONTAINS_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;

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

const COLUMNS: [Column; 3] = [
    Column::new("name", "Name", Width::Flex(1.0)),
    Column::new("date", "Contains", Width::Fixed(COL_CONTAINS_W)),
    Column::new("size", "Size", Width::Fixed(COL_SIZE_W)).align(Align::End),
];

fn dash(words: &str) -> String {
    if words.is_empty() {
        "\u{2014}".into()
    } else {
        words.to_string()
    }
}

fn entry_row(i: usize, total: usize, entry: &Entry, selected: bool, t: &Theme) -> El {
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    let label = format!(
        "{}, {}",
        entry.path,
        if entry.contains.is_empty() {
            &entry.size
        } else {
            &entry.contains
        }
    );
    let cells = vec![
        kit::label(
            format!("{k}-date"),
            dash(&entry.contains),
            type_::META_MONO.lh(15.0),
            meta,
        )
        .w(COL_CONTAINS_W)
        .truncate(),
        kit::label(format!("{k}-size"), dash(&entry.size), type_::META_MONO.lh(15.0), meta)
            .w(COL_SIZE_W)
            .align(Align::End),
    ];
    file_row::row(
        &k,
        &label,
        icon_for(&entry.path, matches!(entry.kind, EntryKind::Folder), None),
        &entry.path,
        cells,
        RowLook {
            selected,
            position: Some((i + 1, total)),
            ..RowLook::default()
        },
        t,
    )
    .checked(selected)
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
    let entries = ui.entries();
    let total = entries.len();
    for (i, entry) in entries.into_iter().enumerate() {
        rows = rows.child(entry_row(i, total, entry, ui.media.entry == Some(i), t));
    }
    let body = col().key("viewer-body").grow(1.0).min_w(0.0).h_full().fill(t.bg);
    if total == 0 {
        // Nothing to list: said, never a header over a blank list.
        let (title, words) = if ui.media.dir.is_empty() {
            ("This archive is empty", "It holds no files or folders.")
        } else {
            ("This folder is empty", "Go back to see the rest of the archive.")
        };
        return body.center().child(view_state(
            ViewState::Empty {
                key: "archive-empty",
                icon: &media_icons::FILE_ARCHIVE,
                title,
                body: words,
                action: None,
            },
            t,
        ));
    }
    body.child(file_row::header(&COLUMNS, None, t)).child(rows)
}
