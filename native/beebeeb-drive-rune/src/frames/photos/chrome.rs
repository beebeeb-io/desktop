//! The Photos window (`.mk-win` with Photos on): caption, sidebar, the section's toolbar, the
//! body, the status line, and what lies in front (a menu, a dialog on its scrim).
//!
//! The toolbar is the one window toolbar (`ui::toolbar::bar`) with this section's title, its
//! controls in the slot and its states for the standard ones.

use crate::ui::toolbar::title_block;
use rune::{Color, El, Icon, TextStyle, col, row, text_input};

use super::Intent;
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::toolbar as tb;
use crate::ui::{button, caption, sidebar, status_bar};

/// The group's own type styles, by the board's class.
pub mod style {
    use crate::theme::scale::text;
    use crate::theme::tokens::type_;
    use rune::TextStyle;
    use rune::tok::Family;

    pub const SEG: TextStyle = TextStyle::new(12.0, 550.0).lh(15.0);
    pub const SUB: TextStyle = text::SUB;
    pub const SUB_STRONG: TextStyle = text::SUB_STRONG;
    /// `.mk-gbadge`.
    pub const BADGE: TextStyle = TextStyle::new(10.0, 600.0).family(Family::Mono).lh(13.0);
    pub const BULK: TextStyle = type_::SEARCH;
    pub const BULK_STRONG: TextStyle = text::FIELD_SEMI;
    pub const BUTTON: TextStyle = text::ITEM_TITLE_TALL;
    pub const CAP: TextStyle = type_::CAPTION;
    pub const CAP_MONO: TextStyle = type_::KBD;
    /// `.kit-list-title`: no token (tracking 0.1).
    pub const LIST_TITLE: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.1);
    pub const ITEM_TITLE: TextStyle = type_::ITEM_TITLE;
    pub const ITEM_SUB: TextStyle = text::SUB;
    pub const ITEM_VALUE: TextStyle = text::READ;
    pub const ITEM_VALUE_MONO: TextStyle = text::FIELD_MONO;
    pub const POP: TextStyle = type_::SEARCH;
    pub const SIZE_MONO: TextStyle = type_::KBD;
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

pub fn toolbar(s: &State, bar: Bar, t: &Theme) -> El {
    let title = title_block(&bar.title, Some(&bar.sub), t);

    const SEARCH_PLACEHOLDER: &str = "Search names";
    let search = tb::search_box(
        "search-input",
        SEARCH_PLACEHOLDER,
        text_input(&s.search, type_::SEARCH)
            .key("search-input")
            .label("Search names")
            .shortcut_hint(concat!(beebeeb_engine::primary_key!(), "F"))
            .single_line()
            .color(t.text)
            .placeholder(SEARCH_PLACEHOLDER, t.text_tertiary)
            .on_edit(|s: &mut State| &mut s.search)
            .on_submit(|s: &mut State, _| {
                let words = s.search.text.to_string();
                s.photos.ask(Intent::Search(words));
            })
            .grow(1.0),
        t,
    );

    tb::bar(
        tb::Bar {
            name: "Photos".into(),
            // Back leaves an open photo; in the grid there is nowhere to go back to.
            back: (bar.back, tb::press(|s, _| s.photos.close_viewer())),
            forward: (false, None),
            title,
            slot: Some(bar.middle),
            slot_sep: false,
            // Photos has one layout: the list/grid switch and Sort show, dimmed, and take nothing.
            view: tb::View::NotHere,
            on_view: None,
            sort: tb::Sort::NotHere,
            on_sort: None,
            // Live over an open photo: it opens Details.
            details: if bar.details {
                tb::Details::Closed
            } else {
                tb::Details::NotHere
            },
            on_details: tb::press(|s, _| s.photos.open_details()),
            inert: false,
            gate_closed: false,
            // Photos shows no folder: the Beebeeb location.
            on_explorer: Box::new(tb::open_beebeeb_location),
            // "New here adds to Files" (DW-0601): the menu is the Files lane's; this asks for it.
            new: tb::New::Idle,
            on_new: tb::press(|s, _| s.photos.ask(Intent::NewInFiles)),
            search,
            trailing_inset: 0.0,
        },
        t,
    )
}

/// The status line (`.mk-foot`): the PC's sync state at the left (the same words and dot as Files), this
/// view's words at the right. No words, no dot.
pub fn status_line(sync: &str, details: &str, t: &Theme) -> El {
    let state = (!sync.is_empty()).then_some(sync);
    status_bar::bar(state.map(status_bar::dot_for), state, details, t)
}

/// The window. `front` is drawn over the whole client area, in order (a menu; a scrim and its dialog).
#[allow(clippy::too_many_arguments)]
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
        .child(status_line(&crate::frames::sync_words(s), status, t));
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

fn mapped(tone: Tone, h: f32) -> (button::Variant, button::Size) {
    let variant = match tone {
        Tone::Plain => button::Variant::Secondary,
        Tone::Quiet => button::Variant::Ghost,
        Tone::Primary => button::Variant::Primary,
    };
    let size = if h <= 24.0 {
        button::Size::Sm
    } else if h <= 30.0 {
        button::Size::Md
    } else {
        button::Size::Lg
    };
    (variant, size)
}

/// A button's width on the board (`ui::button::natural_width`).
pub fn button_width(label: &str, icon: bool, tone: Tone, h: f32, _font: TextStyle) -> f32 {
    let (variant, size) = mapped(tone, h);
    button::natural_width(label, icon, variant, size)
}

pub fn button(key: &str, label: &str, ic: Option<&'static Icon>, tone: Tone, h: f32, _font: TextStyle, t: &Theme) -> El {
    let (variant, size) = mapped(tone, h);
    button::button(button::keys(key), label, ic, variant, size, button::ButtonState::Enabled, t)
}
