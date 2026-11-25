//! Where the macOS popover opens, and how a click on the b toggles it (docs/macos/menu-bar-and-windows.md §4
//! rules 1-4 and 6). Pure: logical points, top-left origin, no scale factor. Compiled on every OS so its tests run
//! everywhere; `flyout.rs` uses it on macOS.
//!
//! Settings anchoring (§4.5) is not carried: the Mac's Settings is a page of the main window.

pub const POPOVER_WIDTH: f64 = 372.0;
pub const POPOVER_HEIGHT: f64 = 488.0;
/// The gap between the icon's bottom edge and the popover's top.
pub const ICON_GAP: f64 = 6.0;
pub const SCREEN_MARGIN: f64 = 8.0;
/// A tray click within this long (exclusive) after a blur-hide is the click that closed the popover.
pub const BLUR_CLICK_GUARD_MS: u64 = 250;

/// A rectangle in logical points, top-left origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Frame {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Frame { x, y, w, h }
    }
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
    /// Half-open: the left and top edges are inside, the right and bottom are not.
    fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    fn overlap(&self, o: &Frame) -> f64 {
        let w = (self.right().min(o.right()) - self.x.max(o.x)).max(0.0);
        let h = (self.bottom().min(o.bottom()) - self.y.max(o.y)).max(0.0);
        w * h
    }
}

/// One display: its full frame (menu bar included), the part the menu bar and Dock leave, and whether it is primary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub frame: Frame,
    pub visible: Frame,
    pub primary: bool,
}

/// Where the popover goes, and how big it is there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The display the icon is on: its centre against each full frame; a centre on none uses the display the icon
/// overlaps most; no overlap at all is `None`.
pub fn icon_display(icon: Frame, displays: &[Display]) -> Option<&Display> {
    let (cx, cy) = (icon.x + icon.w / 2.0, icon.y + icon.h / 2.0);
    if let Some(d) = displays.iter().find(|d| d.frame.contains(cx, cy)) {
        return Some(d);
    }
    let mut best: Option<(&Display, f64)> = None;
    for d in displays {
        let o = icon.overlap(&d.frame);
        if o > 0.0 && best.map_or(true, |(_, b)| o > b) {
            best = Some((d, o));
        }
    }
    best.map(|(d, _)| d)
}

/// Clamped into `v` with `SCREEN_MARGIN` left, right and bottom; the top is the visible frame itself.
fn clamp_into(v: &Frame, want_x: f64, want_y: f64) -> Placement {
    let w = POPOVER_WIDTH.min((v.w - 2.0 * SCREEN_MARGIN).max(0.0));
    let h = POPOVER_HEIGHT.min((v.h - 2.0 * SCREEN_MARGIN).max(0.0));
    let min_x = v.x + SCREEN_MARGIN;
    let max_x = (v.right() - SCREEN_MARGIN - w).max(min_x);
    let max_y = (v.bottom() - SCREEN_MARGIN - h).max(v.y);
    Placement {
        x: want_x.clamp(min_x, max_x),
        y: want_y.clamp(v.y, max_y),
        w,
        h,
    }
}

/// The popover's frame. `icon`: the status item's frame. No displays: no placement.
pub fn place_popover(icon: Option<Frame>, displays: &[Display]) -> Option<Placement> {
    if let Some(d) = icon.and_then(|i| icon_display(i, displays).map(|d| (i, d))) {
        let (i, d) = d;
        let x = i.x + i.w / 2.0 - POPOVER_WIDTH / 2.0;
        let y = i.bottom() + ICON_GAP;
        return Some(clamp_into(&d.visible, x, y));
    }
    // The icon's display is unknown: the top right of the primary display, never centred.
    let d = displays.iter().find(|d| d.primary).or_else(|| displays.first())?;
    let v = &d.visible;
    Some(clamp_into(v, v.right(), v.y))
}

/// A point position as the physical desktop pixels `WindowHandle::set_position` takes.
pub fn to_physical(x: f64, y: f64, scale: f64) -> (i32, i32) {
    let s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    ((x * s).round() as i32, (y * s).round() as i32)
}

pub const SETTINGS_WIDTH: f64 = 560.0;

/// Settings anchored top right on the icon's display (§4.5), its top level with the popover's. Not used by the Mac
/// shell (its Settings is a page of the main window); kept pure and tested as the rule.
pub fn place_settings(icon: Option<Frame>, displays: &[Display], height: f64) -> Option<Placement> {
    let d = icon
        .and_then(|i| icon_display(i, displays))
        .or_else(|| displays.iter().find(|d| d.primary))
        .or_else(|| displays.first())?;
    let v = &d.visible;
    let w = SETTINGS_WIDTH.min((v.w - 2.0 * SCREEN_MARGIN).max(0.0));
    let h = height.min((v.h - 2.0 * SCREEN_MARGIN).max(0.0));
    Some(Placement {
        x: (v.right() - SCREEN_MARGIN - w).max(v.x + SCREEN_MARGIN),
        y: v.y,
        w,
        h,
    })
}

/// What a click on the b does to the popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Show,
    Hide,
    /// This click already closed the popover by taking its focus.
    Ignore,
}

/// Remembers the last blur-hide so the click that caused it does not reopen the popover.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlurGuard {
    blurred_at_ms: Option<u64>,
}

impl BlurGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// The popover hid itself because it lost focus.
    pub fn blurred(&mut self, now_ms: u64) {
        self.blurred_at_ms = Some(now_ms);
    }

    /// A left click on the b. One blur suppresses one click; a click on a visible popover hides it and clears a stale blur.
    pub fn click(&mut self, visible: bool, now_ms: u64) -> Toggle {
        let blurred = self.blurred_at_ms.take();
        if visible {
            return Toggle::Hide;
        }
        match blurred {
            // A clock that steps back (now < blur) counts as the same gesture.
            Some(at) if now_ms < at || now_ms - at < BLUR_CLICK_GUARD_MS => Toggle::Ignore,
            _ => Toggle::Show,
        }
    }
}

#[cfg(test)]
#[path = "macos_anchor_tests.rs"]
mod tests;
