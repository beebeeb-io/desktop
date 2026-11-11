//! The open-file group's icons: each body is the board's inline `<svg>` of its frame, copied by script
//! from the golden (the gate compares the path data). 24-unit stroke icons, 1.6 wide, round caps and joins.

use rune::Icon;

pub use crate::theme::icons::{
    CHEVRON_DOWN, CHEVRON_LEFT, CHEVRON_RIGHT, LOCK, PANEL_TOGGLE, PLUS, SEARCH, SORT, VIEW_GRID, VIEW_LIST,
};

macro_rules! icon24 {
    ($name:expr, $body:expr) => {
        Icon {
            name: $name,
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        }
    };
}

pub static MORE: Icon = icon24!(
    "more",
    r#"<path d="M12 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM19 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM5 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2z"></path>"#
);
pub static FILE_VIDEO: Icon = icon24!(
    "file-video",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 15l5-3-5-3v6z"></path>"#
);
pub static ALERT: Icon = icon24!(
    "alert",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 8v4.6"></path><path d="M12 15.9v.1"></path>"#
);
