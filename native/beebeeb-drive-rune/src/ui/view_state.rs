//! The one shape for a view that has nothing to show yet: it is loading, it is empty, or it failed.
//!
//! - Loading: the load strip's look (`.mk-banner` as a column) with the words and a quiet bar.
//!   Keys: `loading`, `loading-words`, `loading-bar`, `loading-bar-fill`.
//! - Empty (`.kit-empty`): a tile with a glyph, a title, one sentence, and an optional action.
//!   Keys: `<key>`, `<key>-tile`, `<key>-icon`, `<key>-title`, `<key>-text`, the action's own.
//! - Failed: a title, the reason, and "Try again" ([`try_again`], a `ui::button`), in a polite
//!   live region so the failure is read when it appears. Keys: `<key>`, `<key>-title`,
//!   `<key>-text`, the retry's own (`<key>-retry` from [`try_again`]).
//!
//! `ui::empty::empty_state` and `ui::load_strip::load_strip` draw through this module.

use crate::theme::tokens::{Theme, type_};
use rune::{Align, El, LiveRegion, col, el};

use super::banner;
use super::button::{self, ButtonState, Size, Variant};
use super::empty::{SUB, TITLE};
use super::kit;

pub enum ViewState<'a> {
    Loading {
        words: &'a str,
    },
    Empty {
        key: &'a str,
        icon: &'static rune::Icon,
        title: &'a str,
        body: &'a str,
        action: Option<El>,
    },
    Failed {
        key: &'a str,
        title: &'a str,
        why: &'a str,
        retry: Option<El>,
    },
}

/// The words of the retry button.
pub const TRY_AGAIN: &str = "Try again";

/// Where the sentence of Empty and Failed wraps.
const WRAP: f32 = 227.11;

pub fn view_state(state: ViewState, t: &Theme) -> El {
    view_state_sized(state, "loading", Size::Md, t)
}

/// [`view_state`] with a size and the key prefix of the Loading strip (`<loading_key>`,
/// `-words`, `-bar`, `-bar-fill`; Empty and Failed carry their own `key`). Md is the default look;
/// Sm is compact: 16/16 padding, a 40 pt tile with a 20 pt glyph, 6 between parts.
pub fn view_state_sized(state: ViewState, loading_key: &str, size: Size, t: &Theme) -> El {
    match state {
        ViewState::Loading { words } => loading(loading_key, words, t),
        ViewState::Empty {
            key,
            icon,
            title,
            body,
            action,
        } => empty(key, icon, title, body, action, size, t),
        ViewState::Failed { key, title, why, retry } => failed(key, title, why, retry, size, t),
    }
}

/// "Try again" for a failed view: Secondary, Md, keyed `<key>-retry`. The caller adds
/// `.on_press(..)` and passes it as `retry`.
pub fn try_again(key: &str, t: &Theme) -> El {
    button::button(
        button::keys(&format!("{key}-retry")),
        TRY_AGAIN,
        None,
        Variant::Secondary,
        Size::Md,
        ButtonState::Enabled,
        t,
    )
}

/// The strip's box (`.mk-banner` as a column): neutral wash, 10 radius, 9/12/9/10 padding, 6 between
/// its rows, 10 above and 14 at the sides. Shared with `ui::load_strip`.
pub(super) fn strip_box(key: &str, t: &Theme) -> El {
    let (wash, _) = banner::tone(banner::Tone::Neutral, t);
    col()
        .key(key)
        .role("status")
        .mt(10.0)
        .mx(14.0)
        .pt(9.0)
        .pb(9.0)
        .pl(12.0)
        .pr(10.0)
        .gap(6.0)
        .radius(10.0)
        .fill(wash)
}

/// The strip's bar: a 4 pt track with a fill `fraction` (0 to 1) wide. The caller sets the role and
/// the value. Shared with `ui::load_strip`.
pub(super) fn strip_bar(key: &str, fraction: f32, t: &Theme) -> El {
    rune::row()
        .key(key)
        .w_full()
        .h(4.0)
        .radius(2.0)
        .clip_rounded(2.0)
        .fill(t.text.alpha(0.09))
        .child(
            el().key(format!("{key}-fill"))
                .h_full()
                .w_frac(fraction.clamp(0.0, 1.0))
                .radius(2.0)
                .fill(t.text_secondary),
        )
}

/// Loading: the words over a bar with no value (a progressbar without a value reads as busy).
fn loading(key: &str, words: &str, t: &Theme) -> El {
    strip_box(key, t)
        .label(words)
        .live_region(LiveRegion::Polite)
        .child(kit::label(format!("{key}-words"), words, type_::HEAD, t.text))
        .child(
            strip_bar(&format!("{key}-bar"), 0.3, t)
                .role("progressbar")
                .label(words),
        )
}

/// The centred column Empty and Failed share: 28/24 padding, 8 between its parts.
fn column(key: &str, title: &str, size: Size) -> El {
    let (py, px, gap) = if size == Size::Sm {
        (16.0, 16.0, 6.0)
    } else {
        (28.0, 24.0, 8.0)
    };
    col()
        .key(key)
        .role("group")
        .label(title)
        .py(py)
        .px(px)
        .gap(gap)
        .items_center()
}

fn sentence(key: &str, words: &str, t: &Theme) -> El {
    kit::label(format!("{key}-text"), words, SUB, t.text_tertiary)
        .wrap()
        .w(WRAP)
        .align(Align::Center)
}

fn empty(key: &str, ic: &'static rune::Icon, title: &str, body: &str, action: Option<El>, size: Size, t: &Theme) -> El {
    let (tile, glyph) = if size == Size::Sm { (40.0, 20.0) } else { (52.0, 26.0) };
    let mut c = column(key, title, size)
        .child(
            el().key(format!("{key}-tile"))
                .size(tile, tile)
                .radius(15.0)
                .fill(t.tile)
                .center()
                .child(kit::glyph(format!("{key}-icon"), ic, glyph, t.text_secondary)),
        )
        .child(kit::label(format!("{key}-title"), title, TITLE, t.text))
        .child(sentence(key, body, t));
    if let Some(a) = action {
        c = c.child(el().key(format!("{key}-gap")).h(4.0)).child(a);
    }
    c
}

fn failed(key: &str, title: &str, why: &str, retry: Option<El>, size: Size, t: &Theme) -> El {
    let mut c = column(key, title, size)
        .live_region(LiveRegion::Polite)
        .child(kit::label(format!("{key}-title"), title, TITLE, t.text))
        .child(sentence(key, why, t));
    if let Some(r) = retry {
        c = c.child(el().key(format!("{key}-gap")).h(4.0)).child(r);
    }
    c
}

#[cfg(test)]
#[path = "view_state_tests.rs"]
mod tests;
