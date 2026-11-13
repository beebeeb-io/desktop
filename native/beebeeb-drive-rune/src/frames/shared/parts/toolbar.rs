//! A section's toolbar (`.mk-tb`): the one window toolbar (`ui::toolbar::bar`), with the section's own
//! controls in a slot before them and the ones the section cannot use dimmed, as the board
//! draws them.

use crate::State;
use crate::shell::app::Event;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::toolbar::{self as tb, Details, New, Sort, View, press};
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

/// The search field (`.mk-search`), drawn as in Files. It edits these sections' own text
/// (`frames::Ui::search`), never Files' field; Enter asks for the search (the section's
/// `on_key`: `Intent::Do("search", ..)`), which is the Search group's to show (DW-0802).
fn search_field(s: &State, t: &Theme) -> El {
    tb::search_box(
        crate::ui::keys::SEARCH_KEY,
        SEARCH_PLACEHOLDER,
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
        t,
    )
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
    let title = col()
        .key("folder-info")
        .px(4.0)
        .min_w(0.0)
        .child(kit::label("folder-title", bar.title, type_::TITLE, t.text).truncate())
        .child(kit::label("item-count", bar.sub, type_::TITLE_SUB, t.text_tertiary));
    tb::bar(
        tb::Bar {
            name: bar.title.into(),
            back: (
                back,
                if back {
                    press(|s, _| {
                        crate::shell::host::dispatch(s, Event::Back);
                    })
                } else {
                    None
                },
            ),
            forward: (
                forward,
                if forward {
                    press(|s, _| {
                        crate::shell::host::dispatch(s, Event::Forward);
                    })
                } else {
                    None
                },
            ),
            title,
            slot: Some(row().key("toolbar-slot").center_y().gap(8.0).children(bar.slot)),
            slot_sep: bar.slot_sep,
            // List or grid does not apply to these sections: the board draws the switch dimmed, neither side chosen.
            view: View::NotHere,
            on_view: None,
            sort: Sort::Idle,
            on_sort: press(|s, _| s.frames.intents.push(crate::frames::Intent::Frame("DW-0406"))),
            details: Details::NotHere,
            on_details: None,
            inert: false,
            // These sections show no folder: the Beebeeb location.
            on_explorer: Box::new(tb::open_beebeeb_location),
            // New is unavailable in Trash: drawn at 40 %, not focusable.
            new: if bar.new_off { New::Off } else { New::Idle },
            on_new: press(|s, _| s.frames.intents.push(crate::frames::Intent::Frame("DW-0415"))),
            search: search_field(s, t),
            trailing_inset: 0.0,
        },
        t,
    )
}

/// The hairline between a section's switch and its button (`.mk-sep` inside the slot).
pub fn slot_sep(t: &Theme) -> El {
    el().key("toolbar-sep").w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator)
}
