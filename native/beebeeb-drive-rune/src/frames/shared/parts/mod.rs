//! The pieces Shared, Search and Trash are built from, beside `crate::ui`: the section
//! window, a toolbar with a slot, worded segments, buttons, pills, a table with named
//! columns, grouped lists and the empty state. New components of this lane; none changes one
//! of `crate::ui`.

pub mod chrome;
pub mod controls;
pub mod icons;
pub mod list;
pub mod menu;
pub mod table;
pub mod toolbar;

use crate::theme::tokens::Theme;
use rune::{Color, oklch};

/// Type styles the board uses in these groups that `theme::tokens::type_` does not have yet.
pub mod style {
    use rune::TextStyle;
    /// `.kit-seg-sm span`: a worded segment.
    pub const SEG: TextStyle = TextStyle::new(12.0, 550.0).lh(14.5);
    /// `.t-sub` on the desktop.
    pub const SUB: TextStyle = crate::theme::scale::text::SUB;
    /// `.mk-cell.is-sans`: a list cell in words.
    pub const CELL: TextStyle = crate::theme::scale::text::NOTE;
    /// `.mk-cell`: a list cell in figures.
    pub const CELL_MONO: TextStyle = crate::theme::scale::text::META_MONO_TIGHT;
}

/// `--color-red-text`: words of a destructive action.
pub fn red_text(t: &Theme) -> Color {
    if t.dark {
        oklch(0.70, 0.19, 25.0)
    } else {
        oklch(0.48, 0.19, 25.0)
    }
}

/// `.kit-item.is-selected`: a selected item of a grouped list.
pub fn item_selected(t: &Theme) -> Color {
    t.text.alpha(if t.dark { 0.09 } else { 0.06 })
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
