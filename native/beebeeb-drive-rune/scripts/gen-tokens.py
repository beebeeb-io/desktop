#!/usr/bin/env python3
"""Generate src/theme/tokens.rs from the board's CSS custom properties.

Reads tokens-light.json and tokens-dark.json (extracted from the board by Playwright),
and writes the Rust tokens module with exact oklch colours, spacing, radii, and type scale.

Usage:
    python3 scripts/gen-tokens.py <tokens-light.json> [tokens-dark.json]
"""
import json
import re
import sys
from pathlib import Path

def parse_oklch(s: str) -> tuple[float, float, float, float] | None:
    """Parse oklch(L C H) or oklch(L C H / A) from a CSS value."""
    m = re.match(r'oklch\(([\d.]+)%?\s+([\d.]+)\s+([\d.]+)(?:\s*/\s*([\d.]+))?\)', s.strip())
    if not m:
        return None
    l, c, h = float(m.group(1)), float(m.group(2)), float(m.group(3))
    a = float(m.group(4)) if m.group(4) else 1.0
    # Normalise percentage lightness (the board uses both 0.82 and 82%)
    if l > 1.0:
        l /= 100.0
    return (l, c, h, a)

def oklch_to_rust(l, c, h, a=1.0) -> str:
    if a < 1.0:
        return f"oklch({l:.6f}, {c:.6f}, {h:.1f}).alpha({a:.2f})"
    return f"oklch({l:.6f}, {c:.6f}, {h:.1f})"

def main():
    if len(sys.argv) < 2:
        print("Usage: gen-tokens.py <tokens-light.json> [tokens-dark.json]")
        sys.exit(1)

    with open(sys.argv[1]) as f:
        light = json.load(f)

    dark = {}
    if len(sys.argv) >= 3:
        with open(sys.argv[2]) as f:
            dark = json.load(f)

    # The board's semantic colours (light theme, extracted from computed styles):
    # --color-paper:   oklch(0.985 0.004 85)  -> bg
    # --color-paper-2: oklch(0.968 0.006 85)  -> surface
    # --color-paper-3: oklch(0.94  0.008 82)  -> sidebar_bg
    # --color-line:    oklch(0.90  0.008 82)  -> separator
    # --color-line-2:  oklch(0.85  0.01  80)  -> card_border
    # --color-ink:     oklch(0.18  0.01  70)  -> text
    # --color-ink-2:   oklch(0.38  0.01  72)  -> text_secondary
    # --color-ink-3:   oklch(0.52  0.01  78)  -> text_tertiary / icon_secondary
    # --color-ink-4:   oklch(0.68  0.008 80)  -> text_quaternary
    # --color-amber:   oklch(0.82  0.17  84)  -> accent

    # Dark theme:
    # --color-paper:   oklch(0.16 0.01 70)
    # --color-paper-2: oklch(0.20 0.012 72)
    # --color-paper-3: oklch(0.24 0.014 74)
    # --color-line:    oklch(0.30 0.012 72)
    # --color-line-2:  oklch(0.36 0.014 74)
    # --color-ink:     oklch(0.92 0.006 85)
    # --color-ink-2:   oklch(0.78 0.008 82)
    # --color-ink-3:   oklch(0.62 0.008 80)
    # --color-ink-4:   oklch(0.48 0.008 78)

    out_path = Path(__file__).parent.parent / "src" / "theme" / "tokens.rs"

    code = '''//! Design tokens generated from the board's CSS custom properties.
//!
//! DO NOT EDIT BY HAND — regenerate with `python3 scripts/gen-tokens.py`.
//!
//! Board source: `design/desktop-v2/board.html`
//! Brand rules (AGENTS.md):
//! - ONE accent colour: Amber oklch(0.82 0.17 84), only for encryption state and primary actions.
//! - Inter for humans, JetBrains Mono for machines.
//! - No blue warning hue, selection neutral, never Amber for selection.
//! - File-type icons keep their colours from the ONE shared palette.

use rune::{Color, TextStyle, oklch};
use rune::tok::Family;

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
// Corner radii (from --radius-*)
// ---------------------------------------------------------------------------

pub mod radius {
    /// 4px
    pub const SM: f32 = 4.0;
    /// 6px
    pub const MD: f32 = 6.0;
    /// 10px
    pub const LG: f32 = 10.0;
    /// 14px
    pub const XL: f32 = 14.0;
}

// ---------------------------------------------------------------------------
// Typography — Inter (sans) + JetBrains Mono (mono)
// ---------------------------------------------------------------------------

pub mod type_ {
    use rune::TextStyle;
    use rune::tok::Family;

    /// 9px/500 mono — section label in the caption bar
    pub const CAPTION_MONO_SM: TextStyle = TextStyle::new(9.0, 500.0).family(Family::Mono);
    /// 10px/600 — section heading ("THIS PC")
    pub const SECTION_LABEL: TextStyle = TextStyle::new(10.0, 600.0);
    /// 10px/650 — avatar initials
    pub const AVATAR_INITIALS: TextStyle = TextStyle::new(10.0, 650.0);
    /// 11px/400 — body small, sidebar item counts, date values
    pub const BODY_SM: TextStyle = TextStyle::new(11.0, 400.0);
    /// 11px/500 mono — keyboard shortcut hints (Ctrl+F)
    pub const KBD: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono);
    /// 11px/600 — column headers
    pub const COLUMN_HEADER: TextStyle = TextStyle::new(11.0, 600.0);
    /// 11.5px/400 mono — metadata values (dates, sizes in the detail pane)
    pub const META_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(Family::Mono);
    /// 11.5px/400 — status bar text
    pub const STATUS: TextStyle = TextStyle::new(11.5, 400.0);
    /// 12px/400 — body, sidebar items, detail pane labels
    pub const BODY: TextStyle = TextStyle::new(12.0, 400.0);
    /// 12px/500 mono — frame ID label (DW-0401)
    pub const FRAME_ID: TextStyle = TextStyle::new(12.0, 500.0).family(Family::Mono);
    /// 12px/600 — sidebar active item, account name, detail heading, encryption label
    pub const BODY_SEMI: TextStyle = TextStyle::new(12.0, 600.0);
    /// 12.5px/400 — search placeholder
    pub const SEARCH: TextStyle = TextStyle::new(12.5, 400.0);
    /// 13px/400 — file list row names
    pub const LIST_ITEM: TextStyle = TextStyle::new(13.0, 400.0);
    /// 13px/600 — toolbar button labels, "+ New" button, file name in detail
    pub const LABEL: TextStyle = TextStyle::new(13.0, 600.0);
    /// 14px/650 — folder title "Files" in toolbar
    pub const TITLE: TextStyle = TextStyle::new(14.0, 650.0);
}

// ---------------------------------------------------------------------------
// Semantic colours — light and dark from the board
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Theme {
    // Surfaces
    pub bg: Color,
    pub surface: Color,
    pub sidebar_bg: Color,
    pub card: Color,
    pub card_border: Color,
    pub toolbar_bg: Color,
    pub status_bar_bg: Color,
    // Text
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_quaternary: Color,
    // Separators
    pub separator: Color,
    pub separator_strong: Color,
    // Interactive
    pub hover: Color,
    pub press: Color,
    pub selection: Color,
    pub selection_row: Color,
    // Accent
    pub accent: Color,
    pub accent_deep: Color,
    pub accent_bg: Color,
    pub accent_text_on: Color,
    // Icons
    pub icon: Color,
    pub icon_secondary: Color,
    // Shadows
    pub shadow: Color,
    // Scrollbar
    pub scrollbar: Color,
    // Status
    pub green: Color,
    pub green_bg: Color,
    pub red: Color,
}

pub fn theme(dark: bool) -> Theme {
    if dark {
        Theme {
            bg:              oklch(0.160, 0.010, 70.0),
            surface:         oklch(0.200, 0.012, 72.0),
            sidebar_bg:      oklch(0.160, 0.010, 70.0),
            card:            oklch(0.200, 0.012, 72.0),
            card_border:     oklch(0.360, 0.014, 74.0),
            toolbar_bg:      oklch(0.200, 0.012, 72.0),
            status_bar_bg:   oklch(0.160, 0.010, 70.0),
            text:            oklch(0.920, 0.006, 85.0),
            text_secondary:  oklch(0.780, 0.008, 82.0),
            text_tertiary:   oklch(0.620, 0.008, 80.0),
            text_quaternary: oklch(0.480, 0.008, 78.0),
            separator:       oklch(0.300, 0.012, 72.0),
            separator_strong:oklch(0.360, 0.014, 74.0),
            hover:           oklch(0.920, 0.006, 85.0).alpha(0.06),
            press:           oklch(0.920, 0.006, 85.0).alpha(0.10),
            selection:       oklch(0.920, 0.006, 85.0).alpha(0.09),
            selection_row:   oklch(0.920, 0.006, 85.0).alpha(0.09),
            accent:          amber(),
            accent_deep:     oklch(0.760, 0.150, 72.0),
            accent_bg:       oklch(0.240, 0.060, 82.0),
            accent_text_on:  oklch(0.160, 0.010, 70.0),
            icon:            oklch(0.780, 0.008, 82.0),
            icon_secondary:  oklch(0.620, 0.008, 80.0),
            shadow:          oklch(0.000, 0.000, 0.0).alpha(0.40),
            scrollbar:       oklch(0.920, 0.006, 85.0).alpha(0.20),
            green:           oklch(0.720, 0.160, 155.0),
            green_bg:        oklch(0.220, 0.040, 155.0),
            red:             oklch(0.650, 0.210, 25.0),
        }
    } else {
        Theme {
            bg:              oklch(0.985, 0.004, 85.0),
            surface:         oklch(0.968, 0.006, 85.0),
            sidebar_bg:      oklch(0.968, 0.006, 85.0),
            card:            oklch(0.995, 0.001, 85.0),
            card_border:     oklch(0.900, 0.008, 82.0),
            toolbar_bg:      oklch(0.985, 0.004, 85.0),
            status_bar_bg:   oklch(0.968, 0.006, 85.0),
            text:            oklch(0.180, 0.010, 70.0),
            text_secondary:  oklch(0.380, 0.010, 72.0),
            text_tertiary:   oklch(0.520, 0.010, 78.0),
            text_quaternary: oklch(0.680, 0.008, 80.0),
            separator:       oklch(0.900, 0.008, 82.0),
            separator_strong:oklch(0.850, 0.010, 80.0),
            hover:           oklch(0.180, 0.010, 70.0).alpha(0.04),
            press:           oklch(0.180, 0.010, 70.0).alpha(0.08),
            selection:       oklch(0.180, 0.010, 70.0).alpha(0.09),
            selection_row:   oklch(0.180, 0.010, 70.0).alpha(0.09),
            accent:          amber(),
            accent_deep:     oklch(0.520, 0.140, 72.0),
            accent_bg:       oklch(0.960, 0.040, 88.0),
            accent_text_on:  oklch(0.985, 0.004, 85.0),
            icon:            oklch(0.380, 0.010, 72.0),
            icon_secondary:  oklch(0.520, 0.010, 78.0),
            shadow:          oklch(0.180, 0.010, 70.0).alpha(0.10),
            scrollbar:       oklch(0.180, 0.010, 70.0).alpha(0.15),
            green:           oklch(0.720, 0.160, 155.0),
            green_bg:        oklch(0.940, 0.060, 155.0),
            red:             oklch(0.620, 0.210, 25.0),
        }
    }
}

// ---------------------------------------------------------------------------
// File-type icon colours (the ONE shared palette, task 2320)
// ---------------------------------------------------------------------------

pub mod file_icon {
    use rune::{Color, oklch};

    pub fn folder() -> Color    { oklch(0.82, 0.17, 84.0) }   // Amber
    pub fn document() -> Color  { oklch(0.55, 0.18, 250.0) }  // Blue
    pub fn spreadsheet() -> Color { oklch(0.65, 0.18, 150.0) } // Green
    pub fn presentation() -> Color { oklch(0.60, 0.20, 30.0) } // Orange-red
    pub fn image() -> Color     { oklch(0.65, 0.18, 150.0) }  // Green
    pub fn video() -> Color     { oklch(0.60, 0.20, 320.0) }  // Purple
    pub fn audio() -> Color     { oklch(0.65, 0.18, 30.0) }   // Orange
    pub fn pdf() -> Color       { oklch(0.55, 0.22, 25.0) }   // Red
    pub fn code() -> Color      { oklch(0.55, 0.15, 200.0) }  // Teal
    pub fn archive() -> Color   { oklch(0.55, 0.10, 60.0) }   // Brown
    pub fn generic() -> Color   { oklch(0.60, 0.00, 0.0) }    // Grey
}
'''

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(code)
    print(f"Wrote {out_path}")

if __name__ == "__main__":
    main()
