//! Where the Beebeeb window opens (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §7.2, task 1683
//! slice 1): the answer to "randomly in the screen".
//!
//! - First open: centred on the display that holds the menu-bar icon.
//! - After that: where it was left, at the size it was left; never its visibility, never full screen or zoomed.
//! - Off-screen guard: if less than half of that frame is on a connected display, centre it again as on first open.
//!
//! **Logical points, per display.** On a Mac, tao and tray-icon report every position as points × the scale of the
//! window or display it belongs to, so one "physical" number means different places on a 2× built-in and a 1×
//! external (review item 5 on PR #134). Everything here is in logical points (macOS global coordinates, top-left
//! origin): a display is its physical frame divided by its own scale, which is exact, and the window is placed with
//! logical sizes and positions, which AppKit takes as points on whichever display they fall on. For the same reason
//! the window-state plugin is not used on a Mac: it stores physical numbers without their scale, which cannot be
//! placed back on a mixed-DPI desk. `lib.rs` keeps the frame itself ([`MAIN_FRAME_FILENAME`], logical points),
//! written when the window closes and when the app quits, and plans the whole placement before it applies any of it
//! (review item 4). Windows and Linux keep the plugin, its default file and its denylist, unchanged.

use serde::{Deserialize, Serialize};

/// The Beebeeb window's frame on a Mac, in logical points, in the app's config directory. A new name: nothing a
/// 0.8.x build wrote is ever read back.
pub const MAIN_FRAME_FILENAME: &str = "main-window-frame.json";

/// A physical-pixel rectangle as tauri reports a monitor (`Monitor::position` + `size`, `Monitor::work_area`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// A rectangle in logical points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl LRect {
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
    fn area(&self) -> f64 {
        self.w.max(0.0) * self.h.max(0.0)
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    fn overlap(&self, other: &LRect) -> f64 {
        let w = self.right().min(other.right()) - self.x.max(other.x);
        let h = self.bottom().min(other.bottom()) - self.y.max(other.y);
        if w <= 0.0 || h <= 0.0 { 0.0 } else { w * h }
    }
    fn is_usable(&self) -> bool {
        [self.x, self.y, self.w, self.h].iter().all(|v| v.is_finite()) && self.w > 0.0 && self.h > 0.0
    }
}

fn valid_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
}

fn logical(rect: PRect, scale: f64) -> LRect {
    let s = valid_scale(scale);
    LRect {
        x: rect.x as f64 / s,
        y: rect.y as f64 / s,
        w: rect.w as f64 / s,
        h: rect.h as f64 / s,
    }
}

/// A display in logical points, built from what tauri reports (each value is that display's points × its scale), with
/// the scale it was built with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    pub frame: LRect,
    pub work_area: LRect,
    pub scale: f64,
}

impl Display {
    pub fn from_physical(frame: PRect, work_area: PRect, scale: f64) -> Display {
        Display {
            frame: logical(frame, scale),
            work_area: logical(work_area, scale),
            scale: valid_scale(scale),
        }
    }

    /// The menu bar's height: the band between the display's top and its work area (0 with an auto-hiding menu bar).
    fn menu_bar_height(&self) -> f64 {
        (self.work_area.y - self.frame.y).max(0.0)
    }
}

/// The display holding the menu-bar icon, from tray-icon's rect (points × the icon's own scale). A display holds the
/// icon when, read at that display's scale, the icon's centre is inside it. One holder is the answer. On a mixed-DPI
/// desk two can hold the same numbers (2800 px is 1400 pt on a 2× built-in and 2800 pt on a 1× external to its right);
/// then the one whose menu bar is as tall as the icon wins (review round 2, P2-4: the height only breaks a tie).
/// `None` when no display holds it, or the tie stays (the caller then uses the primary display).
pub fn icon_display(icon: PRect, displays: &[Display]) -> Option<usize> {
    let read = |display: &Display| logical(icon, display.scale);
    let holders: Vec<usize> = displays
        .iter()
        .enumerate()
        .filter(|(_, display)| {
            let at = read(display);
            display.frame.contains(at.x + at.w / 2.0, at.y + at.h / 2.0)
        })
        .map(|(index, _)| index)
        .collect();
    if holders.len() <= 1 {
        return holders.first().copied();
    }
    let fitting: Vec<usize> = holders
        .into_iter()
        .filter(|&index| {
            let display = &displays[index];
            let bar = display.menu_bar_height();
            bar > 0.0 && (read(display).h - bar).abs() <= 6.0
        })
        .collect();
    if fitting.len() == 1 { Some(fitting[0]) } else { None }
}

/// The frame that centres `size` in the display's work area. A window larger than the work area keeps its top-left
/// corner inside it, so its title bar is always reachable.
pub fn centred(size: (f64, f64), display: &Display) -> LRect {
    let area = display.work_area;
    LRect {
        x: (area.x + (area.w - size.0) / 2.0).max(area.x),
        y: (area.y + (area.h - size.1) / 2.0).max(area.y),
        w: size.0,
        h: size.1,
    }
}

/// Spec §7.2's guard: at least half of `frame` lies on the work areas of connected displays.
pub fn mostly_on_screen(frame: LRect, displays: &[Display]) -> bool {
    let total = frame.area();
    if total <= 0.0 {
        return false;
    }
    let on: f64 = displays.iter().map(|d| frame.overlap(&d.work_area)).sum();
    on * 2.0 >= total
}

/// The frame to keep when the window closes or the app quits: its outer position and its size, each divided by the
/// window's own scale. A full-screen or zoomed window keeps nothing, so the last ordinary frame stays (review item
/// 8: a full-screen or zoomed frame is never restored as an ordinary, screen-sized window).
pub fn frame_to_keep(
    position: (i32, i32),
    size: (u32, u32),
    scale: f64,
    fullscreen: bool,
    zoomed: bool,
) -> Option<LRect> {
    if fullscreen || zoomed {
        return None;
    }
    let frame = logical(
        PRect {
            x: position.0,
            y: position.1,
            w: size.0,
            h: size.1,
        },
        scale,
    );
    frame.is_usable().then_some(frame)
}

/// The kept frame, read back from [`MAIN_FRAME_FILENAME`]; anything that is not a usable frame is no frame.
pub fn parse_kept_frame(json: &str) -> Option<LRect> {
    serde_json::from_str::<LRect>(json).ok().filter(LRect::is_usable)
}

/// The whole placement, decided before any of it is applied: the kept frame when at least half of it is on screen;
/// else centred on the icon's display, else on `primary`, at the kept size (review round 2: an off-screen recovery
/// keeps the size it was left at) or, with no kept frame, at `default_size` (the window's own config).
pub fn plan_main_window(
    kept: Option<LRect>,
    displays: &[Display],
    icon: Option<usize>,
    primary: usize,
    default_size: (f64, f64),
) -> Option<LRect> {
    let kept = kept.filter(LRect::is_usable);
    if let Some(frame) = kept.filter(|frame| mostly_on_screen(*frame, displays)) {
        return Some(frame);
    }
    let display = icon
        .and_then(|i| displays.get(i))
        .or_else(|| displays.get(primary))
        .or_else(|| displays.first())?;
    Some(centred(kept.map_or(default_size, |frame| (frame.w, frame.h)), display))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f64, f64) = (920.0, 620.0);

    // Review item 5's desk: a 14" built-in at 2× (1512 × 982 pt; a 37 pt menu bar with the notch, a 70 pt Dock) and a
    // 1× external to its right (1920 × 1080 pt, a 25 pt menu bar). As tauri reports them: points × each one's scale.
    const BUILT_IN: f64 = 2.0;
    const EXTERNAL: f64 = 1.0;
    fn desk() -> [Display; 2] {
        [
            Display::from_physical(
                PRect { x: 0, y: 0, w: 3024, h: 1964 },
                PRect { x: 0, y: 74, w: 3024, h: 1750 },
                BUILT_IN,
            ),
            Display::from_physical(
                PRect { x: 1512, y: 0, w: 1920, h: 1080 },
                PRect { x: 1512, y: 25, w: 1920, h: 1055 },
                EXTERNAL,
            ),
        ]
    }

    /// The status item as tray-icon reports it: its frame in points × its own display's scale.
    fn icon_at(points_x: f64, bar_height: f64, scale: f64) -> PRect {
        PRect {
            x: (points_x * scale) as i32,
            y: 0,
            w: (24.0 * scale) as u32,
            h: (bar_height * scale) as u32,
        }
    }

    #[test]
    fn a_display_is_its_physical_frame_over_its_own_scale() {
        let d = desk();
        assert_eq!(d[0].frame, LRect { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 });
        assert_eq!((d[0].scale, d[1].scale), (BUILT_IN, EXTERNAL));
        assert_eq!(d[0].work_area, LRect { x: 0.0, y: 37.0, w: 1512.0, h: 875.0 });
        assert_eq!(d[1].frame, LRect { x: 1512.0, y: 0.0, w: 1920.0, h: 1080.0 });
    }

    #[test]
    fn the_icon_names_its_display_on_a_mixed_dpi_desk_on_either_display() {
        let d = desk();
        // On the built-in at x = 1400 pt: physical 2800, which at 1× would also fall on the external (1512..3432 pt).
        // Only the built-in's 37 pt menu bar is as tall as the icon (74 px / 2).
        assert_eq!(icon_display(icon_at(1400.0, 37.0, BUILT_IN), &d), Some(0));
        // On the external at x = 2900 pt: physical 2900, which at 2× would also fall on the built-in (1450 pt). Only
        // the external's 25 pt menu bar is as tall as the icon.
        assert_eq!(icon_display(icon_at(2900.0, 25.0, EXTERNAL), &d), Some(1));
        assert_eq!(icon_display(icon_at(3300.0, 25.0, EXTERNAL), &d), Some(1));
    }

    #[test]
    fn one_display_holding_the_icon_names_it_whatever_the_icon_height() {
        // Review round 2, P2-4: the height only breaks a tie. A single display that holds the icon is its display,
        // also when its menu bar is not the 37 pt the notch draws (a 39 pt bar, a bigger text size).
        let d = desk();
        assert_eq!(icon_display(icon_at(1400.0, 39.0, BUILT_IN), &d[..1]), Some(0));
        // On the external alone, with a 30 pt bar.
        assert_eq!(icon_display(icon_at(1600.0, 30.0, EXTERNAL), &d[1..]), Some(0));
        // Two 2× displays side by side: only one contains the icon at their (equal) scale, whatever its height.
        let twins = [
            Display::from_physical(PRect { x: 0, y: 0, w: 3024, h: 1964 }, PRect { x: 0, y: 74, w: 3024, h: 1750 }, 2.0),
            Display::from_physical(PRect { x: 3024, y: 0, w: 3024, h: 1964 }, PRect { x: 3024, y: 50, w: 3024, h: 1774 }, 2.0),
        ];
        assert_eq!(icon_display(icon_at(1400.0, 40.0, 2.0), &twins), Some(0));
        assert_eq!(icon_display(icon_at(2900.0, 40.0, 2.0), &twins), Some(1));
    }

    #[test]
    fn off_screen_recovery_keeps_the_kept_size() {
        // Review round 2, P3: centred again, at the size it was left, not the default.
        let d = desk();
        let gone = LRect { x: 4000.0, y: 100.0, w: 1000.0, h: 700.0 };
        assert_eq!(plan_main_window(Some(gone), &d, Some(1), 0, SIZE), Some(centred((1000.0, 700.0), &d[1])));
    }

    #[test]
    fn an_icon_no_display_claims_or_two_displays_claim_is_unknown() {
        let d = desk();
        assert_eq!(icon_display(icon_at(9000.0, 25.0, EXTERNAL), &d), None);
        // Auto-hiding menu bars: no height to tell the displays apart, and both fit, so the caller's primary decides.
        let hidden = [
            Display { work_area: d[0].frame, ..d[0] },
            Display { work_area: d[1].frame, ..d[1] },
        ];
        assert_eq!(icon_display(icon_at(1400.0, 37.0, BUILT_IN), &hidden), None);
    }

    #[test]
    fn first_open_centres_on_the_icon_display_in_points_whichever_display_holds_it() {
        let d = desk();
        // Built-in work area 1512 × 875 from y 37: (1512 - 920) / 2 = 296, 37 + (875 - 620) / 2 = 164.5.
        assert_eq!(plan_main_window(None, &d, Some(0), 0, SIZE), Some(LRect { x: 296.0, y: 164.5, w: 920.0, h: 620.0 }));
        // External work area 1920 × 1055 from (1512, 25): 1512 + 500, 25 + 217.5.
        assert_eq!(plan_main_window(None, &d, Some(1), 0, SIZE), Some(LRect { x: 2012.0, y: 242.5, w: 920.0, h: 620.0 }));
        // No icon display: the primary.
        assert_eq!(plan_main_window(None, &d, None, 1, SIZE), plan_main_window(None, &d, Some(1), 0, SIZE));
    }

    #[test]
    fn a_kept_frame_on_the_external_comes_back_exactly_as_it_was_left() {
        let d = desk();
        // Left on the 1× external at (2200, 300) pt, 1000 × 700: what `frame_to_keep` writes from tao's numbers.
        let kept = frame_to_keep((2200, 300), (1000, 700), EXTERNAL, false, false).unwrap();
        assert_eq!(kept, LRect { x: 2200.0, y: 300.0, w: 1000.0, h: 700.0 });
        assert_eq!(plan_main_window(Some(kept), &d, Some(0), 0, SIZE), Some(kept));
        // Left on the 2× built-in at (100, 80) pt, 920 × 620: tao reports (200, 160) and 1840 × 1240.
        let kept = frame_to_keep((200, 160), (1840, 1240), BUILT_IN, false, false).unwrap();
        assert_eq!(kept, LRect { x: 100.0, y: 80.0, w: 920.0, h: 620.0 });
        assert_eq!(plan_main_window(Some(kept), &d, Some(1), 0, SIZE), Some(kept));
    }

    #[test]
    fn exactly_half_on_screen_is_kept_and_less_than_half_is_centred_again() {
        let d = desk();
        // The desk's right edge is 3432 pt.
        let at = |x| LRect { x, y: 200.0, w: 920.0, h: 620.0 };
        assert!(mostly_on_screen(at(3432.0 - 460.0), &d), "460 of 920 points on screen: half");
        assert!(!mostly_on_screen(at(3432.0 - 459.0), &d), "459 of 920: less than half");
        assert_eq!(plan_main_window(Some(at(3432.0 - 459.0)), &d, Some(0), 1, SIZE), plan_main_window(None, &d, Some(0), 1, SIZE));
    }

    #[test]
    fn a_frame_on_a_display_that_was_unplugged_is_centred_on_the_icon_display() {
        let d = desk();
        let gone = LRect { x: 4000.0, y: 100.0, w: 920.0, h: 620.0 };
        assert_eq!(plan_main_window(Some(gone), &d, Some(1), 0, SIZE), plan_main_window(None, &d, Some(1), 0, SIZE));
        // Unplugged with only the built-in left.
        assert_eq!(plan_main_window(Some(gone), &d[..1], Some(0), 0, SIZE), plan_main_window(None, &d[..1], Some(0), 0, SIZE));
    }

    #[test]
    fn a_full_screen_or_zoomed_window_keeps_no_frame() {
        assert_eq!(frame_to_keep((0, 0), (3024, 1964), BUILT_IN, true, false), None);
        assert_eq!(frame_to_keep((0, 74), (3024, 1750), BUILT_IN, false, true), None);
        assert!(frame_to_keep((200, 160), (1840, 1240), BUILT_IN, false, false).is_some());
    }

    #[test]
    fn under_the_menu_bar_counts_as_off_the_work_area() {
        let d = desk();
        // 620 tall, only its bottom 200 pt below the built-in's 37 pt menu bar band: less than half on screen.
        assert!(!mostly_on_screen(LRect { x: 100.0, y: 37.0 + 200.0 - 620.0, w: 920.0, h: 620.0 }, &d));
    }

    #[test]
    fn a_window_larger_than_the_work_area_keeps_its_title_bar_on_screen() {
        let d = desk();
        assert_eq!(centred((4000.0, 3000.0), &d[1]), LRect { x: 1512.0, y: 25.0, w: 4000.0, h: 3000.0 });
    }

    #[test]
    fn the_file_round_trips_and_anything_else_is_no_frame() {
        let frame = LRect { x: 2200.0, y: 300.5, w: 1000.0, h: 700.0 };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(parse_kept_frame(&json), Some(frame));
        for bad in ["", "not json", "{}", r#"{"x":1,"y":2,"w":0,"h":5}"#, r#"{"x":1,"y":2,"w":-3,"h":5}"#] {
            assert_eq!(parse_kept_frame(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn no_displays_place_nothing_and_a_bad_scale_counts_as_one() {
        assert_eq!(plan_main_window(None, &[], None, 0, SIZE), None);
        let d = Display::from_physical(PRect { x: 0, y: 0, w: 1920, h: 1080 }, PRect { x: 0, y: 25, w: 1920, h: 1055 }, 0.0);
        assert_eq!(d.frame.w, 1920.0);
        assert_eq!(frame_to_keep((10, 10), (920, 620), f64::NAN, false, false), Some(LRect { x: 10.0, y: 10.0, w: 920.0, h: 620.0 }));
    }
}
