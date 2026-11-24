//! The live preview of a svg file: rune's own SVG path (resvg), one raster kept per source hash.
//! Nothing loads from disk or the network: a source that links outside itself is not parsed at all.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use rune::{Image, ImageCache, SurfaceFit, El, col, el};

use super::super::{Ui, style};
use crate::theme::scale;
use crate::theme::tokens::Theme;
use crate::ui::kit;

const OUTSIDE: &str = "This SVG links to files outside it, so the preview is off.";
const INVALID: &str = "This SVG could not be drawn.";
/// The raster's side in points at scale 1; the mips draw it smaller.
const RASTER_PT: f32 = 1024.0;

#[derive(Clone, Debug)]
pub(super) enum Outcome {
    Drawn(Image),
    Outside,
    Invalid,
}

/// True when some `href` / `xlink:href` value starts with neither '#' nor `data:`.
pub(super) fn links_outside(source: &str) -> bool {
    let mut rest = source;
    while let Some(at) = rest.find("href") {
        rest = &rest[at + 4..];
        let after = rest.trim_start();
        let Some(after) = after.strip_prefix('=') else { continue };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            // An unquoted value is not XML; refuse it rather than guess.
            return true;
        };
        let value = after[1..].split(quote).next().unwrap_or("").trim_start();
        if !(value.starts_with('#') || value.starts_with("data:")) {
            return true;
        }
    }
    false
}

fn decoders() -> &'static ImageCache {
    static CACHE: OnceLock<ImageCache> = OnceLock::new();
    CACHE.get_or_init(|| ImageCache::new(Arc::new(|| {})))
}

/// What `source` draws as. The last result is kept by a hash of the source.
pub(super) fn outcome(source: &str) -> Outcome {
    static LAST: Mutex<Option<(u64, Outcome)>> = Mutex::new(None);
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    let key = h.finish();
    if let Some((k, out)) = LAST.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && *k == key {
            return out.clone();
        }
    let out = if links_outside(source) {
        Outcome::Outside
    } else {
        let image = decoders().svg(source.as_bytes().to_vec(), RASTER_PT, 1.0);
        match image.wait() {
            Ok(_) => Outcome::Drawn(image.clone()),
            Err(_) => Outcome::Invalid,
        }
    };
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((key, out.clone()));
    out
}

pub fn view(source: &str, _ui: &Ui, t: &Theme) -> El {
    match outcome(source) {
        Outcome::Drawn(image) => col().key("live-preview-svg").w_full().h_full().fill(t.paper).padding(scale::space::S16).child(
            el().key("live-svg-image").w_full().h_full().pixels_fit(image.pixels(), SurfaceFit::Fit),
        ),
        Outcome::Outside => notice(OUTSIDE, t),
        Outcome::Invalid => notice(INVALID, t),
    }
}

fn notice(text: &str, t: &Theme) -> El {
    kit::label("live-svg-notice", text, style::BANNER, t.text_secondary).padding(scale::space::S16)
}

#[cfg(test)]
#[path = "live_svg_tests.rs"]
mod tests;
