//! Toolbar (`.mk-tb`): back and forward, the folder's title and item count, the list/grid
//! switch, sort, the details-panel toggle, New (the one Amber control) and the search field.

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Icon, col, el, row, text_input};

use super::kit;

/// The search field's placeholder: one string for the field and for the gate's note.
const SEARCH_PLACEHOLDER: &str = "Search names";

/// How a toolbar button (`.mk-tbtn`) shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    /// Ready (ink-2).
    Idle,
    /// Pressed in: a toggle that is on, a menu that is open (neutral wash, ink).
    On,
    /// Nothing to do (ink-4): back with no history.
    Off,
}

/// A toolbar button: 28 pt tall, at least 28 wide, one or more 15/16 pt glyphs.
/// Keys: `<key>` and `<key>-icon` (further glyphs are added by the caller).
pub fn tool_button(
    key: &'static str,
    name: &'static str,
    ic: &'static Icon,
    icon_size: f32,
    state: ToolState,
    t: &Theme,
) -> El {
    let ink = match state {
        ToolState::Idle => t.text_secondary,
        ToolState::On => t.text,
        ToolState::Off => t.text_quaternary,
    };
    let icon_key = match key {
        "panel-toggle-btn" => "panel-icon".to_string(),
        "sort-btn" => "sort-icon".to_string(),
        _ => format!("{key}-icon"),
    };
    row()
        .key(key)
        .role("button")
        .label(name)
        .disabled(state == ToolState::Off)
        .h(28.0)
        .min_w(28.0)
        .px(6.0)
        .gap(4.0)
        .center()
        .radius(radius::CONTROL)
        .fill(if state == ToolState::On {
            t.selection
        } else {
            Color::TRANSPARENT
        })
        .hover(t.hover)
        .press(t.press)
        .focusable(state != ToolState::Off)
        .child(kit::glyph(icon_key, ic, icon_size, ink))
}

/// One segment of a segmented control (`.mk-tgroup .mk-tbtn`): the chosen one is raised.
pub fn segment(key: &'static str, name: &'static str, ic: &'static Icon, on: bool, t: &Theme) -> El {
    let mut seg = el()
        .key(key)
        .role("radio")
        .label(name)
        .checked(on)
        .h(24.0)
        .min_w(28.0)
        .px(6.0)
        .center()
        .radius(radius::CONTROL)
        .focusable(true)
        .child(kit::glyph(
            format!("{key}-icon"),
            ic,
            15.0,
            if on { t.text } else { t.text_secondary },
        ));
    if on {
        seg = seg
            .fill(t.card)
            .box_shadow(kit::drop(1.0, 2.0, t.shadow_raised))
            .box_shadow(kit::ring(kit::HAIR, t.separator));
    } else {
        seg = seg.hover(t.hover).press(t.press);
    }
    seg
}

/// A segmented control's well (`.mk-tgroup`).
pub fn segmented(key: &'static str, name: &'static str, segments: Vec<El>, t: &Theme) -> El {
    row()
        .key(key)
        .role("radiogroup")
        .label(name)
        .padding(2.0)
        .gap(1.0)
        .radius(9.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator))
        .children(segments)
}

/// A keyboard hint (`.kit-kbd`): "Ctrl+F".
pub fn kbd(key: &'static str, keys: &str, t: &Theme) -> El {
    el().key(format!("{key}-box"))
        .h(20.0)
        .px(6.0)
        .radius(6.0)
        .fill(t.card)
        .box_shadow(kit::ring_inset(t.separator))
        .box_shadow(kit::drop(1.0, 0.0, t.separator))
        .center()
        .child(kit::label(key, keys, type_::KBD, t.text_tertiary))
}

pub fn toolbar(s: &State, t: &Theme, trailing_inset: f32) -> El {
    let item_count = s.listing.items.len();

    let nav = row()
        .key("nav")
        .child(
            tool_button(
                "back",
                "Back",
                &icons::CHEVRON_LEFT,
                16.0,
                if s.can_go_back() {
                    ToolState::Idle
                } else {
                    ToolState::Off
                },
                t,
            )
            .shortcut_hint("Alt+Left")
            .on_press(|s: &mut State| {
                s.go_back();
            }),
        )
        .child(
            tool_button("forward", "Forward", &icons::CHEVRON_RIGHT, 16.0, ToolState::Off, t)
                .shortcut_hint("Alt+Right"),
        );

    let title = col()
        .key("folder-info")
        .px(4.0)
        .min_w(0.0)
        .child(kit::label("folder-title", s.listing.folder.name.clone(), type_::TITLE, t.text).truncate())
        .child(kit::label(
            "item-count",
            if item_count == 1 {
                "1 item".to_string()
            } else {
                format!("{item_count} items")
            },
            type_::TITLE_SUB,
            t.text_tertiary,
        ));

    let view_toggle = segmented(
        "view-toggle",
        "View",
        vec![
            segment("view-list", "as List", &icons::VIEW_LIST, true, t),
            segment("view-grid", "as Grid", &icons::VIEW_GRID, false, t),
        ],
        t,
    );

    // Pressed while its menu is open (DW-0404).
    let sorting = s
        .actions
        .menu
        .is_some_and(|m| m.kind == crate::frames::files::MenuKind::Sort);
    let sort_btn = tool_button(
        "sort-btn",
        "Sort",
        &icons::SORT,
        15.0,
        if sorting { ToolState::On } else { ToolState::Idle },
        t,
    )
    .expanded(sorting)
    .on_press(|s: &mut State| s.toggle_menu(crate::frames::files::MenuKind::Sort))
    .child(kit::glyph(
        "sort-caret",
        &icons::CHEVRON_DOWN,
        11.0,
        if sorting { t.text } else { t.text_secondary },
    ));

    let panel_btn = tool_button(
        "panel-toggle-btn",
        "Details panel",
        &icons::PANEL_TOGGLE,
        16.0,
        if s.show_detail_pane {
            ToolState::On
        } else {
            ToolState::Idle
        },
        t,
    )
    .shortcut_hint("Alt+Shift+P")
    .checked(s.show_detail_pane)
    .on_press(|s: &mut State| s.show_detail_pane = !s.show_detail_pane);

    let sep = el().key("toolbar-sep").w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator);

    // Ringed while its menu is open (DW-0414: `box-shadow: 0 0 0 3px` amber at 32 %).
    let adding = s
        .actions
        .menu
        .is_some_and(|m| m.kind == crate::frames::files::MenuKind::New);
    let new_btn = row()
        .key("new-btn")
        .expanded(adding)
        .box_shadow(kit::ring(if adding { 3.0 } else { 0.0 }, t.accent.alpha(0.32)))
        .role("button")
        .label("New")
        .h(28.0)
        .pl(8.0)
        .pr(9.0)
        .gap(5.0)
        .center_y()
        .radius(radius::CONTROL)
        .fill(t.accent)
        .hover(t.press)
        .press(t.press)
        .focusable(true)
        .on_press(|s: &mut State| s.toggle_menu(crate::frames::files::MenuKind::New))
        .child(kit::glyph("plus-icon", &icons::PLUS, 14.0, t.accent_text_on))
        .child(kit::label("new-label", "New", type_::LABEL, t.accent_text_on))
        .child(kit::glyph("new-caret", &icons::CHEVRON_DOWN, 11.0, t.accent_text_on));

    kit::note_text(
        "search-placeholder",
        "search-input",
        SEARCH_PLACEHOLDER,
        type_::SEARCH,
        t.text_tertiary,
    );
    let search_field = row()
        .key("search-box")
        .role("search")
        .w(210.0)
        .h(28.0)
        .gap(6.0)
        .pl(9.0)
        .pr(6.0)
        .radius(8.0)
        .fill(t.field)
        .box_shadow(kit::ring_inset(t.separator))
        .center_y()
        .child(kit::glyph("search-icon", &icons::SEARCH, 13.0, t.text_tertiary))
        .child(
            text_input(&s.search, type_::SEARCH)
                .key("search-input")
                .label("Search names")
                .shortcut_hint("Ctrl+F")
                .single_line()
                .color(t.text)
                .placeholder(SEARCH_PLACEHOLDER, t.text_tertiary)
                .on_edit(|s: &mut State| &mut s.search)
                .grow(1.0),
        )
        .child(kbd("kbd-hint", "Ctrl+F", t));

    row()
        .key("toolbar")
        .role("toolbar")
        .label("Files")
        .h(46.0)
        .w_full()
        .fill(t.toolbar_bg)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .pl(10.0)
        .pr(trailing_inset.max(12.0))
        .center_y()
        .gap(8.0)
        .child(nav)
        .child(title)
        .child(el().key("toolbar-spacer").grow(1.0))
        .child(view_toggle)
        .child(sort_btn)
        .child(panel_btn)
        .child(sep)
        .child(new_btn)
        .child(search_field)
}
