//! The icons of the photo, audio, PDF, table and archive viewers: each body is the board's inline
//! `<svg>` of its frame, copied by script from the golden (the gate compares the path data).

use rune::Icon;

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

pub static SHARE: Icon = icon24!(
    "share",
    r#"<path d="M18 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.59 13.51l6.83 3.98M15.41 6.51l-6.82 3.98"></path>"#
);
pub static OFFLINE: Icon = icon24!(
    "offline",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 7.8v7.4"></path><path d="M8.8 12l3.2 3.2 3.2-3.2"></path>"#
);
pub static TRASH: Icon = icon24!(
    "trash",
    r#"<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>"#
);
pub static BACK: Icon = icon24!("back", r#"<path d="M15 5l-7 7 7 7"></path>"#);
pub static FWD: Icon = icon24!("fwd", r#"<path d="M9 5l7 7-7 7"></path>"#);
pub static PAUSE: Icon = icon24!("pause", r#"<path d="M6 4h4v16H6zM14 4h4v16h-4z"></path>"#);
pub static PLAY: Icon = icon24!("play", r#"<path d="M5 3l14 9-14 9V3z"></path>"#);
pub static FILE_AUDIO: Icon = icon24!(
    "file-audio",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M11 15a2 2 0 1 1-4 0 2 2 0 0 1 4 0zM11 15v-4l4-1"></path>"#
);
pub static MINUS: Icon = icon24!("minus", r#"<path d="M5 12h14"></path>"#);
pub static PLUS_SMALL: Icon = icon24!("plus-small", r#"<path d="M12 5v14M5 12h14"></path>"#);
pub static FIND: Icon = icon24!(
    "find",
    r#"<path d="M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35"></path>"#
);
pub static CHEV_DOWN: Icon = icon24!("chev-down", r#"<path d="M7 10l5 5 5-5"></path>"#);
pub static FOLDER: Icon = icon24!(
    "folder",
    r#"<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>"#
);
pub static FILE_TEXT: Icon = icon24!(
    "file-text",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M16 13H8M16 17H8M10 9H8"></path>"#
);
pub static FILE_SHEET: Icon = icon24!(
    "file-sheet",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M8 13h2v2H8zM14 13h2v2h-2zM8 17h2v2H8zM14 17h2v2h-2z"></path>"#
);
pub static FILE_ARCHIVE: Icon = icon24!(
    "file-archive",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 2v2M12 4v2M10 6v2M12 8v2M10 10v2M12 12v4h-2v-4"></path>"#
);
