//! The Photos group's icons: each body is the board's inline `<svg>` of the Photos frames, copied by
//! script (the gate compares the path data). All are 24-unit stroke icons, 1.6 wide, round caps and joins.

use rune::Icon;

// The same drawings the Files window already has: one body, in `theme/icons.rs`.
pub use crate::theme::icons::{
    AVAILABLE_OFFLINE as OFFLINE, CHEVRON_DOWN, FILES as FOLDER, LINK, LOCK, PHOTOS, PLUS, SHARE,
    SORT_ASC as UPLOADING, SYNC_HISTORY as CLOCK, TRASH,
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

pub static SOURCES: Icon = icon24!(
    "sources",
    r#"<path d="M4 7h9"></path><path d="M17 7h3"></path><path d="M15 5v4"></path><path d="M4 17h3"></path><path d="M11 17h9"></path><path d="M9 15v4"></path>"#
);

pub static CHECK: Icon = icon24!("check", r#"<path d="M20 6L9 17l-5-5"></path>"#);

pub static BACKING_UP: Icon = icon24!(
    "backing-up",
    r#"<path d="M20 11a8 8 0 0 0-14.5-4.5L4 8"></path><path d="M4 4v4h4"></path><path d="M4 13a8 8 0 0 0 14.5 4.5L20 16"></path><path d="M20 20v-4h-4"></path>"#
);

pub static FOLDER_MOVE: Icon = icon24!(
    "folder-move",
    r#"<path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2h9A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"></path><path d="M9.5 13.5h5.5"></path><path d="M13 11.3l2.2 2.2-2.2 2.2"></path>"#
);

pub static PHONE: Icon = icon24!(
    "phone",
    r#"<path d="M8.5 3h7A1.5 1.5 0 0 1 17 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 7 19.5v-15A1.5 1.5 0 0 1 8.5 3z"></path><path d="M11 18h2"></path>"#
);

pub static TABLET: Icon = icon24!(
    "tablet",
    r#"<path d="M6.5 3h11A1.5 1.5 0 0 1 19 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5v-15A1.5 1.5 0 0 1 6.5 3z"></path><path d="M11 18h2"></path>"#
);

pub static MORE: Icon = icon24!(
    "more",
    r#"<path d="M12 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM19 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM5 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2z"></path>"#
);

pub static INFO: Icon = icon24!(
    "info",
    r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01"></path>"#
);

pub static CHEVRON_ROW: Icon = icon24!("chevron-row", r#"<path d="M9 18l6-6-6-6"></path>"#);

pub static PAUSE: Icon = icon24!("pause", r#"<path d="M6 4h4v16H6zM14 4h4v16h-4z"></path>"#);
pub static PLAY: Icon = icon24!("play", r#"<path d="M5 3l14 9-14 9V3z"></path>"#);
