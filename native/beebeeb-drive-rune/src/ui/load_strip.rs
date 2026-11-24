//! The slim strip above the list while the first list loads (DW-0118, `.mk-banner` as a column):
//! what is being prepared, how far it is, and which step runs. The rows under it already work.
//!
//! Keys: `banner` (the strip's box), `strip-title`, `strip-count` (its figures `strip-figure-<i>`),
//! `strip-bar`, `strip-bar-fill`, `strip-detail`. The box and the bar are `ui::view_state`'s, so the
//! loading state looks the same.

use crate::theme::tokens::{Theme, type_};
use rune::{El, LiveRegion, el, row};

use super::banner::{self, Run};
use super::kit;
use super::view_state::{strip_bar, strip_box};

/// The strip. `fraction` is 0 to 1; `count` is the line at the right ("18,240 of 40,112 · about
/// 1 min left"), `detail` the line under the bar.
pub fn load_strip(title: &str, count: &[Run], fraction: f32, detail: &str, t: &Theme) -> El {
    let percent = (fraction.clamp(0.0, 1.0) * 100.0).round();
    strip_box("banner", t)
        .label(format!("{title}. {}. {detail}", banner::sentence(count)))
        // Not a live region: the figures change many times a second. It is read when reached.
        .live_region(LiveRegion::Off)
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
            strip_bar("strip-bar", fraction, t)
                .role("progressbar")
                .label(title)
                .range_value(f64::from(percent), 0.0, 100.0),
        )
        .child(kit::label("strip-detail", detail, type_::CAPTION, t.text_secondary))
}
