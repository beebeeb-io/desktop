//! Contrast ratio (024), the override layer that lifts the generated tokens' failing pairs (035; `tokens.rs`
//! is never edited by hand) and the Windows contrast theme as a [`Theme`] (035).
#![allow(dead_code)]

use super::tokens::Theme;
use rune::{Color, oklch};

/// WCAG 2 relative luminance of an opaque colour (rune's `Color` is already linear sRGB).
fn relative_luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// `top` laid over an opaque `under`, in linear light.
fn over(top: Color, under: Color) -> Color {
    let k = |t: f32, u: f32| t * top.a + u * (1.0 - top.a);
    Color { r: k(top.r, under.r), g: k(top.g, under.g), b: k(top.b, under.b), a: 1.0 }
}

/// WCAG contrast ratio of two colours, 1.0 to 21.0, order-free. A translucent `b` is laid over white,
/// then a translucent `a` over that.
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let b = over(b, Color::WHITE);
    let a = over(a, b);
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
#[path = "contrast_tests.rs"]
mod tests;

/// Lift `fg` (by lightness, away from the backgrounds) until it reaches `min` on every one of `bgs`.
fn lift(fg: Color, bgs: &[Color], min: f32, dark: bool) -> Color {
    let ok = |c: Color| bgs.iter().all(|b| contrast_ratio(c, *b) >= min);
    if ok(fg) {
        return fg;
    }
    let [_, a, b, _] = fg.to_oklab();
    let (chroma, hue) = ((a * a + b * b).sqrt(), b.atan2(a).to_degrees().rem_euclid(360.0));
    let mut l = fg.to_oklab()[0];
    let step = if dark { 0.005 } else { -0.005 };
    for _ in 0..200 {
        l = (l + step).clamp(0.0, 1.0);
        let c = oklch(l, chroma, hue).alpha(fg.a);
        if ok(c) {
            return c;
        }
    }
    if dark { Color::WHITE } else { Color::BLACK }
}

/// The theme as drawn: the generated tokens with the pairs that failed WCAG lifted (the override layer).
pub fn themed(dark: bool) -> Theme {
    let mut t = super::tokens::base(dark);
    let bgs = [t.bg, t.surface, t.card, t.sidebar_bg];
    t.text_quaternary = lift(t.text_quaternary, &bgs, 3.0, dark);
    t.card_border = lift(t.card_border, &bgs, 3.0, dark);
    t
}

/// The eight system colours of a Windows contrast theme, as `GetSysColor` returns them (0x00BBGGRR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemColors {
    pub window: u32,
    pub window_text: u32,
    pub highlight: u32,
    pub highlight_text: u32,
    pub gray_text: u32,
    pub hot_light: u32,
    pub btn_face: u32,
    pub btn_text: u32,
}

fn colorref(c: u32) -> Color {
    let (r, g, b) = (c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff);
    rune::color::hex(&format!("#{r:02x}{g:02x}{b:02x}")).unwrap_or(Color::BLACK)
}

/// A contrast theme as a [`Theme`]: text, borders, focus ring and selection in system colours, every
/// colour opaque (no translucency, no shadows).
pub fn theme_from_system(s: SystemColors) -> Theme {
    let window = colorref(s.window);
    let text = colorref(s.window_text);
    let hl = colorref(s.highlight);
    let hl_text = colorref(s.highlight_text);
    let gray = colorref(s.gray_text);
    let hot = colorref(s.hot_light);
    let face = colorref(s.btn_face);
    let btn_text = colorref(s.btn_text);
    let none = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    let mut t = super::tokens::base(relative_luminance(window) < 0.18);
    t.bg = window;
    t.card = window;
    t.toolbar_bg = window;
    t.status_bar_bg = window;
    t.surface = face;
    t.sidebar_bg = face;
    t.paper = window;
    t.paper2 = face;
    t.paper3 = face;
    t.field = window;
    t.white = window;
    t.card_border = text;
    t.separator = text;
    t.separator_strong = text;
    t.highlight = none;
    t.text = text;
    t.text_secondary = text;
    t.text_tertiary = text;
    t.text_quaternary = gray;
    t.icon = btn_text;
    t.icon_secondary = btn_text;
    t.hover = hot;
    t.press = hl;
    t.selection = hl;
    t.selection_row = hl;
    t.tile = face;
    t.well = face;
    t.badge = face;
    t.accent = hl;
    t.accent_deep = hl;
    t.accent_bg = window;
    t.accent_wash = window;
    t.accent_text_on = hl_text;
    t.shadow = none;
    t.shadow_raised = none;
    t.shadow_page = none;
    t.shadow_menu = none;
    t.scrollbar = text;
    t.green = text;
    t.green_bg = window;
    t.red = text;
    t.caption_hover = hot;
    t.caption_close = hl;
    t
}

/// What the system said last: its contrast colours (`None`: no contrast theme) and the theme made from them,
/// made once per change rather than once per frame.
#[derive(Clone, Copy)]
struct Known {
    colors: Option<SystemColors>,
    theme: Option<Theme>,
}

impl Known {
    fn of(colors: Option<SystemColors>) -> Self {
        Known {
            colors,
            theme: colors.map(theme_from_system),
        }
    }
}

/// The platform's answer, read once at the first frame. Tests start from "none" and never read the desktop
/// they run on.
fn first_read() -> Option<SystemColors> {
    #[cfg(all(windows, not(test)))]
    {
        crate::shell::platform::contrast_colors()
    }
    #[cfg(not(all(windows, not(test))))]
    {
        None
    }
}

// One answer for the process: every window follows the same setting. Per thread under test, so a test that
// turns a contrast theme on cannot change what a test on another thread draws.
#[cfg(not(test))]
static KNOWN: std::sync::Mutex<Option<Known>> = std::sync::Mutex::new(None);
#[cfg(test)]
thread_local! {
    static KNOWN: std::cell::RefCell<Option<Known>> = const { std::cell::RefCell::new(None) };
}

fn with_known<R>(f: impl FnOnce(&mut Option<Known>) -> R) -> R {
    #[cfg(not(test))]
    {
        f(&mut KNOWN.lock().unwrap_or_else(|e| e.into_inner()))
    }
    #[cfg(test)]
    {
        KNOWN.with(|k| f(&mut k.borrow_mut()))
    }
}

/// The Windows contrast theme as a [`Theme`]; `None` when none is on, and off Windows.
pub fn system_contrast() -> Option<Theme> {
    with_known(|k| k.get_or_insert_with(|| Known::of(first_read())).theme)
}

/// A settings change: what the system says now (`None`: no contrast theme). True when that differs from what
/// was known, so the caller redraws.
pub fn system_changed(colors: Option<SystemColors>) -> bool {
    with_known(|k| {
        let was = k.get_or_insert_with(|| Known::of(first_read())).colors;
        if was == colors {
            return false;
        }
        *k = Some(Known::of(colors));
        true
    })
}
