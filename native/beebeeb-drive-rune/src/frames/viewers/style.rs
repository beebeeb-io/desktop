//! The open-file group's type styles and the colours the board mixes for it, where
//! `theme::tokens` has no entry yet (listed in the PR body for the design-system lane).

use rune::tok::Family;
use rune::{Color, TextStyle};

use crate::theme::tokens::Theme;

/// `.mk-crumbs i`: a folder of the path, 11px/500, tracked -0.01em.
pub const CRUMB: TextStyle = TextStyle::new(11.0, 500.0).lh(13.0).tracking(-0.11);
/// `.mk-crumbs > span`: the open file's name, 11px/650.
pub const CRUMB_NAME: TextStyle = TextStyle::new(11.0, 650.0).lh(13.0).tracking(-0.11);
/// `.t-title`: 17px/600 on 22px, tracked -0.012em.
pub const TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
/// `.t-sub`: 12px/400 on 15px.
pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// `.t-sub .font-mono`.
pub const SUB_MONO: TextStyle = TextStyle::new(12.0, 400.0).family(Family::Mono).lh(15.0);
/// `.kit-btn-sm` label: 12px/500.
pub const BUTTON_SM: TextStyle = TextStyle::new(12.0, 500.0).lh(17.1429).tracking(-0.06);
/// `.kit-btn-md` label: 13px/500.
pub const BUTTON_MD: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5714).tracking(-0.065);

fn pick(t: &Theme, light: &str, dark: &str) -> Color {
    rune::color::hex(if t.dark { dark } else { light }).unwrap_or(Color::BLACK)
}

/// The ground behind a file that is not a page of text: `color-mix(in oklch, ink 6%, ground)`.
pub fn stage(t: &Theme) -> Color {
    pick(t, "#E7E4E0", "#1A1713")
}

/// `--mk-warn`: `color-mix(in oklch, amber-deep 45%, red)`, the status dot of "not done, not broken".
pub fn warn(t: &Theme) -> Color {
    pick(t, "#C94B00", "#F67222")
}

/// `--color-red` as the board resolves it.
pub fn red(t: &Theme) -> Color {
    pick(t, "#EA3C3F", "#F54748")
}

/// `.kit-tile.tone-neutral`'s wash.
pub fn tile_neutral(t: &Theme) -> Color {
    Color {
        a: 18.0 / 255.0,
        ..pick(t, "#0E0E0E", "#E3E3E3")
    }
}

/// `.kit-tile.tone-red`'s wash: red at 12 %.
pub fn tile_red(t: &Theme) -> Color {
    Color {
        a: 31.0 / 255.0,
        ..pick(t, "#E63A42", "#F74A4A")
    }
}

/// `.mk-bar`'s track: ink at 9 %.
pub fn bar_track(t: &Theme) -> Color {
    Color {
        a: 23.0 / 255.0,
        ..pick(t, "#16160B", "#E9E9DE")
    }
}
