//! The Files toolbar (`.mk-tb`): the design system's toolbar pieces, with what the Files frames add:
//! Sort and New open their menus (and show pressed, ringed while open), the title follows the
//! selection, tooltips name only shortcuts that exist.

use crate::State;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::kit;
use crate::ui::toolbar::{ToolState, kbd, segment, segmented, tool_button};
use rune::{El, TextStyle, col, el, row, text_input};

use super::{Intent, MenuKind};

/// The search field's placeholder: one string for the field and for the gate's note.
const SEARCH_PLACEHOLDER: &str = "Search names";

/// `.mk-crumbs i`: a folder above this one.
const CRUMB: TextStyle = TextStyle::new(11.0, 500.0).lh(13.0).tracking(-0.11);
/// `.mk-crumbs > span`: the folder shown.
const CRUMB_HERE: TextStyle = TextStyle::new(11.0, 650.0).lh(13.0).tracking(-0.11);

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
                s.back();
            }),
        )
        .child(
            tool_button(
                "forward",
                "Forward",
                &icons::CHEVRON_RIGHT,
                16.0,
                if s.can_go_forward() {
                    ToolState::Idle
                } else {
                    ToolState::Off
                },
                t,
            )
            .shortcut_hint("Alt+Right")
            .on_press(|s: &mut State| {
                s.forward();
            }),
        );

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
    let view_toggle = segmented(
        "view-toggle",
        "View",
        vec![
            // Each folder remembers its own view (`frames::global::grid`, DW-0119).
            segment("view-list", "as List", &icons::VIEW_LIST, !grid, t)
                .on_press_tx(|s: &mut State, tx: &mut rune::Tx| crate::frames::global::grid::switch(s, false, tx)),
            // The grid with the details panel on a photo is another frame (DW-1102), not built here.
            segment("view-grid", "as Grid", &icons::VIEW_GRID, grid, t).on_press_tx(
                |s: &mut State, tx: &mut rune::Tx| {
                    s.intent(Intent::Frame("DW-1102"));
                    crate::frames::global::grid::switch(s, true, tx);
                },
            ),
        ],
        t,
    );

    // Pressed while its menu is open (DW-0404).
    let sorting = s.actions.menu.is_some_and(|m| m.kind == MenuKind::Sort);
    let sort_btn = tool_button(
        "sort-btn",
        "Sort",
        &icons::SORT,
        15.0,
        if sorting { ToolState::On } else { ToolState::Idle },
        t,
    )
    .expanded(sorting)
    .on_press(|s: &mut State| s.toggle_menu(MenuKind::Sort))
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

    // Open in File Explorer: the folder shown, in Explorer. The last icon button of the toolbar;
    // not on the board yet (PR 305, Decision: owner request 2026-11-12).
    let explorer_btn = tool_button(
        "open-explorer",
        "Open in File Explorer",
        &crate::theme::icons_settings::FOLDER,
        16.0,
        ToolState::Idle,
        t,
    )
    .help("Open in File Explorer")
    .on_press_tx(|s: &mut State, tx: &mut rune::Tx| {
        s.open_in_file_explorer(tx);
    });

    let sep = el().key("toolbar-sep").w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator);

    // Ringed while its menu is open (DW-0414: `box-shadow: 0 0 0 3px` amber at 32 %).
    let adding = s.actions.menu.is_some_and(|m| m.kind == MenuKind::New);
    // Dimmed where nothing can be added (`.mk-new.is-off`): Backups, a service folder, Trash, Shared with me.
    let can_add = s.can_add();
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
        .child(kit::glyph("plus-icon", &icons::PLUS, 14.0, t.accent_text_on))
        .child(kit::label("new-label", "New", type_::LABEL, t.accent_text_on))
        .child(kit::glyph("new-caret", &icons::CHEVRON_DOWN, 11.0, t.accent_text_on));
    let new_btn = if can_add {
        new_btn
            .hover(t.press)
            .press(t.press)
            .focusable(true)
            .on_press(|s: &mut State| s.toggle_menu(MenuKind::New))
    } else {
        new_btn.opacity(0.4).disabled(true)
    };

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
        .child(explorer_btn)
        .child(sep)
        .child(new_btn)
        .child(search_field)
}
