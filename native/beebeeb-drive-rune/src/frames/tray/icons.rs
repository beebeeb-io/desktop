//! The tray group's icons: each body is the board's inline `<svg>` of the flyout frames
//! (DW-1202 to DW-1222), shape by shape, written by `gen_icons.py` from the goldens (the gate
//! compares the path data). A glyph the design system already has is re-exported under the
//! name the flyout uses it by.

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

/// Up to date (`st-icon of DW-1202-dark`).
pub static CHECK: Icon = icon24!("check", r#"<path d="M20 6L9 17l-5-5"></path>"#);

/// Syncing: two arrows turning (`st-icon of DW-1203-dark`).
pub static SYNC: Icon = icon24!(
    "sync",
    r#"<path d="M20 11a8 8 0 0 0-14.5-4.5L4 8"></path><path d="M4 4v4h4"></path><path d="M4 13a8 8 0 0 0 14.5 4.5L20 16"></path><path d="M20 20v-4h-4"></path>"#
);

/// Paused, and waiting on a metered connection (`st-icon of DW-1204-dark`).
pub static PAUSE: Icon = icon24!("pause", r#"<path d="M6 4h4v16H6zM14 4h4v16h-4z"></path>"#);

/// Needs a decision, stopped, storage full, too old to connect: the ! (`st-icon of DW-1205-dark`).
pub static ALERT: Icon = icon24!("alert", r#"<path d="M12 7v6"></path><path d="M12 16.8v.2"></path>"#);

/// Can't reach Beebeeb (`st-icon of DW-1206-dark`).
pub static CLOUD_OFF: Icon = icon24!(
    "cloud-off",
    r#"<path d="M3 3l18 18"></path><path d="M9 6.4A6 6 0 0 1 18 9.5a4 4 0 0 1 2.3 7.3"></path><path d="M16.5 18H7a4.5 4.5 0 0 1-1.7-8.66"></path>"#
);

/// Storage almost full (`st-icon of DW-1208-dark`).
pub static STORAGE: Icon = icon24!(
    "storage",
    r#"<path d="M4 6c0-1.66 3.58-3 8-3s8 1.34 8 3-3.58 3-8 3-8-1.34-8-3z"></path><path d="M4 6v12c0 1.66 3.58 3 8 3s8-1.34 8-3V6"></path><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"></path>"#
);

/// A new version (`st-icon of DW-1210-dark`).
pub static DOWNLOAD: Icon = icon24!(
    "download",
    r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"></path>"#
);

/// Locked (`st-icon of DW-1211-dark`).
pub use crate::theme::icons::LOCK;

/// No plan yet (`st-icon of DW-1213-dark`).
pub static INFO: Icon = icon24!(
    "info",
    r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01"></path>"#
);

/// No changes yet (`empty-icon of DW-1213-dark`).
pub use crate::theme::icons::VERSION_HISTORY as CLOCK;

/// Signed out: the password is needed (`st-icon of DW-1219-dark`).
pub static KEY: Icon = icon24!(
    "key",
    r#"<path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"></path>"#
);

/// File Explorer: a folder with its tab (`foot-1-icon of DW-1202-dark`).
pub static EXPLORER: Icon = icon24!(
    "explorer",
    r#"<path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2h9A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"></path><path d="M3 12.5h18"></path>"#
);

/// Settings (`foot-2-icon of DW-1202-dark`).
pub use crate::theme::icons::SETTINGS;

/// Upload speed (`st-speed-icon of DW-1203-dark`).
pub use crate::theme::icons::SORT_ASC as ARROW_UP;

/// A change made on this PC (`row-0-sub-icon of DW-1202-dark`).
pub static FROM_PC: Icon = icon24!(
    "from-pc",
    r#"<path d="M5 6.5A1.5 1.5 0 0 1 6.5 5h11A1.5 1.5 0 0 1 19 6.5V15H5z"></path><path d="M2.5 18.5h19"></path>"#
);

/// A change from a phone (`row-2-sub-icon of DW-1202-dark`).
pub static FROM_PHONE: Icon = icon24!(
    "from-phone",
    r#"<path d="M8.5 3h7A1.5 1.5 0 0 1 17 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 7 19.5v-15A1.5 1.5 0 0 1 8.5 3z"></path><path d="M11 18h2"></path>"#
);

/// A change from a tablet (`row-4-sub-icon of DW-1202-dark`).
pub static FROM_TABLET: Icon = icon24!(
    "from-tablet",
    r#"<path d="M6.5 3h11A1.5 1.5 0 0 1 19 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5v-15A1.5 1.5 0 0 1 6.5 3z"></path><path d="M11 18h2"></path>"#
);

/// A change by someone it is shared with (`row-7-sub-icon of DW-1202-dark`).
pub use crate::theme::icons::SHARED as FROM_PEOPLE;

/// A document (`row-0-icon of DW-1202-dark`).
pub use crate::theme::icons::DOC_GENERIC as FILE_DOC;

/// A spreadsheet (`row-1-icon of DW-1202-dark`).
pub use crate::theme::icons::DOC_SPREADSHEET as FILE_SHEET;

/// A presentation (`row-1-icon of DW-1203-dark`).
pub use crate::theme::icons::DOC_MINIMAL as FILE_DECK;

/// A video (`row-0-icon of DW-1207-dark`).
pub static FILE_VIDEO: Icon = icon24!(
    "file-video",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 15l5-3-5-3v6z"></path>"#
);

/// A sound recording (`row-4-icon of DW-1220-dark`).
pub static FILE_AUDIO: Icon = icon24!(
    "file-audio",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M11 15a2 2 0 1 1-4 0 2 2 0 0 1 4 0zM11 15v-4l4-1"></path>"#
);

/// An archive (`row-1-icon of DW-1221-dark`).
pub static FILE_ARCHIVE: Icon = icon24!(
    "file-archive",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M10 2v2M12 4v2M10 6v2M12 8v2M10 10v2M12 12v4h-2v-4"></path>"#
);

/// A file moved to Trash (`row-6-icon of DW-1202-dark`).
pub use crate::theme::icons::TRASH as FILE_TRASHED;

/// A photo or another image of the account, until the flyout has its thumbnail (the running
/// app's rows, `source::Tile::Image`): the design system's image glyph. No board frame draws it.
pub use crate::theme::icons::PHOTOS as FILE_IMAGE;

/// A folder (`row-7-icon of DW-1202-dark`).
pub use crate::theme::icons::FILES as FOLDER;

// The b as a template image (`#mk-b`): the rounded square with the b cut out of it by a mask
// (`#mk-bmask`). Rune's glyph atlas has no masks, so `view::b_glyph` stacks two one-colour
// layers: the plate, and the b over it in the colour of the surface behind.

/// The whole drawing, which the gate compares: the symbol's rect and the mask's path.
pub static B_TEMPLATE: Icon = Icon {
    name: "b-template",
    view: 512.0,
    body: r##"<rect width="512" height="512" rx="118" mask="url(#mk-bmask)" fill="currentColor"></rect><g fill="currentColor" transform="translate(150.818 382.726) scale(0.1723 -0.1723)"><path d="M754 -19Q663 -19 600.0 12.0Q537 43 497.5 88.5Q458 134 437 179H423V0H128V1490H428V930H437Q457 974 495.5 1021.0Q534 1068 597.5 1100.0Q661 1132 756 1132Q880 1132 983.0 1068.0Q1086 1004 1147.5 876.0Q1209 748 1209 557Q1209 371 1149.0 242.5Q1089 114 986.0 47.5Q883 -19 754 -19ZM662 222Q779 222 840.5 316.0Q902 410 902 558Q902 705 841.0 798.5Q780 892 662 892Q546 892 483.5 801.0Q421 710 421 558Q421 406 484.0 314.0Q547 222 662 222Z"></path></g>"##,
};

/// The plate: the rounded square.
pub static B_PLATE: Icon = Icon {
    name: "b-plate",
    view: 512.0,
    body: r#"<rect width="512" height="512" rx="118" fill="currentColor"></rect>"#,
};

/// The b that the mask cuts out of the plate.
pub static B_CUT: Icon = Icon {
    name: "b-cut",
    view: 512.0,
    body: r#"<g fill="currentColor" transform="translate(150.818 382.726) scale(0.1723 -0.1723)"><path d="M754 -19Q663 -19 600.0 12.0Q537 43 497.5 88.5Q458 134 437 179H423V0H128V1490H428V930H437Q457 974 495.5 1021.0Q534 1068 597.5 1100.0Q661 1132 756 1132Q880 1132 983.0 1068.0Q1086 1004 1147.5 876.0Q1209 748 1209 557Q1209 371 1149.0 242.5Q1089 114 986.0 47.5Q883 -19 754 -19ZM662 222Q779 222 840.5 316.0Q902 410 902 558Q902 705 841.0 798.5Q780 892 662 892Q546 892 483.5 801.0Q421 710 421 558Q421 406 484.0 314.0Q547 222 662 222Z"></path></g>"#,
};
