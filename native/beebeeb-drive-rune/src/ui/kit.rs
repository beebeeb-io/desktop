//! Building blocks every frame uses, and the notes the frame gate reads.
//!
//! `label` and `glyph` are `text(..)` and `icon(..)` with a key, which also tell the gate
//! (`ui::gate`, `scripts/gate.py`) WHAT was drawn under that key: the typography of a text,
//! and which glyph an icon is. Rune's element tree reports a text's string, box and colour
//! but not its font, and an icon's box but not its name, so the gate cannot read those back
//! from the tree. Notes are only kept while a gate export is recording; a running app pays
//! one thread-local read per call.

use std::cell::RefCell;
use std::collections::BTreeMap;

use rune::tok::Family;
use rune::{Color, El, Icon, TextStyle, el, icon, text};

/// What the gate knows about one key beyond its box.
#[derive(Clone, Default)]
pub struct Note {
    pub style: Option<TextStyle>,
    pub text: Option<String>,
    pub color: Option<Color>,
    pub icon: Option<&'static Icon>,
    pub filled: bool,
    pub state: Option<&'static str>,
    /// The box comes from this other key (a control Rune builds, such as a text input).
    pub of: Option<String>,
}

thread_local! {
    static NOTES: RefCell<Option<BTreeMap<String, Note>>> = const { RefCell::new(None) };
}

/// Start keeping notes (the gate export does; nothing else needs to).
pub fn record() {
    NOTES.with(|n| *n.borrow_mut() = Some(BTreeMap::new()));
}

/// Stop keeping notes and hand them over.
pub fn take() -> BTreeMap<String, Note> {
    NOTES.with(|n| n.borrow_mut().take().unwrap_or_default())
}

fn note(key: &str, f: impl FnOnce(&mut Note)) {
    NOTES.with(|n| {
        if let Some(map) = n.borrow_mut().as_mut() {
            f(map.entry(key.to_string()).or_default());
        }
    });
}

/// A keyed text in one style and colour.
pub fn label(key: impl Into<String>, content: impl Into<String>, style: TextStyle, color: Color) -> El {
    let key = key.into();
    note(&key, |n| {
        n.style = Some(style);
        n.color = Some(color);
    });
    text(content).key(key).font(style).color(color)
}

/// A keyed icon that draws in its own colours (the brand mark).
pub fn emblem(key: impl Into<String>, ic: &'static Icon, size: f32) -> El {
    let key = key.into();
    note(&key, |n| n.icon = Some(ic));
    icon(ic, size).key(key)
}

/// A keyed icon: which glyph, how large, in which colour.
pub fn glyph(key: impl Into<String>, ic: &'static Icon, size: f32, color: Color) -> El {
    let key = key.into();
    note(&key, |n| {
        n.icon = Some(ic);
        n.color = Some(color);
    });
    icon(ic, size).key(key).color(color)
}

/// A glyph that stands for a named state (the PC column: `online-only`, `downloaded`,
/// `available-offline`). `filled` draws the first path solid and the rest in `knockout`,
/// as the board does for a kept item.
pub fn state_glyph(
    key: impl Into<String>,
    ic: &'static Icon,
    solid: Option<(&'static Icon, &'static Icon, Color)>,
    state: &'static str,
    size: f32,
    color: Color,
) -> El {
    let key = key.into();
    note(&key, |n| {
        n.icon = Some(ic);
        n.color = Some(color);
        n.filled = solid.is_some();
        n.state = Some(state);
    });
    match solid {
        None => icon(ic, size).key(key).color(color),
        Some((disc, mark, knockout)) => el()
            .key(key)
            .size(size, size)
            .child(icon(disc, size).key("disc").color(color).absolute(0.0, 0.0))
            .child(icon(mark, size).key("mark").color(knockout).absolute(0.0, 0.0)),
    }
}

/// Tell the gate about a text Rune draws itself (a field's placeholder): its string, style
/// and colour, in the box of the element keyed `of`.
pub fn note_text(key: &str, of: &str, content: &str, style: TextStyle, color: Color) {
    note(key, |n| {
        n.style = Some(style);
        n.color = Some(color);
        n.text = Some(content.to_string());
        n.of = Some(of.to_string());
    });
}

/// `sans` or `mono`, as the board's export names a family.
pub fn family_name(style: &TextStyle) -> &'static str {
    match style.family {
        Family::Mono => "mono",
        _ => "sans",
    }
}

/// A hairline: half a point, one device pixel at 200 %.
pub const HAIR: f32 = 0.5;

use rune::Shadow;

/// A hairline ring inside the box: CSS `box-shadow: inset 0 0 0 .5px`.
pub fn ring_inset(color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur: 0.0,
        spread: HAIR,
        color,
        inset: true,
    }
}

/// A ring outside the box `width` wide: CSS `box-shadow: 0 0 0 <width>`.
pub fn ring(width: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur: 0.0,
        spread: width,
        color,
        inset: false,
    }
}

/// A drop shadow: CSS `box-shadow: 0 <y> <blur>`.
pub fn drop(y: f32, blur: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y,
        blur,
        spread: 0.0,
        color,
        inset: false,
    }
}

/// A person's initials on their tint, `d` points across, ringed in the surface it sits on
/// (`.kit-avatar`). `tint` carries the hue; the theme picks its strength.
pub fn avatar(
    key: impl Into<String>,
    initials: &str,
    d: f32,
    tint: Color,
    surface: Color,
    t: &crate::theme::tokens::Theme,
) -> El {
    let key = key.into();
    let style = crate::theme::tokens::type_::avatar_initials(if d >= 26.0 { 10.0 } else { 8.0 });
    note(&key, |n| {
        n.style = Some(style);
        n.color = Some(t.avatar_ink());
        n.text = Some(initials.to_string());
    });
    el().key(key)
        .size(d, d)
        .radius(d / 2.0)
        .fill(t.avatar_tint(tint))
        .box_shadow(ring(2.0, surface))
        .center()
        .child(text(initials).key("initials").font(style).color(t.avatar_ink()))
}
