//! Which windows exist, and their sizes.
//!
//! The board allows two surfaces (DW-1201: "At most two Beebeeb surfaces: this flyout and the
//! one window"). Settings, sign-in, the viewer and the conflict choice are states of the main
//! window (`app.rs`), so there is no window kind for them.

use rune::WindowOptions;

use super::flyout;
use super::window_state::{Monitor, Restored};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowKind {
    Main,
    Flyout,
}

pub const MAIN_KEY: &str = "main";
pub const FLYOUT_KEY: &str = "flyout";
/// The board draws the window at 1280 x 800 (DW-0401); 0.8.x opens at the same size.
pub const MAIN_SIZE: (f32, f32) = (1280.0, 800.0);
/// 0.8.x's minimum (`tauri.conf.json`: 960 x 640). The board states none.
pub const MAIN_MIN_SIZE: (f32, f32) = (960.0, 640.0);

impl WindowKind {
    /// The key Rune's headless tools select the window by (`--window flyout`).
    pub fn key(self) -> &'static str {
        match self {
            WindowKind::Main => MAIN_KEY,
            WindowKind::Flyout => FLYOUT_KEY,
        }
    }

    /// In points.
    pub fn default_size(self) -> (f32, f32) {
        match self {
            WindowKind::Main => MAIN_SIZE,
            WindowKind::Flyout => flyout::SIZE,
        }
    }

    pub fn min_size(self) -> (f32, f32) {
        match self {
            WindowKind::Main => MAIN_MIN_SIZE,
            WindowKind::Flyout => flyout::SIZE,
        }
    }
}

/// Rune's displays as the geometry code reads them.
pub fn monitors(displays: &[rune::windows::Display]) -> Vec<Monitor> {
    displays
        .iter()
        .map(|d| Monitor {
            x: d.position.0,
            y: d.position.1,
            w: d.size.0,
            h: d.size.1,
            scale: d.scale,
            primary: d.primary,
        })
        .collect()
}

/// The main window: resizable, never under its minimum, back where it was.
pub fn main_options(restored: &Restored) -> WindowOptions {
    WindowOptions {
        position: restored.position,
        physical_size: restored.physical_size,
        min_size: Some(MAIN_MIN_SIZE),
        ..WindowOptions::default()
    }
}

/// The flyout: no frame, fixed size, above other windows, hidden until the b is clicked.
pub fn flyout_options() -> WindowOptions {
    WindowOptions {
        borderless: true,
        always_on_top: true,
        visible: false,
        resizable: false,
        min_size: Some(flyout::SIZE),
        max_size: Some(flyout::SIZE),
        ..WindowOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_windows_with_the_boards_sizes() {
        assert_eq!(WindowKind::Main.default_size(), (1280.0, 800.0));
        assert_eq!(WindowKind::Main.min_size(), (960.0, 640.0));
        assert_eq!(WindowKind::Flyout.default_size(), (340.0, 520.0));
        assert_eq!(WindowKind::Flyout.min_size(), WindowKind::Flyout.default_size());
        assert_ne!(WindowKind::Main.key(), WindowKind::Flyout.key());
    }

    #[test]
    fn the_flyout_is_fixed_frameless_topmost_and_starts_hidden() {
        let o = flyout_options();
        assert!(o.borderless && o.always_on_top && !o.visible && !o.resizable);
        assert_eq!((o.min_size, o.max_size), (Some((340.0, 520.0)), Some((340.0, 520.0))));
    }

    #[test]
    fn the_main_window_opens_where_it_was_restored_to() {
        let o = main_options(&Restored {
            position: Some((-1200, 40)),
            physical_size: Some((1600, 1000)),
            maximized: false,
        });
        assert_eq!((o.position, o.physical_size), (Some((-1200, 40)), Some((1600, 1000))));
        assert!(o.resizable && o.visible && !o.borderless);
        assert_eq!(o.min_size, Some((960.0, 640.0)));
        let o = main_options(&Restored::default());
        assert_eq!((o.position, o.physical_size), (None, None));
    }
}
