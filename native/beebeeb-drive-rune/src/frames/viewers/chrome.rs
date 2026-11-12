//! What every open file shares with the Files window (`.mk-win`): the caption, the sidebar, the
//! toolbar with the file's path where the folder's title was, and the status line.
//!
//! The toolbar's right half (the dimmed view switch, Sort, the details-panel toggle, New, the
//! search field) is `ui::toolbar`'s, drawn again here because that file builds it in one private
//! function (a request to the design-system lane in the PR body: make the halves public).

use rune::{Color, Cx, El, col, el, row, text_input};

use super::source::OpenFile;
use super::{Ui, icons, style};
use crate::State;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::toolbar::{ToolState, kbd, segment, segmented, tool_button};
use crate::ui::{caption, kit, sidebar};

/// The status line's dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    /// Decrypted, saved, up to date.
    Green,
    /// Not done and not broken: decrypting, unsaved, no room.
    Warn,
    Red,
}

/// The status line of one frame: the dot, its words, and the words at the right.
pub struct Status {
    pub dot: Dot,
    pub text: String,
    pub details: String,
}

const SEARCH_PLACEHOLDER: &str = "Search names";

/// What the toolbar says about the file beyond its path.
#[derive(Default)]
pub struct Head {
    /// The line under the path, where it is not the file's own facts ("Changes not saved yet").
    pub facts: Option<String>,
    /// A dot after the name: the edit is not saved.
    pub unsaved: bool,
}

/// The path and the line under it (`.mk-title` with `.mk-crumbs`).
fn title(file: &OpenFile, head: &Head, t: &Theme) -> El {
    let facts = head.facts.clone().unwrap_or_else(|| file.facts.clone());
    let mut crumbs = row().key("crumbs").w_full().min_w(0.0).center_y().gap(4.0);
    for (i, folder) in file.path.iter().enumerate() {
        crumbs = crumbs
            .child(style::fitted(
                format!("crumb-{i}"),
                folder,
                style::CRUMB,
                t.text_tertiary,
            ))
            // When the toolbar is full the separators give way first, as the board's do.
            .child(kit::glyph(format!("crumb-sep-{i}"), &icons::CHEVRON_RIGHT, 12.0, t.text_tertiary).shrink());
    }
    crumbs = crumbs.child(style::fitted(
        "crumb-name",
        &file.name,
        style::CRUMB_NAME,
        t.text_tertiary,
    ));
    if head.unsaved {
        crumbs = crumbs.child(
            el().key("crumb-dot")
                .role("image")
                .label("Not saved")
                .size(7.0, 7.0)
                .radius(3.5)
                .ml(2.0)
                .fill(t.text)
                // As the status dot: a little of the ground inside the edge, for Chromium's
                // lighter outermost rows ("Rune limitations", PR body).
                .box_shadow(rune::Shadow {
                    x: 0.0,
                    y: 0.0,
                    blur: 0.0,
                    spread: 0.05,
                    color: t.toolbar_bg,
                    inset: true,
                }),
        );
    }
    col()
        .key("file-info")
        .role("group")
        .label(format!("{}, {}", file.name, facts))
        .px(4.0)
        .shrink()
        .child(crumbs)
        .child(kit::label("item-count", facts, type_::TITLE_SUB, t.text_tertiary).w_full())
}

/// The toolbar of an open file. `tools` are the file's own controls, drawn before the "more" button.
pub fn toolbar(s: &State, ui: &Ui, head: &Head, tools: Vec<El>, t: &Theme) -> El {
    let nav = row()
        .key("nav")
        .child(
            tool_button("back", "Back", &icons::CHEVRON_LEFT, 16.0, ToolState::Idle, t)
                .shortcut_hint("Alt+Left")
                .on_press(super::close),
        )
        .child(
            tool_button("forward", "Forward", &icons::CHEVRON_RIGHT, 16.0, ToolState::Off, t)
                .shortcut_hint("Alt+Right"),
        );

    let more = tool_button("more-btn", "More actions", &icons::MORE, 17.0, ToolState::Idle, t);
    // The file's own controls and "more" sit 6 apart (`.flex.items-center.gap-1.5`).
    let file_tools = row().key("file-tools").center_y().gap(6.0).children(tools).child(more);

    // The folder's view switch does nothing for an open file: drawn, dimmed, not operable.
    let view_toggle = el()
        .key("view-toggle-dim")
        .radius(9.0)
        .fill(t.field)
        .opacity(0.45)
        .child(segmented(
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
        ));

    let sort_btn = tool_button("sort-btn", "Sort", &icons::SORT, 15.0, ToolState::Off, t).child(kit::glyph(
        "sort-caret",
        &icons::CHEVRON_DOWN,
        11.0,
        t.text_quaternary,
    ));

    let panel_btn = tool_button(
        "panel-toggle-btn",
        "Details panel",
        &icons::PANEL_TOGGLE,
        16.0,
        if ui.show_details {
            ToolState::On
        } else {
            ToolState::Idle
        },
        t,
    )
    .shortcut_hint("Alt+Shift+P")
    .checked(ui.show_details)
    .on_press(|s: &mut State| {
        if let Some(ui) = super::ui(s) {
            ui.toggle_details();
        }
    });

    let sep = el().key("toolbar-sep").w(kit::HAIR).h(18.0).mx(2.0).fill(t.separator);

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
        .on_press(|s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.intent(super::Intent::NewMenu);
            }
        })
        .child(kit::glyph("plus-icon", &icons::PLUS, 14.0, t.accent_text_on))
        .child(style::fitted("new-label", "New", type_::LABEL, t.accent_text_on))
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
        .label(ui.file.name.clone())
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
        .child(title(&ui.file, head, t))
        .child(el().key("toolbar-spacer").grow(1.0))
        .child(file_tools)
        .child(view_toggle)
        .child(sort_btn)
        .child(panel_btn)
        .child(sep)
        .child(new_btn)
        .child(search_field)
}

/// The status line (`.mk-foot`) with this frame's words.
pub fn status_bar(status: &Status, t: &Theme) -> El {
    let dot = match status.dot {
        Dot::Green => t.green,
        Dot::Warn => style::warn(t),
        Dot::Red => style::red(t),
    };
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
        .child(
            row()
                .key("status-left")
                .center_y()
                .gap(7.0)
                // A tenth of a device pixel of the bar's own colour inside the dot's edge: Chromium's
                // antialiased 14-pixel disc covers less of its outermost pixel rows than Rune's
                // (measured 91 % against 98 %), which shows on a dot this dark. A size or a scale
                // cannot say it: Rune snaps the box to whole pixels ("Rune limitations", PR body).
                .child(
                    el().key("status-dot")
                        .size(7.0, 7.0)
                        .radius(3.5)
                        .fill(dot)
                        .box_shadow(rune::Shadow {
                            x: 0.0,
                            y: 0.0,
                            blur: 0.0,
                            spread: 0.05,
                            color: t.status_bar_bg,
                            inset: true,
                        }),
                )
                .child(kit::label(
                    "status-text",
                    status.text.clone(),
                    type_::STATUS_STRONG,
                    t.text_secondary,
                )),
        )
        .child(el().key("status-spacer").grow(1.0))
        .child(kit::label(
            "status-details",
            status.details.clone(),
            type_::STATUS,
            t.text_tertiary,
        ))
}

/// The window with an open file in its content area: `body` between the toolbar and the status line.
pub fn window(
    s: &State,
    cx: &mut Cx<State>,
    ui: &Ui,
    head: Head,
    tools: Vec<El>,
    banner: Option<El>,
    body: El,
    status: &Status,
    t: &Theme,
) -> El {
    let mut main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar(s, ui, &head, tools, t));
    if let Some(banner) = banner {
        main = main.child(banner);
    }
    let main = main
        .child({
            let mut content = row().key("viewer-row").grow(1.0).min_h(0.0).w_full().child(body);
            if ui.show_details {
                content = content.child(super::details::panel(&ui.file, t));
            }
            content
        })
        .child(status_bar(status, t));

    col()
        .key("window")
        .role("window")
        .label("Beebeeb Drive")
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(t, cx))
        .child({
            // Under the caption: the window's body, and what sits over it.
            let mut under = el().key("under-caption").grow(1.0).min_h(0.0).w_full().child(
                row()
                    .key("body")
                    .w_full()
                    .h_full()
                    .child(sidebar::sidebar(s, t))
                    .child(main),
            );
            if let Some(words) = &ui.toast {
                under = under.child(super::dialogs::toast(words, t));
            }
            match &ui.dialog {
                Some(super::Dialog::Unsaved) => under = under.child(super::dialogs::unsaved(&ui.file.name, t)),
                Some(super::Dialog::Conflict(other)) => {
                    let mine = ui.editor.as_ref().map(super::Editor::change_words).unwrap_or_default();
                    under = under.child(super::dialogs::conflict(&ui.file.name, &mine, other, t));
                }
                None => {}
            }
            under
        })
}

/// How a button of the kit shows (`.kit-btn`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// Paper with a line: the choice the screen expects.
    Outline,
    /// Words only.
    Ghost,
}

/// A button as wide as its words. `small` is `.kit-btn-sm` (22 high), else `.kit-btn-md` (28).
/// Keys: `<key>`, `<key>-label`. The caller adds `.on_press(..)`.
pub fn button(key: &str, label: &str, kind: ButtonKind, small: bool, t: &Theme) -> El {
    let (ink, text_style, h) = (
        if kind == ButtonKind::Outline {
            t.text
        } else {
            t.text_secondary
        },
        if small { style::BUTTON_SM } else { style::BUTTON_MD },
        if small { 22.0 } else { 28.0 },
    );
    let mut b = row()
        .key(key.to_string())
        .role("button")
        .label(label)
        .h(h)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(6.0)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent_deep)
        .cursor(rune::Cursor::Pointer);
    b = match kind {
        ButtonKind::Outline => b
            .fill(t.paper)
            .border(1.0, t.card_border)
            .border_layout()
            .hover(t.paper2)
            .press(t.paper3),
        ButtonKind::Ghost => b.fill(Color::TRANSPARENT).hover(t.hover).press(t.press),
    };
    b.child(kit::label(format!("{key}-label"), label, text_style, ink))
}
