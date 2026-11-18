//! Render-only scenes of an item's Details and read-only Preview (`--scene trash-details`): no board frame draws
//! them, so they are NOT board frames and not counted as such; they are the renders a person looks at. Every value
//! is the board's sample (or an invented text file's), never an engine's.

use super::source::BoardPreview;
use super::{Origin, stamp};
use crate::State;
use crate::frames::files::actions_data::{Page, PreviewOf, Previewed};
use crate::frames::shared::dialogs::{Dialog, PreviewHeld};
use crate::frames::shared::parts::table::{Extra, Kind, Row};
use crate::frames::trash::TrashUi;
use crate::frames::trash::source::{BoardTrash, Emptying, TrashSource, TrashTotals};

/// The board's Trash with one invented text file among its rows: what a Trash preview shows a page of.
struct WithText(Vec<Row>);

impl TrashSource for WithText {
    fn items(&self) -> Vec<Row> {
        self.0.clone()
    }
    fn totals(&self) -> TrashTotals {
        TrashTotals {
            items: 13,
            size: "240 MB".into(),
        }
    }
    fn emptying(&self) -> Option<Emptying> {
        None
    }
    fn restore(&mut self, _id: &str) -> bool {
        false
    }
}

const NOTES: &str = "t-trip-notes";

fn trash_with_text(s: &mut State) {
    crate::frames::trash::scenes::dw_0747(s);
    let mut rows = BoardTrash::new().items();
    rows.insert(
        1,
        Row {
            id: NOTES.into(),
            name: "Trip notes.txt".into(),
            kind: Kind::Document,
            date: "1 Nov 2026".into(),
            size: "4 KB".into(),
            extra: Extra::Words("From Documents".into()),
            pc: None,
            bytes: None,
            at: None,
        },
    );
    s.frames.trash = TrashUi::on(Box::new(WithText(rows)));
    s.frames.trash.pick_row(1);
}

/// The dialog over the scene's section, as `open_details` / `open_preview` leave it.
fn show(s: &mut State, dialog: Dialog, preview: Option<Previewed>) {
    s.frames.shared.open_dialog(dialog);
    if let Some(preview) = preview {
        s.frames.shared.fields.preview = PreviewHeld::of(preview);
    }
    s.frames.shared.fields.item_stamp = Some(stamp(s));
}

fn trash_details(s: &mut State) {
    crate::frames::trash::scenes::dw_0747(s);
    s.frames.trash.pick_row(1);
    show(
        s,
        Dialog::Details {
            origin: Origin::Trash,
            id: "t-contract-old".into(),
        },
        None,
    );
}

fn trash_preview_text(s: &mut State) {
    trash_with_text(s);
    let page = Previewed::Shown(PreviewOf {
        place: "The whole file".into(),
        page: Page {
            title: "Trip notes.txt".into(),
            byline: "Text".into(),
            sections: vec![(
                String::new(),
                "Train at 08:12 from the central station.\nPack the charger and the blue folder.\nCall the hotel \
                 about the late check-in."
                    .into(),
            )],
        },
    });
    let (source, _) = BoardPreview::new(page.clone());
    s.frames.item_preview = Box::new(source);
    show(
        s,
        Dialog::Preview {
            origin: Origin::Trash,
            id: NOTES.into(),
        },
        Some(page),
    );
}

fn trash_preview_notyet(s: &mut State) {
    crate::frames::trash::scenes::dw_0747(s);
    s.frames.trash.pick_row(1);
    let (source, _) = BoardPreview::new(Previewed::NotYet);
    s.frames.item_preview = Box::new(source);
    show(
        s,
        Dialog::Preview {
            origin: Origin::Trash,
            id: "t-contract-old".into(),
        },
        Some(Previewed::NotYet),
    );
}

fn shared_details(s: &mut State) {
    crate::frames::shared::scenes::dw_0501(s);
    show(
        s,
        Dialog::Details {
            origin: Origin::Shared,
            id: "s-brand-review".into(),
        },
        None,
    );
}

fn search_details(s: &mut State) {
    crate::frames::search::dw_0802(s);
    show(
        s,
        Dialog::Details {
            origin: Origin::Search,
            id: "board-search-1".into(),
        },
        None,
    );
}

/// The names `--scene` knows, for the shell's list of scenes (`frames::scenes`).
pub const RENDER_SCENES: &[(&str, fn(&mut State))] = &[
    ("trash-details", trash_details),
    ("trash-preview-text", trash_preview_text),
    ("trash-preview-notyet", trash_preview_notyet),
    ("shared-details", shared_details),
    ("search-details", search_details),
];
