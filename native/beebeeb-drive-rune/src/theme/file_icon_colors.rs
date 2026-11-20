//! The tint of each icon. Stub: the light value in both themes; ux-05 fills in the real values.
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
        Tint::Archive => hex("#92400e"),
        Tint::Data | Tint::Default => t.text_tertiary,
        Tint::Markdown => hex("#0f766e"),
        Tint::Design => hex("#a855f7"),
    }
}
