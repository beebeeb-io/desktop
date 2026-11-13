//! Icons the Files frames add, copied from the board's inline `<svg>` (the gate compares the path data).

use rune::Icon;

macro_rules! line24 {
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

/// A menu's tick, and the "done" toast's.
pub static CHECK: Icon = line24!("check", r#"<path d="M20 6L9 17l-5-5"></path>"#);

/// The way out of a panel (the preview's, DW-0408): the board's `x`.
pub static CLOSE: Icon = line24!("close", r#"<path d="M18 6L6 18M6 6l12 12"></path>"#);

/// A file of no known kind (DW-0408b's Moodboard.sketch): the board's `file`.
pub static DOC_BLANK: Icon = line24!(
    "doc-blank",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7"></path>"#
);

/// Where a drop goes (DW-0481's tip): the board's `upload`.
pub static UPLOAD: Icon = line24!(
    "upload",
    r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12"></path>"#
);

/// An upload on its way (the PC column, DW-0426): the board's `arrow-up`.
pub static ARROW_UP: Icon = line24!("arrow-up", r#"<path d="M12 19V5M5 12l7-7 7 7"></path>"#);

/// Pause all (DW-0427): the board's `pause`.
pub static PAUSE: Icon = line24!("pause", r#"<path d="M6 4h4v16H6zM14 4h4v16h-4z"></path>"#);

/// The failed message's mark (DW-0109): the board's `alert`.
pub static ALERT: Icon = line24!(
    "alert",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 8v4.6"></path><path d="M12 15.9v.1"></path>"#
);
