//! The sign-in group's icons: each body is the board's inline `<svg>` of the `.au-mac` frames, copied by
//! script (the gate compares the path data). All are 24-unit stroke icons, 1.6 wide, round caps and joins.

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

pub static LOCK: Icon = icon24!(
    "lock",
    r#"<path d="M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2zM7 11V7a5 5 0 0 1 10 0v4"></path>"#
);
pub static PEOPLE: Icon = icon24!(
    "people",
    r#"<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"></path>"#
);
pub static GLOBE: Icon = icon24!(
    "globe",
    r#"<path d="M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9M12 3c-2.5 2.6-3.8 5.6-3.8 9s1.3 6.4 3.8 9"></path>"#
);
pub static FINGERPRINT: Icon = icon24!(
    "fingerprint",
    r#"<path d="M4 12a8 8 0 0 1 16 0"></path><path d="M7 14a5 5 0 0 1 10 0v2"></path><path d="M10 16v2c0 1-.3 2-1 3"></path><path d="M13 14v4c0 1.2-.4 2.4-1 3.5"></path><path d="M16.5 17c0 1.5-.3 2.8-.8 4"></path>"#
);
pub static PHONE: Icon = icon24!(
    "phone",
    r#"<path d="M8 3h8a1.5 1.5 0 0 1 1.5 1.5v15A1.5 1.5 0 0 1 16 21H8a1.5 1.5 0 0 1-1.5-1.5v-15A1.5 1.5 0 0 1 8 3z"></path><path d="M11 18h2"></path>"#
);
pub static MAIL: Icon = icon24!(
    "mail",
    r#"<path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2zM22 6l-10 7L2 6"></path>"#
);
pub static EYE: Icon = icon24!(
    "eye",
    r#"<path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z"></path>"#
);
pub static PASSKEY: Icon = icon24!(
    "passkey",
    r#"<path d="M8.5 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z"></path><path d="M2.5 20v-1a4.5 4.5 0 0 1 4.5-4.5h3"></path><path d="M17 12.5a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5z"></path><path d="M17 17.5V21"></path><path d="M17 19.5h2"></path>"#
);
pub static BACK: Icon = icon24!("back", r#"<path d="M15 5l-7 7 7 7"></path>"#);
pub static INFO: Icon = icon24!(
    "info",
    r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01"></path>"#
);
pub static LAPTOP: Icon = icon24!(
    "laptop",
    r#"<path d="M5.5 5h13A1.5 1.5 0 0 1 20 6.5V15H4V6.5A1.5 1.5 0 0 1 5.5 5z"></path><path d="M2.5 19h19"></path>"#
);
pub static CHECK: Icon = icon24!("check", r#"<path d="M20 6L9 17l-5-5"></path>"#);
pub static CROSS: Icon = icon24!("cross", r#"<path d="M18 6L6 18M6 6l12 12"></path>"#);
pub static RETRY: Icon = icon24!(
    "retry",
    r#"<path d="M20 11a8 8 0 1 0-2.3 5.7"></path><path d="M20 5v6h-6"></path>"#
);
pub static OFFLINE: Icon = icon24!(
    "offline",
    r#"<path d="M3 3l18 18"></path><path d="M7.2 8A5.5 5.5 0 0 0 7 19h10.5"></path><path d="M19.5 17.5A4 4 0 0 0 18 10a6 6 0 0 0-8.4-3.2"></path>"#
);
pub static KEY: Icon = icon24!(
    "key",
    r#"<path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"></path>"#
);
pub static CLOCK: Icon = icon24!(
    "clock",
    r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2"></path>"#
);
pub static CHEVRON: Icon = icon24!("chevron", r#"<path d="M9 18l6-6-6-6"></path>"#);
pub static TRASH: Icon = icon24!(
    "trash",
    r#"<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>"#
);
pub static SHIELD: Icon = icon24!(
    "shield",
    r#"<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>"#
);
