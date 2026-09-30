//! Window-state v2 for the macOS Settings window (spec section 6 rule 8).
//!
//! Why this exists: every 0.8.x install already has a `settings` entry in
//! `.window-state.json` that was saved from POPOVER mode (`decorated: false`, a
//! position under the menu bar, a 680 x 540 pt size). Narrowing the plugin's
//! flags would still restore that stale entry into the new 560 pt Settings
//! window and recreate P2 ("window appears at a random place") for exactly the
//! users who complained. So the macOS build gets a new file name, tracks one
//! label, and validates whatever it restores with [`validate_restore`].
//!
//! What this file does NOT decide: whether a valid saved position beats the
//! top-right anchor from [`super::anchor::place_settings`]. Ruling 6 says
//! Settings opens top right under the menu bar on the icon's display; rule 8
//! (written before that ruling) said "else centre". Slice 6 picks; this file
//! only says which saved entries are safe to use. It also never centres.

use serde::Deserialize;
use std::collections::HashMap;
use tauri_plugin_window_state::StateFlags;

use super::anchor::SETTINGS_WIDTH;

/// New file name, so the v1 `.window-state.json` written by every 0.8.x popover
/// is never read by the macOS build. (Windows and Linux keep the default file.)
pub const FILENAME_V2: &str = "window-state-v2.json";

/// The only label the macOS build tracks. The plugin's flags are global per
/// builder, not per label; the filter is what selects labels.
pub const TRACKED_LABEL: &str = "settings";

/// `with_filter` callback: `true` saves and restores the window, `false` skips it.
pub fn tracks_label(label: &str) -> bool {
    label == TRACKED_LABEL
}

/// Size and position only. Never DECORATIONS (the old file's poison), VISIBLE
/// (the plugin force-shows a window with no entry), MAXIMIZED or FULLSCREEN.
pub fn state_flags() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION
}

/// One entry of the plugin's state file. Field names and units are the
/// plugin's: physical pixels, global desktop coordinates. Keys the plugin
/// writes that we do not need (`prev_x`, `prev_y`) are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SavedWindowState {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "yes")]
    pub decorated: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

fn yes() -> bool {
    true
}

/// Pull the `settings` entry out of a state file's JSON (`{"<label>": {..}}`).
pub fn settings_entry_from_file(json: &str) -> Option<SavedWindowState> {
    let mut map: HashMap<String, SavedWindowState> = serde_json::from_str(json).ok()?;
    map.remove(TRACKED_LABEL)
}

/// A physical-pixel rectangle.
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
    fn contains_rect(&self, inner: &PRect) -> bool {
        (inner.x as i64) >= (self.x as i64)
            && (inner.y as i64) >= (self.y as i64)
            && inner.right() <= self.right()
            && inner.bottom() <= self.bottom()
    }
}

/// A display as tauri reports it: `Monitor::work_area()` (physical, excludes the
/// menu bar, notch band and Dock) and its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalDisplay {
    pub work_area: PRect,
    pub scale_factor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    /// No entry in the file.
    NoEntry,
    /// `decorated: false`. Settings is always decorated, so this entry was
    /// saved from popover mode: the exact poison this module exists to drop.
    PopoverEra,
    Maximized,
    Fullscreen,
    /// Zero width or height: the plugin's own "default" placeholder entry.
    Degenerate,
    /// The saved width is not the new window's width (the old one was 680 pt).
    WrongWidth,
    /// Not fully inside any connected display's work area (display unplugged,
    /// resolution changed, or a position under the menu bar).
    OffScreen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RestoreDecision {
    /// Safe to use. `display` indexes the `displays` slice.
    Restore {
        rect: PRect,
        display: usize,
    },
    Ignore(IgnoreReason),
}

/// Decide whether a saved entry may be applied to the Settings window.
pub fn validate_restore(saved: Option<&SavedWindowState>, displays: &[PhysicalDisplay]) -> RestoreDecision {
    use IgnoreReason::*;
    let Some(s) = saved else {
        return RestoreDecision::Ignore(NoEntry);
    };
    if !s.decorated {
        return RestoreDecision::Ignore(PopoverEra);
    }
    if s.maximized {
        return RestoreDecision::Ignore(Maximized);
    }
    if s.fullscreen {
        return RestoreDecision::Ignore(Fullscreen);
    }
    if s.width == 0 || s.height == 0 {
        return RestoreDecision::Ignore(Degenerate);
    }
    let rect = PRect {
        x: s.x,
        y: s.y,
        w: s.width,
        h: s.height,
    };
    let Some(display) = displays.iter().position(|d| d.work_area.contains_rect(&rect)) else {
        return RestoreDecision::Ignore(OffScreen);
    };
    let scale = if displays[display].scale_factor.is_finite() && displays[display].scale_factor > 0.0 {
        displays[display].scale_factor
    } else {
        1.0
    };
    // One logical pixel of slack for rounding on fractional scales.
    if (rect.w as f64 / scale - SETTINGS_WIDTH).abs() > 1.0 {
        return RestoreDecision::Ignore(WrongWidth);
    }
    RestoreDecision::Restore { rect, display }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real 0.8.x `.window-state.json` shape (the plugin writes every key),
    /// with the `settings` entry a popover left behind: undecorated, 680 x 540
    /// pt at 2x, just under the menu bar near the status item.
    const OLD_POPOVER_FILE: &str = r#"{
      "settings": {"width":1360,"height":1080,"x":1704,"y":34,"prev_x":1704,"prev_y":34,
                   "maximized":false,"visible":true,"decorated":false,"fullscreen":false},
      "onboarding": {"width":1720,"height":1280,"x":640,"y":260,"prev_x":640,"prev_y":260,
                   "maximized":false,"visible":true,"decorated":true,"fullscreen":false}
    }"#;

    // 14" MacBook Pro built-in (2x, work area below the 74 px notch band and
    // above a 90 px Dock) plus a 1x external to its right.
    fn displays() -> [PhysicalDisplay; 2] {
        [
            PhysicalDisplay {
                work_area: PRect {
                    x: 0,
                    y: 74,
                    w: 3024,
                    h: 1800,
                },
                scale_factor: 2.0,
            },
            PhysicalDisplay {
                work_area: PRect {
                    x: 3024,
                    y: 24,
                    w: 1920,
                    h: 1025,
                },
                scale_factor: 1.0,
            },
        ]
    }

    fn valid() -> SavedWindowState {
        SavedWindowState {
            width: 1120, // 560 pt at 2x
            height: 800,
            x: 1800,
            y: 80,
            maximized: false,
            visible: true,
            decorated: true,
            fullscreen: false,
        }
    }

    #[test]
    fn filter_tracks_settings_and_nothing_else() {
        assert!(tracks_label("settings"));
        for other in [
            "popover",
            "onboarding",
            "review",
            "tray",
            "windows-onboarding",
            "main-app",
            "conflict-abc",
            "",
            "Settings",
            "settings2",
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
    fn v2_file_name_never_reads_the_v1_file() {
        assert_ne!(FILENAME_V2, tauri_plugin_window_state::DEFAULT_FILENAME);
        assert_eq!(FILENAME_V2, "window-state-v2.json");
    }

    /// Type-checked, never instantiated. Do NOT call it with `tauri::Wry`: that
    /// links the real webview runtime into the test executable, and on Windows
    /// the lib test binary then fails to start (STATUS_ENTRYPOINT_NOT_FOUND,
    /// 0xc0000139, a comctl32 v6 import with no manifest) - found by CI on PR #82.
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
        // The generic fn above is the compile-time proof that tauri-plugin-window-state
        // 2.4 has every knob rule 8 depends on (`with_filename`, `with_filter`,
        // `with_state_flags`, `skip_initial_state`). This runs the builder half,
        // which needs no runtime, so nothing webview-shaped is linked.
        let _builder = tauri_plugin_window_state::Builder::default()
            .with_filename(FILENAME_V2)
            .with_filter(tracks_label)
            .with_state_flags(state_flags())
            .skip_initial_state(TRACKED_LABEL);
    }

    #[test]
    fn the_old_popover_entry_is_ignored() {
        let entry = settings_entry_from_file(OLD_POPOVER_FILE).expect("fixture has a settings entry");
        assert!(!entry.decorated, "fixture must be the popover-era entry");
        assert_eq!(
            validate_restore(Some(&entry), &displays()),
            RestoreDecision::Ignore(IgnoreReason::PopoverEra)
        );
    }

    #[test]
    fn the_old_popover_entry_is_ignored_even_if_it_were_decorated() {
        // Defence in depth: flip the decorated flag and the entry is STILL not
        // used, because its y (34 px) lies under the menu bar, outside the work
        // area (top 74 px).
        let mut entry = settings_entry_from_file(OLD_POPOVER_FILE).unwrap();
        entry.decorated = true;
        assert_eq!(
            validate_restore(Some(&entry), &displays()),
            RestoreDecision::Ignore(IgnoreReason::OffScreen)
        );
    }

    #[test]
    fn the_old_680_pt_width_is_ignored_even_at_a_valid_position() {
        // A regular-mode 0.8.x entry: decorated, on screen, but 680 pt wide
        // (1360 px at 2x) where the new window is 560 pt.
        let entry = SavedWindowState {
            width: 1360,
            height: 1080,
            x: 800,
            y: 300,
            ..valid()
        };
        assert_eq!(
            validate_restore(Some(&entry), &displays()),
            RestoreDecision::Ignore(IgnoreReason::WrongWidth)
        );
    }

    #[test]
    fn a_good_entry_is_restored_with_its_display() {
        assert_eq!(
            validate_restore(Some(&valid()), &displays()),
            RestoreDecision::Restore {
                rect: PRect {
                    x: 1800,
                    y: 80,
                    w: 1120,
                    h: 800
                },
                display: 0
            }
        );
    }

    #[test]
    fn width_is_judged_against_the_display_it_lands_on() {
        // 560 pt on the 1x external is 560 px; the same 1120 px there is 1120 pt.
        let on_external = SavedWindowState {
            width: 560,
            x: 3100,
            y: 100,
            height: 400,
            ..valid()
        };
        assert!(matches!(
            validate_restore(Some(&on_external), &displays()),
            RestoreDecision::Restore { display: 1, .. }
        ));
        let too_wide_there = SavedWindowState {
            width: 1120,
            ..on_external
        };
        assert_eq!(
            validate_restore(Some(&too_wide_there), &displays()),
            RestoreDecision::Ignore(IgnoreReason::WrongWidth)
        );
    }

    #[test]
    fn entries_outside_every_work_area_are_ignored() {
        // Off the right edge, under the menu bar (y < work area top), below the
        // Dock, and on a display that has been unplugged.
        for (x, y) in [(2500, 80), (1800, 20), (1800, 1300), (9000, 100)] {
            let s = SavedWindowState { x, y, ..valid() };
            assert_eq!(
                validate_restore(Some(&s), &displays()),
                RestoreDecision::Ignore(IgnoreReason::OffScreen),
                "({x}, {y})"
            );
        }
    }

    #[test]
    fn a_window_spanning_two_displays_is_not_fully_inside_either() {
        let s = SavedWindowState {
            x: 2500,
            width: 1120,
            ..valid()
        };
        assert_eq!(
            validate_restore(Some(&s), &displays()),
            RestoreDecision::Ignore(IgnoreReason::OffScreen)
        );
    }

    #[test]
    fn every_other_reason_is_reported_on_its_own() {
        let d = displays();
        assert_eq!(
            validate_restore(None, &d),
            RestoreDecision::Ignore(IgnoreReason::NoEntry)
        );
        assert_eq!(
            validate_restore(
                Some(&SavedWindowState {
                    maximized: true,
                    ..valid()
                }),
                &d
            ),
            RestoreDecision::Ignore(IgnoreReason::Maximized)
        );
        assert_eq!(
            validate_restore(
                Some(&SavedWindowState {
                    fullscreen: true,
                    ..valid()
                }),
                &d
            ),
            RestoreDecision::Ignore(IgnoreReason::Fullscreen)
        );
        // The plugin inserts an all-zero placeholder for a tracked window with
        // no disk entry (`visible` and `decorated` default to true).
        let placeholder = SavedWindowState {
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            ..valid()
        };
        assert_eq!(
            validate_restore(Some(&placeholder), &d),
            RestoreDecision::Ignore(IgnoreReason::Degenerate)
        );
    }

    #[test]
    fn no_displays_restores_nothing() {
        assert_eq!(
            validate_restore(Some(&valid()), &[]),
            RestoreDecision::Ignore(IgnoreReason::OffScreen)
        );
    }

    #[test]
    fn a_bad_scale_factor_is_treated_as_one_not_a_division_by_zero() {
        let d = [PhysicalDisplay {
            work_area: PRect {
                x: 0,
                y: 24,
                w: 1920,
                h: 1025,
            },
            scale_factor: 0.0,
        }];
        let s = SavedWindowState {
            width: 560,
            x: 100,
            y: 100,
            height: 400,
            ..valid()
        };
        assert!(matches!(
            validate_restore(Some(&s), &d),
            RestoreDecision::Restore { .. }
        ));
    }

    #[test]
    fn a_missing_settings_entry_or_garbage_yields_none() {
        assert_eq!(
            settings_entry_from_file(r#"{"onboarding":{"width":1,"height":1,"x":0,"y":0}}"#),
            None
        );
        assert_eq!(settings_entry_from_file("not json"), None);
        assert_eq!(settings_entry_from_file("{}"), None);
    }
}
