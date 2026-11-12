//! The glyphs and the colours Sync history and Available offline add: each icon body is the
//! board's inline `<svg>` (the gate compares the path data), each colour the board's value.

use rune::{Color, Icon, oklch};

use crate::theme::tokens::Theme;

macro_rules! icon24 {
    ($name:expr, $body:expr) => {
        Icon {
            name: $name,
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        }
    };
}

/// This PC, as the origin of a change.
pub static LAPTOP: Icon = icon24!(
    "laptop",
    r#"<path d="M5 6.5A1.5 1.5 0 0 1 6.5 5h11A1.5 1.5 0 0 1 19 6.5V15H5z"></path><path d="M2.5 18.5h19"></path>"#
);

pub static PHONE: Icon = icon24!(
    "phone",
    r#"<path d="M8.5 3h7A1.5 1.5 0 0 1 17 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 7 19.5v-15A1.5 1.5 0 0 1 8.5 3z"></path><path d="M11 18h2"></path>"#
);

pub static TABLET: Icon = icon24!(
    "tablet",
    r#"<path d="M6.5 3h11A1.5 1.5 0 0 1 19 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5v-15A1.5 1.5 0 0 1 6.5 3z"></path><path d="M11 18h2"></path>"#
);

/// Try again.
pub static RETRY: Icon = icon24!(
    "retry",
    r#"<path d="M20 12a8 8 0 1 1-2.3-5.6L20 8.5"></path><path d="M20 4v4.5h-4.5"></path>"#
);

pub static DOC_VIDEO: Icon = icon24!(
    "doc-video",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 15l5-3-5-3v6z"></path>"#
);

pub static DOC_AUDIO: Icon = icon24!(
    "doc-audio",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M11 15a2 2 0 1 1-4 0 2 2 0 0 1 4 0zM11 15v-4l4-1"></path>"#
);

/// Clear the search.
pub static CLOSE: Icon = icon24!("close", r#"<path d="M18 6L6 18M6 6l12 12"></path>"#);

/// `--color-red` as a count's fill (`.mk-badge.is-red`), the same in both themes.
pub fn red_badge() -> Color {
    oklch(0.55, 0.20, 25.0)
}

/// `--color-red-text`: why an item did not sync.
pub fn red_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.70, 0.19, 25.0)
    } else {
        oklch(0.48, 0.19, 25.0)
    }
}

/// The bar under the toolbar (`.mk-bulk`).
pub fn bar_bg(t: &Theme) -> Color {
    if t.dark {
        oklch(0.2288, 0.01176, 72.52)
    } else {
        oklch(0.962_156, 0.001_775, 84.4)
    }
}

/// A note's wash (`.mk-banner`: ink at 6 %).
pub fn note_bg(t: &Theme) -> Color {
    t.text.alpha(0.06)
}

/// A photo's stand-in until the engine hands over pixels: the board's sample pictures
/// (`.mk-ph1` … `.mk-ph6`), as CSS angle and three stops.
pub fn sample_photo(n: u8) -> (f32, [(f32, [u8; 3]); 3]) {
    match n {
        1 => (
            160.0,
            [(0.0, [233, 201, 160]), (0.55, [185, 122, 80]), (1.0, [94, 74, 74])],
        ),
        2 => (
            170.0,
            [(0.0, [156, 196, 220]), (0.60, [233, 226, 207]), (1.0, [141, 154, 120])],
        ),
        3 => (
            150.0,
            [(0.0, [51, 67, 92]), (0.55, [122, 127, 150]), (1.0, [227, 181, 140])],
        ),
        6 => (
            120.0,
            [(0.0, [217, 223, 228]), (0.60, [142, 164, 180]), (1.0, [74, 88, 102])],
        ),
        _ => (
            180.0,
            [(0.0, [243, 217, 168]), (0.50, [213, 140, 106]), (1.0, [63, 58, 88])],
        ),
    }
}
