//! Sidebar (`.mk-side`): the brand block, the sections, "This PC", the storage meter and
//! the account row.

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Span, col, el, row};

use super::kit;

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

    /// The board's tooltip: the name and its shortcut.
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::Files => "Ctrl+1",
            Self::Shared => "Ctrl+2",
            Self::Photos => "Ctrl+3",
            Self::Backups => "Ctrl+4",
            Self::Trash => "Ctrl+5",
        }
    }
}

/// How a sidebar item shows its count.
#[derive(Clone, Copy)]
pub enum Count {
    /// A pill: something waits for the person (`.mk-badge`).
    Pill(u32),
    /// A plain figure (`.mk-side-count`).
    Plain(u32),
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
                t.text_tertiary,
            ));
        }
        None => {}
    }
    item
}

/// The brand block (`.mk-side-brand`): the mark, the wordmark with its Amber ".io", and the
/// product's name under it.
pub fn brand(product: &str, t: &Theme) -> El {
    let word = "beebeeb.io";
    let io = Span {
        start: 7,
        end: word.len() as u32,
        color: Some(t.accent),
        ..Span::default()
    };
    row()
        .key("brand")
        .px(8.0)
        .py(2.0)
        .gap(6.461_54)
        .center_y()
        .child(kit::mark("brand-mark", 24.0))
        .child(
            col()
                .key("brand-text")
                .gap(1.846_15)
                .child(kit::label("brand-word", word, type_::WORDMARK, t.text).spans(vec![io]))
                .child(kit::label(
                    "drive-label",
                    product.to_uppercase(),
                    type_::CAPTION_MONO_SM,
                    t.text_secondary,
                )),
        )
}

/// The badge count for a section, from the state's live sources. `None`: no badge (the source
/// has nothing, or is unwired). The snapshot path's fixtures give the board's numbers; a live
/// window with an engine gives the engine's.
pub(crate) fn badge_for(s: &State, section: SidebarSection) -> Option<Count> {
    match section {
        SidebarSection::Shared => {
            let n = s.frames.shared.source.invitations().len() as u32;
            (n > 0).then_some(Count::Pill(n))
        }
        SidebarSection::Trash => {
            let n = s.frames.trash.source.totals().items as u32;
            (n > 0).then_some(Count::Plain(n))
        }
        _ => None,
    }
}

pub fn sidebar(s: &State, t: &Theme) -> El {
    let storage = s.storage;
    let active = s.sidebar_section;

    let mut sections = col().key("sidebar-sections").gap(1.0);
    for &section in SidebarSection::all() {
        let count = badge_for(s, section);
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

    let kept = s.this_pc.kept_count() as u32;
    let offline_count = (kept > 0).then_some(Count::Plain(kept));
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
            offline_count,
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

    // Storage meter (`.mk-store`). The split (Files / Photos) is from `storage_split` when
    // the engine provides it; the board's snapshot path passes `Some((files, photos))` from
    // the fixture; a live window without a wired split shows one plain segment.
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
    let quota = storage.quota_bytes.max(1) as f64;
    // The split: the board's snapshot path holds a fixture split in `storage_split`; a live
    // window shows one segment (total used) until the engine wires the category breakdown.
    let (seg_0, seg_1) = match s.storage_split {
        Some((files, _photos)) => {
            let files_frac = (files.min(storage.used_bytes) as f64 / quota) as f32;
            let photos_frac = ((storage.used_bytes - files).max(0) as f64 / quota) as f32;
            (files_frac, photos_frac)
        }
        None => {
            let used_frac = (storage.used_bytes as f64 / quota) as f32;
            (used_frac, 0.0)
        }
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
                .child(el().key("meter-fill-0").h_full().w(track_w * seg_0).fill(t.text))
                .child(
                    el().key("meter-fill-1")
                        .h_full()
                        .w(track_w * seg_1)
                        .fill(t.text_quaternary),
                ),
        );
    // The category breakdown: only when the source provides one (never a board sample
    // beside a real account's total).
    if let Some((files, _)) = s.storage_split {
        if s.storage_known {
            meter = meter.child(
                row()
                    .key("storage-breakdown")
                    .justify_between()
                    .child(kit::label(
                        "storage-files",
                        format!("Files {}", format_storage(files)),
                        type_::KBD,
                        t.text_tertiary,
                    ))
                    .child(kit::label(
                        "storage-photos",
                        format!("Photos {}", format_storage((storage.used_bytes - files).max(0))),
                        type_::KBD,
                        t.text_tertiary,
                    )),
            );
        }
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
