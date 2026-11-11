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
