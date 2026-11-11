//! The timeline (DW-0601): the toolbar's three buttons, the source pills and the devices' backup
//! status, the days and their tiles in virtual rows, the selection bar (DW-0602) and the empty
//! state (DW-0116).

use std::borrow::Cow;

use rune::{Color, El, ScrollSpec, col, el, row};

use super::chrome::{self, Bar, Tone, rgb, style};
use super::source::{Kind, Photo, Shape, Totals};
use super::{Intent, Menu, Ui, icons};
use crate::State;
use crate::theme::icons as shared;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;

/// The gap between tiles (`gap-[3px]`).
const GAP: f32 = 3.0;

/// 12430 as "12,430".
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A size as the board writes it: "216 GB", "42 MB", "30.9 MB"; `unit: false` leaves the unit out.
pub fn megabytes(bytes: u64, unit: bool) -> String {
    let (value, name) = if bytes >= 1_000_000_000 {
        (bytes as f64 / 1e9, "GB")
    } else {
        (bytes as f64 / 1e6, "MB")
    };
    let tenths = (value * 10.0).round() / 10.0;
    let figure = if tenths.fract() == 0.0 {
        format!("{tenths:.0}")
    } else {
        format!("{tenths:.1}")
    };
    if unit { format!("{figure} {name}") } else { figure }
}

/// 24 as "0:24".
pub fn length(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn count(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// "12,430 photos · 842 videos · 216 GB"; a part the source has no figure for is left out.
fn totals_line(totals: Totals) -> String {
    if totals.photos == 0 && totals.videos == 0 {
        return "No photos".into();
    }
    let mut parts = vec![count(totals.photos, "photo", "photos")];
    if totals.videos > 0 {
        parts.push(count(totals.videos, "video", "videos"));
    }
    if totals.bytes > 0 {
        parts.push(megabytes(totals.bytes, true));
    }
    parts.join(" \u{00B7} ")
}

pub fn bar(ui: &Ui, t: &Theme) -> Bar {
    let across = chrome::tbtn("ph-across", "Photos across", &shared::VIEW_GRID, 15.0, false, t)
        .expanded(ui.menu.is_some_and(|m| m.which == Menu::Across))
        .child(kit::glyph(
            "ph-across-caret",
            &shared::CHEVRON_DOWN,
            11.0,
            t.text_secondary,
        ))
        .on_press(|s: &mut State| s.photos.toggle_menu(Menu::Across));
    let sources = chrome::tbtn("ph-sources", "Photo sources", &icons::SOURCES, 15.0, false, t)
        .on_press(|s: &mut State| s.photos.open_sources());
    let select = chrome::tbtn("ph-select", "Select", &icons::CHECK, 15.0, false, t)
        .checked(ui.selecting)
        .on_press(|s: &mut State| {
            if s.photos.selecting {
                s.photos.end_selecting();
            } else {
                s.photos.start_selecting();
            }
        });
    Bar {
        title: "Photos".into(),
        sub: totals_line(ui.source.totals(ui.filter)),
        back: false,
        details: false,
        middle: row()
            .key("ph-tools")
            .gap(4.0)
            .center_y()
            .child(across)
            .child(sources)
            .child(select),
    }
}

fn pill(i: usize, name: &str, on: bool, device: Option<usize>, t: &Theme) -> El {
    el().key(format!("ph-pill-{i}"))
        .role("radio")
        .label(name)
        .checked(on)
        .h(26.0)
        .w(chrome::advance(name, style::CHIP) + 20.0)
        .radius(13.0)
        .center()
        .fill(if on { t.text } else { t.tile })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(move |s: &mut State| s.photos.set_filter(device))
        .child(kit::label(
            format!("ph-pill-{i}-label"),
            name,
            style::CHIP,
            if on { t.paper } else { t.text_secondary },
        ))
}

/// The bar under the toolbar: All and one pill per device that shows, and what the devices'
/// backups are doing.
fn head(ui: &Ui, t: &Theme) -> El {
    let mut pills = row()
        .key("ph-pills")
        .role("radiogroup")
        .label("Source")
        .gap(6.0)
        .center_y()
        .child(pill(0, "All", ui.filter.is_none(), None, t));
    let shown = ui.source.devices().into_iter().enumerate().filter(|(_, d)| d.shown);
    for (n, (device, d)) in shown.enumerate() {
        pills = pills.child(pill(n + 1, &d.name, ui.filter == Some(device), Some(device), t));
    }
    let mut head = row()
        .key("ph-head")
        .w_full()
        .px(20.0)
        .py(10.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(pills);
    if let Some(status) = ui.source.backup_status() {
        // One run after the strong one, as the board's text node is: " · last backup 09:12".
        let rest = format!("\u{00B7} {}", status.last);
        kit::note_text(
            "ph-backup-rest",
            "ph-backup-rest-run",
            &rest,
            style::SUB,
            t.text_tertiary,
        );
        head = head.child(
            row()
                .key("ph-backup")
                .role("status")
                .label(format!("{} \u{00B7} {}", status.now, status.last))
                .gap(8.0)
                .center_y()
                .child(kit::glyph("ph-backup-icon", &icons::BACKING_UP, 14.0, t.text_tertiary))
                .child(
                    row()
                        .key("ph-backup-words")
                        .child(
                            el().key("ph-backup-strong-box")
                                .w(chrome::advance(&status.now, style::SUB_STRONG))
                                .child(kit::label("ph-backup-strong", status.now, style::SUB_STRONG, t.text)),
                        )
                        .child(
                            rune::text(format!(" {rest}"))
                                .key("ph-backup-rest-run")
                                .font(style::SUB)
                                .color(t.text_tertiary),
                        ),
                ),
        );
    }
    head
}

/// The selection bar (`.mk-bulk`, DW-0602).
pub fn bulk(ui: &Ui, t: &Theme) -> El {
    let bytes: u64 = ui.selected.values().sum();
    let mut words = count(ui.selected.len() as u64, "photo", "photos");
    if bytes > 0 {
        words = format!("{words} \u{00B7} {}", megabytes(bytes, true));
    }
    let bg = if t.dark { rgb(32, 28, 23) } else { rgb(243, 242, 241) };
    let action = |i: usize, name: &str, ic: Option<&'static rune::Icon>, tone| {
        chrome::button(&format!("ph-bulk-btn-{i}"), name, ic, tone, 22.0, type_::BUTTON_SM, t)
    };
    row()
        .key("ph-bulk")
        .role("toolbar")
        .label("Selection")
        .w_full()
        .px(14.0)
        .py(6.0)
        .gap(8.0)
        .center_y()
        .fill(bg)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(kit::label("ph-bulk-count", words, style::BULK_STRONG, t.text))
        .child(
            kit::label("ph-bulk-all", "Select all", style::BULK, t.text_secondary)
                .spans(vec![rune::Span::new(0, "Select all".len()).underline(t.text_secondary)])
                .role("button")
                .focusable(true)
                .on_press(|s: &mut State| s.photos.select_all()),
        )
        .child(el().key("ph-bulk-spacer").grow(1.0))
        .child(
            action(0, "Share\u{2026}", Some(&shared::SHARE), Tone::Plain).on_press(|s: &mut State| {
                let ids = s.photos.selected.keys().copied().collect();
                s.photos.ask(Intent::ShareSeveral(ids));
            }),
        )
        .child(
            action(1, "Make available offline", Some(&icons::OFFLINE), Tone::Plain).on_press(|s: &mut State| {
                let ids = s.photos.selected.keys().copied().collect();
                s.photos.ask(Intent::KeepSeveral(ids));
            }),
        )
        .child(
            action(2, "Move\u{2026}", Some(&icons::FOLDER_MOVE), Tone::Plain).on_press(|s: &mut State| {
                let ids = s.photos.selected.keys().copied().collect();
                s.photos.ask(Intent::Move(ids));
            }),
        )
        .child(
            action(3, "Move to Trash", Some(&shared::TRASH), Tone::Plain)
                .on_press(|s: &mut State| s.photos.trash_selected()),
        )
        .child(action(4, "Done", None, Tone::Quiet).on_press(|s: &mut State| s.photos.end_selecting()))
}

fn tile(ui: &Ui, day: usize, index: usize, photo: Photo, t: &Theme) -> El {
    let key = format!("ph-tile-{day}-{index}");
    let white = Color::WHITE;
    let day_title = ui.days.get(day).map_or("", |d| d.title.as_str());
    let checked = ui.selected.contains_key(&photo.id);
    let side = ui.tile_side();
    let mut name = match photo.kind {
        Kind::Photo => format!("Photo, {day_title}"),
        Kind::Video(seconds) => format!("Video, {}, {day_title}", length(seconds)),
    };
    if photo.uploading {
        name.push_str(", still uploading");
    }
    let mut tile = el()
        .key(key.clone())
        .role(if ui.selecting { "checkbox" } else { "button" })
        .label(name)
        .size(side, side)
        .radius(3.0)
        .focusable(true)
        .on_press(move |s: &mut State| s.photos.press_tile(day, index))
        .on_double_press(move |s: &mut State| s.photos.open_tile(day, index));
    if ui.selecting {
        tile = tile.checked(checked);
    }
    // A thumbnail that is not there yet: the neutral tile, never a broken picture.
    tile = match ui.source.thumbnail(photo.id, Shape::Square) {
        Some(pixels) => tile.pixels(&pixels),
        None => tile.fill(t.tile),
    };
    if checked {
        tile = tile.child(
            el().key(format!("{key}-ring"))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(3.0)
                .border(3.0, t.text),
        );
    } else if !ui.selecting && ui.cursor == Some((day, index)) {
        tile = tile.outline(2.0, 1.0, t.text);
    }
    let badge = |left: bool| {
        row()
            .key(format!("{key}-badge"))
            .pin(left.then_some(5.0), None, (!left).then_some(5.0), Some(5.0))
            .h(18.0)
            .px(5.0)
            .gap(3.0)
            .center_y()
            .radius(6.0)
            .fill(Color::BLACK.alpha(0.55))
    };
    if photo.uploading {
        tile = tile.child(badge(true).child(kit::glyph(format!("{key}-badge-icon"), &icons::UPLOADING, 10.0, white)));
    } else if let Kind::Video(seconds) = photo.kind {
        tile = tile.child(
            badge(false)
                .child(kit::glyph(format!("{key}-badge-icon"), &icons::PLAY, 9.0, white))
                .child(kit::label(
                    format!("{key}-badge-label"),
                    length(seconds),
                    style::BADGE,
                    white,
                )),
        );
    }
    if ui.selecting {
        // Checked: the white ring is a disc under the ink one (Rune draws a 1.5 pt spread ring
        // thinner than the board's beside an opaque fill).
        let mut check = el()
            .key(format!("{key}-check"))
            .size(20.0, 20.0)
            .radius(10.0)
            .center()
            .fill(if checked { t.text } else { white.alpha(0.55) });
        if checked {
            check = check.child(kit::glyph(format!("{key}-check-icon"), &icons::CHECK, 12.0, t.paper));
            check = el()
                .key(format!("{key}-check-ring"))
                .pin(None, Some(4.5), Some(4.5), None)
                .size(23.0, 23.0)
                .radius(11.5)
                .center()
                .fill(white)
                .child(check);
        } else {
            check = check
                .pin(None, Some(6.0), Some(6.0), None)
                .box_shadow(kit::ring(1.5, white));
        }
        tile = tile.child(check);
    }
    tile
}

/// One row of the grid: the day's heading on its first row, then up to `across` tiles.
fn line(ui: &Ui, i: usize, t: &Theme) -> El {
    let Some(&l) = ui.lines.get(i) else {
        return el().key("ph-line-none");
    };
    let mut tiles = row().key(format!("ph-line-{i}-tiles")).w_full().gap(GAP);
    for column in 0..ui.across {
        let index = l.start + column;
        tiles = tiles.child(match ui.photo_at(l.day, index).filter(|_| column < l.len) {
            Some(photo) => tile(ui, l.day, index, photo, t),
            None => el().key(format!("ph-gap-{i}-{column}")).w(ui.tile_side()),
        });
    }
    let mut out = col().key(format!("ph-line-{i}")).w_full().px(16.0);
    if l.head {
        let title = ui.days.get(l.day).map_or(String::new(), |d| d.title.clone());
        out = out.child(
            el().key(format!("ph-day-{}-row", l.day))
                .px(4.0)
                .pt(12.0)
                .pb(8.0)
                .child(kit::label(format!("ph-day-{}", l.day), title, type_::HEAD, t.text).role("heading")),
        );
    } else {
        out = out.pt(GAP);
    }
    out.child(tiles)
}

/// "No photos yet" (`.kit-empty`, DW-0116): also when a device filter has nothing.
fn empty(t: &Theme) -> El {
    col()
        .key("ph-empty-area")
        .grow(1.0)
        .center()
        .child(
            col()
                .key("ph-empty")
                .role("status")
                .items_center()
                .gap(8.0)
                .px(24.0)
                .py(28.0)
                .child(
                    el().key("ph-empty-tile")
                        .size(52.0, 52.0)
                        .radius(15.0)
                        .fill(t.tile)
                        .center()
                        .child(kit::glyph("ph-empty-icon", &icons::PHOTOS, 26.0, t.text_secondary)),
                )
                .child(kit::label("ph-empty-title", "No photos yet", style::TITLE, t.text).role("heading"))
                .child(
                    kit::label(
                        "ph-empty-sub",
                        "Photos backed up from your iPhone or iPad appear here. Turn on photo backup in Beebeeb on the iPhone or iPad.",
                        style::SUB,
                        t.text_tertiary,
                    )
                    // `max-width: 30ch`.
                    .w(227.11)
                    .wrap()
                    .align(rune::Align::Center),
                ),
        )
}

pub fn body(ui: &Ui, t: &Theme) -> El {
    let content = if ui.lines.is_empty() {
        empty(t)
    } else {
        let t = *t;
        col()
            .key("ph-scroll")
            // The tiles are sized from the width the grid really has.
            .on_layout(|s: &mut State, rect: [f32; 4]| s.photos.set_width(rect[2]))
            .scroll(ScrollSpec { stick_bottom: false })
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .child(
                col()
                    .key("ph-rows")
                    .role("list")
                    .label("Photos by day")
                    .w_full()
                    .growth(rune::GrowthSpec::default())
                    .virtual_rows(
                        ui.lines.len(),
                        150.0,
                        |_: &State, i| Cow::Owned(format!("ph-line-{i}")),
                        move |s: &State, i| line(&s.photos, i, &t),
                    )
                    .virtual_rows_version((ui.lines.len(), ui.across, ui.filter)),
            )
    };
    let mut body = col().key("ph-body").child(head(ui, t)).child(content);
    if let Some(toast) = &ui.toast {
        body = body.child(super::overlay::toast_layer(toast, 24.0, true, t));
    }
    body
}
