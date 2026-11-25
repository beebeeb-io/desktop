//! The status bar (`.mk-foot`): the one place that builds it. A state dot and its word at the left,
//! the counts at the right. [`frame`] is the bar alone (for a caller with its own children);
//! [`bar`] fills it the board's way. `status_line`, `status_offline` and [`status_bar`] (the DW-0401
//! sample) call [`bar`].

use crate::theme::tokens::{Theme, type_};
use rune::{Color, El, el, row};

use super::kit;

const STATUS_H: f32 = 28.0;

/// The words of the live window's status line (contract `docs/superpowers/specs/2026-11-12-native-sync-start-and-sign-out.md`,
/// §4.3; "Offline" is `frames::global::samples::OFFLINE`). "Checking" and "Sync stopped" are the b's words for the
/// same states (`shell::tray`, DW-1201 and DW-1207).
pub const STARTING: &str = "Starting\u{2026}";
pub const SYNCING: &str = "Syncing";
pub const PAUSED: &str = "Paused";
pub const UP_TO_DATE: &str = "Up to date";
pub const CHECKING: &str = "Checking";
pub const SYNC_STOPPED: &str = "Sync stopped";
pub const SYNC_IS_OFF: &str = "Sync is off";
pub const SIGN_IN_AGAIN: &str = "Sign in again";

/// The state dot's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    /// Done: `t.green`.
    Green,
    /// Still moving: the board's darker grey (`--color-ink-3`, DW-0412).
    Busy,
    /// Nothing moving, nothing wrong (`--color-ink-4`).
    Idle,
    /// Not done, not broken (`--mk-warn`).
    Warn,
    /// Broken: `t.red`.
    Red,
}

impl Dot {
    pub fn color(self, t: &Theme) -> Color {
        match self {
            Dot::Green => t.green,
            Dot::Busy => t.text_tertiary,
            Dot::Idle => t.text_quaternary,
            // `--mk-warn`: `color-mix(in oklch, amber-deep 45%, red)`, as `frames/viewers/style.rs` `warn`.
            Dot::Warn => rune::color::hex(if t.dark { "#F67222" } else { "#C94B00" }).unwrap_or(Color::BLACK),
            Dot::Red => t.red,
        }
    }
}

/// The dot for a sync state's word: green only for a sync that is done; a copy still coming down
/// has the board's darker grey (DW-0412); every other word the lighter grey.
pub fn dot_for(state: &str) -> Dot {
    if state == UP_TO_DATE {
        Dot::Green
    } else if state == SYNCING {
        Dot::Busy
    } else {
        Dot::Idle
    }
}

/// The bar alone, around the caller's children: key `status-bar`, role `status`.
pub fn frame(children: Vec<El>, t: &Theme) -> El {
    row()
        .key("status-bar")
        .role("status")
        .h(STATUS_H)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .children(children)
}

/// The bar with a dot and its word at the left (none when `state` is `None`) and `details` at the right.
pub fn bar(dot: Option<Dot>, state: Option<&str>, details: &str, t: &Theme) -> El {
    let mut left = row().key("status-left").center_y().gap(7.0);
    if let Some(state) = state {
        if let Some(dot) = dot {
            // A tenth of a device pixel of the bar's own colour inside the dot's edge: Chromium's
            // antialiased 14-pixel disc covers less of its outermost pixel rows than Rune's
            // (measured 91 % against 98 %), which shows on a dot this dark. A size or a scale
            // cannot say it: Rune snaps the box to whole pixels ("Rune limitations", PR body).
            left = left.child(
                el().key("status-dot")
                    .size(7.0, 7.0)
                    .radius(3.5)
                    .fill(dot.color(t))
                    .box_shadow(rune::Shadow {
                        x: 0.0,
                        y: 0.0,
                        blur: 0.0,
                        spread: 0.05,
                        color: t.status_bar_bg,
                        inset: true,
                    }),
            );
        }
        left = left.child(kit::label("status-text", state, type_::STATUS_STRONG, t.text_secondary));
        if state == PAUSED {
            left = left.child(super::sync::resume_item(t));
        }
    }
    frame(
        vec![
            left,
            el().key("status-spacer").grow(1.0),
            kit::label("status-details", details, type_::STATUS, t.text_tertiary),
        ],
        t,
    )
}

/// The DW-0401 sample's bar.
pub fn status_bar(t: &Theme) -> El {
    bar(
        Some(Dot::Green),
        Some(UP_TO_DATE),
        concat!("4 on ", beebeeb_engine::device_word!(), " \u{00B7} 3 available offline \u{00B7} 4 online only"),
        t,
    )
}

#[cfg(test)]
#[path = "status_bar_tests.rs"]
mod tests;
