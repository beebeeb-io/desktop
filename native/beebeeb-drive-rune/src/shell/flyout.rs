//! The flyout's window rules (DW-1201), as pure functions: where it opens, and what a click on
//! the b does to it. Its content is the tray lane's frame group (`registry::FLYOUT`).
//!
//! "Fixed at 340 x 520 px (effective pixels) in every state." "Opens above the taskbar, 12 px
//! over its top edge and 12 px from the screen's right edge, on the display whose taskbar holds
//! the b that was clicked." "Closes when it loses focus, on Esc, or on a second click on the b."

#[path = "macos_anchor.rs"]
pub mod macos_anchor;

/// In points; Windows scales them with the display. The Mac popover is 372 x 488 pt (docs/macos §4).
#[cfg(not(target_os = "macos"))]
pub const SIZE: (f32, f32) = (340.0, 520.0);
#[cfg(target_os = "macos")]
pub const SIZE: (f32, f32) = (macos_anchor::POPOVER_WIDTH as f32, macos_anchor::POPOVER_HEIGHT as f32);
pub const MARGIN: f32 = 12.0;
/// A click on the b first takes the focus from the flyout, which hides it; the click itself
/// arrives after. Within this long, that click is the one that closed it, not a new open.
#[cfg(not(target_os = "macos"))]
pub const REOPEN_GUARD_MS: u64 = 300;
#[cfg(target_os = "macos")]
pub const REOPEN_GUARD_MS: u64 = macos_anchor::BLUR_CLICK_GUARD_MS;

/// A rectangle in physical desktop pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The display the b is on: its whole rectangle, the part the taskbar leaves free, its scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub monitor: Rect,
    pub work: Rect,
    pub scale: f64,
    /// The status item's frame, logical points, top-left origin (macOS). None on Windows.
    pub icon: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarEdge {
    Bottom,
    Top,
    Left,
    Right,
    /// Auto-hidden, or not on this display: the work area is the whole display.
    None,
}

/// Which edge the taskbar takes, read from what it leaves of the display.
pub fn taskbar_edge(a: &Anchor) -> TaskbarEdge {
    let (m, w) = (a.monitor, a.work);
    if w.y + w.h < m.y + m.h {
        TaskbarEdge::Bottom
    } else if w.y > m.y {
        TaskbarEdge::Top
    } else if w.x > m.x {
        TaskbarEdge::Left
    } else if w.x + w.w < m.x + m.w {
        TaskbarEdge::Right
    } else {
        TaskbarEdge::None
    }
}

/// The flyout's size on this display, in physical pixels.
pub fn physical_size(a: &Anchor) -> (u32, u32) {
    let scale = if a.scale.is_finite() && a.scale > 0.0 {
        a.scale
    } else {
        1.0
    };
    (
        (f64::from(SIZE.0) * scale).round() as u32,
        (f64::from(SIZE.1) * scale).round() as u32,
    )
}

/// The flyout's top-left: 12 px inside the work area, next to where the b is. A taskbar at the
/// bottom, the right or hidden: bottom-right. At the top: top-right. At the left: bottom-left.
/// Never outside the work area's top-left, also on a display smaller than the flyout.
#[cfg(target_os = "macos")]
pub fn position(a: &Anchor) -> (i32, i32) {
    // Logical points. The anchor carries the display the b is on (`monitor` = full frame, `work` = visible frame).
    let f = |r: Rect| macos_anchor::Frame::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
    let display = macos_anchor::Display {
        frame: f(a.monitor),
        visible: f(a.work),
        primary: true,
    };
    let icon = a.icon.map(f);
    // An icon that is not on this display is "unavailable": the top right of the display.
    let icon = icon.filter(|i| macos_anchor::icon_display(*i, &[display]).is_some());
    // `WindowHandle::set_position` takes physical desktop pixels: points times the display's scale (a point
    // position handed over as pixels lands at half the way on a Retina Mac, mid-screen).
    let scale = if a.scale.is_finite() && a.scale > 0.0 {
        a.scale
    } else {
        1.0
    };
    macos_anchor::place_popover(icon, &[display]).map_or(
        (
            (f64::from(a.work.x) * scale).round() as i32,
            (f64::from(a.work.y) * scale).round() as i32,
        ),
        |p| macos_anchor::to_physical(p.x, p.y, scale),
    )
}

#[cfg(not(target_os = "macos"))]
pub fn position(a: &Anchor) -> (i32, i32) {
    let (w, h) = physical_size(a);
    let (w, h) = (w as i32, h as i32);
    let margin = (f64::from(MARGIN)
        * if a.scale.is_finite() && a.scale > 0.0 {
            a.scale
        } else {
            1.0
        })
    .round() as i32;
    let edge = taskbar_edge(a);
    let x = if edge == TaskbarEdge::Left {
        a.work.x + margin
    } else {
        a.work.x + a.work.w - margin - w
    };
    let y = if edge == TaskbarEdge::Top {
        a.work.y + margin
    } else {
        a.work.y + a.work.h - margin - h
    };
    (x.max(a.work.x), y.max(a.work.y))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Show,
    Hide,
    /// This click already closed the flyout by taking its focus.
    Ignore,
}

/// What a left click on the b does. `since_focus_hide_ms`: how long ago the flyout hid itself
/// because it lost focus, if it did.
pub fn on_tray_click(visible: bool, since_focus_hide_ms: Option<u64>) -> Toggle {
    if visible {
        Toggle::Hide
    } else if since_focus_hide_ms.is_some_and(|ms| ms < REOPEN_GUARD_MS) {
        Toggle::Ignore
    } else {
        Toggle::Show
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn on_a_mac_the_popover_opens_centred_under_the_b() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        let a = Anchor {
            monitor: r(0, 0, 1440, 900),
            work: r(0, 25, 1440, 875),
            scale: 2.0,
            icon: Some(r(689, 0, 22, 24)),
        };
        assert_eq!(SIZE, (372.0, 488.0));
        // Points (514, 30) at scale 2 are pixels (1028, 60).
        assert_eq!(position(&a), (1028, 60));
        let near_edge = Anchor {
            icon: Some(r(1420, 0, 22, 24)),
            ..a
        };
        assert_eq!(position(&near_edge).0, (1440 - 8 - 372) * 2);
        assert_eq!(on_tray_click(false, Some(249)), Toggle::Ignore);
        assert_eq!(on_tray_click(false, Some(250)), Toggle::Show);
    }

    #[cfg(not(target_os = "macos"))]
    fn anchor(monitor: (i32, i32, i32, i32), work: (i32, i32, i32, i32), scale: f64) -> Anchor {
        let rect = |(x, y, w, h)| Rect { x, y, w, h };
        Anchor {
            monitor: rect(monitor),
            work: rect(work),
            scale,
            icon: None,
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn bottom_taskbar_puts_it_bottom_right_at_every_scale() {
        // 1920x1080 at 100%, 48 px taskbar.
        let a = anchor((0, 0, 1920, 1080), (0, 0, 1920, 1032), 1.0);
        assert_eq!(taskbar_edge(&a), TaskbarEdge::Bottom);
        assert_eq!(physical_size(&a), (340, 520));
        assert_eq!(position(&a), (1920 - 12 - 340, 1032 - 12 - 520));
        // 150%: 510x780 with an 18 px margin. A second display at 200%: 680x1040, 24 px.
        let a = anchor((0, 0, 2880, 1620), (0, 0, 2880, 1548), 1.5);
        assert_eq!(position(&a), (2880 - 18 - 510, 1548 - 18 - 780));
        let a = anchor((3840, 0, 3840, 2160), (3840, 0, 3840, 2064), 2.0);
        assert_eq!(physical_size(&a), (680, 1040));
        assert_eq!(position(&a), (3840 + 3840 - 24 - 680, 2064 - 24 - 1040));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn each_taskbar_edge_has_its_corner() {
        let top = anchor((0, 0, 1920, 1080), (0, 48, 1920, 1032), 1.0);
        assert_eq!(taskbar_edge(&top), TaskbarEdge::Top);
        assert_eq!(position(&top), (1568, 60));
        let left = anchor((0, 0, 1920, 1080), (48, 0, 1872, 1080), 1.0);
        assert_eq!(taskbar_edge(&left), TaskbarEdge::Left);
        assert_eq!(position(&left), (60, 548));
        let right = anchor((0, 0, 1920, 1080), (0, 0, 1872, 1080), 1.0);
        assert_eq!(taskbar_edge(&right), TaskbarEdge::Right);
        assert_eq!(position(&right), (1872 - 12 - 340, 548));
        let hidden = anchor((0, 0, 1920, 1080), (0, 0, 1920, 1080), 1.0);
        assert_eq!(taskbar_edge(&hidden), TaskbarEdge::None);
        assert_eq!(position(&hidden), (1568, 548));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_display_left_of_the_primary_has_negative_coordinates() {
        let a = anchor((-2560, -200, 2560, 1440), (-2560, -200, 2560, 1392), 1.25);
        assert_eq!(physical_size(&a), (425, 650));
        assert_eq!(position(&a), (-2560 + 2560 - 15 - 425, -200 + 1392 - 15 - 650));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_display_smaller_than_the_flyout_pins_it_to_the_work_areas_corner() {
        let a = anchor((0, 0, 320, 480), (0, 0, 320, 440), 1.0);
        assert_eq!(position(&a), (0, 0));
        let a = anchor((100, 50, 320, 480), (100, 50, 320, 440), 1.0);
        assert_eq!(position(&a), (100, 50));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_broken_scale_is_read_as_one() {
        let a = anchor((0, 0, 1920, 1080), (0, 0, 1920, 1032), f64::NAN);
        assert_eq!(physical_size(&a), (340, 520));
        assert_eq!(position(&a), (1568, 500));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_click_on_the_b_toggles_and_the_closing_click_does_not_reopen() {
        assert_eq!(on_tray_click(true, None), Toggle::Hide);
        assert_eq!(on_tray_click(false, None), Toggle::Show);
        // The flyout hid itself 40 ms ago because this very click took its focus.
        assert_eq!(on_tray_click(false, Some(40)), Toggle::Ignore);
        assert_eq!(on_tray_click(false, Some(REOPEN_GUARD_MS)), Toggle::Show);
        assert_eq!(on_tray_click(false, Some(5_000)), Toggle::Show);
    }
}
