//! Sidebar: section items with icon/label/count badge, THIS PC section,
//! storage meter, and account row.
//!
//! Board measurements (from Playwright geometry export):
//! - Sidebar width: 220 pt (board: x=1, w=220)
//! - Section items: 30 pt tall, 12 pt font, icon 16x16
//! - Section label "THIS PC": 10 pt/600, ink-3 colour
//! - Storage meter: 4 pt bar, "240 GB of 1 TB" + breakdown
//! - Account row: avatar 26x26, name 12/600, email 11/400

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, space, type_};
use rune::{Color, El, col, el, icon, row, text};

/// Board: sidebar element is 220px wide (from golden/DW-0401.json: sidebar w=220).
const SIDEBAR_W: f32 = 220.0;

/// Which sidebar section is active.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidebarSection {
    Files,
    Shared,
    Photos,
    Backups,
    Trash,
}

impl SidebarSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Shared => "Shared",
            Self::Photos => "Photos",
            Self::Backups => "Backups",
            Self::Trash => "Trash",
        }
    }

    pub fn all() -> &'static [SidebarSection] {
        &[
            SidebarSection::Files,
            SidebarSection::Shared,
            SidebarSection::Photos,
            SidebarSection::Backups,
            SidebarSection::Trash,
        ]
    }

    pub fn icon(self) -> &'static rune::Icon {
        match self {
            Self::Files => &icons::FILES,
            Self::Shared => &icons::SHARED,
            Self::Photos => &icons::PHOTOS,
            Self::Backups => &icons::BACKUPS,
            Self::Trash => &icons::TRASH,
        }
    }

    /// Badge count from the board: Shared 1, Trash 12, others none.
    pub fn badge(self) -> Option<u32> {
        match self {
            Self::Shared => Some(1),
            Self::Trash => Some(12),
            _ => None,
        }
    }
}

pub fn sidebar(s: &State, t: &Theme) -> El {
    let storage = s.storage;
    let active = s.sidebar_section;

    // Sections list
    let mut sections = col().key("sidebar-sections").px(space::SM).gap(2.0);

    for &section in SidebarSection::all() {
        let is_active = active == section;
        let badge = section.badge();

        let mut item = row()
            .key(section.label())
            .role("button")
            .label(section.label())
            .h(30.0)
            .px(space::SM)
            .center_y()
            .radius(radius::MD)
            .fill(if is_active { t.hover } else { Color::TRANSPARENT })
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .on_press(move |s: &mut State| s.sidebar_section = section)
            .child(
                icon(section.icon(), 16.0)
                    .key("icon")
                    .color(if is_active { t.accent } else { t.icon_secondary }),
            )
            .child(
                text(section.label())
                    .key("label")
                    .font(if is_active { type_::BODY_SEMI } else { type_::BODY })
                    .color(t.text)
                    .ml(space::SM)
                    .grow(1.0),
            );

        if let Some(count) = badge {
            item = item.child(
                text(format!("{count}"))
                    .key("badge")
                    .font(type_::BODY_SM)
                    .color(t.text_tertiary),
            );
        }

        sections = sections.child(item);
    }

    // THIS PC section
    let this_pc = col()
        .key("this-pc")
        .px(space::SM)
        .mt(space::SM)
        .gap(2.0)
        .child(
            text("THIS PC")
                .key("this-pc-label")
                .font(type_::SECTION_LABEL)
                .color(t.text_tertiary)
                .ml(space::SM)
                .mb(space::XS),
        )
        .child(this_pc_item("Sync history", &icons::SYNC_HISTORY, None, t))
        .child(this_pc_item("Available offline", &icons::AVAILABLE_OFFLINE, Some(3), t));

    // Storage meter
    let used_pct = if storage.quota_bytes > 0 {
        (storage.used_bytes as f64 / storage.quota_bytes as f64).min(1.0)
    } else {
        0.0
    };
    let used_label = format_storage(storage.used_bytes);
    let quota_label = format_storage(storage.quota_bytes);

    let meter = col()
        .key("storage-meter")
        .px(space::MD)
        .gap(space::XS)
        .child(
            row()
                .key("storage-header")
                .justify_between()
                .w_full()
                .child(
                    text("Storage")
                        .key("storage-label")
                        .font(type_::BODY_SM)
                        .color(t.text_tertiary),
                )
                .child(
                    text(format!("{used_label} of {quota_label}"))
                        .key("storage-value")
                        .font(type_::BODY_SM)
                        .color(t.text_tertiary),
                ),
        )
        .child(
            el().key("meter-track")
                .w_full()
                .h(4.0)
                .radius(2.0)
                .fill(t.separator)
                .child(
                    el().key("meter-fill")
                        .h(4.0)
                        .radius(2.0)
                        // Neutral fill: Amber is only encryption state + the primary action.
                        .fill(t.text_quaternary)
                        .w((SIDEBAR_W - space::MD * 2.0) * used_pct as f32),
                ),
        )
        .child(
            text("Files 24 GB  Photos 216 GB")
                .key("storage-breakdown")
                .font(type_::BODY_SM)
                .color(t.text_quaternary),
        );

    // Account row
    let account_row = row()
        .key("account-row")
        .h(40.0)
        .w_full()
        .px(space::MD)
        .center_y()
        .gap(space::SM)
        // Avatar circle
        .child(
            el().key("avatar")
                .size(26.0, 26.0)
                .radius(13.0)
                .fill(t.separator_strong)
                .center()
                .child(text("AV").key("initials").font(type_::AVATAR_INITIALS).color(t.text)),
        )
        .child(
            col()
                .key("account-info")
                .grow(1.0)
                .child(text("Anna Visser").key("name").font(type_::BODY_SEMI).color(t.text))
                .child(
                    text("anna@northwind.st\u{2026}")
                        .key("email")
                        .font(type_::BODY_SM)
                        .color(t.text_tertiary),
                ),
        )
        .child(
            icon(&icons::SETTINGS, 15.0)
                .key("settings-icon")
                .color(t.icon_secondary),
        );

    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .fill(t.sidebar_bg)
        .pt(space::SM)
        .child(sections)
        .child(this_pc)
        .child(el().key("sidebar-spacer").grow(1.0))
        .child(meter)
        .child(account_row)
}

fn this_pc_item(label: &'static str, ic: &'static rune::Icon, count: Option<u32>, t: &Theme) -> El {
    let mut item = row()
        .key(label)
        .h(30.0)
        .px(space::SM)
        .center_y()
        .radius(radius::MD)
        .hover(t.hover)
        .child(icon(ic, 16.0).key("icon").color(t.icon_secondary))
        .child(
            text(label)
                .key("label")
                .font(type_::BODY)
                .color(t.text)
                .ml(space::SM)
                .grow(1.0),
        );

    if let Some(c) = count {
        item = item.child(
            text(format!("{c}"))
                .key("count")
                .font(type_::BODY_SM)
                .color(t.text_tertiary),
        );
    }

    item
}

/// Format bytes as "240 GB", "1 TB", etc. (base-10 for the board's display).
fn format_storage(bytes: i64) -> String {
    const GB: i64 = 1_000_000_000;
    const TB: i64 = 1_000_000_000_000;

    if bytes >= TB {
        let tb = bytes as f64 / TB as f64;
        if (tb - tb.round()).abs() < 0.05 {
            format!("{} TB", tb.round() as i64)
        } else {
            format!("{:.1} TB", tb)
        }
    } else if bytes >= GB {
        let gb = bytes as f64 / GB as f64;
        if (gb - gb.round()).abs() < 0.5 {
            format!("{} GB", gb.round() as i64)
        } else {
            format!("{:.1} GB", gb)
        }
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}
