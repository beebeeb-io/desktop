//! Controls the settings panes share beyond rows: buttons, the inline notice, the drop-down menu.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Button (`.kit-btn.kit-btn-sm`) | `button(index, label, icon, Emphasis, t)` | `btn-<N>`, `btn-<N>-icon`, `btn-<N>-label` |
//! | Inline notice (`.kit-banner`) | `banner(index, icon, title, text, action, t)` | `banner-<N>`, `-icon`, `-title`, `-text` |
//! | Menu (`.wn-menu`) | `menu(index, items, t)` + `menu_item(..)` | `menu-<M>`, `menu-<M>-item-<I>`, `-check`, `-label` |
//!
//! Buttons are numbered in the order the board draws them in a frame (`scripts/keymap.js`
//! keys every `.kit-btn` of the settings view by order). Amber is the primary action only.

use crate::theme::icons_settings as ic;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Icon, TextStyle, col, el, row};

use super::kit;

/// 13px/600 on a 17px line: a notice's first line (`.kit-banner-text b`).
pub const BANNER_TITLE: TextStyle = TextStyle::new(13.0, 600.0).lh(17.0);
/// 12.5px/400 on a 17px line: a notice's words (`.kit-banner-text`).
pub const BANNER_TEXT: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
/// 13px/400 on a 17px line: a menu item (`.wn-mi`).
pub const MENU_ITEM: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);

/// How much a button asks for attention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Emphasis {
    /// The one primary action of a view: Amber.
    Primary,
    /// Any other action: paper with a drawn border.
    Plain,
}

/// A small button (`.kit-btn.kit-btn-sm`, 22 pt). Keys: `btn-<N>`, `btn-<N>-icon`, `btn-<N>-label`.
pub fn button(index: usize, label: &str, icon: Option<&'static Icon>, emphasis: Emphasis, t: &Theme) -> El {
    let key = format!("btn-{index}");
    let (fill, ink) = match emphasis {
        Emphasis::Primary => (t.accent, t.accent_text_on),
        Emphasis::Plain => (t.paper, t.text),
    };
    let mut b = row()
        .key(key.clone())
        .role("button")
        .label(label)
        .h(22.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(radius::MD)
        .fill(fill)
        .focusable(true);
    b = match emphasis {
        // `hover:brightness-95`, `active:brightness-90`: ink over the Amber.
        Emphasis::Primary => b
            .hover(fill.mix(Color::BLACK, 0.05))
            .press(fill.mix(Color::BLACK, 0.10)),
        Emphasis::Plain => b
            .border(1.0, t.card_border)
            .border_layout()
            .hover(t.hover)
            .press(t.press),
    };
    if let Some(icon) = icon {
        b = b.child(kit::glyph(format!("{key}-icon"), icon, 14.0, ink));
    }
    b.child(kit::label(format!("{key}-label"), label, type_::BUTTON_SM, ink))
}

/// An inline notice (`.kit-banner.tone-neutral`): a glyph, a strong first line, the words,
/// and at most one action at the trailing edge. Neutral: it informs, it does not alarm.
/// Keys: `banner-<N>`, `banner-<N>-icon`, `banner-<N>-title`, `banner-<N>-text`.
pub fn banner(index: usize, icon: &'static Icon, title: &str, words: &str, action: Option<El>, t: &Theme) -> El {
    banner_toned(index, false, icon, title, words, action, t)
}

/// The same notice, `red` for something that is off or failed and matters (`.tone-red`).
pub fn banner_toned(
    index: usize,
    red: bool,
    icon: &'static Icon,
    title: &str,
    words: &str,
    action: Option<El>,
    t: &Theme,
) -> El {
    let (fill, ink) = if red {
        (t.red.alpha(0.12), t.red)
    } else {
        (t.tile, t.text_secondary)
    };
    let key = format!("banner-{index}");
    let mut b = row()
        .key(key.clone())
        .role("status")
        .label(format!("{title}. {words}"))
        .w_full()
        .items_start()
        .gap(10.0)
        .py(11.0)
        .px(13.0)
        .radius(13.0)
        .fill(fill)
        .child(
            el().key(format!("{key}-icon-box"))
                .pt(1.0)
                .child(kit::glyph(format!("{key}-icon"), icon, 16.0, ink)),
        )
        .child(
            col()
                .key(format!("{key}-words"))
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(kit::label(format!("{key}-title"), title, BANNER_TITLE, t.text).w_full())
                .child(kit::label(format!("{key}-text"), words, BANNER_TEXT, t.text_secondary).wrap()),
        );
    if let Some(action) = action {
        b = b.child(action);
    }
    b
}

/// One item of a menu (`.wn-mi`): a 14 pt column for the check, then the label. `hot` is
/// the item under the pointer or the keyboard. Keys: `menu-<M>-item-<I>`, `-check`, `-label`.
pub fn menu_item(menu: usize, index: usize, label: &str, checked: bool, hot: bool, t: &Theme) -> El {
    let key = format!("menu-{menu}-item-{index}");
    let mut check = el().key(format!("{key}-check-box")).w(14.0).center();
    if checked {
        check = check.child(kit::glyph(format!("{key}-check"), &ic::CHECK, 12.0, t.text));
    }
    row()
        .key(key.clone())
        .role("menuitemradio")
        .label(label)
        .checked(checked)
        .h(30.0)
        .pl(8.0)
        .pr(12.0)
        .gap(10.0)
        .center_y()
        .radius(radius::SM)
        .fill(if hot { t.press } else { Color::TRANSPARENT })
        .hover(t.press)
        .focusable(true)
        .child(check)
        .child(kit::label(format!("{key}-label"), label, MENU_ITEM, t.text).grow(1.0))
}

/// A drop-down menu (`.wn-menu`): a raised card of items, `min_w` wide at least. The caller
/// places it (`.absolute`) and makes it modal (`.modal`: Esc and a press outside close it).
/// Key: `menu-<M>`.
pub fn menu(index: usize, name: &str, min_w: f32, items: Vec<El>, t: &Theme) -> El {
    let mut m = col()
        .key(format!("menu-{index}"))
        .role("menu")
        .label(name)
        .min_w(min_w)
        .padding(4.0)
        .radius(8.0)
        .fill(t.card)
        .box_shadow(kit::drop(8.0, 28.0, Color::BLACK.alpha(0.20)))
        .box_shadow(kit::ring(kit::HAIR, t.separator));
    for item in items {
        m = m.child(item);
    }
    m
}
