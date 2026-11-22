//! A file being kept on this PC (DW-0412): while its copy comes down, its row says how far
//! ("Downloading, 0.7 of 1.2 MB") with a bar, its PC mark is the download tray, the panel's
//! Available offline card says the same, and the status line says "Syncing".
//!
//! The figures come from `FilesSource::downloads`: on the engine, the transfer board's whole-file
//! downloads of this account (`Engine::downloads_for`). A source that cannot tell how far a copy has
//! come answers nothing, and the row then shows no bar: never a made-up one.

use super::source::{Action, FilesFixture};
use crate::State;
use crate::source::DriveItem;
use crate::theme::icons_settings;
use crate::theme::scale;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use rune::{El, col, el};

/// One copy on its way down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    /// The item's id (its row has the same).
    pub id: String,
    pub done: u64,
    pub total: u64,
    /// This PC keeps the file (Available offline). A copy that comes down for another reason (a player reading it, a
    /// conflict copy) shows its bar and does not claim a keep.
    pub kept: bool,
}

impl Download {
    /// How far, from 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }

    /// "0.7 of 1.2 MB": both in the unit of the whole.
    pub fn figures(&self) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * KB;
        const GB: f64 = MB * KB;
        let (unit, size, decimals) = match self.total as f64 {
            t if t >= GB => ("GB", GB, 1),
            t if t >= MB => ("MB", MB, 1),
            _ => ("KB", KB, 0),
        };
        let part = |n: u64| format!("{:.*}", decimals, n as f64 / size);
        format!("{} of {} {unit}", part(self.done), part(self.total))
    }

    /// The row's words.
    pub fn row_words(&self) -> String {
        format!("Downloading, {}", self.figures())
    }

    /// The panel's second line under Available offline.
    pub fn panel_words(&self) -> String {
        format!("Downloading \u{00B7} {}", self.figures())
    }
}

/// The status line's word: "Syncing" while a copy comes down.
pub fn sync_word(downloads: &[Download]) -> String {
    if downloads.is_empty() { "Up to date" } else { "Syncing" }.into()
}

impl State {
    /// The copy of this item that is coming down now, if the source says so.
    pub fn download_of(&self, id: &str) -> Option<Download> {
        self.actions.source.downloads().into_iter().find(|d| d.id == id)
    }
}

/// The PC column's mark of a row that downloads (`state` "downloading").
pub fn pc_mark(key: String, t: &Theme) -> El {
    kit::state_glyph(
        key,
        &icons_settings::DOWNLOAD_TRAY,
        None,
        "downloading",
        14.0,
        t.text_tertiary,
    )
}

/// The row's words over their bar, where the date and the size stand.
pub fn progress(k: &str, item: &DriveItem, d: &Download, width: f32, t: &Theme) -> El {
    let words = d.row_words();
    col()
        .key(format!("{k}-download"))
        .w(width)
        .gap(scale::space::S4)
        .child(
            kit::label(
                format!("{k}-download-words"),
                words.clone(),
                type_::CAPTION.lh(13.5),
                t.text_secondary,
            )
            .truncate(),
        )
        .child(bar(
            &format!("{k}-bar"),
            d.fraction(),
            &format!("{}, {words}", item.name),
            t,
        ))
}

/// DW-0412: Contract draft.pdf, kept on this PC, 0.7 of 1.2 MB down; the toast says so.
pub fn scene(s: &mut State) {
    const CONTRACT: usize = 6;
    s.select_only(CONTRACT);
    let mut fixture = FilesFixture::default();
    let total = s.listing.items[CONTRACT].size.unwrap_or(0).max(0) as u64;
    fixture.more.downloads = vec![Download {
        id: s.listing.items[CONTRACT].id.clone(),
        done: total * 7 / 12,
        total,
        kept: true,
    }];
    // The row is one of the kept ones, the way `keep_offline` leaves it.
    s.listing.items[CONTRACT].pc_status = Some("available_offline".into());
    s.actions.source = Box::new(fixture);
    s.actions.toast = Some(super::Toast::Keeping);
    let _ = Action::KeepOffline;
}

/// The bar of a copy coming down (`.mk-prog`): the ink's own grey on the faint track. Amber is for
/// encryption state and primary actions, and an upload's bar (`uploads::bar`); a download is neither.
fn bar(key: &str, done: f32, name: &str, t: &Theme) -> El {
    el().key(key.to_string())
        .role("progressbar")
        .label(name)
        .w_full()
        .h(4.0)
        .radius(2.0)
        .clip_rounded(2.0)
        .fill(t.text.alpha(0.09))
        .child(
            el().key(format!("{key}-fill"))
                .h(4.0)
                .w_frac(done.clamp(0.0, 1.0))
                .radius(2.0)
                .fill(t.text_tertiary),
        )
}
