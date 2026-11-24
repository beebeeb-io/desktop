//! Tests: file icon colors.

use super::super::file_icons::Tint;
use super::super::tokens::theme;
use super::tint_color;

fn contrast(a: rune::Color, b: rune::Color) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn theme_bound(t: Tint) -> bool {
    matches!(t, Tint::Folder | Tint::Image | Tint::Data | Tint::Default)
}

#[test]
fn file_icon_colors_dark_hex_tints_reach_3_to_1() {
    let d = theme(true);
    for &t in Tint::ALL {
        let c = tint_color(t, true);
        let (bg, sf) = (contrast(c, d.bg), contrast(c, d.surface));
        println!("{t:?}: {} bg {bg:.2} surface {sf:.2}", c.hex());
        if !theme_bound(t) {
            assert!(bg >= 3.0 && sf >= 3.0, "{t:?} {}: bg {bg:.2} surface {sf:.2}", c.hex());
        }
    }
}

#[test]
fn file_icon_colors_dark_theme_bound_read_dark_tokens() {
    let d = theme(true);
    assert_eq!(tint_color(Tint::Folder, true), d.accent);
    assert_eq!(tint_color(Tint::Image, true), d.accent_deep);
    assert_eq!(tint_color(Tint::Data, true), d.text_tertiary);
    assert_eq!(tint_color(Tint::Default, true), d.text_tertiary);
}

#[test]
fn file_icon_colors_light_is_the_shared_palette() {
    let l = theme(false);
    let hex = |s: &str| rune::color::hex(s).unwrap();
    for &t in Tint::ALL {
        let want = match t {
            Tint::Folder => l.accent,
            Tint::Pdf => hex("#dc2626"),
            Tint::Word => hex("#2563eb"),
            Tint::Excel => hex("#16a34a"),
            Tint::PowerPoint => hex("#ea580c"),
            Tint::Image => l.accent_deep,
            Tint::Video => hex("#9333ea"),
            Tint::Audio => hex("#db2777"),
            Tint::Code => hex("#0d9488"),
            Tint::Archive => hex("#92400e"),
            Tint::Data | Tint::Default => l.text_tertiary,
            Tint::Markdown => hex("#0f766e"),
            Tint::Design => hex("#a855f7"),
        };
        assert_eq!(tint_color(t, false), want, "{t:?}");
    }
}
