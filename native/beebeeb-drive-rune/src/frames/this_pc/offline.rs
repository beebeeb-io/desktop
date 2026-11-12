//! Available offline (DW-0715; its row menu DW-1306; nothing kept DW-0120): what stays on this
//! PC and opens without a connection.
//!
//! The rows are Files' rows (`.mk-row`, `ui::file_list`'s tile and PC glyph) with this
//! section's columns: Name · Status 132 · Size 84 · Contains 110 · PC 34.

use rune::{Align, Color, Cx, El, Icon, TextStyle, col, el, row};

use super::chrome::{self, FilesControls};
use super::source::{Kept, Kind};
use super::{MENU_ITEMS, RowMenu, Section, Ui, icons};
use crate::State;
use crate::source::{DriveItem, format_size};
use crate::theme::icons as base;
use crate::theme::tokens::{self, Theme, radius, type_};
use crate::ui::file_list::{self, PcState, ROW_H};
use crate::ui::kit;

const COL_STATUS_W: f32 = 132.0;
const COL_SIZE_W: f32 = 84.0;
const COL_CONTAINS_W: f32 = 110.0;
const COL_PC_W: f32 = 34.0;
const COL_GAP: f32 = 12.0;

/// "3 items · 1.4 GB", or "No items".
fn summary(kept: &[Kept]) -> String {
    match kept.len() {
        0 => "No items".into(),
        n => format!(
            "{n} {} \u{00B7} {}",
            if n == 1 { "item" } else { "items" },
            format_size(kept.iter().map(|k| k.bytes).sum())
        ),
    }
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    // Nothing answers for This PC yet (a build that ships before the engine lane wires it):
    // the shell's "not built yet" frame, never sample kept items.
    if !s.this_pc.wired() {
        return crate::shell::placeholder::window(s, cx, crate::shell::registry::group_for(&s.shell.app));
    }
    let t = tokens::theme(cx.system_dark());
    let ui = &s.this_pc;
    let focused = chrome::search_focused(cx);
    let kept = ui.kept();

    let mut right = Vec::new();
    // "Remove all is hidden" when nothing is kept (DW-0120).
    if !kept.is_empty() {
        right.push(
            chrome::outline_button("btn-remove-all-from-this-pc", "Remove all from this PC", None, &t)
                .on_press(|s: &mut State| s.this_pc.ask(super::Intent::AskRemoveAll)),
        );
    }
    let toolbar = chrome::toolbar(
        s,
        ui,
        "Available offline",
        summary(&kept),
        right,
        FilesControls::Dimmed,
        chrome::search_field(ui, focused, "Search names", &t),
        &t,
    );

    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar);
    if kept.is_empty() {
        main = main
            .child(empty_state(
                &base::AVAILABLE_OFFLINE,
                "Nothing available offline",
                "Right-click a file or folder and choose Make available offline to keep it on this PC.",
                &t,
            ))
            // The board keeps Files' status line here (PR body, Decision).
            .child(crate::ui::status_bar::status_bar(&t));
    } else {
        let bytes = kept.iter().map(|k| k.bytes).sum();
        // Two board frames, two status lines (PR body, Decision): DW-0715 names the section's
        // total, DW-1306 keeps Files' counts under the open row menu.
        let status = if matches!(ui.menu, Some(open) if open.row < kept.len()) {
            crate::ui::status_bar::status_bar(&t)
        } else {
            chrome::status_line(
                ui,
                format!(
                    "{} available offline \u{00B7} {} on this PC",
                    kept.len(),
                    format_size(bytes)
                ),
                &t,
            )
        };
        main = main
            .child(note(&t))
            .child(headers(&t))
            .child(rows(ui, &kept, &t))
            .child(status);
    }
    let window = super::window(s, cx, Section::AvailableOffline, main);
    match ui.menu {
        Some(open) if open.row < kept.len() => el()
            .key("window-with-menu")
            .w_full()
            .h_full()
            .child(window)
            .child(menu(open, &t)),
        _ => window,
    }
}

/// The note over the list (`.mk-banner`).
fn note(t: &Theme) -> El {
    row()
        .key("banner")
        .role("note")
        .mt(10.0)
        .mx(14.0)
        .py(9.0)
        .pl(12.0)
        .pr(10.0)
        .gap(10.0)
        .center_y()
        .radius(radius::LG)
        .fill(icons::note_bg(t))
        .child(kit::glyph("note-icon", &base::AVAILABLE_OFFLINE, 16.0, t.text_secondary))
        .child(
            kit::label(
                "banner-text",
                "These stay on this PC and open without a connection. They don't count toward your downloaded files limit.",
                type_::SEARCH,
                t.text_secondary,
            )
            .grow(1.0),
        )
}

fn headers(t: &Theme) -> El {
    let head = |key: &'static str, words: &'static str| kit::label(key, words, type_::COLUMN_HEADER, t.text_tertiary);
    let cell = |key: &'static str, w: f32| row().key(key).role("columnheader").w(w).center_y();
    row()
        .key("column-headers")
        .role("row")
        .h(28.0)
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
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .child(head("col-name", "Name")),
        )
        .child(cell("col-status-cell", COL_STATUS_W).child(head("col-date", "Status")))
        .child(
            cell("col-size-cell", COL_SIZE_W)
                .justify_end()
                .child(head("col-size", "Size")),
        )
        .child(cell("col-contains-cell", COL_CONTAINS_W).child(head("col-shared", "Contains")))
        .child(
            row()
                .key("col-pc-cell")
                .role("columnheader")
                .label("On this PC")
                .w(COL_PC_W)
                .center()
                .child(head("col-pc", "PC")),
        )
}

/// A kept item as Files' tile draws it.
fn as_drive_item(k: &Kept) -> DriveItem {
    let mime = match k.kind {
        Kind::Sheet => Some("application/vnd.ms-excel"),
        Kind::Deck => Some("application/vnd.apple.keynote"),
        Kind::Video => Some("video/quicktime"),
        Kind::Audio => Some("audio/mp4"),
        Kind::Photo(_) => Some("image/heic"),
        Kind::Document | Kind::Folder => None,
    };
    DriveItem {
        id: k.id.clone(),
        parent_id: None,
        name: k.name.clone(),
        is_folder: k.kind == Kind::Folder,
        size: Some(k.bytes),
        child_count: None,
        modified_at: String::new(),
        mime: mime.map(str::to_string),
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: Some("available_offline".into()),
        is_protected: false,
    }
}

fn rows(ui: &Ui, kept: &[Kept], t: &Theme) -> El {
    let mut list = col().key("file-list").w_full();
    for (i, item) in kept.iter().enumerate() {
        list = list.child(
            kept_row(i, item, ui.selected == Some(i), t)
                .on_press(move |s: &mut State| s.this_pc.select(i))
                .on_double_press(move |s: &mut State| {
                    s.this_pc.select(i);
                    s.this_pc.open_selected();
                })
                .on_secondary_press(move |s: &mut State, press, _tx| {
                    s.this_pc.open_menu(i, (press.x, press.y));
                }),
        );
    }
    col()
        .key("rows")
        .role("list")
        .label("Available offline")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .py(4.0)
        .px(6.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(list)
}

/// One kept item (`.mk-row`). Keys: `row-<i>` and `row-<i>-tile|icon|name|date|size|dash|pc`
/// (the Files keys: `date` is the Status cell, `dash` the Contains cell).
fn kept_row(i: usize, item: &Kept, selected: bool, t: &Theme) -> El {
    const CONTAINS: TextStyle = TextStyle::new(12.0, 400.0).lh(14.5);
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
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
        .child(
            row()
                .key("name-cell")
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .gap(8.0)
                .child(file_list::file_tile(&k, &as_drive_item(item), t))
                .child(
                    kit::label(format!("{k}-name"), item.name.clone(), type_::LIST_ITEM, t.text)
                        .truncate()
                        .min_w(0.0),
                ),
        )
        .child(
            kit::label(format!("{k}-date"), item.status.label(), type_::META_MONO, meta)
                .w(COL_STATUS_W)
                .truncate(),
        )
        .child(
            kit::label(format!("{k}-size"), format_size(item.bytes), type_::META_MONO, meta)
                .w(COL_SIZE_W)
                .align(Align::End),
        )
        .child(
            row()
                .key("contains-cell")
                .w(COL_CONTAINS_W)
                .min_w(0.0)
                .center_y()
                .child(kit::label(format!("{k}-dash"), item.contains.label(), CONTAINS, meta).truncate()),
        )
        .child(el().key("pc-cell").w(COL_PC_W).center().child(file_list::pc_glyph(
            format!("{k}-pc"),
            PcState::AvailableOffline,
            t.bg,
            t,
        )))
}

/// The row menu (`.wn-menu`, DW-1306): Open · Show in File Explorer · Remove from this PC.
/// Keys: `menu`, `mi-<words>`, `mi-<words>-label`, `mi-open-sc`, `msep-0`.
fn menu(open: RowMenu, t: &Theme) -> El {
    const SHORTCUT: TextStyle = TextStyle::new(12.0, 400.0).lh(14.5);
    const ITEM: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
    let mut card = col()
        .key("menu")
        .role("menu")
        .label("Kept item")
        .absolute(open.at.0, open.at.1)
        .min_w(210.0)
        .padding(4.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.20)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .modal(true, |s: &mut State| {
            s.this_pc.close_menu();
        });
    for (i, words) in MENU_ITEMS.into_iter().enumerate() {
        if i == 2 {
            card = card.child(el().key("msep-0").w_full().h(kit::HAIR).my(4.0).fill(t.separator));
        }
        let mut item = row()
            .key(format!("mi-{}", crate::ui::settings_shell::slug(words)))
            .role("menuitem")
            .label(words)
            .h(30.0)
            .w_full()
            .pl(8.0)
            .pr(12.0)
            .gap(10.0)
            .center_y()
            .radius(radius::SM)
            .fill(if open.hot == i { t.press } else { Color::TRANSPARENT })
            .hover(t.press)
            .focusable(true)
            .on_hover(move |s: &mut State| {
                if let Some(menu) = s.this_pc.menu.as_mut() {
                    menu.hot = i;
                }
            })
            .on_press(move |s: &mut State| s.this_pc.pick(i))
            .child(el().key("check-slot").w(14.0))
            .child(
                kit::label(
                    format!("mi-{}-label", crate::ui::settings_shell::slug(words)),
                    words,
                    ITEM,
                    t.text,
                )
                .grow(1.0),
            );
        if i == 0 {
            item = item.child(kit::label("mi-open-sc", "Enter", SHORTCUT, t.text_tertiary).pl(24.0));
        }
        card = card.child(item);
    }
    card
}

/// Nothing to list (`.kit-empty`): a tile, a title, one sentence. Keys: `empty`, `empty-tile`,
/// `empty-icon`, `empty-title`, `empty-sub`.
pub fn empty_state(ic: &'static Icon, title: &str, words: &str, t: &Theme) -> El {
    const TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
    const WORDS: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
    col().key("empty-area").grow(1.0).min_h(0.0).w_full().center().child(
        col()
            .key("empty")
            .role("group")
            .label(title)
            .py(28.0)
            .px(24.0)
            .gap(8.0)
            .items_center()
            .child(
                el().key("empty-tile")
                    .size(52.0, 52.0)
                    .radius(15.0)
                    .fill(t.tile)
                    .center()
                    .child(kit::glyph("empty-icon", ic, 26.0, t.text_secondary)),
            )
            .child(kit::label("empty-title", title, TITLE, t.text))
            .child(
                kit::label("empty-sub", words, WORDS, t.text_tertiary)
                    .w(227.11)
                    .wrap()
                    .align(Align::Center),
            ),
    )
}
