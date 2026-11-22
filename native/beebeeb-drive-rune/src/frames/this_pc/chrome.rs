//! The Drive window around a This PC section: the toolbar's fixed controls and the status line
//! (the sidebar is `ui::drive_sidebar`, with the section's row lit).
//!
//! `ui::toolbar::toolbar` and `ui::status_bar` hold Files' own words. These are the same pieces
//! with the section's content (see the PR body, "Requests to the design-system lane").

use rune::{El, row, text_input};

use super::Ui;
use crate::State;
use crate::shell::app::Event;
use crate::shell::host;
use crate::theme::scale::space;
use crate::theme::tokens::{Theme, type_};

use crate::ui::toolbar as tb;

/// How the toolbar's Files controls show in a section that has no use for them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FilesControls {
    /// Drawn as in Files (the board's Sync history), and inert (`ui::toolbar::Bar::inert`).
    AsFiles,
    /// The view switch at 45 % and the panel button off (the board's Available offline).
    Dimmed,
}

/// The search field: the one search box (`ui::toolbar::search_box`) around the section's own input. Esc clears it.
pub fn search_field(ui: &Ui, placeholder: &'static str, t: &Theme) -> El {
    let input = text_input(&ui.search, type_::SEARCH)
        .key(crate::ui::keys::SEARCH_KEY)
        .label(placeholder)
        .shortcut_hint("Ctrl+F")
        .single_line()
        .color(t.text)
        .caret(t.accent_deep, t.text.alpha(0.15))
        .placeholder(placeholder, t.text_tertiary)
        .on_edit(|s: &mut State| &mut s.this_pc.search)
        .grow(1.0);
    tb::search_box("search-input", placeholder, input, t)
}

/// The toolbar (`.mk-tb`): the one window toolbar (`ui::toolbar::bar`) with the section's title,
/// its own controls in the slot (`slot`), the Files controls as `how` says, and its search field.
#[allow(clippy::too_many_arguments)]
pub fn toolbar(
    s: &State,
    ui: &Ui,
    name: &'static str,
    sub: String,
    slot: Vec<El>,
    how: FilesControls,
    search: El,
    t: &Theme,
) -> El {
    // Back and Forward go through the route machine.
    let (back, forward) = if ui.first_place {
        (false, false)
    } else {
        (s.shell.app.can_go_back(), s.shell.app.can_go_forward())
    };
    let dimmed = how == FilesControls::Dimmed;
    tb::bar(
        tb::Bar {
            name: name.into(),
            back: (
                back,
                tb::press(|s, _| {
                    host::dispatch(s, Event::Back);
                }),
            ),
            forward: (
                forward,
                tb::press(|s, _| {
                    host::dispatch(s, Event::Forward);
                }),
            ),
            title: tb::title_block(name, (!sub.is_empty()).then_some(sub.as_str()), t),
            slot: Some(row().key("toolbar-slot").center_y().gap(space::S8).children(slot)),
            slot_sep: false,
            // The board draws these and the section has nothing for them: there to be seen,
            // disabled to assistive technology, no focus (`Bar::inert`).
            view: if dimmed { tb::View::NotHere } else { tb::View::List },
            on_view: None,
            sort: tb::Sort::Idle,
            on_sort: None,
            details: if dimmed {
                tb::Details::NotHere
            } else {
                tb::Details::Open
            },
            on_details: None,
            inert: true,
            gate_closed: false,
            // These sections show no folder: the Beebeeb location.
            on_explorer: Box::new(tb::open_beebeeb_location),
            new: tb::New::Idle,
            on_new: tb::press(|s, _| s.this_pc.ask(super::Intent::New)),
            search,
            trailing_inset: 0.0,
        },
        t,
    )
}

/// The status line (`.mk-foot`): the sync state at the left, the section's words at the right. The state is "Sync
/// stopped" while an item of this PC did not sync, else the word the window's own source has for sync (the one
/// Files' status line says); a source with none gets no dot and no word, never "Up to date" nobody read.
pub fn status_line(s: &State, ui: &Ui, right: String, t: &Theme) -> El {
    use crate::ui::status_bar::{Dot, SYNC_STOPPED, bar, dot_for};
    let stopped = ui.problem_count() > 0;
    let state = if stopped {
        Some(SYNC_STOPPED.to_string())
    } else {
        s.actions.source.sync_state()
    };
    let dot = state.as_deref().map(|w| if stopped { Dot::Red } else { dot_for(w) });
    bar(dot, state.as_deref(), &right, t)
}
