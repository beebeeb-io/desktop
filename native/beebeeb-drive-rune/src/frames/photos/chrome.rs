//! The Photos window (`.mk-win` with Photos on): caption, sidebar, the section's toolbar, the
//! body, the status line, and what lies in front (a menu, a dialog on its scrim).
//!
//! The toolbar's right half (the list/grid switch, Sort, the details panel, New, search) is
//! `ui::toolbar::toolbar`'s, which draws the Files folder and cannot be given another title or
//! other buttons: its pieces are repeated here from the same components (the PR asks the
//! design-system lane to split it).

use rune::{Color, El, Icon, TextStyle, col, el, row, text_input};

use super::Intent;
use crate::State;
use crate::theme::icons as shared;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::toolbar::{ToolState, kbd, segment, segmented, tool_button};
use crate::ui::{caption, kit, sidebar};

/// The group's own type styles, by the board's class.
pub mod style {
    use rune::TextStyle;
    use rune::tok::Family;

    /// `.kit-chip`, `.kit-seg-sm span` is 550.
    pub const CHIP: TextStyle = TextStyle::new(12.0, 600.0).lh(15.0);
    pub const SEG: TextStyle = TextStyle::new(12.0, 550.0).lh(15.0);
    /// `.t-sub` and its `b`.
    pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
    pub const SUB_STRONG: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);
    /// `.mk-gbadge`.
    pub const BADGE: TextStyle = TextStyle::new(10.0, 600.0).family(Family::Mono).lh(13.0);
    /// `.t-title`.
    pub const TITLE: TextStyle = TextStyle::new(17.0, 600.0).lh(22.0).tracking(-0.2);
    /// `.mk-bulk` and its `b`.
    pub const BULK: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
    pub const BULK_STRONG: TextStyle = TextStyle::new(12.5, 600.0).lh(17.0);
    /// `.kit-btn-md`.
    pub const BUTTON: TextStyle = TextStyle::new(13.0, 500.0).lh(18.5714).tracking(-0.065);
    /// `.wn-mi`, its head and its shortcut.
    pub const MENU: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
    pub const MENU_HEAD: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0);
    pub const MENU_KEYS: TextStyle = TextStyle::new(12.0, 400.0).lh(14.5);
    /// `.t-cap`, and in the machine face.
    pub const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
    pub const CAP_MONO: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);
    /// `.kit-toast` and the action in it.
    pub const TOAST: TextStyle = TextStyle::new(13.0, 550.0).lh(17.0);
    /// The board computes 900 for this `b` (bolder than 550) and draws 700, the heaviest Inter
    /// it loads (`wght@400..700`): 700 is what a person sees there.
    pub const TOAST_ACTION: TextStyle = TextStyle::new(13.0, 700.0).lh(17.0);
    /// `.kit-list-title`, `.kit-item-title`, `.kit-item-sub`, `.kit-item-value` (and mono).
    pub const LIST_TITLE: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.1);
    pub const ITEM_TITLE: TextStyle = TextStyle::new(13.0, 500.0).lh(17.0).tracking(-0.065);
    pub const ITEM_SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
    pub const ITEM_VALUE: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
    pub const ITEM_VALUE_MONO: TextStyle = TextStyle::new(12.5, 400.0).family(Family::Mono).lh(17.0);
    pub const POP: TextStyle = TextStyle::new(12.5, 400.0).lh(17.0);
    pub const SIZE_MONO: TextStyle = TextStyle::new(11.0, 500.0).family(Family::Mono).lh(14.0);
}

/// A colour from its sRGB bytes, as the board's computed styles give it.
pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    let lin = |c: u8| rune::color::srgb_decode(f32::from(c) / 255.0);
    Color {
        r: lin(r),
        g: lin(g),
        b: lin(b),
        a: 1.0,
    }
}

/// A text's width as the font gives it. Rune rounds a text's box up to the device pixel, so a
/// row of texts drifts from the board by up to half a point each: a box that holds text in
/// such a row is given this width instead.
pub fn advance(words: &str, font: TextStyle) -> f32 {
    rune::text::advance(words, font)
}

/// What the toolbar's title, sub line and middle show.
pub struct Bar {
    pub title: String,
    pub sub: String,
    /// Back leaves an open photo; in the grid there is nowhere to go back to.
    pub back: bool,
    /// The details panel button: live over an open photo (it opens Details).
    pub details: bool,
    /// The section's own controls, left of the list/grid switch.
    pub middle: El,
}

/// A toolbar button of this group (`.mk-tbtn`), keyed by a `String`.
pub fn tbtn(key: &str, name: &str, ic: &'static Icon, size: f32, on: bool, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(name)
        .h(28.0)
        .min_w(28.0)
        .px(6.0)
        .gap(4.0)
        .center()
        .radius(radius::CONTROL)
        .fill(if on { t.selection } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(kit::glyph(
            format!("{key}-icon"),
            ic,
            size,
            if on { t.text } else { t.text_secondary },
        ))
}

pub fn toolbar(s: &State, bar: Bar, t: &Theme) -> El {
    let nav = row()
        .key("nav")
        .child(
            tool_button(
                "back",
                "Back",
                &shared::CHEVRON_LEFT,
                16.0,
                if bar.back { ToolState::Idle } else { ToolState::Off },
                t,
            )
            .shortcut_hint("Alt+Left")
            .on_press(|s: &mut State| s.photos.close_viewer()),
        )
        .child(
            tool_button("forward", "Forward", &shared::CHEVRON_RIGHT, 16.0, ToolState::Off, t)
                .shortcut_hint("Alt+Right"),
        );

    let title = col()
        .key("folder-info")
        .px(4.0)
        .min_w(0.0)
        .shrink()
        // Cut at the box's edge, as the board's `overflow: hidden` does (no ellipsis).
        .clip_rounded(0.0)
        .child(kit::label("folder-title", bar.title, type_::TITLE, t.text).w_full())
        .child(kit::label("item-count", bar.sub, type_::TITLE_SUB, t.text_tertiary).w_full());

    // Photos has one layout: the list/grid switch and Sort show, dimmed, and take nothing.
    let view_toggle = segmented(
        "view-toggle",
        "View",
        vec![
            segment("view-list", "as List", &shared::VIEW_LIST, false, t)
                .disabled(true)
                .focusable(false),
            segment("view-grid", "as Grid", &shared::VIEW_GRID, false, t)
                .disabled(true)
                .focusable(false),
        ],
        t,
    )
    .disabled(true)
    .opacity(0.45);
    let sort_btn = tool_button("sort-btn", "Sort", &shared::SORT, 15.0, ToolState::Off, t).child(kit::glyph(
        "sort-caret",
        &shared::CHEVRON_DOWN,
        11.0,
        t.text_quaternary,
    ));
    let panel_btn = tool_button(
        "panel-toggle-btn",
        "Details panel",
        &shared::PANEL_TOGGLE,
        16.0,
        if bar.details { ToolState::Idle } else { ToolState::Off },
        t,
    )
    .shortcut_hint("Alt+Shift+P")
    .on_press(|s: &mut State| s.photos.open_details());
    let sep = el().key("toolbar-sep").w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator);

    // "New here adds to Files" (DW-0601): the menu is the Files lane's; this asks for it.
    let new_btn = row()
        .key("new-btn")
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
        .on_press(|s: &mut State| s.photos.ask(Intent::NewInFiles))
        .child(kit::glyph("plus-icon", &shared::PLUS, 14.0, t.accent_text_on))
        .child(kit::label("new-label", "New", type_::LABEL, t.accent_text_on))
        .child(kit::glyph("new-caret", &shared::CHEVRON_DOWN, 11.0, t.accent_text_on));

    const SEARCH_PLACEHOLDER: &str = "Search names";
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
        .child(kit::glyph("search-icon", &shared::SEARCH, 13.0, t.text_tertiary))
        .child(
            text_input(&s.search, type_::SEARCH)
                .key("search-input")
                .label("Search names")
                .shortcut_hint("Ctrl+F")
                .single_line()
                .color(t.text)
                .placeholder(SEARCH_PLACEHOLDER, t.text_tertiary)
                .on_edit(|s: &mut State| &mut s.search)
                .on_submit(|s: &mut State, _| {
                    let words = s.search.text.to_string();
                    s.photos.ask(Intent::Search(words));
                })
                .grow(1.0),
        )
        .child(kbd("kbd-hint", "Ctrl+F", t));

    row()
        .key("toolbar")
        .role("toolbar")
        .label("Photos")
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
        .child(bar.middle)
        .child(view_toggle)
        .child(sort_btn)
        .child(panel_btn)
        .child(sep)
        .child(new_btn)
        .child(search_field)
}

/// The status line (`.mk-foot`): the PC's sync state at the left, this view's words at the right.
pub fn status_line(details: &str, t: &Theme) -> El {
    row()
        .key("status-bar")
        .role("status")
        .h(28.0)
        .w_full()
        .fill(t.status_bar_bg)
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .px(16.0)
        .gap(7.0)
        .center_y()
        .child(el().key("status-dot").size(7.0, 7.0).radius(3.5).fill(t.green))
        .child(kit::label(
            "status-text",
            "Up to date",
            type_::STATUS_STRONG,
            t.text_secondary,
        ))
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label("status-details", details, type_::STATUS, t.text_tertiary))
}

/// The window. `front` is drawn over the whole client area, in order (a menu; a scrim and its dialog).
pub fn window(
    s: &State,
    cx: &mut rune::Cx<State>,
    t: &Theme,
    bar: Bar,
    under_bar: Option<El>,
    body: El,
    status: &str,
    front: Vec<El>,
) -> El {
    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar(s, bar, t));
    if let Some(under) = under_bar {
        main = main.child(under);
    }
    let main = main
        .child(body.grow(1.0).min_h(0.0).w_full())
        .child(status_line(status, t));
    col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), t, cx))
        .child(
            row()
                .key("body")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .child(sidebar::sidebar(s, t))
                .child(main),
        )
        .children(front)
}

/// A push button (`.kit-btn`): `Plain` on paper with a line, `Quiet` bare, `Primary` Amber.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Quiet,
    Primary,
}

/// A button's width on the board: the words as the font gives them, the icon and its gap, 12 pt
/// each side (and a plain button's two borders).
pub fn button_width(label: &str, icon: bool, tone: Tone, h: f32, font: TextStyle) -> f32 {
    let icon_size = if h > 24.0 { 16.0 } else { 14.0 };
    let content = advance(label, font) + if icon { icon_size + 7.0 } else { 0.0 };
    content + if tone == Tone::Plain { 26.0 } else { 24.0 }
}

pub fn button(key: &str, label: &str, ic: Option<&'static Icon>, tone: Tone, h: f32, font: TextStyle, t: &Theme) -> El {
    let ink = match tone {
        Tone::Plain => t.text,
        Tone::Quiet => t.text_secondary,
        Tone::Primary => t.accent_text_on,
    };
    let icon_size = if h > 24.0 { 16.0 } else { 14.0 };
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .h(h)
        .w(button_width(label, ic.is_some(), tone, h, font))
        .gap(7.0)
        .center()
        .radius(6.0)
        .focusable(true)
        .hover(t.hover)
        .press(t.press);
    b = match tone {
        Tone::Plain => b.fill(t.paper).border(1.0, t.card_border).border_layout(),
        Tone::Quiet => b,
        Tone::Primary => b.fill(t.accent),
    };
    if let Some(ic) = ic {
        b = b.child(kit::glyph(format!("{key}-icon"), ic, icon_size, ink));
    }
    b.child(kit::label(format!("{key}-label"), label, font, ink))
}
