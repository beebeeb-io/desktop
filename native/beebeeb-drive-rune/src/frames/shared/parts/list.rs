//! A grouped list (`.kit-list` > `.kit-list-body` > `.kit-item`): a card of two-line items,
//! each with a tile, and whatever the section hangs at its end.

use super::icons::MORE;
use super::style;
use super::table::{Kind, tile};
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use rune::{Color, El, Icon, col, el, row};

/// One item. Keys: `item-<n>-<m>` and `-tile`, `-tile-icon`, `-title`, `-sub`, `-more`; `trailing`
/// elements (avatars, a pill, "Link") sit between the words and the menu glyph.
#[allow(clippy::too_many_arguments)]
pub fn item(
    n: usize,
    m: usize,
    kind: Kind,
    glyph: Option<&'static Icon>,
    title: &str,
    sub: &str,
    trailing: Vec<El>,
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
        .child(kit::glyph(format!("{k}-more"), &MORE, 16.0, t.text_tertiary))
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
