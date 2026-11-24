//! The first run's icons: each body is the board's inline `<svg>` of DW-0201 to DW-0209, copied by script from
//! the goldens (the gate compares the path data). All are 24-unit stroke icons, 1.6 wide, round caps and joins.

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

pub static ADDING: Icon = icon24!(
    "adding",
    r#"<path d="M20 11a8 8 0 0 0-14.5-4.5L4 8"></path><path d="M4 4v4h4"></path><path d="M4 13a8 8 0 0 0 14.5 4.5L20 16"></path><path d="M20 20v-4h-4"></path>"#
);
pub static ADDED: Icon = icon24!("added", r#"<path d="M20 6L9 17l-5-5"></path>"#);
pub static ALERT: Icon = icon24!(
    "alert",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 8v4.6"></path><path d="M12 15.9v.1"></path>"#
);
pub static FACE: Icon = icon24!(
    "face",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M9 10v1"></path><path d="M15 10v1"></path><path d="M8.8 14.6a4.6 4.6 0 0 0 6.4 0"></path>"#
);
pub static PADLOCK: Icon = icon24!(
    "padlock",
    r#"<path d="M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2zM7 11V7a5 5 0 0 1 10 0v4"></path>"#
);
pub static FOLDER: Icon = icon24!(
    "folder",
    r#"<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>"#
);
pub static HOME: Icon = icon24!(
    "home",
    r#"<path d="M4 11l8-7 8 7"></path><path d="M6 9.5V20h12V9.5"></path><path d="M10 20v-5h4v5"></path>"#
);
pub static GALLERY: Icon = icon24!(
    "gallery",
    r#"<path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21"></path>"#
);
pub static DESKTOP: Icon = icon24!(
    "desktop",
    r#"<path d="M5 6.5A1.5 1.5 0 0 1 6.5 5h11A1.5 1.5 0 0 1 19 6.5V15H5z"></path><path d="M2.5 18.5h19"></path>"#
);
pub static DOCUMENT: Icon = icon24!(
    "document",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M16 13H8M16 17H8M10 9H8"></path>"#
);
pub static EXPLORER: Icon = icon24!(
    "file-explorer",
    r#"<path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2h9A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"></path><path d="M3 12.5h18"></path>"#
);
pub static BELL: Icon = icon24!(
    "bell",
    r#"<path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M13.73 21a2 2 0 0 1-3.46 0"></path>"#
);
pub static POWER: Icon = icon24!(
    "power",
    r#"<path d="M12 3v8"></path><path d="M6.6 6.6a7.5 7.5 0 1 0 10.8 0"></path>"#
);

/// The plate of the b as Windows shows it in File Explorer's pane (the board's `#mk-b` symbol without its mask).
pub static B_PLATE: Icon = Icon {
    name: "b-plate",
    view: 512.0,
    body: r#"<rect width="512" height="512" rx="118" fill="currentColor"></rect>"#,
};
