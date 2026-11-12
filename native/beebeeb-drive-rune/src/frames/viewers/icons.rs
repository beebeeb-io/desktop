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
pub static COPY: Icon = icon24!(
    "copy",
    r#"<path d="M20 9h-9a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-9a2 2 0 0 0-2-2zM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>"#
);
pub static EDIT: Icon = icon24!(
    "edit",
    r#"<path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z M15 5l4 4"></path>"#
);
pub static INFO: Icon = icon24!(
    "info",
    r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01"></path>"#
);
pub static REFRESH: Icon = icon24!(
    "refresh",
    r#"<path d="M20 11a8 8 0 0 0-14.5-4.5L4 8"></path><path d="M4 4v4h4"></path><path d="M4 13a8 8 0 0 0 14.5 4.5L20 16"></path><path d="M20 20v-4h-4"></path>"#
);
pub static LAPTOP: Icon = icon24!(
    "laptop",
    r#"<path d="M5 6.5A1.5 1.5 0 0 1 6.5 5h11A1.5 1.5 0 0 1 19 6.5V15H5z"></path><path d="M2.5 18.5h19"></path>"#
);
pub static PHONE: Icon = icon24!(
    "phone",
    r#"<path d="M8.5 3h7A1.5 1.5 0 0 1 17 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 7 19.5v-15A1.5 1.5 0 0 1 8.5 3z"></path><path d="M11 18h2"></path>"#
);
