//! The board frames of Sync history and Available offline as states of a fresh `State`
//! (`--scene DW-1301`, the frame gate).

use rune::TextEdit;

use super::source::FixtureSource;
use super::{Filter, RowMenu, Section, Ui};
use crate::State;
use crate::shell::app::{Event, Route};

fn open(s: &mut State, section: Section, source: FixtureSource) {
    crate::shell::host::dispatch(s, Event::Navigate(Route::section(section)));
    s.this_pc = Ui::sample(source);
    s.this_pc.first_place = true;
}

fn history(s: &mut State, filter: Filter) {
    open(s, Section::SyncHistory, FixtureSource::board());
    s.this_pc.filter = filter;
}

pub fn dw_1301(s: &mut State) {
    history(s, Filter::All);
}

pub fn dw_1302(s: &mut State) {
    history(s, Filter::ThisPc);
}

pub fn dw_1303(s: &mut State) {
    history(s, Filter::OtherDevices);
}

pub fn dw_0428(s: &mut State) {
    history(s, Filter::Problems);
}

/// "contract" in Search history.
pub fn dw_1304(s: &mut State) {
    history(s, Filter::All);
    s.this_pc.search = TextEdit::new("contract");
}

pub fn dw_1305(s: &mut State) {
    open(s, Section::SyncHistory, FixtureSource::just_signed_in());
}

pub fn dw_0715(s: &mut State) {
    open(s, Section::AvailableOffline, FixtureSource::synced());
}

/// The menu on Invoices 2026, Remove from this PC lit.
pub fn dw_1306(s: &mut State) {
    dw_0715(s);
    s.this_pc.selected = Some(1);
    s.this_pc.menu = Some(RowMenu {
        row: 1,
        at: (430.0, 222.0),
        hot: 2,
    });
}

pub fn dw_0120(s: &mut State) {
    open(s, Section::AvailableOffline, FixtureSource::nothing_kept());
}

/// The group's board frames, by frame id (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1301", dw_1301),
    ("DW-1302", dw_1302),
    ("DW-1303", dw_1303),
    ("DW-0428", dw_0428),
    ("DW-1304", dw_1304),
    ("DW-1305", dw_1305),
    ("DW-0715", dw_0715),
    ("DW-1306", dw_1306),
    ("DW-0120", dw_0120),
];
