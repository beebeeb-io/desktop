//! The sidebar with its counts from data: `crate::ui::sidebar::sidebar` copied, with two
//! changes the board asks for in these sections (see the PR's "Requests to the design-system
//! lane"): the Shared pill and the Trash figure come from the sources (no pill without an
//! invitation waiting, no figure for an empty Trash: DW-0117), and the figure of the section
//! that shows is ink-2 (`.mk-side-item.on .mk-side-count`).

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::kit;
use crate::ui::sidebar::{Count, SidebarSection, brand};
use rune::{Color, El, col, el, row};

const SIDEBAR_W: f32 = 220.0;

/// The counts beside Shared (invitations waiting) and Trash (items in it).
#[derive(Clone, Copy)]
pub struct Counts {
    pub waiting: u32,
    pub trash: u32,
}

/// One sidebar item (`.mk-side-item`): icon, label, optional count. Keys: `<label>-row`,
/// `<label>-icon`, `<label>`, `<label>-badge`.
pub fn side_item(
    label: &'static str,
    ic: &'static rune::Icon,
    count: Option<Count>,
    on: bool,
    shortcut: &str,
    t: &Theme,
) -> El {
    let mut item = row()
        .key(format!("{label}-row"))
        .role("button")
        .label(label)
        .shortcut_hint(shortcut)
        .help(format!("{label}  {shortcut}"))
        .h(28.0)
        .px(9.0)
        .gap(9.0)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if on { t.selection } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(kit::glyph(
            format!("{label}-icon"),
            ic,
            16.0,
            if on { t.text } else { t.text_tertiary },
        ))
        .child(
            kit::label(
                label,
                label,
                type_::LIST_ITEM,
                if on { t.text } else { t.text_secondary },
            )
            .grow(1.0)
            .truncate(),
        );
    match count {
        Some(Count::Pill(n)) => {
            item = item.child(
                el().key(format!("{label}-pill"))
                    .min_w(18.0)
                    .h(18.0)
                    .px(5.0)
                    .radius(9.0)
                    .fill(t.badge)
                    .center()
                    .child(kit::label(
                        format!("{label}-badge"),
                        n.to_string(),
                        type_::BADGE,
                        t.text,
                    )),
            );
        }
        Some(Count::Plain(n)) => {
            item = item.child(kit::label(
                format!("{label}-badge"),
                n.to_string(),
                type_::KBD,
                // `.mk-side-item.on .mk-side-count`: the count of the section that shows is ink-2.
                if on { t.text_secondary } else { t.text_tertiary },
            ));
        }
        None => {}
    }
    item
}

pub fn sidebar(s: &State, counts: Counts, t: &Theme) -> El {
    let storage = s.storage;
    let active = s.sidebar_section;

    let mut sections = col().key("sidebar-sections").gap(1.0);
    for &section in SidebarSection::all() {
        let count = match section {
            SidebarSection::Shared => (counts.waiting > 0).then_some(Count::Pill(counts.waiting)),
            SidebarSection::Trash => (counts.trash > 0).then_some(Count::Plain(counts.trash)),
            _ => None,
        };
        sections = sections.child(
            side_item(
                section.label(),
                section.icon(),
                count,
                active == section,
                section.shortcut(),
                t,
            )
            .on_press(move |s: &mut State| s.sidebar_section = section),
        );
    }

    let this_pc = col()
        .key("this-pc")
        .gap(1.0)
        .child(el().key("this-pc-title").px(10.0).pb(3.0).child(kit::label(
            "this-pc-label",
            "THIS PC",
            type_::SECTION_LABEL,
            t.text_tertiary,
        )))
        .child(side_item(
            "Sync history",
            &icons::SYNC_HISTORY,
            None,
            false,
            "Ctrl+6",
            t,
        ))
        .child(side_item(
            "Available offline",
            &icons::AVAILABLE_OFFLINE,
            Some(Count::Plain(3)),
            false,
            "Ctrl+7",
            t,
        ));

    let nav = col()
        .key("sidebar-nav")
        .role("navigation")
        .label("Sections")
        .gap(12.0)
        .grow(1.0)
        .min_h(0.0)
        .child(sections)
        .child(this_pc);

    // Storage meter (`.mk-store`). The split is the board's sample: Files 24 GB, Photos 216 GB.
    const FILES_BYTES: i64 = 24_000_000_000;
    let quota = storage.quota_bytes.max(1) as f64;
    let files_frac = (FILES_BYTES.min(storage.used_bytes) as f64 / quota) as f32;
    let photos_frac = ((storage.used_bytes - FILES_BYTES).max(0) as f64 / quota) as f32;
    let track_w = SIDEBAR_W - 2.0 * (10.0 + 6.0 + 4.0) - kit::HAIR;
    let storage_value = if s.storage_known {
        format!(
            "{} of {}",
            format_storage(storage.used_bytes),
            format_storage(storage.quota_bytes)
        )
    } else {
        // No figure without the server (a `--db` source never asks it).
        "Unavailable".to_string()
    };
    let mut meter = col()
        .key("storage-meter")
        .role("group")
        .label(format!("Storage: {storage_value}"))
        .gap(5.0)
        .px(4.0)
        .child(
            row()
                .key("storage-header")
                .justify_between()
                .child(kit::label("storage-label", "Storage", type_::CAPTION, t.text_secondary))
                .child(kit::label(
                    "storage-value",
                    storage_value.clone(),
                    type_::KBD,
                    t.text_tertiary,
                )),
        )
        .child(
            row()
                .key("meter-track")
                .w_full()
                .h(5.0)
                .radius(2.5)
                .clip_rounded(2.5)
                .fill(t.separator)
                // Neutral fills: Amber is only encryption state and the primary action.
                .child(el().key("meter-fill-0").h_full().w(track_w * files_frac).fill(t.text))
                .child(
                    el().key("meter-fill-1")
                        .h_full()
                        .w(track_w * photos_frac)
                        .fill(t.text_quaternary),
                ),
        );
    // The split, like the total, is the server's: nothing is shown in its place.
    if s.storage_known {
        meter = meter.child(
            row()
                .key("storage-breakdown")
                .justify_between()
                .child(kit::label(
                    "storage-files",
                    format!("Files {}", format_storage(FILES_BYTES)),
                    type_::KBD,
                    t.text_tertiary,
                ))
                .child(kit::label(
                    "storage-photos",
                    format!("Photos {}", format_storage((storage.used_bytes - FILES_BYTES).max(0))),
                    type_::KBD,
                    t.text_tertiary,
                )),
        );
    }

    let account = row()
        .key("account-row")
        .center_y()
        .gap(8.0)
        .pt(2.0)
        .px(2.0)
        .child(kit::avatar(
            "avatar",
            "AV",
            26.0,
            rune::oklch(0.88, 0.05, 250.0),
            t.card,
            t,
        ))
        .child(
            col()
                .key("account-info")
                .grow(1.0)
                .min_w(0.0)
                .child(kit::label("account-name", "Anna Visser", type_::BODY_SEMI, t.text).truncate())
                .child(
                    kit::label(
                        "account-email",
                        "anna@northwind.studio",
                        type_::BODY_SM,
                        t.text_tertiary,
                    )
                    .truncate(),
                ),
        )
        .child(
            el().key("settings-btn")
                .role("button")
                .label("Settings")
                .shortcut_hint("Ctrl+,")
                .help("Settings  Ctrl+,")
                .size(26.0, 26.0)
                .radius(radius::CONTROL)
                .center()
                .hover(t.selection)
                .press(t.press)
                .focusable(true)
                .child(kit::glyph("settings-icon", &icons::SETTINGS, 15.0, t.text_secondary)),
        );

    let foot = col()
        .key("sidebar-foot")
        .gap(10.0)
        .pt(10.0)
        .px(6.0)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(meter)
        .child(account);

    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .fill(t.sidebar_bg)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .gap(12.0)
        .pt(12.0)
        .px(10.0)
        .pb(10.0)
        .child(brand("Drive", t))
        .child(nav)
        .child(foot)
}

/// Format bytes as "240 GB", "1 TB" (base 10, as the board shows storage).
fn format_storage(bytes: i64) -> String {
    const GB: i64 = 1_000_000_000;
    const TB: i64 = 1_000_000_000_000;

    if bytes >= TB {
        let tb = bytes as f64 / TB as f64;
        if (tb - tb.round()).abs() < 0.05 {
            format!("{} TB", tb.round() as i64)
        } else {
            format!("{tb:.1} TB")
        }
    } else if bytes >= GB {
        let gb = bytes as f64 / GB as f64;
        if (gb - gb.round()).abs() < 0.05 {
            format!("{} GB", gb.round() as i64)
        } else {
            format!("{gb:.1} GB")
        }
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}
