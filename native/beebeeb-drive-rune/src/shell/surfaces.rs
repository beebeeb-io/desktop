//! Which windows exist, and their sizes.
//!
//! The board allows two surfaces at a time (DW-1201: "At most two Beebeeb surfaces: this
//! flyout and the one window"). Settings, the viewer and the conflict choice are states of the
//! main window (`app.rs`), so there is no window kind for them.
//!
//! **The first-run window is a kind of its own**, 960 x 640. The board: DW-A01 "First-run
//! window 960 × 640 with the brand panel at the left"; DW-A02 (sign-in) "Window with the brand
//! panel"; DW-A07 "the 960 × 640 window"; DW-0201 and seven more first-run steps "A window of
//! its own between sign-in and the main window (960 × 640, like the shared first-run window)".
//! So sign-in and the first-run steps are drawn in it, and it comes BEFORE the main window,
//! never beside it: at any moment there is one window and the flyout. The shell keeps one
//! native window for both kinds and gives it the kind's size when the kind changes
//! ([`kind_for`], [`size_on_change`]); nothing of the main window's saved place is written
//! while it is the first-run window.

use rune::WindowOptions;

use super::app::{GateState, Screen};
use super::flyout;
use super::window_state::{Monitor, Restored, SavedWindow};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowKind {
    Main,
    /// Sign-in and the first-run steps, before the main window (960 x 640).
    FirstRun,
    Flyout,
}

pub const MAIN_KEY: &str = "main";
pub const FLYOUT_KEY: &str = "flyout";
/// Not a Rune window key of its own (the first-run window is the one window in that mode);
/// the name a first-run lane's saved state would go under.
pub const FIRST_RUN_KEY: &str = "first-run";
/// DW-A01: "First-run window 960 × 640".
pub const FIRST_RUN_SIZE: (f32, f32) = (960.0, 640.0);
/// The board draws the window at 1280 x 800 (DW-0401); 0.8.x opens at the same size.
pub const MAIN_SIZE: (f32, f32) = (1280.0, 800.0);
/// 0.8.x's minimum (`tauri.conf.json`: 960 x 640). The board states none.
pub const MAIN_MIN_SIZE: (f32, f32) = (960.0, 640.0);

impl WindowKind {
    /// The key Rune's headless tools select the window by (`--window flyout`).
    pub fn key(self) -> &'static str {
        match self {
            WindowKind::Main => MAIN_KEY,
            WindowKind::FirstRun => FIRST_RUN_KEY,
            WindowKind::Flyout => FLYOUT_KEY,
        }
    }

    /// In points.
    pub fn default_size(self) -> (f32, f32) {
        match self {
            WindowKind::Main => MAIN_SIZE,
            WindowKind::FirstRun => FIRST_RUN_SIZE,
            WindowKind::Flyout => flyout::SIZE,
        }
    }

    pub fn min_size(self) -> (f32, f32) {
        match self {
            WindowKind::Main => MAIN_MIN_SIZE,
            WindowKind::FirstRun => FIRST_RUN_SIZE,
            WindowKind::Flyout => flyout::SIZE,
        }
    }
}

/// Which window a screen of the route machine is drawn in.
pub fn kind_for(screen: &Screen) -> WindowKind {
    match screen {
        Screen::SignIn | Screen::Gate(GateState::FirstRun) => WindowKind::FirstRun,
        Screen::Gate(_) | Screen::Drive(_) | Screen::Settings(_) => WindowKind::Main,
    }
}

/// The size (in points) the one window takes when its kind changes from `from` to `to`:
/// the first-run window's fixed size, or the main window's saved size (its default when
/// nothing was saved). `None`: nothing changes.
pub fn size_on_change(from: WindowKind, to: WindowKind, saved_main: Option<&SavedWindow>) -> Option<(f32, f32)> {
    if from == to {
        return None;
    }
    match to {
        WindowKind::FirstRun => Some(FIRST_RUN_SIZE),
        WindowKind::Main => Some(match saved_main {
            Some(saved) if !saved.maximized => (
                (f64::from(saved.w) / saved.scale).max(f64::from(MAIN_MIN_SIZE.0)) as f32,
                (f64::from(saved.h) / saved.scale).max(f64::from(MAIN_MIN_SIZE.1)) as f32,
            ),
            _ => MAIN_SIZE,
        }),
        WindowKind::Flyout => None,
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

/// The one window opened as the first-run window: its fixed size, wherever the OS puts it.
pub fn first_run_options() -> WindowOptions {
    WindowOptions {
        min_size: Some(FIRST_RUN_SIZE),
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

    /// The first run IS a window of its own on the board (960 x 640), and sign-in is drawn in it.
    #[test]
    fn sign_in_and_the_first_run_steps_belong_to_the_first_run_window() {
        use super::super::app::{Route, Section, SettingsPane};
        assert_eq!(WindowKind::FirstRun.default_size(), (960.0, 640.0));
        assert_eq!(WindowKind::FirstRun.min_size(), (960.0, 640.0));
        assert_eq!(WindowKind::FirstRun.key(), "first-run");
        assert_eq!(kind_for(&Screen::SignIn), WindowKind::FirstRun);
        assert_eq!(kind_for(&Screen::Gate(GateState::FirstRun)), WindowKind::FirstRun);
        for screen in [
            Screen::Gate(GateState::Locked),
            Screen::Gate(GateState::NoPlan),
            Screen::Gate(GateState::NeedsRecoveryPhrase),
            Screen::Gate(GateState::KeyStoreUnavailable),
            Screen::Drive(Route::section(Section::Files)),
            Screen::Settings(SettingsPane::General),
        ] {
            assert_eq!(kind_for(&screen), WindowKind::Main, "{screen:?}");
        }
    }

    #[test]
    fn the_one_window_takes_its_kinds_size_when_the_kind_changes() {
        use WindowKind::{FirstRun, Main};
        let saved = SavedWindow {
            x: 10,
            y: 10,
            w: 3000,
            h: 1800,
            scale: 2.0,
            maximized: false,
        };
        assert_eq!(size_on_change(Main, Main, Some(&saved)), None);
        assert_eq!(size_on_change(FirstRun, FirstRun, None), None);
        assert_eq!(size_on_change(Main, FirstRun, Some(&saved)), Some((960.0, 640.0)));
        assert_eq!(size_on_change(FirstRun, Main, None), Some((1280.0, 800.0)));
        assert_eq!(size_on_change(FirstRun, Main, Some(&saved)), Some((1500.0, 900.0)));
        let tiny = SavedWindow {
            w: 100,
            h: 100,
            ..saved
        };
        assert_eq!(size_on_change(FirstRun, Main, Some(&tiny)), Some((960.0, 640.0)));
        let maximized = SavedWindow {
            maximized: true,
            ..saved
        };
        assert_eq!(size_on_change(FirstRun, Main, Some(&maximized)), Some((1280.0, 800.0)));
        assert_eq!(first_run_options().min_size, Some((960.0, 640.0)));
    }

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
