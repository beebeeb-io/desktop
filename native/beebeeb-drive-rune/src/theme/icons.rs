//! SVG icons extracted from the design board's DW-0401 Windows frame.
//!
//! Each icon is a `&'static Icon` with the SVG body from the board's inline `<svg>` elements.
//! Rune renders these through `icon(&ICON, size)` or as inline spans.
//!
//! DO NOT EDIT BY HAND — extracted by Playwright from `design/desktop-v2/board.html`.

#![allow(dead_code)] // Icons are extracted; not all are used yet.

use rune::Icon;

// ---------------------------------------------------------------------------
// Caption bar
// ---------------------------------------------------------------------------

/// Windows caption: minimize (horizontal line).
pub static CAPTION_MINIMIZE: Icon = Icon {
    name: "caption-minimize",
    view: 10.0,
    body: r#"<path d="M0 5.5h10"></path>"#,
};

/// Windows caption: maximize (rectangle).
pub static CAPTION_MAXIMIZE: Icon = Icon {
    name: "caption-maximize",
    view: 10.0,
    body: r#"<rect x=".5" y=".5" width="9" height="9" rx="1.5"></rect>"#,
};

/// Windows caption: close (X).
pub static CAPTION_CLOSE: Icon = Icon {
    name: "caption-close",
    view: 10.0,
    body: r#"<path d="M.5.5l9 9M9.5.5l-9 9"></path>"#,
};

// ---------------------------------------------------------------------------
// Sidebar sections
// ---------------------------------------------------------------------------

/// Sidebar: Files (folder).
pub static FILES: Icon = Icon {
    name: "files",
    view: 24.0,
    body: r#"<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>"#,
};

/// Sidebar: Shared (users).
pub static SHARED: Icon = Icon {
    name: "shared",
    view: 24.0,
    body: r#"<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"></path>"#,
};

/// Sidebar: Photos (image).
pub static PHOTOS: Icon = Icon {
    name: "photos",
    view: 24.0,
    body: r#"<path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21"></path>"#,
};

/// Sidebar: Backups (archive box).
pub static BACKUPS: Icon = Icon {
    name: "backups",
    view: 24.0,
    body: r#"<path d="M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5V9H4z"></path><path d="M5 9v9.5A1.5 1.5 0 0 0 6.5 20h11a1.5 1.5 0 0 0 1.5-1.5V9"></path><path d="M10 13h4"></path>"#,
};

/// Sidebar: Trash (trash can).
pub static TRASH: Icon = Icon {
    name: "trash",
    view: 24.0,
    body: r#"<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>"#,
};

/// Sidebar: Sync history (clock).
pub static SYNC_HISTORY: Icon = Icon {
    name: "sync-history",
    view: 24.0,
    body: r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2"></path>"#,
};

/// Sidebar: Available offline (download in circle).
pub static AVAILABLE_OFFLINE: Icon = Icon {
    name: "available-offline",
    view: 24.0,
    body: r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 7.8v7.4"></path><path d="M8.8 12l3.2 3.2 3.2-3.2"></path>"#,
};

/// Settings gear.
pub static SETTINGS: Icon = Icon {
    name: "settings",
    view: 24.0,
    body: r#"<path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"></path>"#,
};

// ---------------------------------------------------------------------------
// Toolbar / Navigation
// ---------------------------------------------------------------------------

/// Chevron left (back).
pub static CHEVRON_LEFT: Icon = Icon {
    name: "chevron-left",
    view: 24.0,
    body: r#"<path d="M15 5l-7 7 7 7"></path>"#,
};

/// Chevron right (forward).
pub static CHEVRON_RIGHT: Icon = Icon {
    name: "chevron-right",
    view: 24.0,
    body: r#"<path d="M9 5l7 7-7 7"></path>"#,
};

/// List view (horizontal lines with dots).
pub static VIEW_LIST: Icon = Icon {
    name: "view-list",
    view: 24.0,
    body: r#"<path d="M9 6h11"></path><path d="M9 12h11"></path><path d="M9 18h11"></path><path d="M4.5 6h.01"></path><path d="M4.5 12h.01"></path><path d="M4.5 18h.01"></path>"#,
};

/// Grid view (four squares).
pub static VIEW_GRID: Icon = Icon {
    name: "view-grid",
    view: 24.0,
    body: r#"<path d="M5.5 4h4A1.5 1.5 0 0 1 11 5.5v4A1.5 1.5 0 0 1 9.5 11h-4A1.5 1.5 0 0 1 4 9.5v-4A1.5 1.5 0 0 1 5.5 4z"></path><path d="M14.5 4h4A1.5 1.5 0 0 1 20 5.5v4a1.5 1.5 0 0 1-1.5 1.5h-4A1.5 1.5 0 0 1 13 9.5v-4A1.5 1.5 0 0 1 14.5 4z"></path><path d="M5.5 13h4a1.5 1.5 0 0 1 1.5 1.5v4A1.5 1.5 0 0 1 9.5 20h-4A1.5 1.5 0 0 1 4 18.5v-4A1.5 1.5 0 0 1 5.5 13z"></path><path d="M14.5 13h4a1.5 1.5 0 0 1 1.5 1.5v4a1.5 1.5 0 0 1-1.5 1.5h-4a1.5 1.5 0 0 1-1.5-1.5v-4a1.5 1.5 0 0 1 1.5-1.5z"></path>"#,
};

/// Sort (up/down arrows).
pub static SORT: Icon = Icon {
    name: "sort",
    view: 24.0,
    body: r#"<path d="M7 4v16"></path><path d="M3.5 16.5 7 20l3.5-3.5"></path><path d="M17 20V4"></path><path d="M13.5 7.5 17 4l3.5 3.5"></path>"#,
};

/// Chevron down (dropdown caret).
pub static CHEVRON_DOWN: Icon = Icon {
    name: "chevron-down",
    view: 24.0,
    body: r#"<path d="M7 10l5 5 5-5"></path>"#,
};

/// Details panel toggle (rectangle with vertical line).
pub static PANEL_TOGGLE: Icon = Icon {
    name: "panel-toggle",
    view: 24.0,
    body: r#"<path d="M4.5 4h15A1.5 1.5 0 0 1 21 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5v-13A1.5 1.5 0 0 1 4.5 4z"></path><path d="M15 4v16"></path>"#,
};

/// Plus (new).
pub static PLUS: Icon = Icon {
    name: "plus",
    view: 24.0,
    body: r#"<path d="M12 5v14M5 12h14"></path>"#,
};

/// Search (magnifying glass).
pub static SEARCH: Icon = Icon {
    name: "search",
    view: 24.0,
    body: r#"<path d="M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35"></path>"#,
};

// ---------------------------------------------------------------------------
// File list / status
// ---------------------------------------------------------------------------

/// Lock (padlock, for protected folders like Backups).
pub static LOCK: Icon = Icon {
    name: "lock",
    view: 24.0,
    body: r#"<path d="M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2zM7 11V7a5 5 0 0 1 10 0v4"></path>"#,
};

/// Cloud (Online only status).
pub static CLOUD: Icon = Icon {
    name: "cloud",
    view: 24.0,
    body: r#"<path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"></path>"#,
};

/// Checkmark in circle (On this PC status).
pub static CHECK_CIRCLE: Icon = Icon {
    name: "check-circle",
    view: 24.0,
    body: r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M8.5 12.2l2.4 2.4 4.6-4.8"></path>"#,
};

/// Link chain.
pub static LINK: Icon = Icon {
    name: "link",
    view: 24.0,
    body: r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"></path>"#,
};

/// Share nodes.
pub static SHARE: Icon = Icon {
    name: "share",
    view: 24.0,
    body: r#"<path d="M18 8a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM6 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM18 22a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM8.59 13.51l6.83 3.98M15.41 6.51l-6.82 3.98"></path>"#,
};

/// Version history (clock with arrow).
pub static VERSION_HISTORY: Icon = Icon {
    name: "version-history",
    view: 24.0,
    body: r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2"></path>"#,
};

// ---------------------------------------------------------------------------
// File-type document icons
// ---------------------------------------------------------------------------

/// Generic document (lines).
pub static DOC_GENERIC: Icon = Icon {
    name: "doc-generic",
    view: 24.0,
    body: r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M16 13H8M16 17H8M10 9H8"></path>"#,
};

/// Spreadsheet document (grid cells).
pub static DOC_SPREADSHEET: Icon = Icon {
    name: "doc-spreadsheet",
    view: 24.0,
    body: r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M8 13h2v2H8zM14 13h2v2h-2zM8 17h2v2H8zM14 17h2v2h-2z"></path>"#,
};

/// Minimal document variant (short lines).
pub static DOC_MINIMAL: Icon = Icon {
    name: "doc-minimal",
    view: 24.0,
    body: r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M8 13h8M8 17h5"></path>"#,
};

/// Sort arrow up (ascending, used in column header).
pub static SORT_ASC: Icon = Icon {
    name: "sort-asc",
    view: 24.0,
    body: r#"<path d="M12 19V5M5 12l7-7 7 7"></path>"#,
};

/// Green dot (sync status: up to date).
pub static DOT_GREEN: Icon = Icon {
    name: "dot-green",
    view: 24.0,
    body: r#"<circle cx="12" cy="12" r="4" fill="currentColor"></circle>"#,
};
