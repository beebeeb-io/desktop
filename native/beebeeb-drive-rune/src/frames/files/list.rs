//! The list of the Files frames (`.mk-listwrap`): the design system's headers and row, with what
//! these frames add: several selected rows, Ctrl and Shift click, the file menu on a right-click or
//! the Menu key, a date the source words itself ("Just now"), headers that sort, and drops.
//!
//! To assistive technology it is a table (DW-0401): `rows` is the table and the list's ONE Tab
//! stop; each row reads its name, when it changed, its size, who it is shared with and where it is
//! on this PC. Keyboard focus stays on the table and the selected row is its active one
//! (`aria-activedescendant`), so a screen reader says the row the arrows land on (`keys.rs`).

use std::borrow::Cow;

use super::SortBy;
use crate::State;
use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::file_list::{PcState, ROW_H, file_tile, format_date, pc_glyph};
use crate::ui::kit;
use rune::{Align, Color, Cx, El, PointerPress, Tx, col, el, row};

const COL_DATE_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;
const COL_SHARED_W: f32 = 92.0;
const COL_PC_W: f32 = 34.0;
const COL_GAP: f32 = 12.0;
/// `.mk-cols`
const COLUMN_HEADER_H: f32 = 28.0;

/// The rows, virtualised. Keys as `ui::file_list`: `rows`, `file-list`, `row-<i>` and its parts.
pub fn rows(s: &State, _cx: &mut Cx<State>, t: &Theme) -> El {
    let count = s.listing.items.len();
    let t = *t;
    let mut table = col()
        .key("rows")
        .role("table")
        .label(s.listing.folder.name.clone())
        // The list's one Tab stop: it holds keyboard focus, the selected row is its active one.
        .focusable(true)
        .no_focus_ring();
    if let Some(row) = s.selected.filter(|&i| i < count) {
        table = table.active_descendant(format!("row-{row}"));
    }
    table
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .py(4.0)
        .px(6.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        // Files dragged in from File Explorer upload into the folder shown (DW-0481: "Drop on empty space").
        .on_drop_files(|s: &mut State, files: &[std::path::PathBuf], tx: &mut Tx| {
            let folder = s.listing.folder.id.clone();
            s.upload(folder, files, tx);
        })
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
                        Some(item) => {
                            // The file that could not open without a connection says so in its
                            // row, at 45 % (`frames::global`, DW-0121).
                            let blocked = crate::frames::global::dims(s, &item.id);
                            let date = if blocked {
                                Some("Needs a connection".to_string())
                            } else {
                                s.actions.source.changed_label(item)
                            };
                            // The row being renamed shows its name as a field (DW-0410).
                            let renaming = s.actions.rename.as_ref().is_some_and(|r| r.row == i && r.id == item.id);
                            let field = renaming.then(|| super::rename::name_field(s, &t));
                            let mut row = file_row(i, item, s.is_selected(i), date, field, &t);
                            if blocked {
                                row = row.opacity(0.45);
                            }
                            if item.is_folder {
                                // "Drop on a folder: uploads into that folder"; Backups refuses (`State::upload`).
                                let id = item.id.clone();
                                row = row.on_drop_files(
                                    move |s: &mut State, files: &[std::path::PathBuf], tx: &mut Tx| {
                                        s.upload(Some(id.clone()), files, tx);
                                    },
                                );
                            }
                            row.on_press_tx(move |s: &mut State, tx: &mut Tx| {
                                let mods = tx.mods();
                                tx.focus("rows", false);
                                if mods.shift {
                                    s.extend_to(i);
                                } else if mods.cmd || mods.ctrl {
                                    s.toggle_row(i);
                                } else {
                                    s.select_only(i);
                                }
                            })
                            .on_double_press_tx(move |s: &mut State, tx: &mut Tx| {
                                s.open_row(i, tx);
                            })
                            // A right-click: the menu at the pointer. (The Menu key and Shift+F10 open
                            // it at the selected row: `keys.rs`.)
                            .on_secondary_press(
                                move |s: &mut State, at: PointerPress, tx: &mut Tx| {
                                    tx.focus("rows", false);
                                    if !s.is_selected(i) {
                                        s.select_only(i);
                                    }
                                    s.open_file_menu((at.x, at.y));
                                },
                            )
                        }
                        None => el().key("empty"),
                    },
                ),
        )
}

/// What a screen reader says of a row: "Contract draft.pdf, changed 28 Oct 2026, 1.2 MB, shared
/// with 1 person, online only" (DW-0401: "name, modified, size, shared and the PC state per row").
pub fn row_name(item: &DriveItem, date: &str, size: &str) -> String {
    let mut parts = vec![item.name.clone()];
    if item.is_protected {
        parts.push("protected".into());
    }
    parts.push(format!("changed {date}"));
    if !size.is_empty() {
        parts.push(size.to_string());
    }
    parts.push(match (item.shared_badges.len(), item.shared_type.as_deref()) {
        (0, Some("Link")) => "shared by link".to_string(),
        (0, _) => "not shared".to_string(),
        (1, _) => "shared with 1 person".to_string(),
        (n, _) => format!("shared with {n} people"),
    });
    if let Some(state) = PcState::of(item.pc_status.as_deref()) {
        parts.push(
            match state {
                PcState::OnlineOnly => "online only",
                PcState::Downloaded => "on this PC",
                PcState::AvailableOffline => "available offline",
            }
            .to_string(),
        );
    }
    parts.join(", ")
}

/// The column headers (`.mk-cols`), as `ui::file_list::column_headers` draws them, and each one
/// sorts: a click sorts by that column, a second click reverses the order (DW-0404). The arrow
/// stands beside the column the list is sorted by. Shared and PC are not sort keys.
pub fn column_headers(sort: super::Sort, t: &Theme) -> El {
    let head = |key: &'static str, text: &'static str| kit::label(key, text, type_::COLUMN_HEADER, t.text_tertiary);
    let column = |cell: &'static str, key: &'static str, text: &'static str, by: Option<SortBy>| {
        let mut cell = row().key(cell).role("columnheader").label(text).center_y().gap(3.0);
        cell = cell.child(head(key, text));
        if let Some(by) = by {
            cell = cell.focusable(true).on_press(move |s: &mut State| s.sort_by_column(by));
            if sort.by == by {
                // Ascending: A to Z, oldest first, smallest first.
                let ascending = match by {
                    SortBy::Name | SortBy::Type => !sort.reversed,
                    SortBy::DateChanged | SortBy::Size => sort.reversed,
                };
                cell = cell.sorted(Some(ascending)).child(kit::glyph(
                    "sort-arrow",
                    &icons::CHEVRON_DOWN,
                    11.0,
                    t.text_tertiary,
                ));
            }
        }
        cell
    };
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
            column("col-name-cell", "col-name", "Name", Some(SortBy::Name))
                .grow(1.0)
                .min_w(0.0),
        )
        .child(column("col-date-cell", "col-date", "Date changed", Some(SortBy::DateChanged)).w(COL_DATE_W))
        .child(
            column("col-size-cell", "col-size", "Size", Some(SortBy::Size))
                .w(COL_SIZE_W)
                .justify_end(),
        )
        .child(column("col-shared-cell", "col-shared", "Shared", None).w(COL_SHARED_W))
        .child(
            column("col-pc-cell", "col-pc", "PC", None)
                .label("On this PC")
                .w(COL_PC_W)
                .justify_center(),
        )
}

/// One row (`.mk-row`). `date`: the source's own words for the date, else the list's.
/// `renaming`: the field that stands where the name is while it is being typed (DW-0410).
pub fn file_row(
    i: usize,
    item: &DriveItem,
    selected: bool,
    date: Option<String>,
    renaming: Option<El>,
    t: &Theme,
) -> El {
    let k = format!("row-{i}");
    let date = date.unwrap_or_else(|| format_date(&item.modified_at));
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    let surface = t.bg;

    let mut name = row()
        .key("name-cell")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(8.0)
        .child(file_tile(&k, item, t))
        .child(match renaming {
            Some(field) => field,
            None => kit::label(format!("{k}-name"), item.name.clone(), type_::LIST_ITEM, t.text)
                .truncate()
                .min_w(0.0),
        });
    if item.is_protected {
        name = name.child(
            kit::glyph(format!("{k}-lock"), &icons::LOCK, 11.0, t.text_tertiary)
                .ml(2.0)
                .role("image")
                .label("Protected"),
        );
    }

    let size_text = if item.is_folder {
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
        .role("row")
        .label(row_name(item, &date, &size_text))
        .h(ROW_H)
        .w_full()
        .px(14.0)
        .gap(COL_GAP)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if selected { t.selection_row } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .child(name)
        .child(
            kit::label(format!("{k}-date"), date, type_::META_MONO, meta)
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
        cell.child(stack)
    } else if item.shared_type.as_deref() == Some("Link") {
        cell.child(
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
        )
    } else {
        cell = cell.child(kit::label(format!("{k}-dash"), "\u{2014}", type_::META_MONO, meta));
        cell
    }
}
