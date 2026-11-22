//! File list (`.mk-listwrap`): the column headers and the rows.
//!
//! Columns (`--mk-cols`): Name (takes the rest) · Date changed 132 · Size 84, right-aligned ·
//! Shared 92 · PC 34, centred; 12 pt between columns. Sizes and times are JetBrains Mono.
//!
//! The PC column has three states, each its own glyph (frames.json DW-0401: "online only,
//! downloaded, available offline: the same states File Explorer shows"):
//! `online-only` a cloud · `downloaded` an outlined check · `available-offline` a solid
//! disc with a down arrow.

use crate::State;
use crate::source::DriveItem;
use crate::theme::file_icons::icon_for_item;
use crate::theme::icons;
use crate::theme::tokens::{Theme, type_};
use chrono::Timelike;
use rune::{Align, Color, Cx, El, Icon, col, el, row};
use std::borrow::Cow;

use super::file_row::{self, Column, RowLook, Width};
use super::kit;
use super::view_state::{ViewState, try_again, view_state};

/// `.mk-row { height: 30px }`
pub const ROW_H: f32 = file_row::ROW_H;
const COL_DATE_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;
const COL_SHARED_W: f32 = 92.0;
const COL_PC_W: f32 = 34.0;

/// The list's columns, in order.
pub const COLUMNS: [Column; 5] = [
    Column::new("name", "Name", Width::Flex(1.0)),
    Column::new("date", "Date changed", Width::Fixed(COL_DATE_W)),
    Column::new("size", "Size", Width::Fixed(COL_SIZE_W)).align(Align::End),
    Column::new("shared", "Shared", Width::Fixed(COL_SHARED_W)),
    Column::new("pc", "PC", Width::Fixed(COL_PC_W))
        .align(Align::Center)
        .spoken("On this PC"),
];

/// A row's name never narrower than this: about 24 characters of its 13 pt name (7 pt each) beside its 22 pt tile
/// and the 8 pt between them. Below it the fixed columns give way, Shared first, then PC (RC2-10: at 125 % with
/// the details open the name was 9 characters, "pho...png").
pub const NAME_MIN_W: f32 = 22.0 + 8.0 + 24.0 * 7.0;
/// From the list's edge to a row's cells, both sides: the list's 6 pt and the row's 14 pt (the headers' 20 pt).
const ROW_INSET: f32 = 2.0 * (6.0 + 14.0);
/// `file_row::COL_GAP`: between two columns.
const COL_GAP: f32 = 12.0;

/// Which of the columns that may give way the list has room for, by the list's own width.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fit {
    pub shared: bool,
    pub pc: bool,
}

impl Fit {
    /// Every column (the board's).
    pub const ALL: Fit = Fit { shared: true, pc: true };

    /// The columns a list `list_w` wide shows (`None`: not laid out yet, every column): Shared gives way when
    /// the name would be narrower than [`NAME_MIN_W`], then PC. Date and Size always stay.
    pub fn of(list_w: Option<f32>) -> Fit {
        let Some(w) = list_w.filter(|w| *w > 0.0) else {
            return Fit::ALL;
        };
        [
            Fit::ALL,
            Fit {
                shared: false,
                pc: true,
            },
        ]
        .into_iter()
        .find(|fit| fit.name_w(w) >= NAME_MIN_W)
        .unwrap_or(Fit {
            shared: false,
            pc: false,
        })
    }

    /// The width of a list's `rows` (`Cx::layout_size`, the last frame's), as [`Fit::of`] reads it.
    pub fn of_list<S>(cx: &Cx<S>) -> Fit {
        Fit::of(cx.layout_size("rows").map(|[w, _]| w))
    }

    /// What the Name column has in a list `list_w` wide.
    pub fn name_w(self, list_w: f32) -> f32 {
        let mut fixed = COL_DATE_W + COL_SIZE_W + 2.0 * COL_GAP;
        if self.shared {
            fixed += COL_SHARED_W + COL_GAP;
        }
        if self.pc {
            fixed += COL_PC_W + COL_GAP;
        }
        list_w - ROW_INSET - fixed
    }

    /// [`COLUMNS`] without the ones that gave way; the ones kept keep their order and index.
    pub fn columns(self) -> Vec<Column> {
        COLUMNS
            .iter()
            .filter(|c| match c.key {
                "shared" => self.shared,
                "pc" => self.pc,
                _ => true,
            })
            .copied()
            .collect()
    }
}

/// Where an item is, in the PC column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PcState {
    OnlineOnly,
    Downloaded,
    AvailableOffline,
}

impl PcState {
    /// From the data contract's `pc_status`.
    pub fn of(status: Option<&str>) -> Option<PcState> {
        match status {
            Some("online_only") => Some(PcState::OnlineOnly),
            Some("on_this_pc") => Some(PcState::Downloaded),
            Some("available_offline") => Some(PcState::AvailableOffline),
            _ => None,
        }
    }

    /// The state's name in the gate and in frames.json.
    pub fn id(self) -> &'static str {
        match self {
            PcState::OnlineOnly => "online-only",
            PcState::Downloaded => "downloaded",
            PcState::AvailableOffline => "available-offline",
        }
    }

    /// What Narrator reads (the board's tooltip).
    pub fn name(self) -> &'static str {
        match self {
            PcState::OnlineOnly => "Online only",
            PcState::Downloaded => "Downloaded",
            PcState::AvailableOffline => "Available offline",
        }
    }

    pub fn icon(self) -> &'static Icon {
        match self {
            PcState::OnlineOnly => &icons::CLOUD,
            PcState::Downloaded => &icons::CHECK_CIRCLE,
            PcState::AvailableOffline => &icons::AVAILABLE_OFFLINE,
        }
    }
}

/// The PC column's glyph for a state (`.mk-mac`), 15 pt. `surface` is what it sits on: the
/// kept state's arrow is cut out in that colour.
pub fn pc_glyph(key: impl Into<String>, state: PcState, surface: Color, t: &Theme) -> El {
    match state {
        PcState::AvailableOffline => kit::state_glyph(
            key,
            state.icon(),
            Some((&icons::AVAILABLE_OFFLINE_DISC, &icons::AVAILABLE_OFFLINE_ARROW, surface)),
            state.id(),
            15.0,
            t.text,
        ),
        _ => kit::state_glyph(key, state.icon(), None, state.id(), 15.0, t.text_tertiary),
    }
    .role("image")
    .label(state.name())
}

/// A file's tile (`.mk-ftile`): 22 pt, the kind's outlined glyph on a neutral wash, or the
/// file's thumbnail. Keys: `<key>-tile`, `<key>-icon`.
pub fn file_tile(key: &str, item: &DriveItem, t: &Theme) -> El {
    if item.has_thumbnail {
        return thumbnail(file_row::tile_frame(key));
    }
    file_row::icon_tile(key, icon_for_item(item), t)
}

/// A thumbnail's stand-in until the engine hands over pixels: the board's sample photo
/// (`.mk-ph5`, a harbour at dusk).
pub fn thumbnail(tile: El) -> El {
    let stop = |r: u8, g: u8, b: u8| rune::color::hex(&format!("#{r:02x}{g:02x}{b:02x}")).unwrap_or(Color::BLACK);
    tile.linear_gradient(
        std::f32::consts::PI,
        [
            (0.0, stop(243, 217, 168)),
            (0.5, stop(213, 140, 106)),
            (1.0, stop(63, 58, 88)),
        ],
    )
}

/// The registry's art for an item (folder, Backups root, or its file type).
#[cfg(test)]
pub fn file_type_icon(item: &DriveItem) -> &'static Icon {
    crate::theme::file_icon_art::art(icon_for_item(item).id)
}

/// Column headers (`.mk-thead`), sorted by name, of the columns the list has room for.
pub fn column_headers(fit: Fit, t: &Theme) -> El {
    file_row::header(&fit.columns(), Some((0, true)), t)
}

/// In place of the rows when the folder could not be read: `ui::view_state` Failed with the reason and a
/// Try again that reads the folder again ([`State::retry_list`]). Keys: `file-list-error`, `-title`, `-text`,
/// `-retry`.
pub fn list_error(error: crate::source::SourceError, t: &Theme) -> El {
    let title = match error {
        crate::source::SourceError::NotFound => "This folder can\u{2019}t be opened",
        _ => "Your files can\u{2019}t be read",
    };
    view_state(
        ViewState::Failed {
            key: "file-list-error",
            title,
            why: error.message(),
            retry: Some(try_again("file-list-error", t).on_press(|s: &mut State| s.retry_list())),
        },
        t,
    )
    .grow(1.0)
    .min_h(0.0)
    .w_full()
}

impl State {
    /// Try again on a list that could not be read: the engine is asked for the file list now, and the folder
    /// shown is read again. A read that works clears the failure; one that fails again says so again.
    pub fn retry_list(&mut self) {
        self.global.source.retry_list();
        let id = self.listing.folder.id.clone();
        (self.listing, self.list_error) = crate::load(self.source.as_ref(), id.as_deref());
    }
}

/// The rows, virtualised (`.mk-rows`).
pub fn file_list(s: &State, cx: &mut Cx<State>, t: &Theme) -> El {
    let count = s.listing.items.len();
    let t = *t;
    let fit = Fit::of_list(cx);

    col()
        .key("rows")
        .role("list")
        .label(s.listing.folder.name.clone())
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .py(4.0)
        .px(6.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(
            col()
                .key("file-list")
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    count,
                    ROW_H,
                    |s: &State, i| {
                        if i < s.listing.items.len() {
                            Cow::Owned(format!("row-{i}"))
                        } else {
                            Cow::Borrowed("row-oob")
                        }
                    },
                    move |s: &State, i| match s.listing.items.get(i) {
                        Some(item) => file_row(i, item, s.selected == Some(i), fit, &t)
                            .on_press(move |s: &mut State| s.select(i))
                            .on_double_press(move |s: &mut State| {
                                s.select(i);
                                s.open_selected();
                            }),
                        None => el().key("empty"),
                    },
                ),
        )
}

/// One row (`.mk-row`). Keys: `row-<i>` and `row-<i>-tile|icon|name|lock|date|size|dash|link|avatar-<j>|pc`.
/// `fit`: the columns the list has room for.
pub fn file_row(i: usize, item: &DriveItem, selected: bool, fit: Fit, t: &Theme) -> El {
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    // What the row sits on, for the rings and cut-outs drawn in the surface's colour.
    let surface = t.bg;

    let mut marks = Vec::new();
    if item.is_protected {
        marks.push(
            kit::glyph(format!("{k}-lock"), &icons::LOCK, 11.0, t.text_tertiary)
                .ml(2.0)
                .role("image")
                .label("Protected"),
        );
    }

    let size_text = if item.is_folder {
        // The protected Backups root lists devices, not items (board: "4 devices").
        let unit = if item.is_protected { "devices" } else { "items" };
        item.child_count.map(|c| format!("{c} {unit}")).unwrap_or_default()
    } else {
        item.size.map(crate::source::format_size).unwrap_or_default()
    };

    let mut cells = vec![
        kit::label(
            format!("{k}-date"),
            format_date(&item.modified_at),
            type_::META_MONO,
            meta,
        )
        .w(COL_DATE_W)
        .truncate(),
        kit::label(format!("{k}-size"), size_text, type_::META_MONO, meta)
            .w(COL_SIZE_W)
            .align(Align::End),
    ];
    if fit.shared {
        cells.push(shared_cell(&k, item, meta, surface, t));
    }
    if fit.pc {
        let mut pc = el().key("pc-cell").w(COL_PC_W).center();
        if let Some(state) = PcState::of(item.pc_status.as_deref()) {
            pc = pc.child(pc_glyph(format!("{k}-pc"), state, surface, t));
        }
        cells.push(pc);
    }
    let look = RowLook {
        selected,
        ..RowLook::default()
    };
    file_row::row_with(
        &k,
        &item.name,
        file_tile(&k, item, t),
        &item.name,
        marks,
        cells,
        look,
        t,
    )
}

/// The Shared column: who it is shared with, "Link", or a dash.
pub fn shared_cell(k: &str, item: &DriveItem, meta: Color, surface: Color, t: &Theme) -> El {
    let mut cell = row().key("shared-cell").w(COL_SHARED_W).min_w(0.0).center_y();
    if !item.shared_badges.is_empty() {
        let mut stack = row().key("avatars").role("group").label("Shared with");
        for (j, badge) in item.shared_badges.iter().enumerate() {
            let mut a = kit::avatar(
                format!("{k}-avatar-{j}"),
                &badge.initials,
                20.0,
                badge.color,
                surface,
                t,
            );
            if j > 0 {
                a = a.ml(-7.0);
            }
            stack = stack.child(a);
        }
        cell = cell.child(stack);
    } else if item.shared_type.as_deref() == Some("Link") {
        cell = cell.child(
            row()
                .key("link")
                .center_y()
                .gap(4.0)
                .child(kit::glyph(
                    format!("{k}-link-icon"),
                    &icons::LINK,
                    12.0,
                    t.text_secondary,
                ))
                .child(kit::label(
                    format!("{k}-link"),
                    "Link",
                    type_::STATUS_STRONG.lh(14.0),
                    t.text_secondary,
                )),
        );
    } else {
        cell = cell.child(kit::label(format!("{k}-dash"), "\u{2014}", type_::META_MONO, meta));
    }
    cell
}

/// A date as the board shows it: "Today, 07:02", "Yesterday, 16:42", "Sunday" within the
/// last week, else "28 Oct 2026". One rule for every list and row, in this PC's local time: a
/// UTC stamp (`...Z`) is moved to local time and compared with the local date of now.
pub fn format_date(iso: &str) -> String {
    use chrono::{Local, NaiveDateTime, TimeZone, Utc};

    let local = if let Ok(utc) = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%SZ") {
        Utc.from_utc_datetime(&utc).with_timezone(&Local).naive_local()
    } else if let Ok(naive) = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S") {
        naive
    } else {
        return String::new();
    };
    format_date_at(local, Local::now().naive_local())
}

/// A fixture stamp for the board's mock "now" (5 Nov 2026), moved onto this PC's local clock so
/// the board's wording holds on any day: 4 and 5 Nov become yesterday and today (same time of
/// day, no zone); older stamps in the board's last week (30 Oct to 3 Nov) keep their weekday
/// ("Sunday") on the latest such day at least 2 days back. Every other stamp is unchanged.
pub fn board_stamp(iso: &str) -> String {
    board_stamp_at(iso, chrono::Local::now().date_naive())
}

/// [`board_stamp`] against a given local date (testable).
pub fn board_stamp_at(iso: &str, today: chrono::NaiveDate) -> String {
    use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};

    let Ok(at) = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%SZ") else {
        return iso.to_string();
    };
    let board_today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
    let days_back = (board_today - at.date()).num_days();
    let shifted = match days_back {
        0 | 1 => at + (today - board_today),
        2..=6 => {
            let mut d = today - Duration::days(2);
            while d.weekday() != at.date().weekday() {
                d -= Duration::days(1);
            }
            d.and_time(at.time())
        }
        _ => return iso.to_string(),
    };
    shifted.format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// [`format_date`] for a local time against a local "now" (the rule, testable), in this PC's regional format.
pub fn format_date_at(dt: chrono::NaiveDateTime, now: chrono::NaiveDateTime) -> String {
    format_date_in(dt, now, super::region::current())
}

/// [`format_date_at`] in `region`: its time is 12- or 24-hour ("5:19 PM", "17:19") and a date beyond the last
/// week is in its order ("28 Oct 2026", "Oct 28, 2026", "2026 Oct 28"). The words stay English.
pub fn format_date_in(dt: chrono::NaiveDateTime, now: chrono::NaiveDateTime, region: super::region::Region) -> String {
    use super::region::DateOrder;
    use chrono::{Datelike, Duration, Weekday};

    let (date, today) = (dt.date(), now.date());
    let time = if region.hour12 {
        let (pm, hour) = dt.hour12();
        format!("{hour}:{:02} {}", dt.minute(), if pm { "PM" } else { "AM" })
    } else {
        format!("{:02}:{:02}", dt.hour(), dt.minute())
    };
    let days_ago = (today - date).num_days();
    if days_ago == 0 {
        format!("Today, {time}")
    } else if date == today - Duration::days(1) {
        format!("Yesterday, {time}")
    } else if (2..=6).contains(&days_ago) {
        match date.weekday() {
            Weekday::Mon => "Monday",
            Weekday::Tue => "Tuesday",
            Weekday::Wed => "Wednesday",
            Weekday::Thu => "Thursday",
            Weekday::Fri => "Friday",
            Weekday::Sat => "Saturday",
            Weekday::Sun => "Sunday",
        }
        .to_string()
    } else {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let (day, month, year) = (date.day(), MONTHS[date.month0() as usize], date.year());
        match region.date_order {
            DateOrder::Dmy => format!("{day} {month} {year}"),
            DateOrder::Mdy => format!("{month} {day}, {year}"),
            DateOrder::Ymd => format!("{year} {month} {day}"),
        }
    }
}

#[cfg(test)]
#[path = "file_list_tests.rs"]
mod tests;
