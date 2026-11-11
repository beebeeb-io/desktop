//! The "not built yet" frame: what a route shows until its frame lane registers a view.
//!
//! It is honest by construction: the group's name, the board frames it will follow, and one
//! sentence. No rows, no counts, no names: nothing that could be read as the account's data.
//! It keeps the window's real chrome (the caption, and the sidebar inside Drive) so routing
//! can be seen and tested before the frames exist.

use rune::{Cx, El, col, el, row, text};

use super::app::Screen;
use super::flyout_window::FlyoutState;
use super::registry::FrameGroup;
use crate::State;
use crate::theme::tokens::{self, Theme, radius, space, type_};
use crate::ui;

/// The notice itself: a label, the group, the board frames, one sentence. `narrow`: fill the
/// width given and wrap (the flyout is 340 points wide).
fn notice(t: &Theme, group: FrameGroup, extra: Option<String>, narrow: bool) -> El {
    let line = |key: &'static str, words: String, style, color| {
        let line = text(words).key(key).font(style).color(color);
        if narrow { line.w_full().wrap() } else { line }
    };
    let mut card = col()
        .key("not-built")
        .role("group")
        .label(format!("{}: not built yet", group.name()))
        .gap(space::SM)
        .px(if narrow { space::LG } else { space::XL })
        .py(if narrow { space::LG } else { space::XL })
        .radius(radius::LG)
        .fill(t.surface)
        .border(1.0, t.separator)
        .child(line(
            "not-built-label",
            "NOT BUILT YET".into(),
            type_::SECTION_LABEL,
            t.text_tertiary,
        ))
        .child(line("not-built-title", group.name(), type_::TITLE, t.text))
        .child(line(
            "not-built-line",
            "This part of Beebeeb Drive has no screen yet. Nothing here is your data.".into(),
            type_::BODY,
            t.text_secondary,
        ))
        .child(line(
            "not-built-frames",
            format!("Board: {}", group.board_section()),
            type_::META_MONO,
            t.text_tertiary,
        ));
    if let Some(extra) = extra {
        card = card.child(line("not-built-extra", extra, type_::META_MONO, t.text_tertiary));
    }
    if narrow { card.w_full() } else { card }
}

/// The main window with the placeholder where `group`'s frame will be.
pub fn window(s: &State, cx: &mut Cx<State>, group: FrameGroup) -> El {
    let t = tokens::theme(cx.system_dark());
    let hairline = 1.0 / cx.env().scale;
    let in_drive = matches!(s.shell.app.screen(), Screen::Drive(_));
    let content = col()
        .key("content-area")
        .grow(1.0)
        .h_full()
        .fill(t.bg)
        .center()
        .child(notice(&t, group, None, false));
    let mut body = row().key("body").grow(1.0).w_full();
    // Inside Drive the caption and the sidebar are the real ones and stay. The gate and
    // settings have their own left side (the brand panel, the settings sidebar), so there the
    // caption is only the strip that moves the window and holds Windows' buttons.
    let caption = if in_drive {
        body = body
            .child(ui::sidebar::sidebar(s, &t))
            .child(el().key("sidebar-separator").w(hairline).h_full().fill(t.separator));
        ui::caption::caption_bar(&t, cx)
    } else {
        cx.caption_color(if cx.system_dark() {
            rune::Color::WHITE
        } else {
            rune::Color::BLACK
        });
        row().key("caption-bar").w_full().h(crate::TITLEBAR_H).window_drag()
    };
    col()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(t.bg)
        .child(caption)
        .child(body.child(content))
}

/// The flyout with the placeholder as its content. The one line of state it shows is the
/// shell's own tray model, and it says where that comes from.
pub fn flyout(s: &FlyoutState, cx: &mut Cx<FlyoutState>) -> El {
    let t = tokens::theme(cx.system_dark());
    let source = format!("Tray state ({}): {}", s.source, s.model.label);
    col()
        .key("flyout")
        .role("window")
        .w_full()
        .h_full()
        .fill(t.bg)
        .border(1.0, t.separator_strong)
        .center()
        .px(space::MD)
        .child(notice(&t, FrameGroup::Flyout, Some(source), true))
}
