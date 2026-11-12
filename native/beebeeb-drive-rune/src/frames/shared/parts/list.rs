//! A grouped list (`.kit-list` > `.kit-list-body` > `.kit-item`): a card of two-line items,
//! each with a tile, and whatever the section hangs at its end.

use super::icons::MORE;
use super::style;
use super::table::{Kind, tile};
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use rune::{Color, El, Icon, col, el, row};

/// The key of the More button of item `m` in list `n`: an item's menu opens under it.
pub fn more_key(n: usize, m: usize) -> String {
    format!("item-{n}-{m}-more-btn")
}

/// The More button at an item's end (the board's three dots, `.mk-mac`): a button named for
/// its item; the caller adds what a press does. Its box is the glyph's (the padding is taken
/// back by the margin), so the item's layout is the board's. Not a Tab stop: the Menu key on
/// the item does the same. Keys: `item-<n>-<m>-more-btn`, and the glyph `item-<n>-<m>-more`.
pub fn more(n: usize, m: usize, title: &str, t: &Theme) -> El {
    el().key(more_key(n, m))
        .role("button")
        .label(format!("More actions for {title}"))
        .padding(4.0)
        .mx(-4.0)
        .my(-4.0)
        .radius(6.0)
        .center()
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .tab_index(-1)
        .child(kit::glyph(format!("item-{n}-{m}-more"), &MORE, 16.0, t.text_tertiary))
}

/// One item. Keys: `item-<n>-<m>` and `-tile`, `-tile-icon`, `-title`, `-sub`; `trailing`
/// elements (avatars, a pill, "Link") sit between the words and `more` (`list::more`).
#[allow(clippy::too_many_arguments)]
pub fn item(
    n: usize,
    m: usize,
    kind: Kind,
    glyph: Option<&'static Icon>,
    title: &str,
    sub: &str,
    trailing: Vec<El>,
    more: El,
    selected: bool,
    t: &Theme,
) -> El {
    let k = format!("item-{n}-{m}");
    let lead = match glyph {
        Some(ic) => el()
            .key(format!("{k}-tile"))
            .size(28.0, 28.0)
            .radius(7.0)
            .fill(t.tile)
            .center()
            .child(kit::glyph(format!("{k}-tile-icon"), ic, 17.0, t.text_secondary)),
        None => tile(&k, format!("{k}-tile-icon"), kind, 28.0, 7.0, 17.0, t),
    };
    let words = col()
        .key("item-text")
        .grow(1.0)
        .min_w(0.0)
        .gap(1.0)
        .child(kit::label(format!("{k}-title"), title, type_::ITEM_TITLE, t.text).truncate())
        .child(
            kit::label(
                format!("{k}-sub"),
                sub,
                style::SUB,
                if selected { t.text_secondary } else { t.text_tertiary },
            )
            .truncate(),
        );
    row()
        .key(k.clone())
        .role("listitem")
        .label(title)
        .h(40.0)
        .w_full()
        .px(10.0)
        .gap(8.0)
        .center_y()
        .fill(if selected {
            super::item_selected(t)
        } else {
            Color::TRANSPARENT
        })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .focus_selected(selected)
        .child(lead)
        .child(words)
        .children(trailing)
        .child(more)
}

/// The card around the items (`.kit-list-body`), inset 16 pt (`.mx-4`). Key: `list-<n>`.
pub fn list(n: usize, name: &str, items: Vec<El>, t: &Theme) -> El {
    let inner = col()
        .key(format!("list-{n}-items"))
        .w_full()
        .radius(10.0)
        .clip_rounded(10.0)
        .children(items.into_iter().enumerate().flat_map(|(m, item)| {
            // `.kit-item + .kit-item::before`: a hairline over the item's top edge, from the tile's edge on.
            let rule = (m > 0).then(|| {
                el().key(format!("list-{n}-rule-{m}"))
                    .h(kit::HAIR)
                    .ml(10.0)
                    .mb(-kit::HAIR)
                    .fill(t.separator)
            });
            rule.into_iter().chain(std::iter::once(item))
        }));
    // The ring is outside the card's box, so the card that carries it does not clip; the items' well does.
    el().key(format!("list-{n}-wrap")).w_full().px(16.0).child(
        col()
            .key(format!("list-{n}"))
            .role("list")
            .label(name)
            .w_full()
            .radius(10.0)
            .fill(t.card)
            .box_shadow(kit::ring(kit::HAIR, t.separator))
            .child(inner),
    )
}

/// A section label over a list (`.t-label`). Key: `label-<words>`.
pub fn label(key: &str, words: &str, t: &Theme) -> El {
    kit::label(
        format!("label-{key}"),
        words.to_uppercase(),
        type_::SECTION_LABEL,
        t.text_tertiary,
    )
}
