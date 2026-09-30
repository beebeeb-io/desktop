//! Where the popover and the Settings window open (spec section 6 rule 2,
//! open question 6, section 9 "Mixed-DPI anchoring").
//!
//! Pure functions on LOGICAL points (top-left origin, y down). The caller
//! supplies the screens exactly as AppKit reports them (`NSScreen.frame` and
//! `visibleFrame`, flipped to a top-left origin) and the status item's rect in
//! the same space. No scale factor appears anywhere in this file, which is the
//! point: the 1384 follow-up bug and the false positive the first draft of this
//! design had (see `ambiguous_two_display_case_picks_the_external_display`)
//! both came from converting physical pixels with the wrong display's scale.
//! Converting is the caller's job; matching the icon to a display is ours.
//!
//! Rules, each with a test:
//! - The icon is matched to a display by its CENTRE against the display's FULL
//!   frame (the menu bar is outside `visible`, and the icon lives in it).
//!   Half-open, so a centre exactly on a shared edge belongs to the display on
//!   the right or below. A centre on no display falls back to the display the
//!   icon overlaps most; no overlap at all means "unavailable".
//! - Popover: centred under the icon, `ICON_GAP` below it, clamped into the
//!   display's visible frame with `SCREEN_MARGIN` on the left, right and
//!   bottom. The top edge is clamped to the visible frame itself, not inset by
//!   the margin: `ICON_GAP` (6) is smaller than `SCREEN_MARGIN` (8), so a margin
//!   on top would push every popover 2 pt lower than the spec's "icon bottom
//!   plus 6 pt". AppKit's visible frame already excludes the menu bar and the
//!   notch band.
//! - Height is `min(wanted, visible height - 2 * margin)`. Width gets the same
//!   cap (defensive; the spec only states the height rule).
//! - Icon's display unavailable: the top right of the PRIMARY display's visible
//!   frame. It is never centred and never a remembered position.
//! - Settings (ruling 6, Guus 2026-09-30): the same function with the top-right
//!   anchor on the icon's display, `ICON_GAP` under the menu bar, so its top
//!   edge lines up with the popover's.

/// Popover size in points (spec section 3: it never resizes).
pub const POPOVER_WIDTH: f64 = 372.0;
pub const POPOVER_HEIGHT: f64 = 488.0;
/// Settings window width in points (spec section 5). Height follows content.
pub const SETTINGS_WIDTH: f64 = 560.0;
/// Space kept between a window and the left, right and bottom screen edges.
pub const SCREEN_MARGIN: f64 = 8.0;
/// Space between the icon (or the menu bar) and the window's top edge.
pub const ICON_GAP: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl LRect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn center_x(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn center_y(&self) -> f64 {
        self.y + self.h / 2.0
    }
    /// Half-open: the left and top edges are inside, the right and bottom are not.
    fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    fn overlap_area(&self, other: &LRect) -> f64 {
        let w = self.right().min(other.right()) - self.x.max(other.x);
        let h = self.bottom().min(other.bottom()) - self.y.max(other.y);
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LSize {
    pub w: f64,
    pub h: f64,
}

/// One display in logical points: its full frame, the part not covered by the
/// menu bar, notch band or Dock, and whether it is the primary display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub frame: LRect,
    pub visible: LRect,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin_x: f64,
    pub origin_y: f64,
    pub size: LSize,
    /// Index into the `screens` slice the window was placed on.
    pub screen: usize,
    /// True when the icon's display could not be determined and the window was
    /// put at the top right of the primary display instead.
    pub used_fallback: bool,
}

impl Placement {
    pub fn right(&self) -> f64 {
        self.origin_x + self.size.w
    }
    pub fn bottom(&self) -> f64 {
        self.origin_y + self.size.h
    }
}

/// Which display holds the status item. See the module docs for the rules.
pub fn screen_for_icon(icon: LRect, screens: &[Screen]) -> Option<usize> {
    if let Some(i) = screen_containing_centre(icon, screens) {
        return Some(i);
    }
    screens
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.frame.overlap_area(&icon)))
        .filter(|&(_, area)| area > 0.0)
        // `max_by` keeps the LAST maximum; take the first by comparing strictly.
        .fold(None, |best: Option<(usize, f64)>, cur| match best {
            Some(b) if b.1 >= cur.1 => Some(b),
            _ => Some(cur),
        })
        .map(|(i, _)| i)
}

/// Step one of [`screen_for_icon`]: the display whose FULL frame holds the
/// icon's centre. The menu bar is outside `visible`, so matching against
/// `visible` would miss every real icon.
fn screen_containing_centre(icon: LRect, screens: &[Screen]) -> Option<usize> {
    screens
        .iter()
        .position(|s| s.frame.contains(icon.center_x(), icon.center_y()))
}

fn primary_index(screens: &[Screen]) -> Option<usize> {
    screens
        .iter()
        .position(|s| s.primary)
        .or(if screens.is_empty() { None } else { Some(0) })
}

/// Clamp that never panics: when the range is empty (window larger than the
/// room) the lower bound wins, so the window's top-left stays on screen.
fn clamp_to(v: f64, lo: f64, hi: f64) -> f64 {
    if hi < lo { lo } else { v.clamp(lo, hi) }
}

fn fit(wanted: LSize, visible: &LRect) -> LSize {
    LSize {
        w: wanted.w.min(visible.w - 2.0 * SCREEN_MARGIN).max(0.0),
        h: wanted.h.min(visible.h - 2.0 * SCREEN_MARGIN).max(0.0),
    }
}

fn top_right(screen: usize, visible: &LRect, size: LSize, used_fallback: bool) -> Placement {
    Placement {
        origin_x: visible.right() - SCREEN_MARGIN - size.w,
        origin_y: visible.y + ICON_GAP,
        size,
        screen,
        used_fallback,
    }
}

/// The popover: under the icon, on the icon's display. `None` only when there
/// are no screens at all (the caller must not show a window then).
pub fn place_popover(icon: Option<LRect>, screens: &[Screen]) -> Option<Placement> {
    let wanted = LSize {
        w: POPOVER_WIDTH,
        h: POPOVER_HEIGHT,
    };
    if let Some(icon) = icon {
        if let Some(i) = screen_for_icon(icon, screens) {
            let vis = screens[i].visible;
            let size = fit(wanted, &vis);
            let x = clamp_to(
                icon.center_x() - size.w / 2.0,
                vis.x + SCREEN_MARGIN,
                vis.right() - SCREEN_MARGIN - size.w,
            );
            let y = clamp_to(icon.bottom() + ICON_GAP, vis.y, vis.bottom() - SCREEN_MARGIN - size.h);
            return Some(Placement {
                origin_x: x,
                origin_y: y,
                size,
                screen: i,
                used_fallback: false,
            });
        }
    }
    let i = primary_index(screens)?;
    let vis = screens[i].visible;
    Some(top_right(i, &vis, fit(wanted, &vis), true))
}

/// The Settings window (ruling 6): top right of the icon's display, under the
/// menu bar. `size` is the window's OUTER size (title bar included) in points.
pub fn place_settings(icon: Option<LRect>, screens: &[Screen], size: LSize) -> Option<Placement> {
    let (i, used_fallback) = match icon.and_then(|r| screen_for_icon(r, screens)) {
        Some(i) => (i, false),
        None => (primary_index(screens)?, true),
    };
    let vis = screens[i].visible;
    Some(top_right(i, &vis, fit(size, &vis), used_fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 14" MacBook-like primary: notch band makes the menu bar 37 pt tall and
    // the visible frame starts below it; the Dock takes 45 pt at the bottom.
    const A: Screen = Screen {
        frame: LRect::new(0.0, 0.0, 1512.0, 982.0),
        visible: LRect::new(0.0, 37.0, 1512.0, 900.0),
        primary: true,
    };

    fn icon(x: f64) -> LRect {
        LRect::new(x, 0.0, 36.0, 37.0)
    }

    #[test]
    fn centres_under_the_icon_with_the_gap() {
        // icon centre 718 -> x 718 - 186 = 532; y = icon bottom 37 + 6.
        let p = place_popover(Some(icon(700.0)), &[A]).unwrap();
        assert_eq!((p.origin_x, p.origin_y), (532.0, 43.0));
        assert_eq!((p.size.w, p.size.h), (372.0, 488.0));
        assert!(!p.used_fallback);
    }

    #[test]
    fn clamps_on_the_left_with_the_margin() {
        // icon centre 28 -> ideal x -158 -> 8, never off the screen.
        let p = place_popover(Some(icon(10.0)), &[A]).unwrap();
        assert_eq!(p.origin_x, 8.0);
    }

    #[test]
    fn clamps_on_the_right_with_the_margin() {
        // ideal 1306, max = 1512 - 8 - 372 = 1132.
        let p = place_popover(Some(LRect::new(1480.0, 0.0, 24.0, 37.0)), &[A]).unwrap();
        assert_eq!(p.origin_x, 1132.0);
        assert_eq!(p.right(), 1512.0 - 8.0);
    }

    #[test]
    fn clamps_at_the_bottom_with_the_margin() {
        // A short display; an icon that would put the popover below the visible
        // frame (cannot happen for a real menu-bar icon, but the clamp is the
        // contract): max y = 564 - 8 - 488 = 68.
        let short = Screen {
            frame: LRect::new(0.0, 0.0, 1280.0, 600.0),
            visible: LRect::new(0.0, 24.0, 1280.0, 540.0),
            primary: true,
        };
        let p = place_popover(Some(LRect::new(600.0, 400.0, 24.0, 24.0)), &[short]).unwrap();
        assert_eq!(p.origin_y, 68.0);
        assert_eq!(p.bottom(), 564.0 - 8.0);
    }

    #[test]
    fn clamps_at_the_top_to_the_visible_frame_under_the_notch() {
        // icon bottom 24 + 6 = 30 would sit in the 37 pt notch band: the top
        // edge is the visible frame, not above it.
        let p = place_popover(Some(LRect::new(700.0, 0.0, 36.0, 24.0)), &[A]).unwrap();
        assert_eq!(p.origin_y, 37.0);
    }

    #[test]
    fn height_is_capped_at_the_visible_height_minus_two_margins() {
        let small = Screen {
            frame: LRect::new(0.0, 0.0, 1280.0, 440.0),
            visible: LRect::new(0.0, 24.0, 1280.0, 400.0),
            primary: true,
        };
        let p = place_popover(Some(LRect::new(600.0, 0.0, 24.0, 24.0)), &[small]).unwrap();
        assert_eq!(p.size.h, 384.0); // min(488, 400 - 16)
        assert_eq!(p.size.w, 372.0);
        assert!(p.bottom() <= 424.0 - 8.0);
    }

    #[test]
    fn a_screen_narrower_than_the_popover_keeps_the_top_left_on_screen() {
        let tiny = Screen {
            frame: LRect::new(0.0, 0.0, 300.0, 600.0),
            visible: LRect::new(0.0, 24.0, 300.0, 576.0),
            primary: true,
        };
        let p = place_popover(Some(LRect::new(100.0, 0.0, 24.0, 24.0)), &[tiny]).unwrap();
        assert_eq!(p.size.w, 284.0);
        assert_eq!(p.origin_x, 8.0);
        assert_eq!(p.right(), 292.0);
    }

    #[test]
    fn icon_is_matched_against_the_full_frame_not_the_visible_frame() {
        // The menu bar is outside `visible`. Matching against `visible` would
        // find no display for any real icon and always take the fallback.
        // Step one alone (the overlap rescue in `screen_for_icon` would hide a
        // miss), then the whole placement.
        assert_eq!(screen_containing_centre(icon(700.0), &[A]), Some(0));
        let p = place_popover(Some(icon(700.0)), &[A]).unwrap();
        assert!(!p.used_fallback);
        assert_eq!(screen_for_icon(icon(700.0), &[A]), Some(0));
    }

    // -- two displays -----------------------------------------------------

    /// A 2x primary at logical 0..1512 and a 1x external at logical
    /// 1512..3432 (design spec section 9 worked example).
    fn primary_2x_plus_external_1x() -> [Screen; 2] {
        [
            A,
            Screen {
                frame: LRect::new(1512.0, 0.0, 1920.0, 1080.0),
                visible: LRect::new(1512.0, 24.0, 1920.0, 1025.0),
                primary: false,
            },
        ]
    }

    #[test]
    fn ambiguous_two_display_case_picks_the_external_display() {
        let screens = primary_2x_plus_external_1x();
        // Icon at logical x = 2000 on the external display (physical x = 2000
        // too, because the external is 1x).
        let icon = LRect::new(2000.0, 0.0, 36.0, 24.0);

        // The rejected first-draft approach: physical / primary scale (2.0),
        // then test containment. 2000 / 2 = 1000 lies inside the primary, so it
        // picks the WRONG display. Kept here so the failure stays visible.
        let naive = screens.iter().position(|s| s.frame.contains(icon.x / 2.0, 12.0));
        assert_eq!(naive, Some(0), "documents the rejected approach");

        // Matching logical icon to logical frames is unambiguous.
        assert_eq!(screen_for_icon(icon, &screens), Some(1));
        let p = place_popover(Some(icon), &screens).unwrap();
        assert_eq!(p.screen, 1);
        // centre 2018 - 186 = 1832, within the external's 1520..3424.
        assert_eq!(p.origin_x, 1832.0);
        assert_eq!(p.origin_y, 30.0); // icon bottom 24 + gap 6
        assert!(p.origin_x >= 1512.0 + 8.0 && p.right() <= 3432.0 - 8.0);
    }

    #[test]
    fn a_centre_exactly_on_the_shared_edge_belongs_to_the_right_display() {
        let screens = primary_2x_plus_external_1x();
        // centre x = 1494 + 18 = 1512.
        assert_eq!(screen_for_icon(LRect::new(1494.0, 0.0, 36.0, 24.0), &screens), Some(1));
        // one point left of the edge stays on the primary.
        assert_eq!(screen_for_icon(LRect::new(1493.0, 0.0, 36.0, 24.0), &screens), Some(0));
    }

    #[test]
    fn an_icon_straddling_two_displays_goes_with_its_centre() {
        let screens = primary_2x_plus_external_1x();
        // spans 1500..1536, centre 1518: external. Clamped to its left margin.
        let p = place_popover(Some(LRect::new(1500.0, 0.0, 36.0, 24.0)), &screens).unwrap();
        assert_eq!(p.screen, 1);
        assert_eq!(p.origin_x, 1512.0 + 8.0);
    }

    #[test]
    fn a_display_left_of_the_primary_has_a_negative_origin() {
        let screens = [
            A,
            Screen {
                frame: LRect::new(-1920.0, 0.0, 1920.0, 1080.0),
                visible: LRect::new(-1920.0, 24.0, 1920.0, 1025.0),
                primary: false,
            },
        ];
        let p = place_popover(Some(LRect::new(-100.0, 0.0, 36.0, 24.0)), &screens).unwrap();
        assert_eq!(p.screen, 1);
        // right clamp: 0 - 8 - 372 = -380.
        assert_eq!(p.origin_x, -380.0);
    }

    #[test]
    fn a_centre_off_every_display_uses_the_largest_overlap() {
        let screens = primary_2x_plus_external_1x();
        // Centre above the frames (y = -20) but the rect overlaps the primary
        // by 6 x 30 and the external by 20 x 30.
        let icon = LRect::new(1506.0, -40.0, 26.0, 70.0);
        assert!(
            !screens
                .iter()
                .any(|s| s.frame.contains(icon.center_x(), icon.center_y()))
        );
        assert_eq!(screen_for_icon(icon, &screens), Some(1));
    }

    // -- fallback ---------------------------------------------------------

    #[test]
    fn no_icon_falls_back_to_the_top_right_of_the_primary_display() {
        let screens = primary_2x_plus_external_1x();
        let p = place_popover(None, &screens).unwrap();
        assert!(p.used_fallback);
        assert_eq!(p.screen, 0);
        assert_eq!(p.origin_x, 1512.0 - 8.0 - 372.0);
        assert_eq!(p.origin_y, 37.0 + 6.0);
    }

    #[test]
    fn an_icon_on_no_display_falls_back_to_the_primary_not_the_centre() {
        let p = place_popover(Some(LRect::new(9000.0, 0.0, 36.0, 24.0)), &[A]).unwrap();
        assert!(p.used_fallback);
        assert_eq!((p.origin_x, p.origin_y), (1132.0, 43.0));
        let centred_x = (1512.0 - 372.0) / 2.0;
        assert_ne!(p.origin_x, centred_x);
    }

    #[test]
    fn with_no_primary_flag_the_first_display_is_the_fallback() {
        let mut screens = primary_2x_plus_external_1x();
        screens[0].primary = false;
        assert_eq!(place_popover(None, &screens).unwrap().screen, 0);
    }

    #[test]
    fn no_displays_means_no_placement() {
        assert_eq!(place_popover(Some(icon(10.0)), &[]), None);
        assert_eq!(place_popover(None, &[]), None);
        assert_eq!(place_settings(None, &[], LSize { w: 560.0, h: 400.0 }), None);
    }

    // -- settings ---------------------------------------------------------

    #[test]
    fn settings_opens_top_right_under_the_menu_bar_on_the_icons_display() {
        let screens = primary_2x_plus_external_1x();
        let size = LSize { w: 560.0, h: 420.0 };
        // Icon on the external display: the window goes to the EXTERNAL's top right.
        let p = place_settings(Some(LRect::new(2000.0, 0.0, 36.0, 24.0)), &screens, size).unwrap();
        assert_eq!(p.screen, 1);
        assert!(!p.used_fallback);
        assert_eq!(p.origin_x, 3432.0 - 8.0 - 560.0);
        assert_eq!(p.origin_y, 24.0 + 6.0);
        assert_eq!((p.size.w, p.size.h), (560.0, 420.0));
    }

    #[test]
    fn settings_top_edge_lines_up_with_the_popover() {
        let size = LSize { w: 560.0, h: 420.0 };
        let s = place_settings(None, &[A], size).unwrap();
        let p = place_popover(None, &[A]).unwrap();
        assert_eq!(s.origin_y, p.origin_y);
        assert_eq!(s.right(), p.right());
    }

    #[test]
    fn settings_without_an_icon_display_falls_back_to_the_primary_top_right() {
        let s = place_settings(None, &primary_2x_plus_external_1x(), LSize { w: 560.0, h: 420.0 }).unwrap();
        assert!(s.used_fallback);
        assert_eq!(s.screen, 0);
        assert_eq!(s.origin_x, 1512.0 - 8.0 - 560.0);
    }

    #[test]
    fn settings_height_is_capped_to_the_visible_frame() {
        let s = place_settings(None, &[A], LSize { w: 560.0, h: 5000.0 }).unwrap();
        assert_eq!(s.size.h, 900.0 - 16.0);
        assert!(s.bottom() <= 937.0 - 8.0 + f64::EPSILON);
    }

    // -- invariants over a sweep -----------------------------------------

    #[test]
    fn sweep_every_placement_stays_inside_the_margins_and_never_centres() {
        let layouts: [Vec<Screen>; 3] = [
            vec![A],
            primary_2x_plus_external_1x().to_vec(),
            vec![
                Screen {
                    frame: LRect::new(-1920.0, -200.0, 1920.0, 1080.0),
                    visible: LRect::new(-1920.0, -176.0, 1920.0, 1025.0),
                    primary: false,
                },
                A,
            ],
        ];
        let mut checked = 0;
        for screens in &layouts {
            let mut x = -2200.0;
            while x < 3600.0 {
                for y in [-200.0, 0.0, 10.0] {
                    let icon = LRect::new(x, y, 36.0, 24.0);
                    let p = place_popover(Some(icon), screens).expect("screens are not empty");
                    let vis = screens[p.screen].visible;
                    assert!(p.origin_x >= vis.x + SCREEN_MARGIN, "left edge at icon x {x}");
                    assert!(p.right() <= vis.right() - SCREEN_MARGIN, "right edge at icon x {x}");
                    assert!(p.origin_y >= vis.y, "top edge at icon x {x}");
                    assert!(p.bottom() <= vis.bottom() - SCREEN_MARGIN, "bottom edge at icon x {x}");
                    let centred = vis.x + (vis.w - p.size.w) / 2.0;
                    if p.used_fallback {
                        assert_ne!(p.origin_x, centred, "fallback must not centre (icon x {x})");
                    }
                    if let Some(i) = screen_for_icon(icon, screens) {
                        assert_eq!(p.screen, i, "icon x {x} must stay on its own display");
                    }
                    checked += 1;
                }
                x += 13.0;
            }
        }
        // A sweep that ran zero iterations proves nothing.
        assert!(checked > 1_000, "only {checked} placements checked");
    }
}
