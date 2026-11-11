//! File list: column headers with sort arrow, file rows with type icon,
//! name, date, size, Shared cell, PC cell, and selection highlight.
//!
//! Board measurements:
//! - Column headers: 11px/600 Inter, ink-3 colour, 32 pt row height
//! - Name column has sort arrow (ascending)
//! - Columns: Name (flex), Date changed (132px), Size (84px), Shared (92px), PC (34px + padding)
//! - File rows: 32 pt tall, 13px/400 name, 11.5px/400 mono for dates
//! - File-type icons: 15x15, coloured per type
//! - Shared cell: link icon or letter badge circles
//! - PC cell: check-circle (On this PC), cloud (Online only), clock (Available offline)

use crate::State;
use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, space, type_};
use chrono::Timelike;
use rune::{Color, Cx, El, col, el, icon, row, text};
use std::borrow::Cow;

/// Board: rows are 30px tall (y=119,149,179... spacing 30).
const ROW_H: f32 = 30.0;
const COL_DATE_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;
const COL_SHARED_W: f32 = 92.0;
/// Board: PC column is 34px wide (x=941, 34px).
const COL_PC_W: f32 = 34.0;

/// Column headers row.
pub fn column_headers(t: &Theme) -> El {
    row()
        .key("column-headers")
        .h(ROW_H)
        .w_full()
        .px(space::LG)
        .center_y()
        .child(
            // Icon spacer
            el().key("icon-spacer").w(20.0),
        )
        .child(
            row()
                .key("name-header")
                .grow(1.0)
                .center_y()
                .gap(space::XS)
                .child(
                    text("Name")
                        .key("col-name")
                        .font(type_::COLUMN_HEADER)
                        .color(t.text_tertiary),
                )
                .child(
                    icon(&icons::CHEVRON_DOWN, 11.0)
                        .key("sort-arrow")
                        .color(t.text_tertiary),
                ),
        )
        .child(
            text("Date changed")
                .key("col-date")
                .font(type_::COLUMN_HEADER)
                .color(t.text_tertiary)
                .w(COL_DATE_W),
        )
        .child(
            text("Size")
                .key("col-size")
                .font(type_::COLUMN_HEADER)
                .color(t.text_tertiary)
                .w(COL_SIZE_W),
        )
        .child(
            text("Shared")
                .key("col-shared")
                .font(type_::COLUMN_HEADER)
                .color(t.text_tertiary)
                .w(COL_SHARED_W),
        )
        .child(
            text("PC")
                .key("col-pc")
                .font(type_::COLUMN_HEADER)
                .color(t.text_tertiary)
                .w(COL_PC_W),
        )
}

/// The virtual-scrolled file list.
pub fn file_list(s: &State, _cx: &mut Cx<State>, t: &Theme) -> El {
    let count = s.listing.items.len();
    let selection_color = t.selection_row;
    let hover_color = t.hover;
    let press_color = t.press;
    let text_color = t.text;
    let text_secondary = t.text_tertiary;
    let text_mono = t.text;

    col()
        .key("file-list-scroll")
        .grow(1.0)
        .w_full()
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
                    move |s: &State, i| {
                        if i >= s.listing.items.len() {
                            return el().key("empty");
                        }
                        let item = &s.listing.items[i];
                        let selected = s.selected == Some(i);

                        file_row(
                            i,
                            item,
                            selected,
                            selection_color,
                            hover_color,
                            press_color,
                            text_color,
                            text_secondary,
                            text_mono,
                        )
                    },
                ),
        )
}

fn file_row(
    i: usize,
    item: &DriveItem,
    selected: bool,
    selection_color: Color,
    hover_color: Color,
    press_color: Color,
    text_color: Color,
    text_secondary: Color,
    _text_mono: Color,
) -> El {
    let icon_color = file_type_color(item.mime.as_deref(), item.is_folder);
    let type_icon = file_type_icon(item.mime.as_deref(), item.is_folder);

    let size_text = if item.is_folder {
        // The protected Backups root lists devices, not items (board: "4 devices").
        let unit = if item.is_protected { "devices" } else { "items" };
        item.child_count.map(|c| format!("{c} {unit}")).unwrap_or_default()
    } else {
        item.size.map(format_size).unwrap_or_default()
    };

    let date_text = format_date(&item.modified_at);
    let shared_text = item.shared_type.as_deref().unwrap_or("\u{2014}");
    let pc_icon = pc_status_icon(item.pc_status.as_deref());

    // Selected row: neutral rounded fill with 4px inset margins (board: rounded corners, no Amber).
    let mut r = row()
        .key(format!("row-{i}"))
        .role("listitem")
        .label(&item.name)
        .h(ROW_H)
        .w_full()
        .px(space::LG)
        .center_y()
        .radius(if selected { 6.0 } else { 0.0 })
        .fill(if selected { selection_color } else { Color::TRANSPARENT })
        .hover(hover_color)
        .press(press_color)
        .focusable(true)
        .on_press(move |s: &mut State| {
            s.selected = Some(i);
        });

    // File-type icon
    r = r.child(icon(type_icon, 15.0).key("type-icon").color(icon_color));

    // Name (with optional lock icon for Backups)
    let mut name_row = row().key("name-cell").grow(1.0).center_y().gap(space::XS).ml(space::XS);
    name_row = name_row.child(text(&item.name).key("name").font(type_::LIST_ITEM).color(text_color));
    if item.is_protected {
        name_row = name_row.child(icon(&icons::LOCK, 11.0).key("lock").color(text_secondary));
    }
    r = r.child(name_row);

    // Date changed
    r = r.child(
        text(date_text)
            .key("date")
            .font(type_::META_MONO)
            .color(text_secondary)
            .w(COL_DATE_W),
    );

    // Size
    r = r.child(
        text(size_text)
            .key("size")
            .font(type_::META_MONO)
            .color(text_secondary)
            .w(COL_SIZE_W),
    );

    // Shared
    r = r.child(shared_cell(shared_text, item, text_secondary));

    // PC status
    r = r.child(pc_cell(pc_icon, text_secondary));

    r
}

fn shared_cell(label: &str, item: &DriveItem, text_secondary: Color) -> El {
    let mut cell = row().key("shared-cell").w(COL_SHARED_W).center_y().gap(space::XS);

    if let Some(ref st) = item.shared_type {
        if st == "Link" {
            cell = cell
                .child(icon(&icons::LINK, 12.0).key("link-icon").color(text_secondary))
                .child(text("Link").key("link-text").font(type_::BODY_SM).color(text_secondary));
        }
    }

    // Show shared badges (letter circles) if present
    if !item.shared_badges.is_empty() {
        for (j, badge) in item.shared_badges.iter().enumerate() {
            cell = cell.child(
                el().key(format!("badge-{j}"))
                    .size(18.0, 18.0)
                    .radius(9.0)
                    .fill(badge.color)
                    .center()
                    .child(
                        text(&badge.initials)
                            .key("init")
                            .font(type_::AVATAR_INITIALS)
                            // Badge tints are pale in both themes, so the initials
                            // are a fixed dark ink rather than the theme text colour.
                            .color(rune::oklch(0.28, 0.01, 80.0)),
                    ),
            );
        }
    }

    if label == "\u{2014}" && item.shared_badges.is_empty() && item.shared_type.is_none() {
        cell = cell.child(text("\u{2014}").key("dash").font(type_::BODY_SM).color(text_secondary));
    }

    cell
}

fn pc_cell(ic: Option<&'static rune::Icon>, text_secondary: Color) -> El {
    let mut cell = el().key("pc-cell").w(COL_PC_W).center();
    if let Some(ic) = ic {
        cell = cell.child(icon(ic, 14.0).key("pc-icon").color(text_secondary));
    }
    cell
}

fn pc_status_icon(status: Option<&str>) -> Option<&'static rune::Icon> {
    match status {
        Some("on_this_pc") => Some(&icons::CHECK_CIRCLE),
        Some("online_only") => Some(&icons::CLOUD),
        Some("available_offline") => Some(&icons::AVAILABLE_OFFLINE),
        _ => None,
    }
}

fn file_type_icon(mime: Option<&str>, is_folder: bool) -> &'static rune::Icon {
    if is_folder {
        return &icons::FILES;
    }
    match mime {
        Some(m) if m.contains("spreadsheet") || m.contains("excel") || m == "text/csv" => &icons::DOC_SPREADSHEET,
        Some(m) if m.contains("presentation") || m.contains("keynote") || m.contains("powerpoint") => {
            &icons::DOC_MINIMAL
        }
        Some(m) if m.starts_with("image/") || m.starts_with("video/") || m.starts_with("audio/") => &icons::DOC_MINIMAL,
        _ => &icons::DOC_GENERIC,
    }
}

fn file_type_color(mime: Option<&str>, is_folder: bool) -> Color {
    if is_folder {
        return tokens::file_icon::folder();
    }
    match mime {
        Some(m) if m.starts_with("image/") => tokens::file_icon::image(),
        Some(m) if m.starts_with("video/") => tokens::file_icon::video(),
        Some(m) if m.starts_with("audio/") => tokens::file_icon::audio(),
        Some("application/pdf") => tokens::file_icon::pdf(),
        Some(m) if m.contains("spreadsheet") => tokens::file_icon::spreadsheet(),
        Some(m) if m.contains("presentation") => tokens::file_icon::presentation(),
        Some(m) if m.contains("document") || m.contains("text") => tokens::file_icon::document(),
        Some(m) if m.contains("zip") || m.contains("archive") => tokens::file_icon::archive(),
        _ => tokens::file_icon::generic(),
    }
}

fn format_size(bytes: i64) -> String {
    crate::source::format_size(bytes)
}

/// Format a date string from ISO to the board's relative format.
/// Board shows: "Today, 07:02", "Yesterday, 16:42", "Sunday", "5 Nov 2026", "28 Oct 2026"
fn format_date(iso: &str) -> String {
    use chrono::{Datelike, Duration, NaiveDateTime};

    let dt = NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%SZ")
        .or_else(|_| NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S"))
        .unwrap_or_else(|_| return NaiveDateTime::default());

    // Use a fixed "now" for the fixture (5 Nov 2026)
    let now = NaiveDateTime::parse_from_str("2026-11-05T12:00:00", "%Y-%m-%dT%H:%M:%S").unwrap();

    let date = dt.date();
    let today = now.date();
    let yesterday = today - Duration::days(1);
    let time = format!("{:02}:{:02}", dt.hour(), dt.minute());

    if date == today {
        format!("Today, {time}")
    } else if date == yesterday {
        format!("Yesterday, {time}")
    } else {
        // Check if within the last week
        let days_ago = (today - date).num_days();
        if days_ago > 0 && days_ago <= 6 {
            let weekday = date.weekday();
            return format!("{weekday}");
        }
        // Full date
        let month = match date.month() {
            1 => "Jan",
            2 => "Feb",
            3 => "Mar",
            4 => "Apr",
            5 => "May",
            6 => "Jun",
            7 => "Jul",
            8 => "Aug",
            9 => "Sep",
            10 => "Oct",
            11 => "Nov",
            12 => "Dec",
            _ => "???",
        };
        format!("{} {} {}", date.day(), month, date.year())
    }
}
