//! Where the Beebeeb window opens (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.2, task 1683
//! slice 1): the answer to "randomly in the screen".
//!
//! - First open: centred on the display that holds the menu-bar icon.
//! - After that: where it was left, at the size it was left. The window-state plugin restores SIZE and POSITION for
//!   `main` only, never its visibility, and every other label is untracked.
//! - Off-screen guard: if less than half of the restored frame is on a connected display, centre it again as on
//!   first open.
//!
//! The macOS build writes a file of its own ([`FILENAME_V2`]), so nothing a 0.8.x build saved (the popover-era
//! `settings` entry, a visible `main-app`) is ever read back on a Mac. Windows and Linux keep the default file and
//! their denylist (`lib.rs`), unchanged.
//!
//! Everything here is a plain function of plain inputs, in physical pixels (what tauri reports for monitors and what
//! the plugin stores), so it is tested on Linux CI. `lib.rs` (`place_main_window`) feeds it the real monitors.

use tauri_plugin_window_state::StateFlags;

/// The macOS build's state file. Never the plugin's default file, which 0.8.x builds wrote.
pub const FILENAME_V2: &str = "window-state-v2.json";

/// The only label the macOS build tracks: the Beebeeb window.
pub const TRACKED_LABEL: &str = "main";

/// `with_filter` callback: `true` saves and restores the window, `false` skips it.
pub fn tracks_label(label: &str) -> bool {
    label == TRACKED_LABEL
}

/// Size and position only. Never VISIBLE (the plugin would show a window on restore), DECORATIONS, MAXIMIZED or
/// FULLSCREEN.
pub fn state_flags() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION
}

/// A physical-pixel rectangle in global desktop coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl PRect {
    fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }
    fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }
    fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }
    fn contains_point(&self, x: i32, y: i32) -> bool {
        (x as i64) >= self.x as i64 && (x as i64) < self.right() && (y as i64) >= self.y as i64 && (y as i64) < self.bottom()
    }
    fn overlap(&self, other: &PRect) -> u64 {
        let w = self.right().min(other.right()) - (self.x as i64).max(other.x as i64);
        let h = self.bottom().min(other.bottom()) - (self.y as i64).max(other.y as i64);
        if w <= 0 || h <= 0 { 0 } else { (w as u64) * (h as u64) }
    }
}

/// A display as tauri reports it: its whole frame (`Monitor::position` + `size`, which includes the menu bar where
/// the status item sits) and its work area (`Monitor::work_area`, without the menu bar and the Dock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalDisplay {
    pub frame: PRect,
    pub work_area: PRect,
}

/// The display whose frame holds this point (the menu-bar icon's centre), if any.
pub fn display_containing(x: i32, y: i32, displays: &[PhysicalDisplay]) -> Option<usize> {
    displays.iter().position(|d| d.frame.contains_point(x, y))
}

/// The top-left corner that centres a window of `size` in the display's work area. A window larger than the work
/// area keeps its top-left corner inside it, so its title bar is always reachable.
pub fn centred_origin(size: (u32, u32), display: &PhysicalDisplay) -> (i32, i32) {
    let area = display.work_area;
    let x = area.x as i64 + (area.w as i64 - size.0 as i64) / 2;
    let y = area.y as i64 + (area.h as i64 - size.1 as i64) / 2;
    (x.max(area.x as i64) as i32, y.max(area.y as i64) as i32)
}

/// Spec §7.2's guard: at least half of `frame` lies on the work areas of connected displays. A frame with no area
/// is never on screen.
pub fn mostly_on_screen(frame: PRect, displays: &[PhysicalDisplay]) -> bool {
    let total = frame.area();
    if total == 0 {
        return false;
    }
    let on: u64 = displays.iter().map(|d| frame.overlap(&d.work_area)).sum();
    on * 2 >= total
}

/// After the saved frame was restored (or none was), where the window must move: `None` keeps it; `Some` is the
/// centred origin on the icon's display, else on `fallback` (the primary display), when the guard fails.
pub fn main_window_origin(
    frame: PRect,
    displays: &[PhysicalDisplay],
    icon_display: Option<usize>,
    fallback: usize,
) -> Option<(i32, i32)> {
    if mostly_on_screen(frame, displays) {
        return None;
    }
    let display = displays.get(icon_display.unwrap_or(fallback)).or_else(|| displays.first())?;
    Some(centred_origin((frame.w, frame.h), display))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 14" MacBook Pro built-in (2x: 3024 x 1964 px, menu bar and notch band 74 px, Dock 90 px) and a 1x external
    // to its right (1920 x 1080, menu bar 24 px, no Dock).
    fn displays() -> [PhysicalDisplay; 2] {
        [
            PhysicalDisplay {
                frame: PRect { x: 0, y: 0, w: 3024, h: 1964 },
                work_area: PRect { x: 0, y: 74, w: 3024, h: 1800 },
            },
            PhysicalDisplay {
                frame: PRect { x: 3024, y: 0, w: 1920, h: 1080 },
                work_area: PRect { x: 3024, y: 24, w: 1920, h: 1056 },
            },
        ]
    }

    /// 920 x 620 pt at 2x.
    fn frame(x: i32, y: i32) -> PRect {
        PRect { x, y, w: 1840, h: 1240 }
    }

    #[test]
    fn the_plugin_tracks_the_beebeeb_window_and_nothing_else() {
        assert!(tracks_label("main"));
        for other in [
            "popover",
            "settings",
            "macos-settings",
            "onboarding",
            "review",
            "tray",
            "windows-onboarding",
            "main-app",
            "conflict-abc",
            "",
            "Main",
        ] {
            assert!(!tracks_label(other), "{other:?} must not be tracked");
        }
    }

    #[test]
    fn flags_are_size_and_position_only() {
        let f = state_flags();
        assert!(f.contains(StateFlags::SIZE) && f.contains(StateFlags::POSITION));
        for poison in [
            StateFlags::DECORATIONS,
            StateFlags::VISIBLE,
            StateFlags::MAXIMIZED,
            StateFlags::FULLSCREEN,
        ] {
            assert!(!f.intersects(poison), "{poison:?} must not be restored");
        }
    }

    #[test]
    fn the_macos_file_is_never_the_file_0_8_builds_wrote() {
        assert_ne!(FILENAME_V2, tauri_plugin_window_state::DEFAULT_FILENAME);
        assert_eq!(FILENAME_V2, "window-state-v2.json");
    }

    /// Type-checked, never instantiated. Do NOT call it with `tauri::Wry`: that links the real webview runtime into
    /// the test executable, and on Windows the lib test binary then fails to start (STATUS_ENTRYPOINT_NOT_FOUND,
    /// 0xc0000139, a comctl32 v6 import with no manifest), found by CI on PR #82.
    #[allow(dead_code)]
    fn plugin_for_macos<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
        tauri_plugin_window_state::Builder::default()
            .with_filename(FILENAME_V2)
            .with_filter(tracks_label)
            .with_state_flags(state_flags())
            .skip_initial_state(TRACKED_LABEL)
            .build::<R>()
    }

    #[test]
    fn the_plugin_builder_accepts_this_configuration() {
        let _builder = tauri_plugin_window_state::Builder::default()
            .with_filename(FILENAME_V2)
            .with_filter(tracks_label)
            .with_state_flags(state_flags())
            .skip_initial_state(TRACKED_LABEL);
    }

    #[test]
    fn the_menu_bar_icon_names_its_display_even_though_it_is_outside_the_work_area() {
        let d = displays();
        // The status item sits in the menu bar, above each work area.
        assert_eq!(display_containing(2800, 20, &d), Some(0));
        assert_eq!(display_containing(4800, 10, &d), Some(1));
        assert_eq!(display_containing(9000, 10, &d), None);
        assert_eq!(display_containing(3024, 0, &d), Some(1), "the left edge belongs to the display that starts there");
    }

    #[test]
    fn first_open_centres_in_the_work_area_of_the_icon_display() {
        let d = displays();
        // Built-in: (3024 - 1840) / 2 = 592, 74 + (1800 - 1240) / 2 = 354.
        assert_eq!(centred_origin((1840, 1240), &d[0]), (592, 354));
        // External, 920 x 620 pt at 1x: 3024 + 500, 24 + 218.
        assert_eq!(centred_origin((920, 620), &d[1]), (3524, 242));
    }

    #[test]
    fn a_window_larger_than_the_work_area_keeps_its_title_bar_on_screen() {
        let d = displays();
        assert_eq!(centred_origin((4000, 3000), &d[1]), (3024, 24));
    }

    #[test]
    fn a_frame_left_on_screen_stays_where_it_was() {
        let d = displays();
        assert_eq!(main_window_origin(frame(300, 200), &d, Some(1), 0), None);
        // Across both displays, entirely on screen: kept.
        assert_eq!(main_window_origin(PRect { x: 2500, y: 200, w: 1000, h: 600 }, &d, Some(0), 0), None);
    }

    #[test]
    fn exactly_half_on_screen_is_kept_and_less_than_half_is_centred_again() {
        let d = displays();
        // 920 x 620 pt at 1x on the external, its right part past the desktop's right edge (4944).
        let at = |x| PRect { x, y: 200, w: 920, h: 620 };
        assert!(mostly_on_screen(at(4944 - 460), &d), "460 of 920 columns on screen: half");
        assert!(!mostly_on_screen(at(4944 - 459), &d), "459 of 920: less than half");
        assert_eq!(main_window_origin(at(4944 - 459), &d, Some(0), 1), Some(centred_origin((920, 620), &d[0])));
    }

    #[test]
    fn a_frame_on_a_display_that_was_unplugged_is_centred_on_the_icon_display() {
        let d = displays();
        // Saved on a third display to the right that is gone now.
        let gone = frame(6000, 100);
        assert_eq!(main_window_origin(gone, &d, Some(1), 0), Some(centred_origin((1840, 1240), &d[1])));
    }

    #[test]
    fn without_a_known_icon_display_the_fallback_display_is_used() {
        let d = displays();
        assert_eq!(main_window_origin(frame(9000, 9000), &d, None, 1), Some(centred_origin((1840, 1240), &d[1])));
        // A fallback that no longer exists falls back to the first display.
        assert_eq!(main_window_origin(frame(9000, 9000), &d, None, 7), Some(centred_origin((1840, 1240), &d[0])));
    }

    #[test]
    fn under_the_menu_bar_counts_as_off_the_work_area() {
        let d = displays();
        // 1240 tall, only its bottom 400 px below the built-in's menu bar band: less than half on screen.
        assert!(!mostly_on_screen(frame(300, 74 + 400 - 1240), &d));
    }

    #[test]
    fn no_displays_or_an_empty_frame_never_counts_as_on_screen() {
        assert!(!mostly_on_screen(frame(0, 0), &[]));
        assert_eq!(main_window_origin(frame(0, 0), &[], Some(0), 0), None, "nothing to centre on");
        assert!(!mostly_on_screen(PRect { x: 100, y: 100, w: 0, h: 600 }, &displays()));
    }
}
