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
//! | Pane | `pane(children, t)` | `set-main`, `set-col` |
//! | Way back to the parent pane | `crumb(label, t)` | `crumb-row`, `crumb-icon`, `crumb` |
//! | Pane heading | `heading(title, sub, t)` | `set-title`, `set-sub` |
//!
//! Nothing here knows which panes exist or what a press does: a frame attaches `.on_press`.

use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Icon, TextStyle, col, el, row};

use super::kit;

/// The settings sidebar is 224 pt wide on the board (the Drive sidebar is 220).
pub const SIDEBAR_W: f32 = 224.0;
/// A pane's column (`.mk-set-col`).
pub const COLUMN_W: f32 = 540.0;

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
/// 540 pt column whose children sit 14 pt apart.
pub fn pane(children: Vec<El>, t: &Theme) -> El {
    let mut column = col().key("set-col").w(COLUMN_W).gap(14.0);
    for child in children {
        column = column.child(child);
    }
    col()
        .key("set-main")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(t.surface)
        .py(22.0)
        .px(30.0)
        .child(column)
}

/// The way back to the pane this one opened from, above the heading ("‹ General").
/// A link. Keys: `crumb-row`, `crumb-icon`, `crumb`.
pub fn crumb(label: &str, t: &Theme) -> El {
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
