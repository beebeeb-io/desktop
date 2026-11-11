//! The rows of the Files frames (`.mk-rows`): the design system's row, with what these frames add to
//! it: several selected rows, Ctrl and Shift click, the file menu on a right-click or the Menu key,
//! and a date the source words itself ("Just now").

use std::borrow::Cow;

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

/// The rows, virtualised. Keys as `ui::file_list`: `rows`, `file-list`, `row-<i>` and its parts.
pub fn rows(s: &State, _cx: &mut Cx<State>, t: &Theme) -> El {
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
        // Files dragged in from File Explorer upload into the folder shown (DW-0481).
        .on_drop_files(|s: &mut State, files: &[std::path::PathBuf], _tx: &mut Tx| {
            let folder = s.listing.folder.id.clone();
            s.intent(super::Intent::Upload(folder, files.to_vec()));
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
                            let date = s.actions.source.changed_label(item);
                            file_row(i, item, s.is_selected(i), date, &t)
                                .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                                    let mods = tx.mods();
                                    if mods.shift {
                                        s.extend_to(i);
                                    } else if mods.cmd || mods.ctrl {
                                        s.toggle_row(i);
                                    } else {
                                        s.select_only(i);
                                    }
                                })
                                .on_double_press(move |s: &mut State| {
                                    s.select_only(i);
                                    s.open_row(i);
                                })
                                // A right-click, and the Menu key or Shift+F10 on a focused row (Rune sends
                                // those here, with the row's own place as the point).
                                .on_secondary_press(move |s: &mut State, at: PointerPress, _tx: &mut Tx| {
                                    if !s.is_selected(i) {
                                        s.select_only(i);
                                    }
                                    s.open_file_menu((at.x, at.y));
                                })
                        }
                        None => el().key("empty"),
                    },
                ),
        )
}

/// One row (`.mk-row`). `date`: the source's own words for the date, else the list's.
pub fn file_row(i: usize, item: &DriveItem, selected: bool, date: Option<String>, t: &Theme) -> El {
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    let surface = t.bg;

    let mut name = row()
        .key("name-cell")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(8.0)
        .child(file_tile(&k, item, t))
        .child(
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
                date.unwrap_or_else(|| format_date(&item.modified_at)),
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
