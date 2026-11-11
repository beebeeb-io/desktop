//! Toolbar: back/forward arrows, folder title + item count,
//! view toggle (list/grid), sort button, details-panel toggle,
//! "+ New" amber button, search field with icon and Ctrl+F hint.
//!
//! Board measurements (from Playwright):
//! - Toolbar height: ~40 pt (from y=68 to y=108 approximately)
//! - Back/forward: 28x28 rounded pill buttons with chevron icons
//! - "Files" title: 14px/650, "11 items" subtitle: 11px/400
//! - View toggle buttons: 28x24 each, list active (filled bg), grid inactive
//! - Sort: 42x28 icon button with sort icon
//! - Details panel toggle: 28x28
//! - "+ New": 81x28, amber bg, white text, 13px/600
//! - Search: 210x28, paper-3 bg, "Search names" 12.5px/400, "Ctrl+F" 11px/500 mono

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, space, type_};
use rune::{Color, El, col, el, icon, row, text, text_input};

pub fn toolbar(s: &State, t: &Theme, trailing_inset: f32) -> El {
    let item_count = s.listing.items.len();

    // Back / forward navigation arrows
    let nav = row()
        .key("nav")
        .gap(0.0)
        .center_y()
        .child(nav_button("back", &icons::CHEVRON_LEFT, t))
        .child(nav_button("forward", &icons::CHEVRON_RIGHT, t));

    // Folder title + item count
    let title = col()
        .key("folder-info")
        .ml(space::SM)
        .child(text("Files").key("folder-title").font(type_::TITLE).color(t.text))
        .child(
            text(format!("{item_count} items"))
                .key("item-count")
                .font(type_::BODY_SM)
                .color(t.text_tertiary),
        );

    // View toggle: list | grid
    let view_toggle = row()
        .key("view-toggle")
        .gap(0.0)
        .center_y()
        .child(toggle_button("view-list", &icons::VIEW_LIST, true, t))
        .child(toggle_button("view-grid", &icons::VIEW_GRID, false, t));

    // Sort button
    let sort_btn = row()
        .key("sort-btn")
        .role("button")
        .label("Sort")
        .h(28.0)
        .px(space::SM)
        .center()
        .radius(radius::MD)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(icon(&icons::SORT, 15.0).key("sort-icon").color(t.text_secondary))
        .child(
            icon(&icons::CHEVRON_DOWN, 11.0)
                .key("sort-caret")
                .color(t.text_secondary),
        );

    // Details panel toggle
    let panel_btn = el()
        .key("panel-toggle-btn")
        .role("button")
        .label("Toggle details panel")
        .size(28.0, 28.0)
        .center()
        .radius(radius::MD)
        .fill(t.selection)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(icon(&icons::PANEL_TOGGLE, 15.0).key("panel-icon").color(t.text));

    // "+ New" amber button
    let new_btn = row()
        .key("new-btn")
        .role("button")
        .label("New")
        .h(28.0)
        .px(space::MD)
        .center()
        .radius(radius::MD)
        .fill(t.accent)
        .hover(t.accent) // TODO: slightly different hover
        .press(t.accent)
        .focusable(true)
        .gap(space::XS)
        .child(icon(&icons::PLUS, 14.0).key("plus-icon").color(t.accent_text_on))
        .child(text("New").key("new-label").font(type_::LABEL).color(t.accent_text_on))
        .child(
            icon(&icons::CHEVRON_DOWN, 11.0)
                .key("new-caret")
                .color(t.accent_text_on),
        );

    // Search field
    let search_field = row()
        .key("search-box")
        .h(28.0)
        .w(210.0)
        .px(space::SM)
        .radius(radius::MD)
        .fill(t.surface)
        .border(1.0, t.card_border)
        .center_y()
        .gap(space::XS)
        .child(icon(&icons::SEARCH, 13.0).key("search-icon").color(t.text_tertiary))
        .child(
            text_input(&s.search, type_::SEARCH)
                .key("search-input")
                .single_line()
                .color(t.text)
                .placeholder("Search names", t.text_tertiary)
                .on_edit(|s: &mut State| &mut s.search)
                .grow(1.0),
        )
        .child(
            el().key("kbd-hint")
                .h(20.0)
                .px(space::XS)
                .radius(radius::SM)
                .fill(t.card)
                .center()
                .child(text("Ctrl+F").key("kbd-text").font(type_::KBD).color(t.text_tertiary)),
        );

    // Assemble toolbar
    row()
        .key("toolbar")
        .h(46.0) // Board: toolbar h=46 (y=33 to y=79)
        .w_full()
        .fill(t.bg)
        .pl(space::LG)
        .pr(trailing_inset.max(space::LG))
        .center_y()
        .gap(space::SM)
        .child(nav)
        .child(title)
        .child(el().key("toolbar-spacer").grow(1.0))
        .child(view_toggle)
        .child(sort_btn)
        .child(panel_btn)
        .child(new_btn)
        .child(search_field)
}

fn nav_button(key: &'static str, ic: &'static rune::Icon, t: &Theme) -> El {
    el().key(key)
        .role("button")
        .label(key)
        .size(28.0, 28.0)
        .center()
        .radius(radius::MD)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(icon(ic, 15.0).key("icon").color(t.text_quaternary))
}

fn toggle_button(key: &'static str, ic: &'static rune::Icon, active: bool, t: &Theme) -> El {
    el().key(key)
        .role("button")
        .label(key)
        .size(28.0, 24.0)
        .center()
        .radius(radius::SM)
        .fill(if active { t.card } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(
            icon(ic, 15.0)
                .key("icon")
                .color(if active { t.text } else { t.text_secondary }),
        )
}
