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
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use chrono::Timelike;
use rune::{Align, Color, Cx, El, Icon, col, el, row};
use std::borrow::Cow;

use super::kit;

/// `.mk-row { height: 30px }`
pub const ROW_H: f32 = 30.0;
const COLUMN_HEADER_H: f32 = 28.0;
const COL_DATE_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;
const COL_SHARED_W: f32 = 92.0;
const COL_PC_W: f32 = 34.0;
const COL_GAP: f32 = 12.0;

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
    let tile = el()
        .key(format!("{key}-tile"))
        .size(22.0, 22.0)
        .radius(6.0)
        .clip_rounded(6.0)
        .center();
    if item.has_thumbnail {
        return thumbnail(tile);
    }
    tile.fill(t.tile).child(kit::glyph(
        format!("{key}-icon"),
        file_type_icon(item),
        14.0,
        t.text_secondary,
    ))
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

/// The board draws one outlined glyph per kind, all in ink-2 (no per-type colour in the
/// desktop list): a folder, the Backups drive, a sheet, a deck or media file, a document.
pub fn file_type_icon(item: &DriveItem) -> &'static Icon {
    if item.is_folder {
        return if item.is_protected {
            &icons::BACKUPS
        } else {
            &icons::FILES
        };
    }
    match item.mime.as_deref() {
        Some(m) if m.contains("spreadsheet") || m.contains("excel") || m == "text/csv" => &icons::DOC_SPREADSHEET,
        Some(m) if m.contains("presentation") || m.contains("keynote") || m.contains("powerpoint") => {
            &icons::DOC_MINIMAL
        }
        Some(m) if m.starts_with("image/") || m.starts_with("video/") || m.starts_with("audio/") => &icons::DOC_MINIMAL,
        _ => &icons::DOC_GENERIC,
    }
}

/// A header cell's label.
fn head(key: &'static str, label: &'static str, t: &Theme) -> El {
    kit::label(key, label, type_::COLUMN_HEADER, t.text_tertiary)
}

/// Column headers (`.mk-thead`).
pub fn column_headers(t: &Theme) -> El {
    row()
        .key("column-headers")
        .role("row")
        .h(COLUMN_HEADER_H)
        .w_full()
        .mt(4.0)
        .px(20.0)
        .gap(COL_GAP)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("col-name-cell")
                .role("columnheader")
                .label("Name")
                .sorted(Some(true))
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .gap(3.0)
                .child(head("col-name", "Name", t))
                .child(kit::glyph("sort-arrow", &icons::CHEVRON_DOWN, 11.0, t.text_tertiary)),
        )
        .child(
            row()
                .key("col-date-cell")
                .role("columnheader")
                .w(COL_DATE_W)
                .center_y()
                .child(head("col-date", "Date changed", t)),
        )
        .child(
            row()
                .key("col-size-cell")
                .role("columnheader")
                .w(COL_SIZE_W)
                .center_y()
                .justify_end()
                .child(head("col-size", "Size", t)),
        )
        .child(
            row()
                .key("col-shared-cell")
                .role("columnheader")
                .w(COL_SHARED_W)
                .center_y()
                .child(head("col-shared", "Shared", t)),
        )
        .child(
            row()
                .key("col-pc-cell")
                .role("columnheader")
                .label("On this PC")
                .w(COL_PC_W)
                .center()
                .child(head("col-pc", "PC", t)),
        )
}

/// The rows, virtualised (`.mk-rows`).
pub fn file_list(s: &State, _cx: &mut Cx<State>, t: &Theme) -> El {
    let count = s.listing.items.len();
    let t = *t;

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
                        Some(item) => file_row(i, item, s.selected == Some(i), &t)
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
pub fn file_row(i: usize, item: &DriveItem, selected: bool, t: &Theme) -> El {
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    // What the row sits on, for the rings and cut-outs drawn in the surface's colour.
    let surface = t.bg;

    let mut name = row()
        .key("name-cell")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(8.0)
        .child(file_tile(&k, item, t));
    name = name.child(
        kit::label(format!("{k}-name"), item.name.clone(), type_::LIST_ITEM, t.text)
            .truncate()
            .min_w(0.0),
    );
    if item.is_protected {
        name = name.child(
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

    let mut pc = el().key("pc-cell").w(COL_PC_W).center();
    if let Some(state) = PcState::of(item.pc_status.as_deref()) {
        pc = pc.child(pc_glyph(format!("{k}-pc"), state, surface, t));
    }

    row()
        .key(k.clone())
        .role("listitem")
        .label(&item.name)
        .h(ROW_H)
        .w_full()
        .px(14.0)
        .gap(COL_GAP)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if selected { t.selection_row } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .focus_selected(selected)
        .child(name)
        .child(
            kit::label(
                format!("{k}-date"),
                format_date(&item.modified_at),
                type_::META_MONO,
                meta,
            )
            .w(COL_DATE_W)
            .truncate(),
        )
        .child(
            kit::label(format!("{k}-size"), size_text, type_::META_MONO, meta)
                .w(COL_SIZE_W)
                .align(Align::End),
        )
        .child(shared_cell(&k, item, meta, surface, t))
        .child(pc)
}

/// The Shared column: who it is shared with, "Link", or a dash.
fn shared_cell(k: &str, item: &DriveItem, meta: Color, surface: Color, t: &Theme) -> El {
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
/// last week, else "28 Oct 2026".
pub fn format_date(iso: &str) -> String {
    use chrono::{Datelike, Duration, NaiveDateTime, Weekday};

    let Ok(dt) = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%SZ")
        .or_else(|_| NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S"))
    else {
        return String::new();
    };
    // The fixture's "now" (5 Nov 2026); the engine's source brings its own clock.
    let Ok(now) = NaiveDateTime::parse_from_str("2026-11-05T12:00:00", "%Y-%m-%dT%H:%M:%S") else {
        return String::new();
    };

    let (date, today) = (dt.date(), now.date());
    let time = format!("{:02}:{:02}", dt.hour(), dt.minute());
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
        format!("{} {} {}", date.day(), MONTHS[date.month0() as usize], date.year())
    }
}
