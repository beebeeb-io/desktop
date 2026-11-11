//! Beebeeb design tokens derived from the design board and AGENTS.md brand rules.
//!
//! Brand rules (AGENTS.md):
//! - ONE accent colour: Amber `oklch(0.82 0.17 84)`, only for encryption state and primary actions.
//! - Inter for humans, JetBrains Mono for machines.
//! - Voice: honest over reassuring. No emojis in product UI.
//! - No blue warning hue, selection neutral, never Amber for selection.

use rune::{Color, TextStyle, oklch};

/// The Beebeeb Amber accent: `oklch(0.82, 0.17, 84)`.
pub fn amber() -> Color {
    oklch(0.82, 0.17, 84.0)
}

/// Spacing scale (points).
pub mod space {
    pub const S1: f32 = 4.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S5: f32 = 20.0;
    pub const S6: f32 = 24.0;
    pub const S7: f32 = 32.0;
    pub const S8: f32 = 40.0;
}

/// Corner radii.
pub mod radius {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
    pub const XL: f32 = 12.0;
}

/// Typography (Inter face, system default for now; bundled Inter in production).
pub mod type_ {
    use rune::TextStyle;

    pub const CAPTION: TextStyle = TextStyle::new(11.0, 400.0);
    pub const LABEL: TextStyle = TextStyle::new(12.0, 500.0);
    pub const BODY: TextStyle = TextStyle::new(13.0, 400.0);
    pub const BODY_MEDIUM: TextStyle = TextStyle::new(13.0, 500.0);
    pub const TITLE: TextStyle = TextStyle::new(13.0, 600.0);
    pub const HEADING: TextStyle = TextStyle::new(15.0, 600.0);
    pub const MONO: TextStyle = TextStyle::new(12.0, 400.0);
}

/// Semantic colour tokens, dark and light.
#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Color,
    pub surface: Color,
    pub sidebar_bg: Color,
    pub card: Color,
    pub card_border: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub separator: Color,
    pub hover: Color,
    pub press: Color,
    pub selection: Color,
    pub accent: Color,
    pub shadow: Color,
    pub icon: Color,
    pub icon_secondary: Color,
    pub status_bar_bg: Color,
    pub toolbar_bg: Color,
    pub scrollbar: Color,
}

pub fn theme(dark: bool) -> Theme {
    if dark {
        Theme {
            bg: oklch(0.15, 0.0, 0.0),
            surface: oklch(0.18, 0.0, 0.0),
            sidebar_bg: oklch(0.13, 0.0, 0.0),
            card: oklch(0.20, 0.0, 0.0),
            card_border: oklch(0.25, 0.0, 0.0),
            text: oklch(0.95, 0.0, 0.0),
            text_secondary: oklch(0.70, 0.0, 0.0),
            text_tertiary: oklch(0.50, 0.0, 0.0),
            separator: oklch(0.25, 0.0, 0.0),
            hover: oklch(0.95, 0.0, 0.0).alpha(0.06),
            press: oklch(0.95, 0.0, 0.0).alpha(0.10),
            selection: oklch(0.50, 0.0, 0.0).alpha(0.25),
            accent: amber(),
            shadow: oklch(0.0, 0.0, 0.0).alpha(0.40),
            icon: oklch(0.75, 0.0, 0.0),
            icon_secondary: oklch(0.55, 0.0, 0.0),
            status_bar_bg: oklch(0.12, 0.0, 0.0),
            toolbar_bg: oklch(0.17, 0.0, 0.0),
            scrollbar: oklch(0.50, 0.0, 0.0).alpha(0.3),
        }
    } else {
        Theme {
            bg: oklch(0.98, 0.0, 0.0),
            surface: oklch(0.96, 0.0, 0.0),
            sidebar_bg: oklch(0.94, 0.0, 0.0),
            card: oklch(1.0, 0.0, 0.0),
            card_border: oklch(0.88, 0.0, 0.0),
            text: oklch(0.15, 0.0, 0.0),
            text_secondary: oklch(0.45, 0.0, 0.0),
            text_tertiary: oklch(0.60, 0.0, 0.0),
            separator: oklch(0.88, 0.0, 0.0),
            hover: oklch(0.0, 0.0, 0.0).alpha(0.04),
            press: oklch(0.0, 0.0, 0.0).alpha(0.08),
            selection: oklch(0.50, 0.0, 0.0).alpha(0.15),
            accent: amber(),
            shadow: oklch(0.0, 0.0, 0.0).alpha(0.10),
            icon: oklch(0.35, 0.0, 0.0),
            icon_secondary: oklch(0.55, 0.0, 0.0),
            status_bar_bg: oklch(0.92, 0.0, 0.0),
            toolbar_bg: oklch(0.96, 0.0, 0.0),
            scrollbar: oklch(0.0, 0.0, 0.0).alpha(0.2),
        }
    }
}

/// File-type icon colours (the one palette shared by web, desktop, mobile per task 2320).
pub mod file_icon {
    use rune::{Color, oklch};

    pub fn folder() -> Color {
        oklch(0.82, 0.17, 84.0)
    }
    pub fn image() -> Color {
        oklch(0.65, 0.18, 150.0)
    }
    pub fn video() -> Color {
        oklch(0.60, 0.20, 320.0)
    }
    pub fn audio() -> Color {
        oklch(0.65, 0.18, 30.0)
    }
    pub fn document() -> Color {
        oklch(0.55, 0.18, 250.0)
    }
    pub fn spreadsheet() -> Color {
        oklch(0.65, 0.18, 150.0)
    }
    pub fn presentation() -> Color {
        oklch(0.60, 0.20, 30.0)
    }
    pub fn code() -> Color {
        oklch(0.55, 0.15, 200.0)
    }
    pub fn archive() -> Color {
        oklch(0.55, 0.10, 60.0)
    }
    pub fn pdf() -> Color {
        oklch(0.55, 0.22, 25.0)
    }
    pub fn generic() -> Color {
        oklch(0.60, 0.0, 0.0)
    }
}
