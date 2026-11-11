//! A section's toolbar (`.mk-tb`): the Files toolbar's controls, with the section's own
//! controls in a slot before them and the ones the section cannot use dimmed, as the board
//! draws them.

use crate::State;
use crate::shell::app::Event;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::kit;
use crate::ui::toolbar::{ToolState, kbd, segment, segmented, tool_button};
use rune::{El, col, el, row, text_input};

const SEARCH_PLACEHOLDER: &str = "Search names";

/// What a section puts in its toolbar.
pub struct Bar {
    pub title: &'static str,
    /// Under the title: a count in words.
    pub sub: String,
    /// The section's own controls (`.mk-tb > .flex.items-center.gap-2`).
    pub slot: Vec<El>,
    /// The slot has its own hairline (`slot_sep`), so the toolbar's is the second one.
    pub slot_sep: bool,
    /// New is unavailable here (Trash): drawn at 40 %, not focusable.
    pub new_off: bool,
}

/// New (`.mk-new`), the window's one Amber control.
fn new_button(off: bool, t: &Theme) -> El {
    let button = row()
        .key("new-btn")
        .role("button")
        .label("New")
        .disabled(off)
        .h(28.0)
        .pl(8.0)
        .pr(9.0)
        .gap(5.0)
        .center_y()
        .radius(radius::CONTROL)
        .fill(t.accent)
        .child(kit::glyph("plus-icon", &icons::PLUS, 14.0, t.accent_text_on))
        .child(kit::label("new-label", "New", type_::LABEL, t.accent_text_on))
        .child(kit::glyph("new-caret", &icons::CHEVRON_DOWN, 11.0, t.accent_text_on));
    if off {
        button.opacity(0.4)
    } else {
        button
            .hover(t.press)
            .press(t.press)
            .focusable(true)
            .on_press(|s: &mut State| s.frames.intents.push(crate::frames::Intent::Frame("DW-0415")))
    }
}

/// The search field (`.mk-search`), drawn as in Files. It edits these sections' own text
/// (`frames::Ui::search`), never Files' field; Enter asks for the search (the section's
/// `on_key`: `Intent::Do("search", ..)`), which is the Search group's to show (DW-0802).
fn search_field(s: &State, t: &Theme) -> El {
    kit::note_text(
        "search-placeholder",
        "search-input",
        SEARCH_PLACEHOLDER,
        type_::SEARCH,
        t.text_tertiary,
    );
    row()
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
            text_input(&s.frames.search, type_::SEARCH)
                .key(crate::ui::keys::SEARCH_KEY)
                .label("Search names")
                .shortcut_hint("Ctrl+F")
                .single_line()
                .color(t.text)
                .placeholder(SEARCH_PLACEHOLDER, t.text_tertiary)
                .on_edit(|s: &mut State| &mut s.frames.search)
                .on_input_key(|s: &mut State, ev: &rune::KeyEvent, _: &mut rune::Tx| search_key(s, ev))
                .grow(1.0),
        )
        .child(kbd("kbd-hint", "Ctrl+F", t))
}

/// The search field's keys while it has focus: Enter asks for what is typed (the Search
/// group shows the results, DW-0802). Every other key is the field's.
pub fn search_key(s: &mut State, ev: &rune::KeyEvent) -> bool {
    if ev.key != rune::Key::Enter || !crate::frames::plain(ev) {
        return false;
    }
    let typed = s.frames.search.text.trim().to_string();
    if typed.is_empty() {
        return false;
    }
    s.frames.intents.push(crate::frames::Intent::Do("search", typed));
    true
}

pub fn toolbar(s: &State, bar: Bar, t: &Theme) -> El {
    // Back and Forward are the shell's history: live when there is somewhere to go.
    let (back, forward) = crate::frames::history(s);
    let live = |on: bool| if on { ToolState::Idle } else { ToolState::Off };
    let mut back_button =
        tool_button("back", "Back", &icons::CHEVRON_LEFT, 16.0, live(back), t).shortcut_hint("Alt+Left");
    if back {
        back_button = back_button.on_press(|s: &mut State| {
            crate::shell::host::dispatch(s, Event::Back);
        });
    }
    let mut forward_button =
        tool_button("forward", "Forward", &icons::CHEVRON_RIGHT, 16.0, live(forward), t).shortcut_hint("Alt+Right");
    if forward {
        forward_button = forward_button.on_press(|s: &mut State| {
            crate::shell::host::dispatch(s, Event::Forward);
        });
    }
    let nav = row().key("nav").child(back_button).child(forward_button);
    let title = col()
        .key("folder-info")
        .px(4.0)
        .min_w(0.0)
        .child(kit::label("folder-title", bar.title, type_::TITLE, t.text).truncate())
        .child(kit::label("item-count", bar.sub, type_::TITLE_SUB, t.text_tertiary));
    // List or grid does not apply to these sections: the board draws the switch dimmed, neither side chosen.
    let view_toggle = segmented(
        "view-toggle",
        "View",
        vec![
            segment("view-list", "as List", &icons::VIEW_LIST, false, t)
                .disabled(true)
                .focusable(false),
            segment("view-grid", "as Grid", &icons::VIEW_GRID, false, t)
                .disabled(true)
                .focusable(false),
        ],
        t,
    )
    .disabled(true)
    .opacity(0.45);
    let sort = tool_button("sort-btn", "Sort", &icons::SORT, 15.0, ToolState::Idle, t)
        .child(kit::glyph("sort-caret", &icons::CHEVRON_DOWN, 11.0, t.text_secondary))
        .on_press(|s: &mut State| s.frames.intents.push(crate::frames::Intent::Frame("DW-0406")));
    let panel = tool_button(
        "panel-toggle-btn",
        "Details panel",
        &icons::PANEL_TOGGLE,
        16.0,
        ToolState::Off,
        t,
    );
    let sep_key = if bar.slot_sep { "toolbar-sep-2" } else { "toolbar-sep" };

    row()
        .key("toolbar")
        .role("toolbar")
        .label(bar.title)
        .h(46.0)
        .w_full()
        .fill(t.toolbar_bg)
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .pl(10.0)
        .pr(12.0)
        .center_y()
        .gap(8.0)
        .child(nav)
        .child(title)
        .child(el().key("toolbar-spacer").grow(1.0))
        .child(row().key("toolbar-slot").center_y().gap(8.0).children(bar.slot))
        .child(view_toggle)
        .child(sort)
        .child(panel)
        .child(el().key(sep_key).w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator))
        .child(new_button(bar.new_off, t))
        .child(search_field(s, t))
}

/// The hairline between a section's switch and its button (`.mk-sep` inside the slot).
pub fn slot_sep(t: &Theme) -> El {
    el().key("toolbar-sep").w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator)
}
