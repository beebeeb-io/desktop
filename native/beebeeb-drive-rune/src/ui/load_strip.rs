//! The slim strip above the list while the first list loads (DW-0118, `.mk-banner` as a column):
//! what is being prepared, how far it is, and which step runs. The rows under it already work.
//!
//! Keys: `banner` (the strip's box), `strip-title`, `strip-count` (its figures `strip-figure-<i>`),
//! `strip-bar`, `strip-bar-fill`, `strip-detail`.

use crate::theme::tokens::{Theme, type_};
use rune::{El, LiveRegion, col, el, row};

use super::banner::{self, Run};
use super::kit;

/// The strip. `fraction` is 0 to 1; `count` is the line at the right ("18,240 of 40,112 · about
/// 1 min left"), `detail` the line under the bar.
pub fn load_strip(title: &str, count: &[Run], fraction: f32, detail: &str, t: &Theme) -> El {
    let (wash, _) = banner::tone(banner::Tone::Neutral, t);
    let percent = (fraction.clamp(0.0, 1.0) * 100.0).round();
    col()
        .key("banner")
        .role("status")
        .label(format!("{title}. {}. {detail}", banner::sentence(count)))
        // Not a live region: the figures change many times a second. It is read when reached.
        .live_region(LiveRegion::Off)
        .mt(10.0)
        .mx(14.0)
        .pt(9.0)
        .pb(9.0)
        .pl(12.0)
        .pr(10.0)
        .gap(6.0)
        .radius(10.0)
        .fill(wash)
        .child(
            row()
                .key("strip-head")
                .items_center()
                .gap(8.0)
                .child(kit::label("strip-title", title, type_::HEAD, t.text))
                .child(el().key("strip-spacer").grow(1.0))
                .child(banner::runs_line(
                    "strip-count",
                    "strip-figure",
                    count,
                    type_::CAPTION,
                    type_::KBD,
                    t.text_secondary,
                )),
        )
        .child(
            row()
                .key("strip-bar")
                .role("progressbar")
                .label(title)
                .range_value(f64::from(percent), 0.0, 100.0)
                .w_full()
                .h(4.0)
                .radius(2.0)
                .clip_rounded(2.0)
                .fill(t.text.alpha(0.09))
                .child(
                    el().key("strip-bar-fill")
                        .h_full()
                        .w_frac(fraction.clamp(0.0, 1.0))
                        .radius(2.0)
                        .fill(t.text_secondary),
                ),
        )
        .child(kit::label("strip-detail", detail, type_::CAPTION, t.text_secondary))
}
