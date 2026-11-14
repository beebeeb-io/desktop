//! The office editor's icons: each body is the board's inline `<svg>`, copied by script from the golden
//! (the gate compares the path data). 24-unit stroke icons, 1.6 wide, round caps and joins.

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

pub static UNDO: Icon = icon24!(
    "of-undo",
    r#"<path d="M9 14L4 9l5-5"></path><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"></path>"#
);
pub static REDO: Icon = icon24!(
    "of-redo",
    r#"<path d="M15 14l5-5-5-5"></path><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"></path>"#
);
pub static TEXT_COLOUR: Icon = icon24!(
    "of-text-colour",
    r#"<path d="M6 18l6-14 6 14"></path><path d="M8.5 12.5h7"></path><path d="M4 21h16"></path>"#
);
pub static HIGHLIGHT: Icon = icon24!(
    "of-highlight",
    r#"<path d="M15 4l5 5-9 9H6v-5z"></path><path d="M4 21h16"></path>"#
);
pub static LIST_BULLETS: Icon = icon24!(
    "of-list-bullets",
    r#"<path d="M9 6h11"></path><path d="M9 12h11"></path><path d="M9 18h11"></path><path d="M4.5 6h.01"></path><path d="M4.5 12h.01"></path><path d="M4.5 18h.01"></path>"#
);
pub static LIST_NUMBERS: Icon = icon24!(
    "of-list-numbers",
    r#"<path d="M10 6h10"></path><path d="M10 12h10"></path><path d="M10 18h10"></path><path d="M4 5h1.5v4"></path><path d="M4 15.5a1.5 1.5 0 0 1 3 0c0 1.5-3 2.5-3 4h3"></path>"#
);
pub static ALIGN_LEFT: Icon = icon24!(
    "of-align-left",
    r#"<path d="M4 6h16"></path><path d="M4 10h10"></path><path d="M4 14h16"></path><path d="M4 18h10"></path>"#
);
pub static ALIGN_CENTRE: Icon = icon24!(
    "of-align-centre",
    r#"<path d="M4 6h16"></path><path d="M7 10h10"></path><path d="M4 14h16"></path><path d="M7 18h10"></path>"#
);
pub static ALIGN_RIGHT: Icon = icon24!(
    "of-align-right",
    r#"<path d="M4 6h16"></path><path d="M10 10h10"></path><path d="M4 14h16"></path><path d="M10 18h10"></path>"#
);
pub static JUSTIFY: Icon = icon24!(
    "of-justify",
    r#"<path d="M4 6h16"></path><path d="M4 10h16"></path><path d="M4 14h16"></path><path d="M4 18h16"></path>"#
);
pub static OUTDENT: Icon = icon24!(
    "of-outdent",
    r#"<path d="M11 6h9"></path><path d="M11 12h9"></path><path d="M11 18h9"></path><path d="M7 9l-3 3 3 3"></path>"#
);
pub static INDENT: Icon = icon24!(
    "of-indent",
    r#"<path d="M11 6h9"></path><path d="M11 12h9"></path><path d="M11 18h9"></path><path d="M4 9l3 3-3 3"></path>"#
);
pub static LINK: Icon = icon24!(
    "of-link",
    r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"></path>"#
);
pub static IMAGE: Icon = icon24!(
    "of-image",
    r#"<path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21"></path>"#
);
pub static TABLE: Icon = icon24!(
    "of-table",
    r#"<path d="M4.5 4h15A1.5 1.5 0 0 1 21 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5v-13A1.5 1.5 0 0 1 4.5 4z"></path><path d="M3 10h18"></path><path d="M3 15h18"></path><path d="M9 4v16"></path><path d="M15 4v16"></path>"#
);
pub static SHARE: Icon = icon24!(
    "of-share",
    r#"<path d="M18 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.59 13.51l6.83 3.98M15.41 6.51l-6.82 3.98"></path>"#
);
pub static PADLOCK: Icon = icon24!(
    "of-padlock",
    r#"<path d="M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2zM7 11V7a5 5 0 0 1 10 0v4"></path>"#
);
pub static FIND: Icon = icon24!(
    "of-find",
    r#"<path d="M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35"></path>"#
);
pub static CLOSE_PANE: Icon = icon24!("of-close-pane", r#"<path d="M15 5l-7 7 7 7"></path>"#);
pub static LESS: Icon = icon24!("of-less", r#"<path d="M5 12h14"></path>"#);
pub static MORE: Icon = icon24!("of-more", r#"<path d="M12 5v14M5 12h14"></path>"#);
pub static CARET: Icon = icon24!("of-caret", r#"<path d="M7 10l5 5 5-5"></path>"#);
