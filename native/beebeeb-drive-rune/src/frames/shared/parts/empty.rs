//! An empty state (`.kit-empty`): a tile with the section's glyph, a title, one sentence, and
//! at most one plain button.

use super::style;
use crate::theme::tokens::Theme;
use crate::ui::kit;
use rune::{Align, El, Icon, col, el};

/// Keys: `empty`, `empty-tile`, `empty-icon`, `empty-title`, `empty-sub`.
pub fn empty(ic: &'static Icon, title: &str, sub: &str, action: Option<El>, t: &Theme) -> El {
    let mut block = col()
        .key("empty")
        .role("group")
        .label(title)
        .items_center()
        .gap(8.0)
        .py(28.0)
        .px(24.0)
        .child(
            el().key("empty-tile")
                .size(52.0, 52.0)
                .radius(15.0)
                .fill(t.tile)
                .center()
                .child(kit::glyph("empty-icon", ic, 26.0, t.text_secondary)),
        )
        .child(kit::label("empty-title", title, style::TITLE_LG, t.text))
        // `.kit-empty .t-sub { max-width: 30ch }`: 227.11 px at this size.
        .child(
            kit::label("empty-sub", sub, style::SUB, t.text_tertiary)
                .w(227.11)
                .wrap()
                .align(Align::Center),
        );
    if let Some(action) = action {
        block = block.child(action);
    }
    el().key("empty-area").center().child(block)
}
