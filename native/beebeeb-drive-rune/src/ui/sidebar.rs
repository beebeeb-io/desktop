//! The Drive sidebar's pieces: the sections, an item, the brand, the standings. The sidebar
//! itself is `ui::drive_sidebar`; [`sidebar`] and [`sidebar_in`] draw it.

use crate::State;
use crate::theme::file_icon_colors::tint_color;
use crate::theme::file_icons::{FileIcon, IconId, Tint};
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Span, col, el, row};

use super::file_icon::file_icon;
use super::drive_sidebar::Lit;
use super::kit;

/// Which sidebar section is active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// The folder art a place is drawn with; Trash keeps its line glyph.
    pub fn file_icon(self) -> Option<FileIcon> {
        let (id, tint) = match self {
            Self::Files => (IconId::Folder, Tint::Folder),
            Self::Photos => (IconId::FolderPhotos, Tint::Image),
            Self::Shared => (IconId::FolderShared, Tint::Folder),
            Self::Backups => (IconId::FolderBackups, Tint::Folder),
            Self::Trash => return None,
        };
        Some(FileIcon { id, tint })
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
    let glyph = kit::glyph(
        format!("{label}-icon"),
        ic,
        16.0,
        if on { t.text } else { t.text_tertiary },
    );
    side_row(label, glyph, count, on, shortcut, t)
}

/// A sidebar place: the folder art in its tint (a selected row keeps the tint), or the line
/// glyph for Trash.
pub fn section_item(
    section: SidebarSection,
    count: Option<Count>,
    on: bool,
    t: &Theme,
) -> El {
    let label = section.label();
    let Some(fi) = section.file_icon() else {
        return side_item(label, section.icon(), count, on, section.shortcut(), t);
    };
    let glyph = file_icon(format!("{label}-icon"), fi, 16.0, t);
    side_row(label, glyph, count, on, section.shortcut(), t)
}

fn side_row(
    label: &'static str,
    glyph: El,
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
        .child(glyph)
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
            // The source's count: `None` is no badge (Beebeeb has not said a number for this session and account).
            let n = s.frames.trash.source.count().unwrap_or(0) as u32;
            (n > 0).then_some(Count::Plain(n))
        }
        _ => None,
    }
}

/// How the sidebar stands while the session's gate is closed (the board's DW-0009 and DW-0014).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    /// The window is open: a section is selected, every item can be pressed.
    Open,
    /// App lock is on (DW-0009): the same sidebar, dimmed, nothing selected, nothing pressable.
    Locked,
    /// The account has no plan (DW-0014): dimmed, no counts, "No plan" over an empty meter and
    /// where to get one.
    NoPlan,
}

/// Whether the locked window blanks the account's details (decision 2483 is open; the
/// recommendation is "blank when locked" and showing less is the safe side until ruled).
pub const LOCKED_BLANKS_ACCOUNT: bool = true;

impl Standing {
    /// Whether the account's email, its storage block and the items' counts are drawn in this
    /// standing. When `LOCKED_BLANKS_ACCOUNT` is true the locked window draws the sidebar's shape
    /// but no email, no storage figure and no section counts — neutral placeholders that keep the
    /// layout. The board's golden for DW-0009 then goes red on those keys; that red is expected.
    pub fn shows_account_details(self) -> bool {
        match self {
            Standing::Open | Standing::NoPlan => true,
            Standing::Locked => !LOCKED_BLANKS_ACCOUNT,
        }
    }
}

/// The Drive sidebar with the window's section lit (`ui::drive_sidebar`).
pub fn sidebar(s: &State, t: &Theme) -> El {
    sidebar_in(s, t, Standing::Open)
}

/// The sidebar in `standing`. [`sidebar`] is `Standing::Open`.
pub fn sidebar_in(s: &State, t: &Theme, standing: Standing) -> El {
    super::drive_sidebar::drive_sidebar(s, Lit::Section(s.sidebar_section), standing, t)
}

/// Format bytes as "240 GB", "1 TB" (base 10, as the board shows storage).
pub(crate) fn format_storage(bytes: i64) -> String {
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
    } else if bytes >= 1_000_000 {
        format!("{} MB", bytes / 1_000_000)
    } else if bytes >= 1_000 {
        format!("{} KB", bytes / 1_000)
    } else {
        // A new account: "0 B of 1 TB" (DW-0114).
        format!("{} B", bytes.max(0))
    }
}

#[cfg(test)]
#[path = "sidebar_icon_tests.rs"]
mod sidebar_icon_tests;
