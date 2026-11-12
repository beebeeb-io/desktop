//! The status line with its words from the caller (`.mk-foot`): `status_bar::status_bar` says the
//! DW-0401 sample's; this one says what the folder holds. Same keys: `status-bar`, `status-dot`,
//! `status-text`, `status-details`. `state` is the sync state's word ("Up to date"); `None` when
//! nothing syncs here: then there is no dot and no word, never a sample.
//!
//! The dot is green only for the two words of a sync that runs and is fine ([`SYNCING`],
//! [`UP_TO_DATE`]); every other word has the grey dot of `status_offline` (`--color-ink-4`).

use crate::theme::tokens::{Theme, type_};
use rune::{El, el, row};

use super::kit;

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

pub fn status_line(state: Option<&str>, details: &str, t: &Theme) -> El {
    let mut left = row().key("status-left").center_y().gap(7.0);
    if let Some(state) = state {
        let dot = if matches!(state, SYNCING | UP_TO_DATE) {
            t.green
        } else {
            t.text_quaternary
        };
        left = left
            .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(dot))
            .child(kit::label("status-text", state, type_::STATUS_STRONG, t.text_secondary));
    }
    row()
        .key("status-bar")
        .role("status")
        .h(28.0)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .child(left)
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label("status-details", details, type_::STATUS, t.text_tertiary))
}
