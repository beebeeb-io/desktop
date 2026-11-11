//! Icons the global elements add (banners, the loading strip, a preview that cannot open), copied
//! from the board's inline `<svg>` (the gate compares the path data). 24-unit line icons, 1.6 wide.

use rune::Icon;

// The same drawings as the Files frames already have: one body each.
pub use crate::theme::icons_files::ALERT;

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

/// No connection: a cloud with a line through it (DW-0101, DW-0121).
pub static OFFLINE: Icon = line24!(
    "offline",
    r#"<path d="M3 3l18 18"></path><path d="M9 6.4A6 6 0 0 1 18 9.5a4 4 0 0 1 2.3 7.3"></path><path d="M16.5 18H7a4.5 4.5 0 0 1-1.7-8.66"></path>"#
);
/// Storage (DW-0103, DW-0104).
pub static STORAGE: Icon = line24!(
    "storage",
    r#"<path d="M4 6c0-1.66 3.58-3 8-3s8 1.34 8 3-3.58 3-8 3-8-1.34-8-3z"></path><path d="M4 6v12c0 1.66 3.58 3 8 3s8-1.34 8-3V6"></path><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"></path>"#
);
/// The session ended: a key (DW-0105).
pub static KEY: Icon = line24!(
    "key",
    r#"<path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"></path>"#
);
/// Dismiss a banner, close a panel.
pub static CLOSE: Icon = line24!("close", r#"<path d="M18 6L6 18M6 6l12 12"></path>"#);
