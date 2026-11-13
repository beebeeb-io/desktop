//! Two states of the Files list that the engine reports through `GlobalSource` (DW-1108, DW-1107).
//!
//! **Offline (DW-1108).** Everything stays visible. A FILE that is not on this PC is dimmed and
//! its date reads "Needs a connection"; opening it says so (DW-0121) and the source is not asked.
//! Folders are never dimmed (they are browsed from the list on this PC), and a file that is on
//! this PC opens as usual. The status line counts what needs a connection: the window never
//! says a file is here when it is not.
//!
//! **The first sign-in on a PC (DW-1107).** The list is still downloading and being decrypted
//! here. A card above the rows says so, with the count, the time left and the two steps; the rows
//! listed so far work at once. Once something is opened the card gives way to the slim strip
//! (DW-0118, the global lane's).

use crate::State;
use crate::frames::global::source::Loading;
use crate::source::DriveItem;
use crate::theme::icons_settings;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use rune::tok::Family;
use rune::{Color, El, TextStyle, col, el, row};

/// Is there no connection (the engine says so)?
pub fn offline(s: &State) -> bool {
    s.global.source.status().offline.is_some()
}

/// Can this row not be opened now: a file that is only online, with no connection?
pub fn needs_connection(s: &State, item: &DriveItem) -> bool {
    !item.is_folder && item.pc_status.as_deref() == Some("online_only") && offline(s)
}

fn loading(s: &State) -> Option<Loading> {
    let status = s.global.source.status();
    if status.locked || status.signed_out {
        return None;
    }
    status.loading
}

fn listed_words(loading: &Loading) -> String {
    format!(
        "{} of {} listed",
        super::protected::grouped(loading.listed as i64),
        super::protected::grouped(loading.total as i64)
    )
}

/// The words under the window's title while the first list arrives: how many are listed so far.
/// Only in the root, where the count is the whole drive's; a folder inside counts its own rows.
pub fn title_words(s: &State) -> Option<String> {
    s.listing
        .folder
        .id
        .is_none()
        .then(|| loading(s))
        .flatten()
        .map(|l| listed_words(&l))
}

/// The status line's right-hand words in these states. `None`: the window counts its own way.
pub fn status_words(s: &State) -> Option<String> {
    if let Some(loading) = loading(s) {
        return Some(listed_words(&loading));
    }
    if !offline(s) || s.listing.items.is_empty() {
        return None;
    }
    let count = |status: &str| {
        s.listing
            .items
            .iter()
            .filter(|i| i.pc_status.as_deref() == Some(status))
            .count()
    };
    Some(format!(
        "{} on this PC \u{00B7} {} available offline \u{00B7} {} need a connection",
        count("on_this_pc"),
        count("available_offline"),
        count("online_only")
    ))
}

fn line(dot: Color, state: &str, details: &str, t: &Theme) -> El {
    row()
        .key("status-bar")
        .role("status")
        .h(28.0)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .child(
            row()
                .key("status-left")
                .center_y()
                .gap(7.0)
                .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(dot))
                .child(kit::label("status-text", state, type_::STATUS_STRONG, t.text_secondary)),
        )
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label("status-details", details, type_::STATUS, t.text_tertiary))
}

/// The status line of these states: "Getting your files ready" while the first list loads,
/// "Offline" without a connection (whatever the sync state's own word is). `None`: neither.
pub fn status_line(s: &State, details: &str, t: &Theme) -> Option<El> {
    if loading(s).is_some() {
        return Some(line(t.text_tertiary, "Getting your files ready", details, t));
    }
    offline(s).then(|| line(t.text_quaternary, "Offline", details, t))
}

const CARD_TITLE: TextStyle = TextStyle::new(13.0, 600.0).lh(17.0);
const CARD_TEXT: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
const CARD_FIGURE: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(15.0);
const CARD_STEP: TextStyle = TextStyle::new(11.5, 400.0).lh(15.0);
const CARD_STEP_VALUE: TextStyle = TextStyle::new(11.5, 500.0).lh(15.0);

/// The card above the rows while the first list loads (DW-1107). `None`: nothing loads, the
/// person already opened something (the strip takes over), or a banner has the place.
pub fn card(s: &State, t: &Theme) -> Option<El> {
    let loading = loading(s)?;
    if s.actions.first_load_strip || s.global.banner().is_some() {
        return None;
    }
    let figure = format!(
        "{} of {}",
        super::protected::grouped(loading.listed as i64),
        super::protected::grouped(loading.total as i64)
    );
    let left = loading.left.as_ref().map(|left| format!(" \u{00B7} {left} left"));
    // The bar moves in whole percents, as the strip's does.
    let percent = (loading.listed.saturating_mul(100))
        .checked_div(loading.total)
        .unwrap_or(0)
        .min(100);
    let title = "Getting your files ready";
    let words = "Your file list is downloading and being decrypted on this PC. You can start using Beebeeb before it's finished.";
    let step = |key: &str, name: &str, value: &str| {
        row()
            .key(format!("ready-{key}"))
            .gap(4.0)
            .child(kit::label(
                format!("ready-{key}-name"),
                format!("{name}:"),
                CARD_STEP,
                t.text_tertiary,
            ))
            .child(kit::label(format!("ready-{key}-value"), value, CARD_STEP_VALUE, t.text))
    };
    let mut count = row().key("ready-count").items_center().child(kit::label(
        "ready-figure",
        figure.clone(),
        CARD_FIGURE,
        t.text_secondary,
    ));
    if let Some(left) = &left {
        count = count.child(kit::label("ready-left", left.clone(), CARD_TEXT, t.text_secondary));
    }
    let head = row()
        .key("ready-head")
        .w_full()
        .gap(12.0)
        .items_center()
        .child(
            el().key("ready-tile")
                .a11y_hidden()
                .size(34.0, 34.0)
                .radius(9.0)
                .fill(t.text.alpha(0.06))
                .center()
                .child(kit::glyph("ready-icon", &icons_settings::SYNC, 17.0, t.text_secondary)),
        )
        .child(
            col()
                .key("ready-words")
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label("ready-title", title, CARD_TITLE, t.text))
                .child(kit::label("ready-text", words, CARD_TEXT, t.text_secondary).truncate()),
        )
        .child(count);
    // The filled part and the rest share the bar's width by their percents.
    let bar = row()
        .key("ready-bar")
        .role("progressbar")
        .label(format!("{title}: {percent} percent"))
        .w_full()
        .h(3.0)
        .radius(1.5)
        .clip_rounded(1.5)
        .fill(t.separator)
        .child(
            el().key("ready-bar-fill")
                .h_full()
                .grow(percent as f32)
                .radius(1.5)
                .fill(t.text),
        )
        .child(el().key("ready-bar-rest").h_full().grow((100 - percent) as f32));
    Some(
        col()
            .key("ready-card")
            .role("status")
            .label(format!(
                "{title}. {words} {figure}{}. File list: {}. Thumbnails: {}.",
                left.unwrap_or_default(),
                loading.file_list.words(),
                loading.thumbnails.words()
            ))
            .mt(12.0)
            .mx(16.0)
            .padding(16.0)
            .gap(10.0)
            .radius(12.0)
            .fill(t.card)
            .box_shadow(kit::ring(kit::HAIR, t.separator))
            .child(head)
            .child(bar)
            .child(
                row()
                    .key("ready-steps")
                    .gap(24.0)
                    .child(step("list", "File list", loading.file_list.words()))
                    .child(step("thumbs", "Thumbnails", loading.thumbnails.words())),
            ),
    )
}

/// The rows the list holds so far during the first download: the first seven of the board's.
pub struct ListedSoFar(pub super::source::BoardDrive);

impl crate::source::DriveSource for ListedSoFar {
    fn list(&self, folder_id: Option<&str>) -> Result<crate::source::Listing, crate::source::SourceError> {
        let mut listing = self.0.list(folder_id)?;
        if folder_id.is_none() {
            listing.items.truncate(7);
        }
        Ok(listing)
    }

    fn storage(&self) -> Option<crate::source::StorageSummary> {
        self.0.storage()
    }
}

/// The board frames of the two states.
pub const SCENES: &[crate::ui::gate::Scene<State>] = &[
    // DW-1108: no connection; the list is from today at 09:12.
    ("DW-1108", |s| {
        use crate::frames::global::{GlobalFixture, Ui, source::board};
        s.clear_selection();
        s.show_detail_pane = false;
        s.global = Ui::new(Box::new(GlobalFixture::of(board::offline())));
    }),
    // DW-1107: the first sign-in on this PC: 18,240 of 40,112 listed.
    ("DW-1107", |s| {
        use crate::frames::global::{GlobalFixture, Ui, source::board};
        s.source = Box::new(ListedSoFar(super::source::BoardDrive::new()));
        (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
        s.clear_selection();
        s.show_detail_pane = false;
        s.global = Ui::new(Box::new(GlobalFixture::of(board::loading())));
    }),
    // DW-1102: Lisbon trip keeps its own view (a grid), the details panel on Alfama.heic.
    ("DW-1102", |s| {
        let grid = super::long_list::SCENES.iter().find(|(id, _)| *id == "DW-0421");
        if let Some((_, grid)) = grid {
            grid(s);
        }
        s.show_detail_pane = true;
    }),
];
