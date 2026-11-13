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

/// The failed message's mark (DW-0109): the board's `alert`.
pub static ALERT: Icon = line24!(
    "alert",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 8v4.6"></path><path d="M12 15.9v.1"></path>"#
);
