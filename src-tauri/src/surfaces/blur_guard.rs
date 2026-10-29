//! Click-to-close must not re-open (spec section 6 rule 4).
//!
//! Suspected race (UNVERIFIED on a Mac until slice 8): the popover hides on
//! blur, and blur fires on MOUSE-DOWN on the status item. The tray click
//! handler then runs on mouse-UP, finds the popover already hidden, and opens it
//! again, so the user can never close it by clicking the icon. The guard records
//! the instant of every blur-hide and treats a tray click shortly after it as
//! "stay closed".
//!
//! Time is injected as monotonic milliseconds so the rule is deterministic.

/// A tray click within this many milliseconds AFTER a blur-hide is the same
/// gesture. Strictly less than the window: a click exactly 250 ms later is a
/// new gesture.
pub const BLUR_CLICK_GUARD_MS: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayClickAction {
    Show,
    Hide,
    /// The popover was hidden by the blur of this very click: leave it closed.
    StayHidden,
}

#[derive(Debug, Default)]
pub struct BlurClickGuard {
    last_blur_hide_ms: Option<u64>,
}

impl BlurClickGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call when the popover hides because it lost focus.
    pub fn record_blur_hide(&mut self, now_ms: u64) {
        self.last_blur_hide_ms = Some(now_ms);
    }

    /// Call on a tray-icon click. One recorded blur suppresses at most one click.
    pub fn on_tray_click(&mut self, now_ms: u64, popover_visible: bool) -> TrayClickAction {
        let recent_blur = self
            .last_blur_hide_ms
            .take()
            .is_some_and(|t| now_ms.saturating_sub(t) < BLUR_CLICK_GUARD_MS);
        if popover_visible {
            TrayClickAction::Hide
        } else if recent_blur {
            TrayClickAction::StayHidden
        } else {
            TrayClickAction::Show
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_right_after_a_blur_hide_does_not_reopen() {
        let mut g = BlurClickGuard::new();
        g.record_blur_hide(10_000);
        assert_eq!(g.on_tray_click(10_020, false), TrayClickAction::StayHidden);
    }

    #[test]
    fn the_window_is_250_ms_exclusive() {
        for (delta, expected) in [
            (0, TrayClickAction::StayHidden),
            (1, TrayClickAction::StayHidden),
            (249, TrayClickAction::StayHidden),
            (250, TrayClickAction::Show),
            (251, TrayClickAction::Show),
            (5_000, TrayClickAction::Show),
        ] {
            let mut g = BlurClickGuard::new();
            g.record_blur_hide(1_000);
            assert_eq!(g.on_tray_click(1_000 + delta, false), expected, "delta {delta} ms");
        }
        assert_eq!(BLUR_CLICK_GUARD_MS, 250);
    }

    #[test]
    fn with_no_blur_a_click_opens() {
        let mut g = BlurClickGuard::new();
        assert_eq!(g.on_tray_click(5, false), TrayClickAction::Show);
    }

    #[test]
    fn one_blur_suppresses_one_click() {
        let mut g = BlurClickGuard::new();
        g.record_blur_hide(100);
        assert_eq!(g.on_tray_click(120, false), TrayClickAction::StayHidden);
        // The user clicks again a moment later: that one is a real open.
        assert_eq!(g.on_tray_click(200, false), TrayClickAction::Show);
    }

    #[test]
    fn a_click_on_a_visible_popover_hides_it_and_clears_a_stale_blur() {
        let mut g = BlurClickGuard::new();
        g.record_blur_hide(100);
        assert_eq!(g.on_tray_click(110, true), TrayClickAction::Hide);
        assert_eq!(g.on_tray_click(120, false), TrayClickAction::Show);
    }

    #[test]
    fn a_clock_that_steps_backwards_counts_as_the_same_gesture() {
        let mut g = BlurClickGuard::new();
        g.record_blur_hide(5_000);
        assert_eq!(g.on_tray_click(4_000, false), TrayClickAction::StayHidden);
    }
}
