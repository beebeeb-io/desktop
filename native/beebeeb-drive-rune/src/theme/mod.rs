//! Theme: design tokens and SVG icons extracted from the design board.

pub mod contrast;
pub mod file_icon_art;
pub mod file_icon_colors;
pub mod file_icons;
pub mod icons;
pub mod icons_files;
pub mod icons_settings;
pub mod scale;
pub mod tokens;

/// The theme to draw: the Windows contrast theme when one is on, else the tokens with the contrast fixes.
pub fn current(dark: bool) -> tokens::Theme {
    contrast::system_contrast().unwrap_or_else(|| contrast::themed(dark))
}
