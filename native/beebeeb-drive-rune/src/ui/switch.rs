//! The one on/off switch: a track and a knob, role `switch`, checked when on. The caller attaches
//! the press.
//!
//! Keys: `<key>` (the switch), `<key>-track`, `<key>-knob`, `<key>-label`.

use crate::theme::tokens::{Theme, type_};
use rune::{El, el, row};

use super::button::Size;
use super::kit;

pub const TRACK_W: f32 = 30.0;
pub const TRACK_H: f32 = 18.0;
pub const KNOB: f32 = 14.0;

/// The default switch: [`switch_sized`] with no row height, the track at [`TRACK_W`] x [`TRACK_H`].
pub fn switch(key: &str, label: &str, on: bool, enabled: bool, t: &Theme) -> El {
    build(key, label, on, enabled, None, t)
}

/// A switch on the theme scale. `key` is the prefix of every key it draws (see the top). The row is
/// 22 (Sm) or 28 (Md, settings rows) high; Sm also draws a 26 x 16 track with a 12 knob. Lg is Md.
pub fn switch_sized(key: &str, label: &str, on: bool, enabled: bool, size: Size, t: &Theme) -> El {
    build(key, label, on, enabled, Some(size), t)
}

/// (row height, track w, track h, knob) of a size.
pub fn metrics(size: Size) -> (f32, f32, f32, f32) {
    match size {
        Size::Sm => (22.0, 26.0, 16.0, 12.0),
        Size::Md => (28.0, TRACK_W, TRACK_H, KNOB),
        Size::Lg => (32.0, TRACK_W, TRACK_H, KNOB),
    }
}

fn build(key: &str, label: &str, on: bool, enabled: bool, size: Option<Size>, t: &Theme) -> El {
    let (row_h, track_w, track_h, knob_d) = match size {
        Some(sz) => {
            let (h, w, th, k) = metrics(sz);
            (Some(h), w, th, k)
        }
        None => (None, TRACK_W, TRACK_H, KNOB),
    };
    let track_fill = if on { t.accent } else { t.paper3 };
    // On the amber track the knob is the dark ink the primary button's words use (white on amber is under 2:1).
    let knob_fill = if on { t.accent_text_on } else { t.white };
    let knob = el().key(format!("{key}-knob")).w(knob_d).h(knob_d).radius(knob_d / 2.0).fill(knob_fill);
    let mut track = row()
        .key(format!("{key}-track"))
        .w(track_w)
        .h(track_h)
        .px(2.0)
        .center_y()
        .radius(track_h / 2.0)
        .fill(track_fill)
        .border(1.0, t.separator_strong)
        .border_layout();
    if on {
        track = track.justify_end();
    }
    let mut s = row()
        .key(key)
        .role("switch")
        .label(label)
        .checked(on)
        .gap(8.0)
        .center_y()
        .child(track.child(knob));
    if let Some(h) = row_h {
        s = s.h(h);
    }
    if !label.is_empty() {
        s = s.child(kit::label(format!("{key}-label"), label, type_::BODY, t.text));
    }
    if enabled {
        s.focusable(true)
            .focus_ring(super::focus::RING_WIDTH, super::focus::RING_OFFSET, t.accent_deep)
            .cursor(rune::Cursor::Pointer)
    } else {
        s.opacity(0.5).disabled(true).inert()
    }
}

#[cfg(test)]
#[path = "switch_tests.rs"]
mod tests;
