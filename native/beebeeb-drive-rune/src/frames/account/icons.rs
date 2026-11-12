//! The account group's icons. Nearly all are drawings the settings and sign-in groups already
//! have (one body each, re-exported here under the name the account panes use); the three
//! below are new, each body copied from the board's inline `<svg>` (the gate compares the path
//! data). All are 24-unit stroke icons, 1.6 wide, round caps and joins.

// Copied for the whole group in one pass; some belong to frames listed as not built in `FRAMES.md`.
#![allow(dead_code, unused_imports)]

use rune::Icon;

pub use crate::frames::signin::icons::LAPTOP as DEVICE_LAPTOP;
pub use crate::frames::signin::icons::RETRY as REFRESH;
pub use crate::theme::icons::LINK;
pub use crate::theme::icons_settings::{
    ACTIVITY, APP_LOCK, BACK, BELL, CHAT, CHECK, CHEV_RIGHT, CLOCK, COPY, DATABASE, DEVICE_PHONE, EXTERNAL, FOLDER,
    GEAR, GLOBE, KEY, MAIL, PADLOCK, PASSKEY, PEOPLE, SCAN, SHIELD, SHIELD_CHECK, SIGN_OUT, TRASH,
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

/// A command-line session (DW-A55).
pub static TERMINAL: Icon = icon24!(
    "terminal",
    r#"<path d="M4 6l5 5-5 5"></path><path d="M12 17h8"></path>"#
);
/// The menu on one passkey (DW-A73).
pub static MORE: Icon = icon24!(
    "more",
    r#"<path d="M5 12h.01"></path><path d="M12 12h.01"></path><path d="M19 12h.01"></path>"#
);
/// Password not in a breach (DW-A84).
pub static EYE_OFF: Icon = icon24!(
    "eye-off",
    r#"<path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24M1 1l22 22"></path>"#
);
