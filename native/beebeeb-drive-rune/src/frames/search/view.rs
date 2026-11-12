//! The Files window while searching (DW-0801 to DW-0803): the Files sidebar and status line, the
//! toolbar with "Search" for a title and the field focused, and in the content area the recent
//! searches, the results with Where (and the selected one's details), the note of a first
//! sign-in, or the empty state.
//!
//! Keys are the board's (`scripts/keymap.js`): the Files window's, and for what the Files lane
//! never drew here the grouped list (`label-recent`, `list-0`, `item-0-<m>`), the pills (`seg-0`)
//! and the empty state (`empty-*`). The board's export also names the details panel's Available
//! offline card and its two buttons a second time (`list-0`, `item-0-0`, `btn-share`,
//! `btn-version-history`): the same elements, so the panel names them twice as well (see
//! [`details_pane`]).

use rune::{Align, Color, Cx, El, Shadow, Span, Tx, col, el, row, text_input};

use super::source::{Hit, Loading, Narrow, matched};
use super::{Shows, close, open_result, run};
use crate::State;
use crate::frames::Intent;
use crate::frames::files::source::Details;
use crate::frames::shared::parts::controls::segments;
use crate::frames::shared::parts::empty::empty;
use crate::frames::shared::parts::style;
use crate::frames::shared::parts::table::tile;
use crate::theme::tokens::{self, Theme, radius, type_};
use crate::theme::{icons, icons_settings};
use crate::ui::detail_pane::{action_button, encrypted_badge, meta_row, page_preview, pane, preview, switch};
use crate::ui::field::{advance, caret_mark, snap};
use crate::ui::file_list::{self, pc_glyph};
use crate::ui::keys::SEARCH_KEY;
use crate::ui::settings_controls::BANNER_TEXT;
use crate::ui::toolbar::{ToolState, kbd, segment, segmented, tool_button};
use crate::ui::{caption, global_icons, kit, status_line};

/// The field's placeholder: one string for the field and for the gate's note.
const PLACEHOLDER: &str = "Search names";
/// Under the recent searches (DW-0801).
const COVERS: &str = "Searches file and folder names on this PC, even offline. It doesn't look inside files.";
/// Beside the pills while there are results (DW-0802).
const SCOPE: &str = "Names only \u{00B7} Trash isn't searched";
/// DW-0803's sentence.
const NO_MATCH: &str = "Check the spelling, or try fewer letters. Search only looks at names, not inside files.";
/// A window whose search is not wired yet: the honest state, never the board's sample.
const UNAVAILABLE_TITLE: &str = "Search isn't available yet";
const UNAVAILABLE_TEXT: &str = "This version can't search your files yet. Find them by browsing Files.";

/// The key prefix of a recent search (`.kit-list` > `.kit-item`, the board's grouped list).
pub const RECENT_ITEM: &str = "item-0-";

/// The key of recent search `m`.
pub fn recent_key(m: usize) -> String {
    format!("{RECENT_ITEM}{m}")
}

// Columns (`SEARCH_COLS`: "minmax(0,1fr) 132px 84px 170px 34px", 12 pt between).
const GAP: f32 = 12.0;
const DATE_W: f32 = 132.0;
const SIZE_W: f32 = 84.0;
const WHERE_W: f32 = 170.0;
const PC_W: f32 = 34.0;

/// What the field's box holds before the text (`.mk-search`: 9 pt, the 13 pt glyph, 6 pt).
const FIELD_TEXT_X: f32 = 9.0 + 13.0 + 6.0;
/// The board's caret (`.kit-caret`, 14 pt here) stands 7.25 pt under the field's top edge: on
/// whole device pixels of a 200 % display, its foot 3 pt under the text's baseline.
const CARET_TOP: f32 = 7.25;
const CARET_H: f32 = 14.0;

fn push(intent: Intent) -> impl Fn(&mut State) + 'static {
    move |s: &mut State| s.frames.intents.push(intent.clone())
}

pub fn window(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    let ui = &s.frames.find;
    // Rune reports the caret of the field that has the keyboard.
    let focused = cx.caret(SEARCH_KEY).is_some();
    let shows = ui.shows(&s.search.text);
    let content = match &shows {
        Shows::Unavailable => unavailable(&t),
        Shows::Recent(recent) => recent_searches(recent, &t),
        Shows::Results { hits, loading } => results(s, hits, *loading, &t),
        Shows::Nothing { loading } => nothing(s, ui.narrow, *loading, &t),
    };
    let main = col()
        .key("content-area")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.bg)
        .child(toolbar(s, &shows.count_words(), focused, &t))
        // The one banner of the whole window (`frames::global`: "on every view of the window").
        .children(crate::frames::global::banner_under_toolbar(s, &t))
        .child(content.grow(1.0).min_h(0.0).w_full())
        .child(status(s, &t));
    let body = row()
        .key("body")
        .w_full()
        .child(crate::frames::files::sidebar::sidebar(s, &t))
        .child(main);
    col()
        .key("window")
        .role("window")
        .label(s.identity.main_window_title())
        .on_key(super::on_key)
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption::caption_bar(s.identity.main_window_title(), &t, cx))
        .child(crate::frames::global::fit(s, body, &t))
}

/// The status line: the PC's sync state and the Files window's summary, as in Files (DW-0401).
fn status(s: &State, t: &Theme) -> El {
    let state = s.actions.source.sync_state();
    let details = crate::frames::global::status_words(s).unwrap_or_else(|| crate::frames::files::window::pc_summary(s));
    let name = match &state {
        Some(state) => format!("{state}. {details}"),
        None => details.clone(),
    };
    crate::frames::global::status_line(state.as_deref(), &details, t)
        .unwrap_or_else(|| status_line::status_line(state.as_deref(), &details, t))
        .role("button")
        .label(name)
        .description("Shows sync status")
        .focusable(true)
        .on_press(push(Intent::Frame("DW-1201")))
}

/// The toolbar (`.mk-tb`) while searching: "Search" and what was found, the Files window's
/// controls as the board draws them here, and the field focused.
fn toolbar(s: &State, sub: &str, focused: bool, t: &Theme) -> El {
    // The board draws Back and Forward off while searching: Esc is the way back.
    let nav = row()
        .key("nav")
        .child(tool_button("back", "Back", &icons::CHEVRON_LEFT, 16.0, ToolState::Off, t).shortcut_hint("Alt+Left"))
        .child(
            tool_button("forward", "Forward", &icons::CHEVRON_RIGHT, 16.0, ToolState::Off, t)
                .shortcut_hint("Alt+Right"),
        );
    let title = col()
        .key("folder-info")
        .px(4.0)
        .min_w(0.0)
        .child(kit::label("folder-title", "Search", type_::TITLE, t.text).truncate())
        .child(kit::label("item-count", sub, type_::TITLE_SUB, t.text_tertiary));
    let view_toggle = segmented(
        "view-toggle",
        "View",
        vec![
            segment("view-list", "as List", &icons::VIEW_LIST, true, t),
            segment("view-grid", "as Grid", &icons::VIEW_GRID, false, t).on_press(push(Intent::Frame("DW-1102"))),
        ],
        t,
    );
    let sort = tool_button("sort-btn", "Sort", &icons::SORT, 15.0, ToolState::Idle, t)
        .child(kit::glyph("sort-caret", &icons::CHEVRON_DOWN, 11.0, t.text_secondary))
        .on_press(push(Intent::Frame("DW-0406")));
    let panel = tool_button(
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
    // "Open in File Explorer", between the panel toggle and the separator (the Files toolbar's,
    // DW-0401): the b's menu item (`host::on_menu`).
    let explorer = tool_button(
        "open-explorer",
        "Open in File Explorer",
        &icons::FILES,
        16.0,
        ToolState::Idle,
        t,
    )
    .on_press_tx(|s: &mut State, tx: &mut Tx| {
        crate::shell::host::on_menu(s, crate::shell::tray::MenuAction::OpenInFileExplorer, tx);
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
        .on_press(push(Intent::Frame("DW-0415")))
        .child(kit::glyph("plus-icon", &icons::PLUS, 14.0, t.accent_text_on))
        .child(kit::label("new-label", "New", type_::LABEL, t.accent_text_on))
        .child(kit::glyph("new-caret", &icons::CHEVRON_DOWN, 11.0, t.accent_text_on));
    row()
        .key("toolbar")
        .role("toolbar")
        .label("Search")
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
        .child(view_toggle)
        .child(sort)
        .child(panel)
        .child(explorer)
        .child(sep)
        .child(new_btn)
        .child(search_field(s, focused, t))
}

/// The search field (`.mk-search.is-focus`): lit while it has the keyboard or holds words, the
/// hint giving way to × (which ends search) once something is typed. It edits the window's
/// search field (`State::search`), the one the Files toolbar shows.
fn search_field(s: &State, focused: bool, t: &Theme) -> El {
    let ui = &s.frames.find;
    let words = s.search.text.as_str();
    let has_words = !words.is_empty();
    // A board frame's focused field: Rune draws its own caret only where a window has focus.
    let pictured = ui.pictured && !focused;
    let lit = focused || pictured || has_words;
    // `.is-focus` sets the field's words in ink, the placeholder too.
    let ink = if lit { t.text } else { t.text_tertiary };
    let shown = if has_words { words } else { PLACEHOLDER };
    kit::note_text("search-placeholder", SEARCH_KEY, shown, type_::SEARCH, ink);
    let mut field = row()
        .key("search-box")
        .role("search")
        .w(210.0)
        .h(28.0)
        .gap(6.0)
        .pl(9.0)
        .pr(6.0)
        .radius(8.0)
        .center_y()
        .child(kit::glyph("search-icon", &icons::SEARCH, 13.0, ink))
        .child(
            text_input(&s.search, type_::SEARCH)
                .key(SEARCH_KEY)
                .label(PLACEHOLDER)
                .shortcut_hint("Ctrl+F")
                .single_line()
                .color(t.text)
                .caret(t.accent_deep, t.text.alpha(0.15))
                .placeholder(PLACEHOLDER, ink)
                .on_edit(|s: &mut State| &mut s.search)
                .on_input_key(super::field_key)
                .grow(1.0),
        );
    field = if lit {
        field
            .fill(t.card)
            .box_shadow(kit::ring(3.0, t.accent.alpha(0.3)))
            .box_shadow(Shadow {
                spread: 1.0,
                ..kit::ring_inset(t.accent_deep)
            })
    } else {
        field.fill(t.field).box_shadow(kit::ring_inset(t.separator))
    };
    field = if has_words {
        field.child(
            el().key("search-clear-btn")
                .role("button")
                .label("Clear search")
                .size(12.0, 12.0)
                .focusable(true)
                .tab_index(-1)
                .on_press_tx(close)
                .child(kit::glyph("search-clear", &global_icons::CLOSE, 12.0, t.text_tertiary)),
        )
    } else {
        field.child(kbd("kbd-hint", "Ctrl+F", t))
    };
    if pictured {
        // `.kit-caret` after the words: 1 pt of margin, on the device pixel the board puts it on.
        let x = snap(FIELD_TEXT_X + advance(shown, type_::SEARCH) + 1.0);
        field = field.child(
            el().key("search-caret-at")
                .a11y_hidden()
                .absolute(x, CARET_TOP)
                .child(caret_mark("search-caret".into(), CARET_H, t)),
        );
    }
    field
}

/// A window whose search is not wired yet: it says so, and shows nothing of the board's sample.
fn unavailable(t: &Theme) -> El {
    col().key("search-unavailable").w_full().child(
        empty(&icons::SEARCH, UNAVAILABLE_TITLE, UNAVAILABLE_TEXT, None, t)
            .grow(1.0)
            .w_full(),
    )
}

/// The recent searches (DW-0801): a label with Clear, the list, and what search covers.
fn recent_searches(recent: &[String], t: &Theme) -> El {
    let mut panel = col()
        .key("recent")
        .grow(1.0)
        .max_w(560.0)
        .h_full()
        .padding(24.0)
        .gap(12.0);
    if !recent.is_empty() {
        let head = row()
            .key("recent-head")
            .w_full()
            .center_y()
            .child(kit::label(
                "label-recent",
                "RECENT",
                type_::SECTION_LABEL,
                t.text_tertiary,
            ))
            .child(el().key("recent-head-space").grow(1.0))
            .child(clear_button(t));
        let items: Vec<El> = recent.iter().enumerate().map(|(m, q)| recent_item(m, q, t)).collect();
        panel = panel.child(head).child(grouped_list("Recent searches", items, t));
    }
    // `p.t-cap`: one sentence under the list.
    let note = kit::label("search-covers", COVERS, type_::CAPTION, t.text_tertiary)
        .wrap()
        .w_full();
    row().key("search-body").w_full().child(panel.child(note))
}

/// Clear (`.t-cap.underline.underline-offset-2`): the recent searches go.
fn clear_button(t: &Theme) -> El {
    let style = type_::CAPTION;
    el().key("recent-clear")
        .role("button")
        .label("Clear recent searches")
        .focusable(true)
        .on_press(|s: &mut State| s.frames.find.source.clear_recent())
        .child(kit::label("recent-clear-text", "Clear", style, t.text_tertiary))
        // The underline 2 pt under the baseline, 1 pt thick, as the board draws it (Rune's own
        // underline has no offset: Rune limitation 3).
        .child(
            el().key("recent-clear-line")
                .a11y_hidden()
                .absolute(0.0, 13.0)
                .w(advance("Clear", style))
                .h(1.0)
                .fill(t.text_tertiary),
        )
}

/// A grouped list (`.kit-list-body`): a card of rows with a hairline between them, from the
/// row's 10 pt inset on. Key: `list-0`.
fn grouped_list(name: &str, items: Vec<El>, t: &Theme) -> El {
    let inner = col()
        .key("list-0-items")
        .w_full()
        .radius(radius::LG)
        .clip_rounded(radius::LG)
        .children(items.into_iter().enumerate().flat_map(|(m, item)| {
            let rule = (m > 0).then(|| {
                el().key(format!("list-0-rule-{m}"))
                    .h(kit::HAIR)
                    .ml(10.0)
                    .mb(-kit::HAIR)
                    .fill(t.separator)
            });
            rule.into_iter().chain(std::iter::once(item))
        }));
    col()
        .key("list-0")
        .role("list")
        .label(name)
        .w_full()
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(inner)
}

/// One recent search (`.kit-item` with a clock): pressing it runs it.
fn recent_item(m: usize, query: &str, t: &Theme) -> El {
    let k = recent_key(m);
    let q = query.to_string();
    row()
        .key(k.clone())
        .role("button")
        .label(query)
        .description("Recent search")
        .h(28.0)
        .w_full()
        .px(10.0)
        .gap(8.0)
        .center_y()
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        // One Tab stop for the list: the arrows move inside it.
        .tab_index(if m == 0 { 0 } else { -1 })
        .on_press_tx(move |s: &mut State, tx: &mut Tx| run(s, &q, tx))
        .child(el().key(format!("{k}-icon-box")).w(30.0).center().child(kit::glyph(
            format!("{k}-icon"),
            &icons_settings::CLOCK,
            18.0,
            t.text_secondary,
        )))
        .child(
            kit::label(format!("{k}-title"), query, type_::ITEM_TITLE, t.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
}

/// The pills (`.kit-seg.kit-seg-sm`) and, with results, what search covers at the right.
fn narrow_row(narrow: Narrow, scope: bool, t: &Theme) -> El {
    let tabs: Vec<(&'static str, bool)> = Narrow::ALL.iter().map(|n| (n.words(), *n == narrow)).collect();
    let pills = segments(0, "Narrow the results", &tabs, t, |i, tab| {
        let to = Narrow::ALL[i];
        // `.kit-seg-sm span { padding: 0 10px }` around the words at their own width: Rune lays a
        // text out a little wider, and four tabs in a row would add that up.
        tab.w(advance(to.words(), style::SEG) + 20.0)
            .on_press(move |s: &mut State| s.frames.find.narrow_to(to))
    });
    let mut r = row()
        .key("narrow-row")
        .w_full()
        .px(20.0)
        .pt(12.0)
        .gap(12.0)
        .center_y()
        .child(pills);
    if scope {
        r = r.child(el().key("narrow-space").grow(1.0)).child(kit::label(
            "search-scope",
            SCOPE,
            type_::CAPTION,
            t.text_tertiary,
        ));
    }
    r
}

/// Digits in groups of three: "18,240".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The note of a first sign-in (`.kit-banner.tone-neutral`, DW-0802b): some results may be
/// missing until the list has loaded, with the live count in figures.
fn loading_note(l: Loading, t: &Theme) -> El {
    let figure = format!("{} of {}", grouped(l.listed), grouped(l.total));
    let words =
        format!("Still getting your files ready. Results may be incomplete until the list has loaded: {figure}.");
    let from = words.len() - figure.len() - 1;
    el().key("loading-wrap").w_full().px(20.0).pt(8.0).child(
        row()
            .key("loading-note")
            .role("status")
            .label(words.clone())
            .w_full()
            .items_start()
            .gap(10.0)
            .py(11.0)
            .px(13.0)
            .radius(13.0)
            .fill(t.tile)
            .child(el().key("loading-icon-box").pt(1.0).child(kit::glyph(
                "loading-icon",
                &icons_settings::INFO,
                16.0,
                t.text_secondary,
            )))
            .child(
                kit::label("loading-text", words, BANNER_TEXT, t.text_secondary)
                    .spans(vec![Span::new(from, from + figure.len()).mono()])
                    .grow(1.0)
                    .min_w(0.0)
                    .wrap(),
            ),
    )
}

/// The column headers (`.mk-thead`): Name, Date changed, Size, Where, PC. Results come in the
/// source's order: no header carries a sort mark.
fn header(t: &Theme) -> El {
    let head = |key: &'static str, words: &'static str| kit::label(key, words, type_::COLUMN_HEADER, t.text_tertiary);
    row()
        .key("column-headers")
        .role("row")
        .h(28.0)
        .w_full()
        .mt(4.0)
        .px(20.0)
        .gap(GAP)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(
            row()
                .key("col-name-cell")
                .role("columnheader")
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .child(head("col-name", "Name")),
        )
        .child(
            row()
                .key("col-date-cell")
                .role("columnheader")
                .w(DATE_W)
                .center_y()
                .child(head("col-date", "Date changed")),
        )
        .child(
            row()
                .key("col-size-cell")
                .role("columnheader")
                .w(SIZE_W)
                .center_y()
                .justify_end()
                .child(head("col-size", "Size")),
        )
        .child(
            row()
                .key("col-shared-cell")
                .role("columnheader")
                .w(WHERE_W)
                .center_y()
                .child(head("col-shared", "Where")),
        )
        .child(
            row()
                .key("col-pc-cell")
                .role("columnheader")
                .label("On this PC")
                .w(PC_W)
                .center()
                .child(head("col-pc", "PC")),
        )
}

/// One result (`.mk-row`): the name with the match in bold, when it changed, its size, where it
/// is, and the PC column. Keys: `row-<i>` and `-tile`, `-icon`, `-name`, `-date`, `-size`,
/// `-dash` (Where), `-pc`.
fn result_row(i: usize, hit: &Hit, query: &str, selected: bool, t: &Theme) -> El {
    let k = format!("row-{i}");
    let meta = if selected { t.text_secondary } else { t.text_tertiary };
    let mut name = kit::label(format!("{k}-name"), hit.name.clone(), type_::LIST_ITEM, t.text)
        .truncate()
        .min_w(0.0);
    if let Some((from, to)) = matched(&hit.name, query) {
        // `mark { font-weight: 700 }`: the letters that match.
        name = name.spans(vec![Span::new(from, to).weight(700.0)]);
    }
    let name_cell = row()
        .key("name-cell")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(8.0)
        .child(tile(&k, format!("{k}-icon"), hit.kind, 22.0, 6.0, 14.0, t))
        .child(name);
    let place = row().key("where-cell").w(WHERE_W).min_w(0.0).center_y().child(
        kit::label(format!("{k}-dash"), hit.place.clone(), style::CELL, meta)
            .truncate()
            .min_w(0.0),
    );
    let mut pc = el().key("pc-cell").w(PC_W).center();
    if let Some(state) = hit.pc {
        pc = pc.child(pc_glyph(format!("{k}-pc"), state, t.bg, t));
    }
    row()
        .key(k.clone())
        .role("listitem")
        .label(&hit.name)
        .description(format!("In {}", hit.place))
        .h(30.0)
        .w_full()
        .px(14.0)
        .gap(GAP)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if selected { t.selection_row } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .focus_selected(selected)
        // The list is one Tab stop: the selected result; the arrows move inside it.
        .tab_index(if selected { 0 } else { -1 })
        .child(name_cell)
        .child(
            kit::label(format!("{k}-date"), hit.changed.clone(), style::CELL_MONO, meta)
                .w(DATE_W)
                .truncate(),
        )
        .child(
            kit::label(format!("{k}-size"), hit.size.clone(), style::CELL_MONO, meta)
                .w(SIZE_W)
                .align(Align::End),
        )
        .child(place)
        .child(pc)
}

/// The results (DW-0802, DW-0802b): the pills, the note of a first sign-in, the list, and the
/// selected result's details. While the list is still loading the board draws no details panel
/// (DW-0802b): the panel waits until the list has loaded.
fn results(s: &State, hits: &[Hit], loading: Option<Loading>, t: &Theme) -> El {
    let ui = &s.frames.find;
    let query = s.search.text.trim();
    let at = ui.selected_index(hits);
    let rows: Vec<El> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| {
            let (pressed, opened, menu) = (hit.id.clone(), hit.id.clone(), hit.id.clone());
            result_row(i, hit, query, at == Some(i), t)
                .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                    s.frames.find.selected = Some(pressed.clone());
                    tx.focus(format!("row-{i}"), false);
                })
                .on_double_press(move |s: &mut State| {
                    let hits = super::hits(s);
                    if let Some(at) = hits.iter().position(|h| h.id == opened) {
                        open_result(s, &hits, at);
                    }
                })
                // The usual file menu, plus Show in enclosing folder (DW-0402).
                .on_secondary_press(move |s: &mut State, _, _| {
                    s.frames.find.selected = Some(menu.clone());
                    s.frames.intents.push(Intent::About("DW-0402", menu.clone()));
                })
        })
        .collect();
    let list = col()
        .key("results")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(narrow_row(ui.narrow, true, t))
        .children(loading.map(|l| loading_note(l, t)))
        .child(header(t))
        .child(
            col()
                .key("rows")
                .role("list")
                .label("Results")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .py(4.0)
                .px(6.0)
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .children(rows),
        );
    let mut body = row().key("search-body").w_full().child(list);
    if s.show_detail_pane
        && loading.is_none()
        && let Some(hit) = at.and_then(|i| hits.get(i))
        && let Some(facts) = ui.source.details(&hit.id)
    {
        body = body.child(details_pane(hit, &facts, t));
    }
    body
}

/// No result (DW-0803): the pills, and the empty state that says why that might be.
fn nothing(s: &State, narrow: Narrow, loading: Option<Loading>, t: &Theme) -> El {
    let query = s.search.text.trim();
    col()
        .key("search-body")
        .w_full()
        .child(narrow_row(narrow, false, t))
        .children(loading.map(|l| loading_note(l, t)))
        .child(
            empty(
                &icons::SEARCH,
                &format!("No results for \u{201C}{query}\u{201D}"),
                NO_MATCH,
                None,
                t,
            )
            .grow(1.0)
            .w_full(),
        )
}

/// The details panel of the selected result (`.mk-det`), as Files draws it (DW-0401), from
/// `SearchSource::details`. Its controls ask for what they do about that result.
///
/// The board's export names three of its elements twice: the Available offline card is also its
/// grouped list (`list-0`) with one item (`item-0-0`, `-title`, `-sub`), and Share… and Version
/// history are also `btn-share` and `btn-version-history` (each with `-icon`, `-label`). The
/// panel names them twice too: a box by a second element of the same box and fill, a text by a
/// note on the same text (`kit::note_text`), and a glyph by a second, unpainted glyph on the same
/// place (`kit` has no way to name a drawn glyph twice; see the PR's requests).
fn details_pane(hit: &Hit, d: &Details, t: &Theme) -> El {
    let kind_line = if d.size.is_empty() {
        d.kind.clone()
    } else {
        format!("{} \u{00B7} {}", d.kind, d.size)
    };
    // The size is a figure: JetBrains Mono inside the Inter line.
    let figure = Span {
        start: (kind_line.len() - d.size.len()) as u32,
        end: kind_line.len() as u32,
        mono: true,
        ..Span::default()
    };
    let picture = if d.picture {
        file_list::thumbnail(preview(el().key("preview-picture"), t))
    } else {
        preview(page_preview(t), t)
    };
    let mut facts = col().key("detail-meta").role("list").gap(6.0).child(meta_row(
        "detail-changed",
        "Changed",
        &d.changed,
        true,
        t,
    ));
    if let Some(v) = &d.changed_on {
        facts = facts.child(meta_row("detail-changed-on", "Changed on", v, false, t));
    }
    if let Some(v) = &d.added {
        facts = facts.child(meta_row("detail-added", "Added", v, true, t));
    }
    facts = facts.child(meta_row("detail-where", "Where", &d.where_, false, t));
    if let Some(v) = &d.versions {
        facts = facts.child(meta_row("detail-version-history", "Version history", v, true, t));
    }
    facts = facts.child(meta_row("detail-shared", "Shared", &d.shared, false, t));

    let id = hit.id.clone();
    pane(t)
        .child(picture)
        .child(
            col()
                .key("detail-name-group")
                .child(kit::label("detail-name", hit.name.clone(), type_::HEAD, t.text).truncate())
                .child(
                    kit::label("detail-kind", kind_line, type_::CAPTION, t.text_tertiary)
                        .spans(vec![figure])
                        .mt(2.0),
                ),
        )
        .child(encrypted_badge(t))
        .child(facts)
        .child(offline_card(&id, d, t))
        .child(
            row()
                .key("detail-actions")
                .gap(8.0)
                .child(twice(
                    "btn-share",
                    action_button("share", "Share\u{2026}", &icons::SHARE, t)
                        .on_press(push(Intent::About("DW-0407", id.clone()))),
                    ("share", "Share\u{2026}", &icons::SHARE),
                    t,
                ))
                .child(twice(
                    "btn-version-history",
                    action_button("versions", "Version history", &icons::VERSION_HISTORY, t)
                        .on_press(push(Intent::About("DW-0413", id))),
                    ("versions", "Version history", &icons::VERSION_HISTORY),
                    t,
                )),
        )
}

/// An outlined button of the panel (`ui::detail_pane::action_button`, keyed `<key>-box`) under its
/// second name `name` (`btn-<words>`, `-icon`, `-label`): a box of the same size and fill around
/// it, a note on its words, and its glyph a second time, unpainted, where the drawn one is.
fn twice(name: &str, button: El, (key, words, glyph): (&str, &str, &'static rune::Icon), t: &Theme) -> El {
    kit::note_text(
        &format!("{name}-label"),
        &format!("{key}-btn"),
        words,
        type_::BUTTON_SM,
        t.text,
    );
    el().key(name.to_string())
        .radius(radius::MD)
        .fill(t.paper)
        .child(button)
        .child(
            // `action_button`: 12 pt in, the 22 pt button's 14 pt glyph centred: 4 pt down.
            kit::glyph(format!("{name}-icon"), glyph, 14.0, Color { a: 0.0, ..t.text })
                .a11y_hidden()
                .absolute(12.0, 4.0),
        )
}

/// The Available offline card (`.kit-list-body` > `.kit-item`): what the switch does is the
/// engine's; until Search is wired it asks for it (`keep-offline`).
fn offline_card(id: &str, d: &Details, t: &Theme) -> El {
    let title = "Available offline";
    let sub_style = type_::BODY.lh(15.0);
    kit::note_text("item-0-0-title", "offline-label", title, type_::ITEM_TITLE, t.text);
    kit::note_text(
        "item-0-0-sub",
        "offline-detail",
        &d.offline_note,
        sub_style,
        t.text_tertiary,
    );
    let id = id.to_string();
    let item = row()
        .key("item-0-0")
        .w_full()
        .min_h(40.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .child(
            col()
                .key("offline-text")
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(kit::label("offline-label", title, type_::ITEM_TITLE, t.text).truncate())
                .child(kit::label("offline-detail", d.offline_note.clone(), sub_style, t.text_tertiary).truncate()),
        )
        .child(
            switch("offline", title, d.kept_offline, t)
                .description(d.offline_note.clone())
                .on_press(push(Intent::Do("keep-offline", id))),
        );
    el().key("offline-card")
        .w_full()
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(el().key("list-0").w_full().radius(radius::LG).fill(t.card).child(item))
}
