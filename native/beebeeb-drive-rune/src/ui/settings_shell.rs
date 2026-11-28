//! Settings inside the main window (`.mk-set`): the scaffold every settings pane shares.
//!
//! The board (DW-0701, DESIGN §5.4): settings is a STATE of the main window, never a window
//! of its own. The caption stays; the Drive sidebar gives way to the settings sidebar (Back
//! to Drive, the sections, a foot note) and the content area to one pane, a 540 pt column
//! on the ground colour.
//!
//! | Piece | Function | Keys it gives |
//! | --- | --- | --- |
//! | Sidebar item | `nav_item(label, icon, icon_size, on, t)` | `<label>-row`, `<label>-icon`, `<label>` |
//! | Sidebar heading | `nav_title(text, t)` | `side-title-<slug>` |
//! | Sidebar | `sidebar(back, sections, foot, t)` | `sidebar`, `side-foot-box`, `side-foot` |
//! | The whole settings sidebar, one Tab stop, arrows and type-select | `app_sidebar(selected, previews, go, t)` | as `sidebar` |
//! | Pane | `pane(children, t)` | `set-main`, `set-col` |
//! | Way back to the parent pane | `crumb(label, t, back)` | `crumb-row`, `crumb-icon`, `crumb` |
//! | Pane heading | `heading(title, sub, t)` | `set-title`, `set-sub` |
//!
//! Nothing here knows which panes exist or what a press does: a frame attaches `.on_press`.

use crate::theme::tokens::{radius, type_, Theme};
use rune::{col, el, row, Color, El, Icon, TextStyle};

use super::kit;
use crate::theme::icons_settings as ic;
use crate::State;

/// The settings sidebar is 224 pt wide on the board (the Drive sidebar is 220).
pub const SIDEBAR_W: f32 = 224.0;
/// A pane's column (`.mk-set-col`): at least this wide.
pub const COLUMN_W: f32 = 540.0;
/// A pane never shows more card columns than this, however wide the window is.
pub const MAX_COLUMNS: usize = 4;
/// A card is never narrower than this: n columns need room for n of them and the gaps between.
pub const CARD_MIN_W: f32 = 420.0;
/// The gap between a pane's children, in both directions.
pub const FLOW_GAP: f32 = 14.0;

thread_local! {
    /// How many full-width lead pieces (a crumb, a heading) were built since the last pane:
    /// those stay above the cards instead of flowing with them.
    static LEAD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A lead piece (crumb, heading) was built: the next pane keeps it full width.
pub fn mark_lead() {
    LEAD.with(|l| l.set((l.get() + 1).min(2)));
}

/// The least width of a flow item in a column `column_w` wide: [`CARD_MIN_W`], and in a column
/// with room for more than [`MAX_COLUMNS`] of those, just over a share of one more (in 16 pt
/// steps, so a resize does not rebuild the pane at every point), so no more than
/// [`MAX_COLUMNS`] fit side by side.
fn item_min_w(column_w: f32) -> f32 {
    let over = MAX_COLUMNS as f32 + 1.0;
    if column_w < over * CARD_MIN_W + (over - 1.0) * FLOW_GAP {
        return CARD_MIN_W;
    }
    ((column_w / over / 16.0).floor() + 1.0) * 16.0
}

/// A pane's column: the lead pieces (crumb, heading) stacked full width, then the rest in a
/// row that wraps. The column takes the whole content width. Each flowing child sits in an
/// item at least [`CARD_MIN_W`] wide that grows, so the row holds as many items side by side as
/// fit (854 pt: two, 1288 pt: three, 1722 pt: four) and never more than [`MAX_COLUMNS`]
/// ([`item_min_w`], from the column's width in the last layout).
/// Every item is one column wide, also alone in the last row: empty items after the last
/// card take the columns it would otherwise stretch over.
/// Order and whole children are kept. `min_w` is the column's floor (`COLUMN_W`).
pub fn flow(children: Vec<El>, min_w: f32) -> El {
    let lead = LEAD.with(|l| l.replace(0)).min(children.len());
    let item_w = rune::memo::layout_with("set-col", |r| item_min_w(r[2])).unwrap_or(CARD_MIN_W);
    let mut column = col().key("set-col").w_full().min_w(min_w).gap(FLOW_GAP);
    // The gap between rows is each card item's bottom padding, taken back once under the last
    // row: a row of only empty items is then 0 pt high and adds nothing.
    let mut cards = row()
        .key("set-flow")
        .w_full()
        .flex_wrap()
        .items_start()
        .gaps(0.0, FLOW_GAP)
        .mb(-FLOW_GAP);
    let item = |key: String| col().key(key).min_w(item_w).grow(1.0);
    for (i, child) in children.into_iter().enumerate() {
        if i < lead {
            column = column.child(child);
        } else {
            cards = cards.child(
                item(format!("set-item-{}", i - lead))
                    .pb(FLOW_GAP)
                    .child(child.w_full()),
            );
        }
    }
    for i in 0..MAX_COLUMNS - 1 {
        cards = cards.child(item(format!("set-fill-{i}")).h(0.0));
    }
    column.child(cards)
}

/// 22px/650 on a 28px line, tracked -0.022em: a pane's title (`.t-large` at the desktop scale).
pub const PANE_TITLE: TextStyle = TextStyle::new(22.0, 650.0).lh(28.0).tracking(-0.484);
/// 12px/400 on a 15px line: a pane's or a card's second line (`.t-sub`).
pub const SUB: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);

/// `a-b-c` from any text, as `scripts/keymap.js` slugs a heading.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// One item of the settings sidebar (`.mk-side-item`): a 28 pt row, selected in the neutral
/// selection tint. A link: it shows another pane. Keys: `<label>-row`, `<label>-icon`, `<label>`.
pub fn nav_item(label: &str, ic: &'static Icon, icon_size: f32, on: bool, t: &Theme) -> El {
    row()
        .key(format!("{label}-row"))
        .role("link")
        .label(label)
        .h(28.0)
        .px(9.0)
        .gap(9.0)
        .center_y()
        .radius(radius::CONTROL)
        .fill(if on { t.selection } else { Color::TRANSPARENT })
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(kit::glyph(
            format!("{label}-icon"),
            ic,
            icon_size,
            if on { t.text } else { t.text_tertiary },
        ))
        .child(
            kit::label(
                label,
                label,
                type_::LIST_ITEM,
                if on { t.text } else { t.text_secondary },
            )
            .grow(1.0)
            .truncate(),
        )
}

/// A heading over a group of sidebar items (`.mk-side-title`), drawn in capitals.
/// Key: `side-title-<slug>`.
pub fn nav_title(text: &str, t: &Theme) -> El {
    el().key(format!("side-title-box-{}", slug(text)))
        .px(10.0)
        .pb(3.0)
        .child(kit::label(
            format!("side-title-{}", slug(text)),
            text.to_uppercase(),
            type_::SECTION_LABEL,
            t.text_tertiary,
        ))
}

/// The settings sidebar (`.mk-side` inside `.mk-set`): the way back on top, the titled
/// groups of items, and a foot note under a hairline.
pub fn sidebar(back: El, sections: Vec<(&str, Vec<El>)>, foot: &str, t: &Theme) -> El {
    let mut nav = col()
        .key("settings-nav")
        .role("navigation")
        .label("Settings sections")
        .gap(12.0)
        .grow(1.0)
        .min_h(0.0);
    for (title, items) in sections {
        let mut group = col()
            .key(format!("side-sec-{}", slug(title)))
            .role("group")
            .label(title)
            .gap(1.0)
            .child(nav_title(title, t));
        for item in items {
            group = group.child(item);
        }
        nav = nav.child(group);
    }
    col()
        .key("sidebar")
        .w(SIDEBAR_W)
        .h_full()
        .fill(t.sidebar_bg)
        .border_sides([0.0, kit::HAIR, 0.0, 0.0], t.separator)
        .border_layout()
        .gap(12.0)
        .pt(12.0)
        .px(10.0)
        .pb(10.0)
        .child(back)
        .child(nav)
        .child(
            col()
                .key("side-foot-box")
                .items_start()
                .pt(10.0)
                .px(6.0)
                .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
                .border_layout()
                .child(kit::label("side-foot", foot, type_::CAPTION, t.text_tertiary).wrap()),
        )
}

/// A pane (`.mk-set-main` > `.mk-set-col`): the ground, 22 pt above and 30 pt beside a
/// column (540 pt on the board, wider with the window) whose children sit 14 pt apart. A pane taller than the window scrolls
/// (the board draws those frames taller than the window "so the whole section shows").
pub fn pane(children: Vec<El>, t: &Theme) -> El {
    let column = flow(children, COLUMN_W);
    col()
        .key("set-main")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .fill(t.surface)
        .py(22.0)
        .px(30.0)
        .scroll(rune::ScrollSpec { stick_bottom: false })
        .child(column)
}

/// The way back to the pane this one opened from, above the heading ("‹ General").
/// A link; `back` is what a press does (on the link itself: the element that takes the
/// focus is the one that takes the press). Keys: `crumb-row`, `crumb-icon`, `crumb`.
pub fn crumb<S: 'static>(label: &str, t: &Theme, back: impl Fn(&mut S) + 'static) -> El {
    mark_lead();
    row().key("crumb-line").child(
        row()
            .key("crumb-row")
            .role("link")
            .label(format!("Back to {label}"))
            .gap(4.0)
            .center_y()
            .radius(radius::SM)
            .hover(t.hover)
            .press(t.press)
            .focusable(true)
            .on_press(back)
            .child(kit::glyph(
                "crumb-icon",
                &crate::theme::icons_settings::BACK,
                13.0,
                t.text_tertiary,
            ))
            .child(kit::label("crumb", label, SUB, t.text_tertiary)),
    )
}

/// A pane's heading (`.mk-set-h`): the title, and a second line where the board has one.
/// Keys: `set-title`, `set-sub`.
pub fn heading(title: &str, sub: Option<&str>, t: &Theme) -> El {
    mark_lead();
    let mut h = col().key("set-h").items_start().gap(3.0).child(
        kit::label("set-title", title, PANE_TITLE, t.text)
            .role("heading")
            .level(1)
            .w_full(),
    );
    if let Some(sub) = sub {
        // `max-width: 60ch`: sixty zeros of Inter at 12 px.
        h = h.child(kit::label("set-sub", sub, SUB, t.text_tertiary).wrap().w(454.22));
    }
    h
}

/// Where a sidebar item leads: the frame it shows. The frame lane owns what a press does.
pub type Go = fn(&mut State, &'static str);

/// The SETTINGS group of the sidebar: label, icon, board frame.
pub const SETTINGS_ITEMS: &[(&str, &Icon, &str)] = &[
    ("General", &ic::GEAR, "DW-0701"),
    ("Account", &ic::PEOPLE, "DW-A51"),
    ("Security", &ic::SHIELD, "DW-A52"),
    ("Devices and sessions", &ic::LAPTOP, "DW-A55"),
    ("Notifications", &ic::BELL, "DW-0750"),
];
/// The DRIVE group. The board draws a chevron here; the owner wants a glyph like the SETTINGS group,
/// so a pane's row never reads as an expandable folder.
#[cfg(not(target_os = "macos"))]
pub const DRIVE_ITEMS: &[(&str, &Icon, &str)] = &[
    ("Storage", &ic::DATABASE, "DW-0712"),
    ("Backups", &ic::CLOCK, "DW-0737"),
    (
        concat!("Sync and ", beebeeb_engine::file_manager!()),
        &ic::SYNC,
        "DW-0746",
    ),
    ("Thumbnails and previews", &ic::IMAGE, "DW-0744"),
    ("Bandwidth", &ic::GAUGE, "DW-0785"),
];
/// On a Mac the group also lists Beebeeb in Finder (dm-18, frame `DM-18`).
#[cfg(target_os = "macos")]
pub const DRIVE_ITEMS: &[(&str, &Icon, &str)] = &[
    ("Storage", &ic::DATABASE, "DW-0712"),
    ("Backups", &ic::CLOCK, "DW-0737"),
    (
        concat!("Sync and ", beebeeb_engine::file_manager!()),
        &ic::SYNC,
        "DW-0746",
    ),
    ("Beebeeb in Finder", &ic::FOLDER_TAB, "DM-18"),
    ("Thumbnails and previews", &ic::IMAGE, "DW-0744"),
    ("Bandwidth", &ic::GAUGE, "DW-0785"),
];

/// The whole settings sidebar as the board draws it (Back to Drive, Settings, Drive): the one list
/// every settings frame and the not-built placeholder share. `selected` is an item's label (none:
/// no item selected); `previews`: list Thumbnails and previews; `go` is called with the board frame
/// an item shows (`DW-0401`: Back to Drive).
pub fn app_sidebar(selected: Option<&str>, previews: bool, go: Go, t: &Theme) -> El {
    let items = |list: &'static [(&'static str, &'static Icon, &'static str)]| -> Vec<El> {
        list.iter()
            // Not listed where nothing answers for it: a pane that only says "not built yet" is a dead door.
            .filter(|(label, _, _)| previews || *label != "Thumbnails and previews")
            .map(|&(label, icon, frame)| {
                let on = selected == Some(label);
                roving_item(nav_item(label, icon, 16.0, on, t), on, t).on_press(move |s: &mut State| go(s, frame))
            })
            .collect()
    };
    let back = roving_item(nav_item("Back to Drive", &ic::BACK, 15.0, false, t), false, t)
        .shortcut_hint("Esc")
        .on_press(move |s: &mut State| go(s, "DW-0401"));
    // One Tab stop (Tab lands on the selected item, else Back to Drive): Up/Down/Home/End
    // move, typing a name's first letters goes to it (Rune's list focus group).
    sidebar(
        back,
        vec![("Settings", items(SETTINGS_ITEMS)), ("Drive", items(DRIVE_ITEMS))],
        "Settings open inside the app. Esc goes back.",
        t,
    )
    .role("list")
    .label("Settings sidebar")
    .focus_group()
}

/// An item of the roving sidebar: the group's Tab stop when `on`, and the shared ring (a list's
/// items draw none of their own) so the keyboard's place shows on an unselected item.
fn roving_item(item: El, on: bool, t: &Theme) -> El {
    super::focus::apply(item.focus_selected(on), t.accent_deep)
}

#[cfg(test)]
mod sidebar_icon_tests {
    use super::*;

    #[test]
    fn no_sidebar_entry_uses_a_chevron() {
        for (label, icon, _) in SETTINGS_ITEMS.iter().chain(DRIVE_ITEMS) {
            assert!(!icon.name.contains("chev"), "{label} shows a chevron");
        }
        let mut names: Vec<_> = DRIVE_ITEMS.iter().map(|(_, i, _)| i.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), DRIVE_ITEMS.len(), "each Drive entry has its own icon");
    }
}

#[cfg(test)]
mod settings_flow_tests {
    use super::*;
    use crate::theme::tokens::theme;
    use crate::ui::settings_card;
    use rune::{Cx, Env, Runtime};

    /// A pane of a heading and five cards beside the sidebar.
    fn view(_: &(), _: &mut Cx<()>) -> El {
        let t = theme(false);
        let mut kids = vec![heading("Title", Some("A line under it."), &t)];
        for i in 0..5 {
            kids.push(settings_card::card(i, &t).child(settings_card::head(i, &format!("Card {i}"), &t)));
        }
        row()
            .key("root")
            .w_full()
            .h_full()
            .child(el().w(SIDEBAR_W).h_full())
            .child(pane(kids, &t))
    }

    fn at(content: f32) -> Runtime<()> {
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new((), view, (SIDEBAR_W + content, 900.0), env);
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn rect(rt: &Runtime<()>, key: &str) -> [f32; 4] {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
            .unwrap_or_else(|| panic!("no element keyed {key}"))
            .rect
    }

    /// (column width, cards side by side in the first row) for a window whose content area is
    /// `content` wide.
    fn shape(content: f32) -> (f32, usize) {
        let rt = at(content);
        let col = rect(&rt, "set-col");
        let top = rect(&rt, "card-0")[1];
        let mut beside = 0;
        for i in 0..5 {
            let c = rect(&rt, &format!("card-{i}"));
            assert!(
                c[0] >= col[0] - 0.5 && c[0] + c[2] <= col[0] + col[2] + 0.5,
                "card {i} sticks out of the column at {content}"
            );
            assert!(c[2] >= CARD_MIN_W - 0.5, "card {i} is {} wide at {content}", c[2]);
            let h = rect(&rt, &format!("card-{i}-head"));
            assert!(
                h[0] + h[2] <= c[0] + c[2] + 0.5 && h[1] + h[3] <= c[1] + c[3] + 0.5,
                "card {i} is cut at {content}"
            );
            if (c[1] - top).abs() < 0.5 {
                beside += 1;
            }
        }
        (col[2], beside)
    }

    #[test]
    fn narrow_window_keeps_the_540_column() {
        let (w, cols) = shape(600.0);
        assert!(
            (COLUMN_W..=600.0 - 60.0 + 0.5).contains(&w) || (w - COLUMN_W).abs() < 0.5,
            "column {w}"
        );
        assert_eq!(cols, 1);
        let (w, cols) = shape(540.0 + 60.0);
        assert!((w - 540.0).abs() < 0.5, "column {w}");
        assert_eq!(cols, 1);
    }

    #[test]
    fn column_fills_the_window_up_to_four_cards() {
        let need = |n: usize| n as f32 * CARD_MIN_W + (n - 1) as f32 * FLOW_GAP;
        // content area = window - sidebar; 30 pt either side
        let room = |window: f32| window - SIDEBAR_W - 60.0;
        // Just under and at the room n cards need.
        for n in 2..=MAX_COLUMNS {
            assert_eq!(shape(need(n) + 60.0 - 2.0).1, n - 1);
            assert_eq!(shape(need(n) + 60.0).1, n);
        }
        // 1600 leaves 1316 pt: three cards (1288 pt) fit, four (1722 pt) do not. From 2000 on:
        // four, never five, and the column is as wide as the room at every width.
        for (window, want) in [
            (1000.0, 1),
            (1400.0, 2),
            (1600.0, 3),
            (2010.0, 4),
            (2440.0, 4),
            (2500.0, 4),
            (3400.0, 4),
            (5000.0, 4),
        ] {
            let (w, cols) = shape(window - SIDEBAR_W);
            assert!(
                (w - room(window)).abs() < 0.5,
                "{window}: column {w}, room {}",
                room(window)
            );
            assert_eq!(cols, want, "{window}: columns");
        }
    }

    #[test]
    fn every_card_is_one_column_wide() {
        for window in [1000.0_f32, 1400.0, 1600.0, 2010.0, 2500.0, 3400.0] {
            let rt = at(window - SIDEBAR_W);
            let col = rect(&rt, "set-col");
            let cards: Vec<[f32; 4]> = (0..5).map(|i| rect(&rt, &format!("card-{i}"))).collect();
            let cols = cards.iter().filter(|c| (c[1] - cards[0][1]).abs() < 0.5).count();
            let one = (col[2] - (cols - 1) as f32 * FLOW_GAP) / cols as f32;
            for (i, c) in cards.iter().enumerate() {
                assert!(
                    (c[2] - one).abs() < 0.5,
                    "{window}: card {i} is {} wide, a column is {one}",
                    c[2]
                );
            }
            // The first row reaches the column's right edge.
            let right = cards[cols - 1][0] + cards[cols - 1][2];
            assert!(
                (right - (col[0] + col[2])).abs() < 0.5,
                "{window}: the row ends at {right}"
            );
        }
    }

    /// Five cards in four columns leave the fifth alone in the last row; in two columns too, and
    /// in three there are two. None stretches over the empty columns.
    #[test]
    fn a_lone_last_card_keeps_the_column_width() {
        for (window, in_last_row) in [(1400.0_f32, 1), (1600.0, 2), (2500.0, 1), (3400.0, 1)] {
            let rt = at(window - SIDEBAR_W);
            let first = rect(&rt, "card-0");
            let last = rect(&rt, "card-4");
            let lead = rect(&rt, &format!("card-{}", 5 - in_last_row));
            assert!(last[1] > first[1] + 0.5, "{window}: the last card is in a later row");
            assert!(
                (last[1] - lead[1]).abs() < 0.5,
                "{window}: {in_last_row} in the last row"
            );
            assert!(
                (lead[0] - first[0]).abs() < 0.5,
                "{window}: the last row starts at the column's left"
            );
            assert!(
                (last[2] - first[2]).abs() < 0.5,
                "{window}: last card {} wide, first {}",
                last[2],
                first[2]
            );
            let col = rect(&rt, "set-col");
            assert!(
                last[0] + last[2] < col[0] + col[2] - CARD_MIN_W,
                "{window}: the last card leaves a column empty"
            );
        }
    }

    /// The empty items add no height: the flow ends 0 pt under its last card, at every width.
    #[test]
    fn empty_items_add_no_height() {
        for window in [1000.0_f32, 1400.0, 1600.0, 2500.0, 3400.0] {
            let rt = at(window - SIDEBAR_W);
            let col = rect(&rt, "set-col");
            let bottom = (0..5)
                .map(|i| rect(&rt, &format!("card-{i}")))
                .map(|c| c[1] + c[3])
                .fold(f32::MIN, f32::max);
            assert!(
                (col[1] + col[3] - bottom).abs() < 0.5,
                "{window}: column ends at {}, cards at {bottom}",
                col[1] + col[3]
            );
        }
    }

    /// A pane of one card follows the same rule: the card is one column wide, not the window.
    #[test]
    fn a_single_card_is_one_column_wide() {
        fn one(_: &(), _: &mut Cx<()>) -> El {
            let t = theme(false);
            let kids = vec![
                heading("Title", None, &t),
                settings_card::card(0, &t).child(settings_card::head(0, "Card", &t)),
            ];
            row()
                .key("root")
                .w_full()
                .h_full()
                .child(el().w(SIDEBAR_W).h_full())
                .child(pane(kids, &t))
        }
        let mut rt = Runtime::new(
            (),
            one,
            (2500.0, 900.0),
            Env {
                system_dark: false,
                scale: 2.0,
            },
        );
        rt.frame(0.0);
        rt.settle_all();
        rt.frame(0.0);
        let col = rect(&rt, "set-col")[2];
        let one = (col - 3.0 * FLOW_GAP) / 4.0;
        assert!((col - (2500.0 - SIDEBAR_W - 60.0)).abs() < 0.5, "column {col}");
        assert!(
            (rect(&rt, "card-0")[2] - one).abs() < 0.5,
            "card {}, a column is {one}",
            rect(&rt, "card-0")[2]
        );
    }

    /// Not a check: prints how long a switch to each settings pane takes to build and lay out
    /// (a full-screen 16" window; no window, no paint), and the frame after it.
    #[test]
    #[ignore = "timing read-out: cargo test settings_shell -- --ignored --nocapture"]
    fn pane_build_times() {
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut state = crate::init();
        crate::frames::settings::go(&mut state, "DW-0701");
        let mut rt = Runtime::new(state, crate::view, (1728.0, 1080.0), env);
        rt.frame(0.0);
        // Twice round: the first visit of a pane also shapes its text for the first time.
        for round in ["first", "again"] {
            for frame in [
                "DW-0751", "DW-0750", "DW-0791", "DW-0788", "DW-0746", "DW-0712", "DW-0783", "DW-0785", "DW-0790",
                "DW-0744", "DM-18", "DW-A51", "DW-A52", "DW-A55", "DW-0737", "DW-0701",
            ] {
                let start = std::time::Instant::now();
                rt.update(|s, _| crate::frames::settings::go(s, frame));
                rt.frame(16.0);
                let switch = start.elapsed();
                let start = std::time::Instant::now();
                rt.frame(16.0);
                let title = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some("set-title"))
                    .is_some();
                println!(
                    "{round} {frame} switch={switch:.1?} next={:.1?} elements={} pane={title}",
                    start.elapsed(),
                    rt.elements().len()
                );
            }
        }
    }

    #[test]
    fn cards_keep_their_order() {
        let rt = at(2500.0 - SIDEBAR_W);
        let mut last = (f32::MIN, f32::MIN);
        for i in 0..5 {
            let c = rect(&rt, &format!("card-{i}"));
            assert!(c[1] > last.0 - 0.5);
            last = (c[1], c[0]);
        }
        let h = rect(&rt, "set-h");
        assert!(
            rect(&rt, "card-0")[1] >= h[1] + h[3],
            "the heading stays above the cards"
        );
    }
}
