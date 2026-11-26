//! Available offline (DW-0715; its row menu DW-1306; nothing kept DW-0120): what stays on this
//! PC and opens without a connection.
//!
//! The rows are Files' rows (`.mk-row`, `ui::file_list`'s tile and PC glyph) with this
//! section's columns: Name · Status 132 · Size 84 · Contains 110 · PC 34.

use rune::{Align, Cx, El, TextStyle, col, el, row};

use super::chrome::{self, FilesControls};
use super::source::{CopyState, Kept, KeptProgress, Kind, Standing};
use super::{MENU_ITEMS, RowMenu, Section, icons};
use crate::State;
use crate::source::{DriveItem, format_size};
use crate::theme::scale::space;
use crate::theme::file_icons::icon_for;
use crate::theme::icons as base;
use crate::theme::tokens::{self, Theme, radius, type_};
use crate::ui::file_list::{self, PcState, ROW_H};
use crate::ui::file_row::{self, Column, RowLook, Width};
use crate::ui::menu::{self as shared_menu, Entry};
use crate::ui::view_state::{self, ViewState};
use crate::ui::{button, kit};

const COL_STATUS_W: f32 = 200.0;
const COL_SIZE_W: f32 = 84.0;
const COL_CONTAINS_W: f32 = 110.0;
const COL_PC_W: f32 = 34.0;

/// Name · Status · Size · Contains · PC.
const COLUMNS: [Column; 5] = [
    Column::new("name", "Name", Width::Flex(1.0)),
    Column::new("status", "Status", Width::Fixed(COL_STATUS_W)),
    Column::new("size", "Size", Width::Fixed(COL_SIZE_W)).align(Align::End),
    Column::new("contains", "Contains", Width::Fixed(COL_CONTAINS_W)),
    Column::new("pc", if cfg!(target_os = "macos") { "Mac" } else { "PC" }, Width::Fixed(COL_PC_W)).align(Align::Center).spoken(concat!("On ", beebeeb_engine::device_word!())),
];

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
    // This PC turned off (`engine_host::this_pc::TURNED_ON`):
    // the shell's "not built yet" frame, never sample kept items.
    if !s.this_pc.wired() {
        return crate::shell::placeholder::window(s, cx, crate::shell::registry::group_for(&s.shell.app));
    }
    let t = tokens::theme(cx.system_dark());
    let ui = &s.this_pc;
    let kept = ui.kept();
    let kept = &kept[..];
    let standing = ui.standing();

    let mut right = Vec::new();
    // "Remove all is hidden" when nothing is kept (DW-0120).
    if !kept.is_empty() {
        right.push(
            button::button(button::keys("btn-remove-all-from-this-pc"), concat!("Remove all from ", beebeeb_engine::device_word!()), None, button::Variant::Secondary, button::Size::Sm, button::ButtonState::Enabled, &t)
                .on_press(|s: &mut State| s.this_pc.ask(super::Intent::AskRemoveAll)),
        );
    }
    let toolbar = chrome::toolbar(
        s,
        ui,
        "Available offline",
        // A count only of what was read.
        if standing == Standing::Ready {
            summary(kept)
        } else {
            String::new()
        },
        right,
        FilesControls::Dimmed,
        chrome::search_field(ui, "Search names", &t),
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
    if let Some(state) = not_ready(standing, &t) {
        // No lists: the section says why, and the status line names no count.
        main = main
            .child(state)
            .child(chrome::status_line(s, ui, String::new(), &t));
    } else if kept.is_empty() {
        main = main
            .child(area(view_state::view_state(
                ViewState::Empty {
                    key: "empty",
                    icon: &base::AVAILABLE_OFFLINE,
                    title: "Nothing available offline",
                    body: concat!("Right-click a file or folder and choose Make available offline to keep it on ", beebeeb_engine::device_word!(), "."),
                    action: None,
                },
                &t,
            )))
            // The board keeps Files' sample status line here (PR body, Decision); a real PC says what it read.
            .child(if ui.is_sample() {
                crate::ui::status_bar::status_bar(&t)
            } else {
                chrome::status_line(s, ui, "Nothing available offline".into(), &t)
            });
    } else {
        let bytes = kept.iter().map(|k| k.bytes).sum();
        // Two board frames, two status lines (PR body, Decision): DW-0715 names the section's
        // total, DW-1306 keeps Files' sample counts under the open row menu. A real PC always
        // names the section's total: the sample counts are no PC's.
        let status = if ui.is_sample() && matches!(ui.menu, Some(open) if open.row < kept.len()) {
            crate::ui::status_bar::status_bar(&t)
        } else {
            chrome::status_line(
                s,
                ui,
                // While kept files come down the line says how far they are (dm-68): the kept total is not on
                // this device yet, so it is not claimed as such.
                match ui.downloading() {
                    Some(running) => downloading_words(running),
                    None => format!(
                        "{} available offline \u{00B7} {} on {os}",
                        kept.len(),
                        format_size(bytes),
                        os = beebeeb_engine::device_word!()
                    ),
                },
                &t,
            )
        };
        main = main
            .child(note(&t))
            .child(file_row::header(&COLUMNS, None, &t))
            .child(rows(kept.len(), &t))
            .child(status);
    }
    let window = super::window(s, cx, Section::AvailableOffline, main);
    if ui.confirm_remove_all && !kept.is_empty() {
        return el()
            .key("window-with-ask")
            .w_full()
            .h_full()
            .child(window)
            .child(ask_remove_all(kept.len(), &t));
    }
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

/// A centred area for a state that stands in place of the list.
fn area(state: El) -> El {
    col().key("empty-area").grow(1.0).min_h(0.0).w_full().center().child(state)
}

/// What a section says in place of its list while the source has none: Loading while the first answer is out,
/// Failed with Try again (which reads again) when the read failed. `None`: the lists are what this PC holds.
pub fn not_ready(standing: Standing, t: &Theme) -> Option<El> {
    let state = match standing {
        Standing::Ready => return None,
        Standing::Reading => {
            return Some(
                col()
                    .key("empty-area")
                    .w_full()
                    .child(view_state::view_state(ViewState::Loading { words: concat!("Reading ", beebeeb_engine::device_word!()) }, t)),
            );
        }
        Standing::Unreadable => view_state::view_state(
            ViewState::Failed {
                key: "unreadable",
                title: concat!("Couldn't read ", beebeeb_engine::device_word!()),
                why: concat!("What ", beebeeb_engine::device_word!(), " keeps couldn't be read just now."),
                retry: Some(
                    view_state::try_again("unreadable", t).on_press(|s: &mut State| s.this_pc.read_again()),
                ),
            },
            t,
        ),
        Standing::AnotherAccounts => view_state::view_state(
            ViewState::Failed {
                key: "another-account",
                title: "Not available for this account",
                why: concat!("The synced files on ", beebeeb_engine::device_word!(), " aren't confirmed as this account's, so nothing is shown here."),
                retry: None,
            },
            t,
        ),
    };
    Some(area(state))
}

/// The ask before "Remove all from this PC": once, before anything is removed. The way out is the last button
/// and Esc; nothing is removed until the first is pressed.
fn ask_remove_all(count: usize, t: &Theme) -> El {
    use crate::ui::settings_dialog::{Weight, button, dialog};
    let sub = format!(
        "{count} {} will be online only. Nothing is deleted from Beebeeb.",
        if count == 1 { "item" } else { "items" }
    );
    let card = dialog(
        concat!("Remove all from ", beebeeb_engine::device_word!()),
        400.0,
        concat!("Remove all from ", beebeeb_engine::device_word!(), "?"),
        Some(&sub),
        Vec::new(),
        vec![
            button(0, "Remove all", Weight::Primary, t)
                .key("btn-remove-all-yes")
                .on_press(|s: &mut State| s.this_pc.remove_all_confirmed()),
            button(1, "Cancel", Weight::Quiet, t)
                .key("btn-remove-all-cancel")
                .on_press(|s: &mut State| {
                    s.this_pc.remove_all_cancelled();
                }),
        ],
        t,
    );
    // A modal's keys are heard on the modal itself (Esc says no), not on the window under it.
    el().key("dialog-remove-all-from-this-pc")
        .pin(Some(0.0), Some(32.0), Some(0.0), Some(0.0))
        .child(card)
        .on_key(super::on_key)
        // A press outside the card does nothing; the way out is Cancel or Esc.
        .modal(false, |s: &mut State| {
            s.this_pc.remove_all_cancelled();
        })
}

/// A row as Files' actions take it (`FilesSource::open`, `show_in_file_explorer`): they find the file by its id.
pub fn drive_item(id: &str, name: &str, is_folder: bool) -> DriveItem {
    DriveItem {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        is_folder,
        size: None,
        child_count: None,
        modified_at: String::new(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: None,
        is_protected: false,
    }
}

/// The note over the list (`.mk-banner`).
fn note(t: &Theme) -> El {
    row()
        .key("banner")
        .role("note")
        .mt(10.0)
        .mx(14.0)
        .py(space::S9)
        .pl(12.0)
        .pr(10.0)
        .gap(space::S10)
        .center_y()
        .radius(radius::LG)
        .fill(icons::note_bg(t))
        .child(kit::glyph("note-icon", &base::AVAILABLE_OFFLINE, 16.0, t.text_secondary))
        .child(
            kit::label(
                "banner-text",
                concat!("These stay on ", beebeeb_engine::device_word!(), " and open without a connection. They don't count toward your downloaded files limit."),
                type_::SEARCH,
                t.text_secondary,
            )
            .grow(1.0),
        )
}

fn mime_of(kind: &Kind) -> Option<&'static str> {
    match kind {
        Kind::Sheet => Some("application/vnd.ms-excel"),
        Kind::Deck => Some("application/vnd.apple.keynote"),
        Kind::Video => Some("video/quicktime"),
        Kind::Audio => Some("audio/mp4"),
        Kind::Photo(_) => Some("image/heic"),
        Kind::Document | Kind::Folder => None,
    }
}

/// The rows, realised in view only: each is read from the list as it comes into view.
fn rows(count: usize, t: &Theme) -> El {
    let t = *t;
    col()
        .key("rows")
        .role("list")
        .label("Available offline")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .py(space::S4)
        .px(space::S6)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(
            col()
                .key("file-list")
                .w_full()
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    count,
                    ROW_H,
                    move |s: &State, i| {
                        if i < s.this_pc.kept_count() {
                            std::borrow::Cow::Owned(format!("row-{i}"))
                        } else {
                            std::borrow::Cow::Borrowed("row-oob")
                        }
                    },
                    move |s: &State, i| match s.this_pc.kept().get(i) {
                        Some(item) => kept_row(i, item, s.this_pc.copy_state(&item.id), s.this_pc.selected == Some(i), count, &t)
                            .on_press(move |s: &mut State| s.this_pc.select(i))
                            .on_double_press(move |s: &mut State| {
                                s.this_pc.select(i);
                                s.this_pc.open_selected();
                            })
                            .on_secondary_press(move |s: &mut State, press, _tx| {
                                s.this_pc.open_menu(i, (press.x, press.y));
                            }),
                        None => el().key("empty"),
                    },
                ),
        )
}

/// One kept item: the shared row with this section's cells. Keys: `row-<i>` and
/// `row-<i>-tile|icon|name|date|size|dash|pc` (`date` is the Status cell, `dash` the Contains cell).
fn kept_row(i: usize, item: &Kept, copy: Option<CopyState>, selected: bool, count: usize, t: &Theme) -> El {
    const CONTAINS: TextStyle = crate::theme::scale::text::NOTE;
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    let icon = icon_for(&item.name, item.kind == Kind::Folder, mime_of(&item.kind));
    let cells = vec![
        status_cell(&k, item, &copy, meta, t),
        kit::label(format!("{k}-size"), format_size(item.bytes), type_::META_MONO, meta)
            .w(COL_SIZE_W)
            .align(Align::End),
        row()
            .key("contains-cell")
            .w(COL_CONTAINS_W)
            .min_w(0.0)
            .center_y()
            .child(kit::label(format!("{k}-dash"), item.contains.label(), CONTAINS, meta).truncate()),
        el().key("pc-cell").w(COL_PC_W).center().child(file_list::pc_glyph(
            format!("{k}-pc"),
            PcState::AvailableOffline,
            t.bg,
            t,
        )),
    ];
    let look = RowLook {
        selected,
        position: Some((i + 1, count)),
        ..RowLook::default()
    };
    file_row::row(&k, &item.name, icon, &item.name, cells, look, t)
}

/// The words over a copy's bar: "45% · 5.1 of 11.3 MB", a folder's "3 of 12 files · 45%".
pub fn progress_words(p: &KeptProgress) -> String {
    match p.files {
        Some((n, m)) => format!("{n} of {m} files \u{00B7} {}%", p.percent),
        None => format!("{}% \u{00B7} {}", p.percent, figures(p.done, p.total)),
    }
}

/// The status line while kept files come down: "Downloading 3 files · 5.1 of 11.3 MB".
pub fn downloading_words((files, done, total): (u32, u64, u64)) -> String {
    format!(
        "Downloading {files} {} \u{00B7} {}",
        if files == 1 { "file" } else { "files" },
        figures(done, total)
    )
}

/// "5.1 of 11.3 MB": both in the unit of the whole.
fn figures(done: u64, total: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * KB;
    const GB: f64 = MB * KB;
    let (unit, size, decimals) = match total as f64 {
        t if t >= GB => ("GB", GB, 1),
        t if t >= MB => ("MB", MB, 1),
        _ => ("KB", KB, 0),
    };
    let part = |n: u64| format!("{:.*}", decimals, n as f64 / size);
    format!("{} of {} {unit}", part(done), part(total))
}

/// The Status cell: the state's words; while a copy comes down its words over a bar; a failed copy says so, with Try
/// again (dm-68).
fn status_cell(k: &str, item: &Kept, copy: &Option<CopyState>, meta: rune::Color, t: &Theme) -> El {
    match copy {
        Some(CopyState::Running(p)) => {
            let words = progress_words(p);
            col()
                .key(format!("{k}-download"))
                .w(COL_STATUS_W)
                .gap(space::S4)
                .child(kit::label(format!("{k}-date"), words.clone(), type_::CAPTION.lh(13.5), meta).truncate())
                .child(
                    el().key(format!("{k}-bar"))
                        .role("progressbar")
                        .label(format!("{}, {words}", item.name))
                        .w_full()
                        .h(4.0)
                        // A capsule: the radius is clamped to half the 4px height.
                        .radius(crate::theme::scale::radius::PILL)
                        .clip_rounded(crate::theme::scale::radius::PILL)
                        .fill(t.text.alpha(0.09))
                        .child(
                            el().key(format!("{k}-bar-fill"))
                                .h(4.0)
                                .w_frac(f32::from(p.percent) / 100.0)
                                .radius(crate::theme::scale::radius::PILL)
                                .fill(t.text_tertiary),
                        ),
                )
        }
        Some(CopyState::Failed) => {
            let id = item.id.clone();
            row()
                .key(format!("{k}-failed"))
                .w(COL_STATUS_W)
                .center_y()
                .gap(space::S6)
                .child(kit::label(format!("{k}-date"), "Download failed", type_::META_MONO, t.red).truncate())
                .child(
                    button::button(
                        button::keys(&format!("{k}-retry")),
                        "Try again",
                        None,
                        button::Variant::Secondary,
                        button::Size::Sm,
                        button::ButtonState::Enabled,
                        t,
                    )
                    .on_press(move |s: &mut State| s.this_pc.try_keep_again(&id)),
                )
        }
        None => kit::label(format!("{k}-date"), item.status.label(), type_::META_MONO, meta)
            .w(COL_STATUS_W)
            .truncate(),
    }
}

const MENU_KEY: &str = "menu";

/// The row menu (DW-1306) on the shared menu: Open · Show in File Explorer · Remove from this PC.
fn menu(open: RowMenu, t: &Theme) -> El {
    let entries = [
        Entry::keyed("open", MENU_ITEMS[0], "Enter"),
        Entry::item("show", MENU_ITEMS[1]),
        Entry::Sep,
        Entry::item("remove", MENU_ITEMS[2]),
    ];
    // The separator sits between the second and third item; the menu's index is the item's.
    let pick = |s: &mut State, at: usize, _: &mut rune::Tx| s.this_pc.pick(if at > 2 { at - 1 } else { at });
    let hover = |s: &mut State, at: usize| {
        if let Some(menu) = s.this_pc.menu.as_mut() {
            menu.hot = if at > 2 { at - 1 } else { at };
        }
    };
    let close = |s: &mut State, _: &mut rune::Tx| {
        s.this_pc.close_menu();
    };
    let hot = if open.hot >= 2 { open.hot + 1 } else { open.hot };
    shared_menu::menu(MENU_KEY, "Kept item", 210.0, &entries, Some(hot), t, pick, hover, close)
        .on_key(super::on_key)
        .absolute(open.at.0, open.at.1)
}
