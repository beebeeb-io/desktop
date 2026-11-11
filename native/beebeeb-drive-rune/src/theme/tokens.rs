//! Design tokens: the board's CSS custom properties, by the board's names.
//!
//! Board source: `design/desktop-v2/board.html` (`--color-*`, `--kit-*`, `--mk-*`, the
//! `.kit-p-desktop` type scale). Maintained by hand against the board; the frame gate
//! (`scripts/compare.sh`) checks every use of them, text colour by value and fills by pixel.
//! Brand rules (AGENTS.md):
//! - ONE accent colour: Amber oklch(0.82 0.17 84), only for encryption state and primary actions.
//! - Inter for humans, JetBrains Mono for machines.
//! - No blue warning hue, selection neutral, never Amber for selection.

#![allow(dead_code)] // The whole scale is here; a frame uses the part it draws.

use rune::{Color, oklch};

// ---------------------------------------------------------------------------
// Accent
// ---------------------------------------------------------------------------

/// The Beebeeb Amber accent: oklch(0.82, 0.17, 84).
pub fn amber() -> Color {
    oklch(0.82, 0.17, 84.0)
}

/// Amber deep (used in encryption badge text): oklch(0.52, 0.14, 72).
pub fn amber_deep() -> Color {
    oklch(0.52, 0.14, 72.0)
}

// ---------------------------------------------------------------------------
// Spacing (from --spacing-*)
// ---------------------------------------------------------------------------

pub mod space {
    /// 4px
    pub const XS: f32 = 4.0;
    /// 8px
    pub const SM: f32 = 8.0;
    /// 12px
    pub const MD: f32 = 12.0;
    /// 18px (board's --spacing-lg)
    pub const LG: f32 = 18.0;
    /// 24px
    pub const XL: f32 = 24.0;
    /// 36px
    pub const XXL: f32 = 36.0;
}

// ---------------------------------------------------------------------------
// Corner radii (from --radius-*; CONTROL and CARD are the desktop kit's)
// ---------------------------------------------------------------------------

pub mod radius {
    /// 4px
    pub const SM: f32 = 4.0;
    /// 6px
    pub const MD: f32 = 6.0;
    /// 7px: rows, toolbar buttons, sidebar items (`.mk-row`, `.mk-tbtn`, `.mk-side-item`)
    pub const CONTROL: f32 = 7.0;
    /// 10px: cards and menus (`--card-r`)
    pub const LG: f32 = 10.0;
    /// 14px
    pub const XL: f32 = 14.0;
}

// ---------------------------------------------------------------------------
// Typography: Inter (sans) + JetBrains Mono (mono). Size/weight, then the board's
// line height and letter spacing where it sets them.
// ---------------------------------------------------------------------------

pub mod type_ {
    use rune::TextStyle;
    use rune::tok::Family;

    /// 9px/500 mono, tracked 0.18em: the product name under the wordmark ("DRIVE")
    pub const CAPTION_MONO_SM: TextStyle = TextStyle::new(9.0, 500.0).family(Family::Mono).lh(12.0).tracking(1.62);
    /// 10px/600, tracked 0.07em: sidebar section heading ("THIS PC")
    pub const SECTION_LABEL: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.7);
    /// 10px/650: avatar initials in the account row (`avatar_initials(size)` for other sizes)
    pub const AVATAR_INITIALS: TextStyle = TextStyle::new(10.0, 650.0);
    /// 10.5px/600, tabular figures: count pill in the sidebar (`.mk-badge`)
    pub const BADGE: TextStyle = TextStyle::new(10.5, 600.0).lh(13.0).features("'tnum' 1");
    /// 11px/400: small body (the account's e-mail, the toolbar's item count)
    pub const BODY_SM: TextStyle = TextStyle::new(11.0, 400.0).lh(14.0);
    /// 11px/500: caption (`.t-cap`)
    pub const CAPTION: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
    /// 11px/500 mono: caption figures (storage, counts) and keyboard hints (Ctrl+F)
    pub const KBD: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);
    /// 11px/600: column headers
    pub const COLUMN_HEADER: TextStyle = TextStyle::new(11.0, 600.0).lh(14.0);
    /// 11.5px/400 mono: sizes and times (`.mk-cell`, `.mk-kv dd.m`)
    pub const META_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono).lh(16.0);
    /// 11.5px/400: status line
    pub const STATUS: TextStyle = TextStyle::new(11.5, 400.0).lh(17.0);
    /// 11.5px/500: status line's state ("Up to date"), the Shared column's "Link"
    pub const STATUS_STRONG: TextStyle = TextStyle::new(11.5, 500.0).lh(17.0);
    /// 12px/400: details rows (`.mk-kv`), a list item's second line, the caption title
    pub const BODY: TextStyle = TextStyle::new(12.0, 400.0).lh(16.0);
    /// 12px/500, tracked -0.005em: small button label (`.kit-btn-sm`)
    pub const BUTTON_SM: TextStyle = TextStyle::new(12.0, 500.0).lh(17.0).tracking(-0.06);
    /// 12px/500 mono: frame ID label (DW-0401)
    pub const FRAME_ID: TextStyle = TextStyle::new(12.0, 500.0).family(Family::Mono);
    /// 12px/600: account name, the encryption mark
    pub const BODY_SEMI: TextStyle = TextStyle::new(12.0, 600.0).lh(15.0);
    /// 12.5px/400: search field
    pub const SEARCH: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
    /// 13px/500: file names, sidebar items (`.mk-name > b`, `.mk-side-item`)
    pub const LIST_ITEM: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0);
    /// 13px/500, tracked -0.005em: a list item's title (`.kit-item-title`)
    pub const ITEM_TITLE: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0).tracking(-0.065);
    /// 13px/600: the "+ New" button
    pub const LABEL: TextStyle = TextStyle::new(13.0, 600.0).lh(17.0);
    /// 13px/600, tracked -0.006em: heading (`.t-head`), the file name in the details panel
    pub const HEAD: TextStyle = TextStyle::new(13.0, 600.0).lh(17.0).tracking(-0.078);
    /// 14px/650, tracked -0.01em: folder title in the toolbar
    pub const TITLE: TextStyle = TextStyle::new(14.0, 650.0).lh(17.0).tracking(-0.14);
    /// 11px/400 on a 13px line: the toolbar title's second line ("11 items")
    pub const TITLE_SUB: TextStyle = TextStyle::new(11.0, 400.0).lh(13.0);
    /// 15.69px/700, tracked -0.0278em, line height 1: the "beebeeb.io" wordmark at 24px mark size
    pub const WORDMARK: TextStyle = TextStyle::new(15.6923, 700.0).lh(15.6923).tracking(-0.4359);

    /// Avatar initials for an avatar `d` points across (the board: 8px in a 20px avatar, 10px in 26px).
    pub const fn avatar_initials(size: f32) -> TextStyle {
        TextStyle::new(size, 650.0)
    }
}

// ---------------------------------------------------------------------------
// Semantic colours, light and dark, from the board
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    // Surfaces
    /// The content area: lists, toolbar, status line (`--mk-content` = `--kit-card`).
    pub bg: Color,
    /// The ground: sidebar, caption, details panel (`--kit-ground`).
    pub surface: Color,
    pub sidebar_bg: Color,
    /// A raised card on the ground (`--kit-card`).
    pub card: Color,
    /// A drawn 1px border (`--color-line`): outlined buttons.
    pub card_border: Color,
    pub toolbar_bg: Color,
    pub status_bar_bg: Color,
    /// A recessed field: search, the segmented control's well (`--kit-field`).
    pub field: Color,
    /// `--color-paper`: an outlined button's fill.
    pub paper: Color,
    /// `--color-paper-3`: a switch's off track.
    pub paper3: Color,
    /// `--color-white`, in both themes: a page preview, a switch knob.
    pub white: Color,
    // Text (`--color-ink` … `--color-ink-4`)
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_quaternary: Color,
    // Lines
    /// The hairline (`--kit-hair`): separators and rings, drawn `kit::HAIR` wide.
    pub separator: Color,
    /// `--color-line-2`: a switch's border.
    pub separator_strong: Color,
    /// The top highlight inside a raised surface (`--kit-hi`).
    pub highlight: Color,
    // Interactive
    pub hover: Color,
    pub press: Color,
    /// Selected or switched-on, neutral (`--mk-selbg`). Never Amber.
    pub selection: Color,
    pub selection_row: Color,
    /// A file tile's wash (`.mk-ftile`: ink at 7%).
    pub tile: Color,
    /// A preview well's wash (`.mk-prev`: ink at 5%).
    pub well: Color,
    /// A count pill's wash (`.mk-badge`: ink at 12%).
    pub badge: Color,
    // Accent
    pub accent: Color,
    pub accent_deep: Color,
    pub accent_bg: Color,
    /// The encryption mark's wash (`.mk-enc`: Amber at 18%).
    pub accent_wash: Color,
    /// Ink on Amber (`.mk-new`), the same in both themes.
    pub accent_text_on: Color,
    // Icons
    pub icon: Color,
    pub icon_secondary: Color,
    // Shadows (black at the board's strengths)
    pub shadow: Color,
    /// Black at 12%: a raised segment (`.mk-tgroup .on`).
    pub shadow_raised: Color,
    /// Black at 14%: a page preview's drop shadow and its text lines.
    pub shadow_page: Color,
    /// Black at 22%: a menu's drop shadow.
    pub shadow_menu: Color,
    // Scrollbar
    pub scrollbar: Color,
    // Status
    pub green: Color,
    pub green_bg: Color,
    pub red: Color,
    // Windows caption buttons (system colours, not the board's: hover and the red close)
    pub caption_hover: Color,
    pub caption_close: Color,
}

impl Theme {
    /// An avatar's tint from its hue: only the hue is data (`.kit-avatar.hue-*`).
    pub fn avatar_tint(&self, source: Color) -> Color {
        let [_, a, b, _] = source.to_oklab();
        let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
        if self.dark {
            oklch(0.72, 0.07, hue)
        } else {
            oklch(0.88, 0.05, hue)
        }
    }

    /// Initials on an avatar: ink in light, paper in dark (`.kit-screen.dark .kit-avatar`).
    pub fn avatar_ink(&self) -> Color {
        if self.dark { oklch(0.16, 0.01, 70.0) } else { self.text }
    }
}

pub fn theme(dark: bool) -> Theme {
    let black = Color::BLACK;
    let white = Color::WHITE;
    if dark {
        let (paper, paper2, paper3) = (
            oklch(0.16, 0.010, 70.0),
            oklch(0.20, 0.012, 72.0),
            oklch(0.24, 0.014, 74.0),
        );
        let ink = oklch(0.92, 0.006, 85.0);
        Theme {
            dark,
            bg: paper2,
            surface: paper,
            sidebar_bg: paper,
            card: paper2,
            card_border: oklch(0.30, 0.012, 72.0),
            toolbar_bg: paper2,
            status_bar_bg: paper2,
            field: paper3,
            paper,
            paper3,
            white,
            text: ink,
            text_secondary: oklch(0.78, 0.008, 82.0),
            text_tertiary: oklch(0.62, 0.008, 80.0),
            text_quaternary: oklch(0.48, 0.008, 78.0),
            separator: ink.alpha(0.12),
            separator_strong: oklch(0.36, 0.014, 74.0),
            highlight: white.alpha(0.08),
            hover: ink.alpha(0.06),
            press: ink.alpha(0.11),
            selection: ink.alpha(0.09),
            selection_row: ink.alpha(0.09),
            tile: ink.alpha(0.07),
            well: ink.alpha(0.05),
            badge: ink.alpha(0.12),
            accent: amber(),
            accent_deep: oklch(0.76, 0.15, 72.0),
            accent_bg: oklch(0.24, 0.06, 82.0),
            accent_wash: amber().alpha(0.18),
            accent_text_on: oklch(0.22, 0.01, 70.0),
            icon: oklch(0.78, 0.008, 82.0),
            icon_secondary: oklch(0.62, 0.008, 80.0),
            shadow: black.alpha(0.30),
            shadow_raised: black.alpha(0.12),
            shadow_page: black.alpha(0.14),
            shadow_menu: black.alpha(0.22),
            scrollbar: ink.alpha(0.20),
            green: oklch(0.72, 0.16, 155.0),
            green_bg: oklch(0.22, 0.04, 155.0),
            red: oklch(0.65, 0.21, 25.0),
            caption_hover: white.alpha(0.06),
            caption_close: rune::color::hex("#c42b1c").unwrap_or(Color::BLACK),
        }
    } else {
        let (paper, paper2, paper3) = (
            oklch(0.985, 0.004, 85.0),
            oklch(0.968, 0.006, 85.0),
            oklch(0.94, 0.008, 82.0),
        );
        let ink = oklch(0.18, 0.010, 70.0);
        // color-mix(in oklch, paper 35%, white) and color-mix(in oklch, paper-2 70%, paper-3)
        let card = oklch(0.99475, 0.0014, 85.0);
        let field = oklch(0.9596, 0.0066, 84.1);
        Theme {
            dark,
            bg: card,
            surface: paper2,
            sidebar_bg: paper2,
            card,
            card_border: oklch(0.90, 0.008, 82.0),
            toolbar_bg: card,
            status_bar_bg: card,
            field,
            paper,
            paper3,
            white,
            text: ink,
            text_secondary: oklch(0.38, 0.010, 72.0),
            text_tertiary: oklch(0.52, 0.010, 78.0),
            text_quaternary: oklch(0.68, 0.008, 80.0),
            separator: ink.alpha(0.09),
            separator_strong: oklch(0.85, 0.010, 80.0),
            highlight: white.alpha(0.70),
            hover: ink.alpha(0.04),
            press: ink.alpha(0.11),
            selection: ink.alpha(0.09),
            selection_row: ink.alpha(0.09),
            tile: ink.alpha(0.07),
            well: ink.alpha(0.05),
            badge: ink.alpha(0.12),
            accent: amber(),
            accent_deep: oklch(0.52, 0.14, 72.0),
            accent_bg: oklch(0.96, 0.04, 88.0),
            accent_wash: amber().alpha(0.18),
            accent_text_on: oklch(0.22, 0.01, 70.0),
            icon: oklch(0.38, 0.010, 72.0),
            icon_secondary: oklch(0.52, 0.010, 78.0),
            shadow: ink.alpha(0.06),
            shadow_raised: black.alpha(0.12),
            shadow_page: black.alpha(0.14),
            shadow_menu: black.alpha(0.22),
            scrollbar: ink.alpha(0.15),
            green: oklch(0.72, 0.16, 155.0),
            green_bg: oklch(0.94, 0.06, 155.0),
            red: oklch(0.62, 0.21, 25.0),
            caption_hover: black.alpha(0.06),
            caption_close: rune::color::hex("#c42b1c").unwrap_or(Color::BLACK),
        }
    }
}

// ---------------------------------------------------------------------------
// File-type icon colours (the ONE shared palette, task 2320)
// ---------------------------------------------------------------------------

pub mod file_icon {
    use rune::{Color, oklch};

    pub fn folder() -> Color {
        oklch(0.82, 0.17, 84.0)
    } // Amber
    pub fn document() -> Color {
        oklch(0.55, 0.18, 250.0)
    } // Blue
    pub fn spreadsheet() -> Color {
        oklch(0.65, 0.18, 150.0)
    } // Green
    pub fn presentation() -> Color {
        oklch(0.60, 0.20, 30.0)
    } // Orange-red
    pub fn image() -> Color {
        oklch(0.65, 0.18, 150.0)
    } // Green
    pub fn video() -> Color {
        oklch(0.60, 0.20, 320.0)
    } // Purple
    pub fn audio() -> Color {
        oklch(0.65, 0.18, 30.0)
    } // Orange
    pub fn pdf() -> Color {
        oklch(0.55, 0.22, 25.0)
    } // Red
    pub fn code() -> Color {
        oklch(0.55, 0.15, 200.0)
    } // Teal
    pub fn archive() -> Color {
        oklch(0.55, 0.10, 60.0)
    } // Brown
    pub fn generic() -> Color {
        oklch(0.60, 0.00, 0.0)
    } // Grey
}
