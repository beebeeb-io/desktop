//! The one on/off switch: a track and a knob, role `switch`, checked when on. The caller attaches
//! the press.
//!
//! Keys: `<key>` (the switch), `<key>-track`, `<key>-knob`, `<key>-label`.

use crate::theme::tokens::{Theme, type_};
use rune::{El, el, row};

use super::kit;

pub const TRACK_W: f32 = 30.0;
pub const TRACK_H: f32 = 18.0;
pub const KNOB: f32 = 14.0;

pub fn switch(key: &str, label: &str, on: bool, enabled: bool, t: &Theme) -> El {
    let track_fill = if on { t.selection } else { t.paper3 };
    let knob = el().key(format!("{key}-knob")).w(KNOB).h(KNOB).radius(KNOB / 2.0).fill(t.white);
    let mut track = row()
        .key(format!("{key}-track"))
        .w(TRACK_W)
        .h(TRACK_H)
        .px(2.0)
        .center_y()
        .radius(TRACK_H / 2.0)
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
