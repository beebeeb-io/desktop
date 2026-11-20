//! The tint of each file-type icon: the ONE shared palette (Guus's ruling 1.9, 2026-11-02),
//! light values from `repos/web/src/components/file-icon.tsx`.
//!
//! Dark values, for task 2320 (web and mobile) to adopt. A hex tint keeps its light value when it
//! already reaches 3:1 against both dark grounds (`theme(true).bg` oklch(0.20 0.012 72) and
//! `.surface` oklch(0.16 0.010 70)); otherwise it keeps its oklch hue and chroma and is lightened
//! to the first 0.01 step of L that does. Contrast is WCAG, against bg / surface.
//!
//! | Tint       | Light     | Dark                               | Dark contrast |
//! |------------|-----------|------------------------------------|---------------|
//! | Folder     | accent    | accent (oklch 0.82 0.17 84)        | 10.25 / 10.98 |
//! | Pdf        | #dc2626   | #dc2626                            | 3.75 / 4.02   |
//! | Word       | #2563eb   | #2563eb                            | 3.51 / 3.76   |
//! | Excel      | #16a34a   | #16a34a                            | 5.50 / 5.89   |
//! | PowerPoint | #ea580c   | #ea580c                            | 5.09 / 5.46   |
//! | Image      | accent-deep | dark accent-deep (#eb9f2c)      | 8.23 / 8.82   |
//! | Video      | #9333ea   | #9333ea                            | 3.37 / 3.61   |
//! | Audio      | #db2777   | #db2777                            | 3.94 / 4.22   |
//! | Code       | #0d9488   | #0d9488                            | 4.84 / 5.19   |
//! | Archive    | #92400e   | #a14e20 (oklch 0.52 0.125 46.2)    | 3.13 / 3.35   |
//! | Data       | ink-3     | ink-3 (oklch 0.62 0.008 80)        | 4.97 / 5.33   |
//! | Markdown   | #0f766e   | #0f766e                            | 3.31 / 3.55   |
//! | Design     | #a855f7   | #a855f7                            | 4.58 / 4.91   |
//! | Default    | ink-3     | ink-3 (oklch 0.62 0.008 80)        | 4.97 / 5.33   |

#![allow(dead_code)]

use super::file_icons::Tint;
use super::tokens;

fn hex(s: &str) -> rune::Color {
    rune::color::hex(s).unwrap_or(rune::Color::BLACK)
}

pub fn tint_color(tint: Tint, dark: bool) -> rune::Color {
    let t = tokens::theme(dark);
    match tint {
        Tint::Folder => t.accent,
        Tint::Pdf => hex("#dc2626"),
        Tint::Word => hex("#2563eb"),
        Tint::Excel => hex("#16a34a"),
        Tint::PowerPoint => hex("#ea580c"),
        Tint::Image => t.accent_deep,
        Tint::Video => hex("#9333ea"),
        Tint::Audio => hex("#db2777"),
        Tint::Code => hex("#0d9488"),
        Tint::Archive => hex(if dark { "#a14e20" } else { "#92400e" }),
        Tint::Data | Tint::Default => t.text_tertiary,
        Tint::Markdown => hex("#0f766e"),
        Tint::Design => hex("#a855f7"),
    }
}

#[cfg(test)]
#[path = "file_icon_colors_tests.rs"]
mod tests;
