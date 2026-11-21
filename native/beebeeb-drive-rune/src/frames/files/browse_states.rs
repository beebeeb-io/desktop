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
use crate::theme::tokens::Theme;
use crate::ui::kit;
use crate::ui::status_bar::{Dot, bar};
use crate::ui::view_state::{ViewState, view_state};
use rune::El;

/// DW-1108 against DW-0101 and DW-0121 (the same state, two pictures on the board): offline, does
/// the list itself say so (rows that need a connection dimmed and worded, the status line
/// "Offline")? `false` is the other answer: only the banner says it, as DW-0101 draws.
pub const LIST_SAYS_OFFLINE: bool = true;

/// Is there no connection (the engine says so)?
pub fn offline(s: &State) -> bool {
    LIST_SAYS_OFFLINE && s.global.source.status().offline.is_some()
}

/// Can this row not be opened now: a file that is only online, with no connection?
pub fn needs_connection(s: &State, item: &DriveItem) -> bool {
    !item.is_folder && item.pc_status.as_deref() == Some("online_only") && offline(s)
}

fn loading(s: &State) -> Option<Loading> {
    let status = s.global.source.status();
    // Once the card has given way to the strip, the window counts its own way again (DW-0118).
    if status.locked || status.signed_out || s.actions.first_load_strip {
        return None;
    }
    status.loading
}

fn listed_words(loading: &Loading) -> String {
    match figure(loading) {
        Some(figure) => format!("{figure} listed"),
        // w31: the list's size is not known yet.
        None => crate::frames::global::source::LIST_ON_ITS_WAY.to_string(),
    }
}

fn figure(loading: &Loading) -> Option<String> {
    loading.figure(|n| super::protected::grouped(n as i64))
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

/// The status line of these states: "Getting your files ready" while the first list loads,
/// "Offline" without a connection (whatever the sync state's own word is). `None`: neither.
pub fn status_line(s: &State, details: &str, t: &Theme) -> Option<El> {
    if loading(s).is_some() {
        return Some(bar(Some(Dot::Busy), Some("Getting your files ready"), details, t));
    }
    offline(s).then(|| bar(Some(Dot::Idle), Some("Offline"), details, t))
}

/// The card above the rows while the first list loads (DW-1107): `ui::view_state` Loading, its words the
/// title and how far the list is. `None`: nothing loads, the person already opened something (the strip
/// takes over), or a banner has the place.
pub fn card(s: &State, t: &Theme) -> Option<El> {
    let loading = loading(s)?;
    if s.actions.first_load_strip || s.global.banner().is_some() {
        return None;
    }
    // w31: while the list's size is not known yet the card stands with no count ("0 of 0" is no count).
    let words = match figure(&loading) {
        Some(figure) => format!(
            "Getting your files ready: {figure}{}",
            loading
                .left
                .as_ref()
                .map(|left| format!(" \u{00B7} {left} left"))
                .unwrap_or_default()
        ),
        None => "Getting your files ready".to_string(),
    };
    Some(view_state(ViewState::Loading { words: &words }, t))
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
