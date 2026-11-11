//! Caption bar: the Beebeeb "b" mark + "Beebeeb Drive" title, with window drag
//! and Windows minimize/maximize/close buttons (drawn by Rune's Titlebar).
//!
//! Board: 32 pt tall, sidebar-coloured left half with the b mark and "DRIVE" label,
//! transparent right half for drag + caption buttons.

use crate::State;
use crate::TITLEBAR_H;
use crate::theme::icons;
use crate::theme::tokens::{self, Theme, space};
use rune::{Cx, El, el, icon, row};

pub fn caption_bar(t: &Theme, cx: &mut Cx<State>) -> El {
    let insets = cx.chrome_insets();
    let dark = cx.system_dark();

    // Caption icon colour for Windows buttons
    cx.caption_color(if dark { rune::Color::WHITE } else { rune::Color::BLACK });

    // The board shows: [b logo] [DRIVE label] ... [drag area] ... [min][max][close]
    // The b logo + DRIVE text is in the sidebar-bg area on the left.
    // Rune draws the actual caption buttons; we reserve `insets.trailing` for them.

    row()
        .key("caption-bar")
        .w_full()
        .h(TITLEBAR_H)
        .window_drag()
        .center_y()
        .child(
            // Sidebar-coloured left portion
            row()
                .key("caption-left")
                .w(220.0) // sidebar width (board: 220px)
                .h(TITLEBAR_H)
                .fill(t.sidebar_bg)
                .pl(insets.leading + space::MD)
                .center_y()
                // b mark: the board uses a 16x16 icon from the beebeeb mark (x=13, y=8)
                // drive-label "DRIVE" is in the sidebar at y=64 (see sidebar.rs header)
                .child(
                    el().key("b-mark")
                        .size(16.0, 16.0)
                        .radius(3.0)
                        .fill(tokens::amber())
                        .center()
                        .child(icon(&icons::FILES, 9.0).key("b-mark-icon").color(t.text)),
                ),
        )
        .child(
            // Main area: title text
            row()
                .key("caption-main")
                .grow(1.0)
                .h(TITLEBAR_H)
                .fill(t.bg)
                .center_y()
                .pl(space::MD),
        )
}
