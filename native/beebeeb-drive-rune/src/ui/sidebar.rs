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
    // Board: sections col px=12 gives icon at x=1+12+8=21, text ml=8 → x=45 ✓
    // Board: items h=29, gap=0 gives text tops 29px apart matching board y positions
    let mut sections = col().key("sidebar-sections").px(12.0).gap(0.0);

    for &section in SidebarSection::all() {
        let is_active = active == section;
        let badge = section.badge();

        let mut item = row()
            .key(format!("{}-row", section.label()))
            .role("button")
            .label(section.label())
            .h(29.0) // Board: items h=29 (gap=0 → text rows 29px apart)
            .px(space::SM)
            .gap(8.0) // Board gap=8 between flex items (icon|text|badge); constrains text grow width
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
                // Board key on the TEXT (board measures text: x=45, h=17, not the row h=29)
                // gap(8.0) on row already provides icon→text spacing; no extra ml needed
                text(section.label())
                    .key(section.label())
                    .font(if is_active { type_::BODY_SEMI } else { type_::BODY })
                    .color(t.text)
                    .grow(1.0),
            );

        if let Some(count) = badge {
            // Board keys: "Shared-badge" (18×18 pill for single digit),
            // "Trash-badge" (plain text for double digit)
            let badge_key: String = format!("{}-badge", section.label());
            if count < 10 {
                // Single-digit: circular pill badge (board: Shared-badge w=18, h=18)
                item = item.child(
                    el().key(badge_key)
                        .size(18.0, 18.0)
                        .radius(9.0)
                        .fill(t.separator)
                        .center()
                        .child(text(format!("{count}")).font(type_::BODY_SM).color(t.text_secondary)),
                );
            } else {
                // Double-digit: plain text (board: Trash-badge w=14, h=14)
                item = item.child(
                    text(format!("{count}"))
                        .key(badge_key)
                        .font(type_::BODY_SM)
                        .color(t.text_tertiary)
                        .w(14.0), // board: Trash-badge w=14
                );
            }
        }

        sections = sections.child(item);
    }

    // THIS PC section
    // Board: this-pc-label x=11, y=246, w=199, h=16
    // Trash row bottom: y=235. mt=11 → this_pc starts at y=246.
    // Label h=16 (forced), mb=1 → effective=17 → Sync history row at y=263.
    // col pl=0, pr=11 → items (pl=20) icon at x=21, text at x=45. Badge right at 201.
    let this_pc = col()
        .key("this-pc")
        .pl(0.0) // no col left padding; label uses ml=10 for x=11
        .pr(12.0) // col right: 221-12=209; item pr=8 → content right=201; badge right=201 ✓
        .mt(11.0) // Board: gap from Trash row bottom (y=235) to this-pc-label (y=246) = 11
        .gap(0.0)
        .child(
            text("THIS PC")
                .key("this-pc-label")
                .font(type_::SECTION_LABEL)
                .color(t.text_tertiary)
                .ml(10.0) // → x = 1(sidebar) + 10 = 11 ✓ (board: x=11)
                .w(199.0) // board: this-pc-label w=199
                .h(16.0) // board: h=16; ensures effective height 17 with mb=1
                .mb(1.0), // 1px margin-bottom: Sync history row top = 246+16+1=263; text at 269 ✓
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

    // Board: storage-label y=705. Meter top = 706 with pb=14 (shrinks spacer, pulls meter up).
    // pb=14 makes meter_h=54; spacer fills 385px; meter_top(abs)=33+57+145+86+385=706. Δy=1 ✓.
    let meter = col()
        .key("storage-meter")
        .px(space::MD)
        .pb(14.0) // board: extra bottom padding makes meter taller → spacer shorter → meter higher
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
                        .color(t.text_tertiary)
                        .ml(8.0), // board: storage-label x=21=1+12(px)+8(ml) ✓
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
    // Board: account-name x=53. With sidebar x=1, pl=12, avatar 26px: gap = 53-1-12-26=14.
    // Board: account-name y=760 = account_row_top → no center_y; children top-aligned.
    // account_row_top = sidebar_bottom-40 = (33+767)-40=760. name at 760, email at 775 ✓.
    let account_row = row()
        .key("account-row")
        .h(40.0)
        .w_full()
        .px(space::MD)
        // No center_y: board shows account-name at y=account_row_top (top-aligned)
        .gap(14.0) // board: gap=14 between avatar and account-info (53-1-12-26=14)
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
                .child(
                    text("Anna Visser")
                        .key("account-name")
                        .font(type_::BODY_SEMI)
                        .color(t.text)
                        .w(115.0), // board: account-name w=115 (text ink width)
                )
                .child(
                    text("anna@northwind.st\u{2026}")
                        .key("account-email")
                        .font(type_::BODY_SM)
                        .color(t.text_tertiary)
                        .w(115.0), // board: account-email w=115 (text ink width)
                ),
        )
        .child(
            icon(&icons::SETTINGS, 15.0)
                .key("settings-icon")
                .color(t.icon_secondary),
        );

    // Sidebar header: holds the drive-label "DRIVE" text at board position x=50, y=64.
    // Board: drive-label x=50 (relative x=49 from sidebar left), y=64 (relative y=31 from sidebar top).
    // Header h=57 so sections start at relative y=57 (absolute y=90), giving Files text at y=96 ✓.
    let sidebar_header = el()
        .key("sidebar-header")
        .w_full()
        .h(57.0)
        .pt(31.0) // drive-label top at relative y=31 → absolute y=33+31=64 ✓
        .pl(49.0) // drive-label at relative x=49 → absolute x=1+49=50 ✓
        .child(
            text("DRIVE")
                .key("drive-label")
                .font(type_::CAPTION_MONO_SM)
                .color(t.text_secondary)
                .w(81.0) // board: drive-label w=81 (wider due to letter-spacing in board CSS)
                .h(12.0), // board: drive-label h=12 (line-height 12 in board CSS)
        );

    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .fill(t.sidebar_bg)
        // No pt: sidebar-header element provides the top spacing
        .child(sidebar_header)
        .child(sections)
        .child(this_pc)
        .child(el().key("sidebar-spacer").grow(1.0))
        .child(meter)
        .child(account_row)
}

fn this_pc_item(label: &'static str, ic: &'static rune::Icon, count: Option<u32>, t: &Theme) -> El {
    // Board: this-pc items h=29. col pl=0 → item pl=20 gives icon at x=21, text at x=45.
    // Board key on the TEXT (board measures text position, not row).
    let mut item = row()
        .key(format!("{label}-row"))
        .h(29.0) // Board: h=29 to match section item rows
        .pl(20.0) // col has pl=0, so pl=20 on item → icon at x=1+20=21, text at x=45 ✓
        .pr(space::SM)
        .gap(8.0) // Board gap=8: icon→text→badge spacing; no extra ml on text
        .center_y()
        .radius(radius::MD)
        .hover(t.hover)
        .child(icon(ic, 16.0).key("icon").color(t.icon_secondary))
        .child(
            // Board key on TEXT (e.g. "Sync history", "Available offline")
            // gap(8.0) on row provides icon→text spacing; no extra ml needed
            text(label)
                .key(label) // board key: board measures text at x=45, h=17
                .font(type_::BODY)
                .color(t.text)
                .grow(1.0),
        );

    if let Some(c) = count {
        // Board key: "<label>-badge" (e.g. "Available offline-badge")
        // Board shows Available offline-badge as plain text "3" (w=7, h=14)
        let badge_key = format!("{label}-badge");
        item = item.child(
            text(format!("{c}"))
                .key(badge_key)
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
