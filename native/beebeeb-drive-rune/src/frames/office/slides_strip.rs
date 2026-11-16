//! The deck's filmstrip as a scrolling window (P3-8): its offset, the wheel over it, and which slides it draws and
//! asks. The offset lives in the deck, so a key that moves the current slide ([`DeckBody::go`]) keeps that slide's
//! line in view without a viewport to command.

use super::{DeckBody, STRIP_GAP, STRIP_PITCH, THUMB_H};

impl DeckBody {
    /// The filmstrip's content height: every slide's row and the gaps between them.
    pub(super) fn strip_content(&self) -> u32 {
        (self.view.count * STRIP_PITCH).saturating_sub(STRIP_GAP)
    }

    /// Where the strip can scroll to at most.
    pub(super) fn strip_max(&self) -> u32 {
        self.strip_content().saturating_sub(self.strip_h)
    }

    /// The strip scrolls by `dy` points (toward you: down), kept inside the deck.
    pub fn strip_scroll_by(&mut self, dy: i32) {
        let y = i64::from(self.strip_y) + i64::from(dy);
        self.strip_y = y.clamp(0, i64::from(self.strip_max())) as u32;
    }

    /// The wheel over the filmstrip: it scrolls the strip and leaves the slide. A sideways delta is not the strip's.
    pub fn strip_wheel(&mut self, ev: &rune::WheelEvent) -> bool {
        if ev.dy.abs() < ev.dx.abs() {
            return false;
        }
        self.strip_scroll_by(-(ev.dy.round() as i32));
        true
    }

    /// The strip's height was laid out as `h` points. `true`: it changed.
    pub fn set_strip_h(&mut self, h: f32) -> bool {
        let h = if h.is_finite() { h.max(0.0).round() as u32 } else { 0 };
        if h == self.strip_h {
            return false;
        }
        self.strip_h = h;
        self.reveal_current();
        true
    }

    /// Scroll the strip the least that shows the current slide's whole line.
    pub(super) fn reveal_current(&mut self) {
        let top = self.view.current.saturating_sub(1).saturating_mul(STRIP_PITCH);
        let bottom = top + THUMB_H as u32;
        if top < self.strip_y {
            self.strip_y = top;
        } else if bottom > self.strip_y + self.strip_h {
            self.strip_y = bottom.saturating_sub(self.strip_h);
        }
        self.strip_scroll_by(0);
    }

    /// The slides the strip draws and asks: those in view and one row each side. 0-based indexes, first to last.
    pub fn strip_rows(&self) -> std::ops::RangeInclusive<usize> {
        let last_slide = self.view.slides.len().saturating_sub(1);
        let first = (self.strip_y.saturating_sub(STRIP_PITCH) / STRIP_PITCH) as usize;
        let last = ((self.strip_y + self.strip_h + STRIP_PITCH) / STRIP_PITCH) as usize;
        first.min(last_slide)..=last.min(last_slide)
    }
}
