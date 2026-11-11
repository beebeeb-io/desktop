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

/// The failed message's mark (DW-0109): the board's `alert`.
pub static ALERT: Icon = line24!(
    "alert",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 8v4.6"></path><path d="M12 15.9v.1"></path>"#
);
