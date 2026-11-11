//! The pieces Shared, Search and Trash are built from, beside `crate::ui`: the section
//! window, a toolbar with a slot, worded segments, buttons, pills, a table with named
//! columns, grouped lists and the empty state. New components of this lane; none changes one
//! of `crate::ui`.

pub mod chrome;
pub mod controls;
pub mod empty;
pub mod icons;
pub mod list;
pub mod sidebar;
pub mod table;
pub mod toolbar;

use crate::theme::tokens::Theme;
use rune::{Color, oklch};

/// Type styles the board uses in these groups that `theme::tokens::type_` does not have yet.
pub mod style {
    use rune::TextStyle;
    /// `.kit-seg-sm span`: a worded segment.
    pub const SEG: TextStyle = TextStyle::new(12.0, 550.0).lh(14.5);
    /// `.kit-btn-md` on the desktop.
    pub const BUTTON_MD: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5).tracking(-0.065);
    /// `.kit-pill`.
    pub const PILL: TextStyle = TextStyle::new(11.0, 600.0).lh(13.0).tracking(0.11);
    /// `.t-title` on the desktop: an empty state's or a dialog's title.
    pub const TITLE_LG: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.204);
    /// `.t-sub` on the desktop.
    pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
    /// `.mk-cell.is-sans`: a list cell in words.
    pub const CELL: TextStyle = TextStyle::new(12.0, 400.0).lh(14.5);
    /// `.mk-cell`: a list cell in figures.
    pub const CELL_MONO: TextStyle = TextStyle::new(11.5, 400.0).family(rune::tok::Family::Mono).lh(15.0);
}

/// `--color-red-text`: words of a destructive action.
pub fn red_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.70, 0.19, 25.0)
    } else {
        oklch(0.48, 0.19, 25.0)
    }
}

/// `--color-green-text`: words on a green wash.
pub fn green_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.72, 0.16, 155.0)
    } else {
        oklch(0.46, 0.13, 155.0)
    }
}

/// A board avatar hue by its letter (`.kit-avatar.hue-a` … `hue-e`): only the hue is data.
pub fn hue(letter: char) -> Color {
    let h = match letter {
        'a' => 250.0,
        'b' => 160.0,
        'c' => 20.0,
        'd' => 300.0,
        _ => 85.0,
    };
    oklch(0.88, 0.05, h)
}
