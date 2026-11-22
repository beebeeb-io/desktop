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
