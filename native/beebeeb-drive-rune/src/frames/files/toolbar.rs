//! The Files toolbar (`.mk-tb`): the one window toolbar (`ui::toolbar::bar`), with what the Files frames add:
//! Sort and New open their menus (and show pressed, ringed while open), the title follows the
//! selection, tooltips name only shortcuts that exist.

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, type_};
use crate::ui::kit;
use crate::ui::toolbar::{self as tb, Bar, Details, New, Sort, View, press};
use rune::{El, TextStyle, col, row, text_input};

use super::{Intent, MenuKind};

/// The search field's placeholder: one string for the field and for the gate's note.
const SEARCH_PLACEHOLDER: &str = "Search names";

/// `.mk-crumbs i`: a folder above this one.
const CRUMB: TextStyle = TextStyle::new(11.0, 500.0).lh(13.0).tracking(-0.11);
/// `.mk-crumbs > span`: the folder shown.
const CRUMB_HERE: TextStyle = TextStyle::new(11.0, 650.0).lh(13.0).tracking(-0.11);

pub fn toolbar(s: &State, t: &Theme, trailing_inset: f32) -> El {
    let item_count = s.listing.items.len();

    let count = match item_count {
        0 => "No items".to_string(),
        1 => "1 item".to_string(),
        n => format!("{n} items"),
    };
    let mut title = col().key("folder-info").px(4.0).min_w(0.0);
    title = if s.listing.path.len() > 1 {
        // Inside a folder the title is the path (`.mk-crumbs`): each folder above is a way up.
        let mut crumbs = row()
            .key("crumbs")
            .role("navigation")
            .label("Path")
            .center_y()
            .gap(4.0)
            .min_w(0.0);
        let last = s.listing.path.len() - 1;
        for (i, crumb) in s.listing.path.iter().enumerate() {
            if i < last {
                crumbs = crumbs
                    .child(
                        kit::label(format!("crumb-{i}"), crumb.name.clone(), CRUMB, t.text_tertiary)
                            .role("link")
                            .focusable(true)
                            .on_press(move |s: &mut State| s.open_crumb(i)),
                    )
                    .child(kit::glyph(
                        format!("crumb-sep-{i}"),
                        &icons::CHEVRON_RIGHT,
                        12.0,
                        t.text_tertiary,
                    ));
            } else {
                crumbs = crumbs.child(
                    kit::label("crumb-here", crumb.name.clone(), CRUMB_HERE, t.text_tertiary)
                        .truncate()
                        .min_w(0.0),
                );
            }
        }
        title.child(crumbs)
    } else {
        title.child(kit::label("folder-title", s.listing.folder.name.clone(), type_::TITLE, t.text).truncate())
    }
    .child(kit::label("item-count", count, type_::TITLE_SUB, t.text_tertiary));

    let grid = crate::frames::global::grid::on(s);
    // Pressed while its menu is open (DW-0404).
    let sorting = s.actions.menu.is_some_and(|m| m.kind == MenuKind::Sort);
    // Ringed while its menu is open (DW-0414).
    let adding = s.actions.menu.is_some_and(|m| m.kind == MenuKind::New);

    let search = tb::search_box(
        "search-input",
        SEARCH_PLACEHOLDER,
        text_input(&s.search, type_::SEARCH)
            .key("search-input")
            .label("Search names")
            .shortcut_hint("Ctrl+F")
            .single_line()
            .color(t.text)
            .placeholder(SEARCH_PLACEHOLDER, t.text_tertiary)
            .on_edit(|s: &mut State| &mut s.search)
            .grow(1.0),
        t,
    );

    tb::bar(
        Bar {
            name: "Files".into(),
            back: (
                s.can_go_back(),
                press(|s, _| {
                    s.back();
                }),
            ),
            forward: (
                s.can_go_forward(),
                press(|s, _| {
                    s.forward();
                }),
            ),
            title,
            slot: None,
            slot_sep: false,
            view: if grid { View::Grid } else { View::List },
            // Each folder remembers its own view (`frames::global::grid`, DW-0119). The grid with
            // the details panel on a photo is another frame (DW-1102), not built here.
            on_view: Some(Box::new(|s, tx, grid| {
                if grid {
                    s.intent(Intent::Frame("DW-1102"));
                }
                crate::frames::global::grid::switch(s, grid, tx);
            })),
            sort: if sorting { Sort::Open } else { Sort::Idle },
            on_sort: press(|s, _| s.toggle_menu(MenuKind::Sort)),
            details: if s.show_detail_pane {
                Details::Open
            } else {
                Details::Closed
            },
            on_details: press(|s, _| s.show_detail_pane = !s.show_detail_pane),
            inert: false,
            // The folder shown, in File Explorer.
            on_explorer: Box::new(|s, tx| {
                s.open_in_file_explorer(tx);
            }),
            // Dimmed where nothing can be added (`.mk-new.is-off`): Backups, a service folder, Trash, Shared with me.
            new: if !s.can_add() {
                New::Off
            } else if adding {
                New::Open
            } else {
                New::Idle
            },
            on_new: press(|s, _| s.toggle_menu(MenuKind::New)),
            search,
            trailing_inset,
        },
        t,
    )
}
