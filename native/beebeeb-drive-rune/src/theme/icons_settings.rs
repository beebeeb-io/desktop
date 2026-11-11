//! The settings panes' icons, copied from the board's inline `<svg>` elements (the frame gate
//! compares the path data). GENERATED from the board by the lane's extraction script: every
//! distinct glyph inside `.mk-set` of the 50 Windows settings frames. All are 24-unit line
//! icons: `fill="none" stroke="currentColor" stroke-width="1.6"`, round caps and joins.

#![allow(dead_code)] // The whole set of the settings group; a pane uses the part it draws.

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

/// First drawn in DW-0701 (beside "Back to Drive Settings").
pub static BACK: Icon = line24!("back", r#"<path d="M15 5l-7 7 7 7"></path>"#);
/// First drawn in DW-0701 (beside "General").
pub static GEAR: Icon = line24!(
    "gear",
    r#"<path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"></path>"#
);
/// First drawn in DW-0701 (beside "Account").
pub static PEOPLE: Icon = line24!(
    "people",
    r#"<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"></path>"#
);
/// First drawn in DW-0701 (beside "Security").
pub static SHIELD: Icon = line24!(
    "shield",
    r#"<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>"#
);
/// First drawn in DW-0701 (beside "Devices and sessions").
pub static LAPTOP: Icon = line24!(
    "laptop",
    r#"<path d="M5 6.5A1.5 1.5 0 0 1 6.5 5h11A1.5 1.5 0 0 1 19 6.5V15H5z"></path><path d="M2.5 18.5h19"></path>"#
);
/// First drawn in DW-0701 (beside "Notifications Drive").
pub static BELL: Icon = line24!(
    "bell",
    r#"<path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M13.73 21a2 2 0 0 1-3.46 0"></path>"#
);
/// First drawn in DW-0701 (beside "Storage").
pub static CHEV_RIGHT: Icon = line24!("chev-right", r#"<path d="M9 18l6-6-6-6"></path>"#);
/// First drawn in DW-0701 (beside "Theme Follows Windows. Syste").
pub static THEME: Icon = line24!(
    "theme",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 3.5v17"></path><path d="M12 7.5h4.6"></path><path d="M12 11.5h6.9"></path><path d="M12 15.5h5.8"></path>"#
);
/// First drawn in DW-0701 (beside "Help").
pub static CARET_DOWN: Icon = line24!("caret-down", r#"<path d="M7 10l5 5 5-5"></path>"#);
/// First drawn in DW-0701 (beside "Show the guide again").
pub static INFO: Icon = line24!(
    "info",
    r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01"></path>"#
);
/// First drawn in DW-0701 (beside "Help and report a problem").
pub static CHAT: Icon = line24!(
    "chat",
    r#"<path d="M21 11.5a8.4 8.4 0 0 1-12.3 7.4L3 21l2.1-5.6A8.4 8.4 0 1 1 21 11.5z"></path>"#
);
/// First drawn in DW-0701 (beside "Diagnostics App activity and").
pub static ACTIVITY: Icon = line24!("activity", r#"<path d="M22 12h-4l-3 9L9 3l-3 9H2"></path>"#);
/// First drawn in DW-0701 (beside "Export my data").
pub static DOWNLOAD_TRAY: Icon = line24!(
    "download-tray",
    r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3"></path>"#
);
/// First drawn in DW-0701 (beside "Storage location Europe").
pub static GLOBE: Icon = line24!(
    "globe",
    r#"<path d="M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9M12 3c-2.5 2.6-3.8 5.6-3.8 9s1.3 6.4 3.8 9"></path>"#
);
/// First drawn in DW-0701 (beside "Terms and privacy policy").
pub static DOCUMENT: Icon = line24!(
    "document",
    r#"<path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM13 2v7h7M16 13H8M16 17H8M10 9H8"></path>"#
);
/// First drawn in DW-0701 (beside "").
pub static EXTERNAL: Icon = line24!(
    "external",
    r#"<path d="M14 4h6v6"></path><path d="M20 4l-9 9"></path><path d="M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10"></path>"#
);
/// First drawn in DW-0708 (beside "Drive plan · 1 TB 240 GB use").
pub static DATABASE: Icon = line24!(
    "database",
    r#"<path d="M4 6c0-1.66 3.58-3 8-3s8 1.34 8 3-3.58 3-8 3-8-1.34-8-3z"></path><path d="M4 6v12c0 1.66 3.58 3 8 3s8-1.34 8-3V6"></path><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"></path>"#
);
/// First drawn in DW-0708 (beside "Drive Signed in").
pub static FOLDER: Icon = line24!(
    "folder",
    r#"<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>"#
);
/// First drawn in DW-0708 (beside "Pass Not signed in on this P").
pub static KEY: Icon = line24!(
    "key",
    r#"<path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"></path>"#
);
/// First drawn in DW-0712 (beside "Photos and videos 216 GB").
pub static IMAGE: Icon = line24!(
    "image",
    r#"<path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21"></path>"#
);
/// First drawn in DW-0712 (beside "Thumbnails and previews 3.1").
pub static GRID: Icon = line24!(
    "grid",
    r#"<path d="M5.5 4h4A1.5 1.5 0 0 1 11 5.5v4A1.5 1.5 0 0 1 9.5 11h-4A1.5 1.5 0 0 1 4 9.5v-4A1.5 1.5 0 0 1 5.5 4z"></path><path d="M14.5 4h4A1.5 1.5 0 0 1 20 5.5v4a1.5 1.5 0 0 1-1.5 1.5h-4A1.5 1.5 0 0 1 13 9.5v-4A1.5 1.5 0 0 1 14.5 4z"></path><path d="M5.5 13h4a1.5 1.5 0 0 1 1.5 1.5v4A1.5 1.5 0 0 1 9.5 20h-4A1.5 1.5 0 0 1 4 18.5v-4A1.5 1.5 0 0 1 5.5 13z"></path><path d="M14.5 13h4a1.5 1.5 0 0 1 1.5 1.5v4a1.5 1.5 0 0 1-1.5 1.5h-4a1.5 1.5 0 0 1-1.5-1.5v-4a1.5 1.5 0 0 1 1.5-1.5z"></path>"#
);
/// First drawn in DW-0712 (beside "Older versions 1.2 GB").
pub static CLOCK: Icon = line24!(
    "clock",
    r#"<path d="M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2"></path>"#
);
/// First drawn in DW-0712 (beside "Trash 240 MB").
pub static TRASH: Icon = line24!(
    "trash",
    r#"<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>"#
);
/// First drawn in DW-0712 (beside "Available offline 1.4 GB").
pub static OFFLINE_PIN: Icon = line24!(
    "offline-pin",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 7.8v7.4"></path><path d="M8.8 12l3.2 3.2 3.2-3.2"></path>"#
);
/// First drawn in DW-0716 (beside "Security checkup · 7 of 7 Ap").
pub static SHIELD_CHECK: Icon = line24!(
    "shield-check",
    r#"<path d="M12 3l7 3v5.5c0 4.4-3 7.6-7 9.5-4-1.9-7-5.1-7-9.5V6z"></path><path d="M9 12l2 2 4-4"></path>"#
);
/// First drawn in DW-0716 (beside "Lock Beebeeb Windows Hello ·").
pub static APP_LOCK: Icon = line24!(
    "app-lock",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M9 10v1"></path><path d="M15 10v1"></path><path d="M8.8 14.6a4.6 4.6 0 0 0 6.4 0"></path>"#
);
/// First drawn in DW-0716 (beside "Password 3 weeks ago").
pub static PADLOCK: Icon = line24!(
    "padlock",
    r#"<path d="M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2zM7 11V7a5 5 0 0 1 10 0v4"></path>"#
);
/// First drawn in DW-0716 (beside "Passkeys 2").
pub static PASSKEY: Icon = line24!(
    "passkey",
    r#"<path d="M8.5 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z"></path><path d="M2.5 20v-1a4.5 4.5 0 0 1 4.5-4.5h3"></path><path d="M17 12.5a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5z"></path><path d="M17 17.5V21"></path><path d="M17 19.5h2"></path>"#
);
/// First drawn in DW-0716 (beside "Devices and sessions 4").
pub static DEVICE_PHONE: Icon = line24!(
    "device-phone",
    r#"<path d="M8 3h8a1.5 1.5 0 0 1 1.5 1.5v15A1.5 1.5 0 0 1 16 21H8a1.5 1.5 0 0 1-1.5-1.5v-15A1.5 1.5 0 0 1 8 3z"></path><path d="M11 18h2"></path>"#
);
/// First drawn in DW-0716 (beside "Approve a sign-in Let anothe").
pub static SCAN: Icon = line24!(
    "scan",
    r#"<path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2M8 12h8M12 8v8"></path>"#
);
/// First drawn in DW-0716 (beside "Sign out of Drive").
pub static SIGN_OUT: Icon = line24!(
    "sign-out",
    r#"<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"></path><path d="M16 17l5-5-5-5"></path><path d="M21 12H9"></path>"#
);
/// First drawn in DW-0716 (beside "5 min 30 min 60 min").
pub static CHECK: Icon = line24!("check", r#"<path d="M20 6L9 17l-5-5"></path>"#);
/// First drawn in DW-0744 (beside "Rebuild previews Clearing th").
pub static REBUILD: Icon = line24!(
    "rebuild",
    r#"<path d="M20 12a8 8 0 1 1-2.3-5.6L20 8.5"></path><path d="M20 4v4.5h-4.5"></path>"#
);
/// First drawn in DW-0746 (beside "Beebeeb in File Explorer In").
pub static FOLDER_TAB: Icon = line24!(
    "folder-tab",
    r#"<path d="M3 7.5A1.5 1.5 0 0 1 4.5 6h4l2 2h9A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"></path><path d="M3 12.5h18"></path>"#
);
/// First drawn in DW-0746 (beside "Show sync status on files").
pub static SYNC_BADGE: Icon = line24!(
    "sync-badge",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M8.5 12.2l2.4 2.4 4.6-4.8"></path>"#
);
/// First drawn in DW-0746 (beside "Status Up to date · last cha").
pub static SYNC: Icon = line24!(
    "sync",
    r#"<path d="M20 11a8 8 0 0 0-14.5-4.5L4 8"></path><path d="M4 4v4h4"></path><path d="M4 13a8 8 0 0 0 14.5 4.5L20 16"></path><path d="M20 20v-4h-4"></path>"#
);
/// First drawn in DW-0746 (beside "Open Beebeeb when you log in").
pub static POWER: Icon = line24!(
    "power",
    r#"<path d="M12 3v8"></path><path d="M6.6 6.6a7.5 7.5 0 1 0 10.8 0"></path>"#
);
/// First drawn in DW-0746 (beside "Limit Beebeeb removes the ol").
pub static SLIDERS: Icon = line24!(
    "sliders",
    r#"<path d="M4 7h9"></path><path d="M17 7h3"></path><path d="M15 5v4"></path><path d="M4 17h3"></path><path d="M11 17h9"></path><path d="M9 15v4"></path>"#
);
/// First drawn in DW-0746d (beside "Couldn't repair Beebeeb in F").
pub static WARN_CIRCLE: Icon = line24!(
    "warn-circle",
    r#"<path d="M12 3.5a8.5 8.5 0 1 1 0 17 8.5 8.5 0 0 1 0-17z"></path><path d="M12 8v4.6"></path><path d="M12 15.9v.1"></path>"#
);
/// First drawn in DW-0746e (beside "Status Waiting · this PC is").
pub static WIFI: Icon = line24!(
    "wifi",
    r#"<path d="M2 8.8a15 15 0 0 1 20 0"></path><path d="M5 12.5a10 10 0 0 1 14 0"></path><path d="M8.5 16.2a5 5 0 0 1 7 0"></path><path d="M12 20h.01"></path>"#
);
/// First drawn in DW-0750 (beside "Recent notifications Sign-in").
pub static INBOX: Icon = line24!(
    "inbox",
    r#"<path d="M4 13l2.5-7.5A1.5 1.5 0 0 1 7.9 4.5h8.2a1.5 1.5 0 0 1 1.4 1L20 13"></path><path d="M4 13v5.5A1.5 1.5 0 0 0 5.5 20h13a1.5 1.5 0 0 0 1.5-1.5V13"></path><path d="M4 13h4.5l1.5 2.5h4l1.5-2.5H20"></path>"#
);
/// First drawn in DW-0751 (beside "Copy Version 0.9.1 (311)").
pub static COPY: Icon = line24!(
    "copy",
    r#"<path d="M20 9h-9a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-9a2 2 0 0 0-2-2zM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>"#
);
/// First drawn in DW-0782 (beside "Paths, file and folder names").
pub static CLOSE_X: Icon = line24!("close-x", r#"<path d="M18 6L6 18M6 6l12 12"></path>"#);
/// First drawn in DW-0782 (beside "Sent from your own address S").
pub static MAIL: Icon = line24!(
    "mail",
    r#"<path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2zM22 6l-10 7L2 6"></path>"#
);
/// First drawn in DW-0783d (beside "Nothing is stored yet Once y").
pub static CLOUD: Icon = line24!(
    "cloud",
    r#"<path d="M18 10h-1.26A8 8 0 1 0 9 20h9a5 5 0 0 0 0-10z"></path>"#
);
/// First drawn in DW-0784 (beside "").
pub static CHEV_RIGHT_TREE: Icon = line24!("chev-right-tree", r#"<path d="M9 5l7 7-7 7"></path>"#);
/// First drawn in DW-0785 (beside "Upload 3.2 MB/s Encrypting a").
pub static ARROW_UP: Icon = line24!(
    "arrow-up",
    r#"<path d="M12 19V5"></path><path d="M6.5 10.5 12 5l5.5 5.5"></path>"#
);
/// First drawn in DW-0785 (beside "Download 410 KB/s 2 files fr").
pub static ARROW_DOWN: Icon = line24!(
    "arrow-down",
    r#"<path d="M12 5v14"></path><path d="M6.5 13.5 12 19l5.5-5.5"></path>"#
);
/// First drawn in DW-0785 (beside "Anna's iPhone · iOS sending").
pub static PHONE: Icon = line24!(
    "phone",
    r#"<path d="M8.5 3h7A1.5 1.5 0 0 1 17 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 7 19.5v-15A1.5 1.5 0 0 1 8.5 3z"></path><path d="M11 18h2"></path>"#
);
/// First drawn in DW-0785 (beside "Anna's iPad · iPadOS last re").
pub static TABLET: Icon = line24!(
    "tablet",
    r#"<path d="M6.5 3h11A1.5 1.5 0 0 1 19 4.5v15a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5v-15A1.5 1.5 0 0 1 6.5 3z"></path><path d="M11 18h2"></path>"#
);
/// First drawn in DW-0785 (beside "Test connection speed 5 ping").
pub static GAUGE: Icon = line24!(
    "gauge",
    r#"<path d="M4.6 17.5a8.5 8.5 0 1 1 14.8 0"></path><path d="M12 13.5l3.6-4.2"></path><path d="M12 13.4v.2"></path>"#
);
/// First drawn in DW-0788 (beside "Germany").
pub static SERVER: Icon = line24!(
    "server",
    r#"<path d="M5.5 4h13A1.5 1.5 0 0 1 20 5.5v4a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 9.5v-4A1.5 1.5 0 0 1 5.5 4z"></path><path d="M5.5 13h13a1.5 1.5 0 0 1 1.5 1.5v4a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-4A1.5 1.5 0 0 1 5.5 13z"></path><path d="M8 7.5h.01"></path><path d="M8 16.5h.01"></path>"#
);
/// First drawn in DW-0790 (beside "CPU").
pub static CPU: Icon = line24!(
    "cpu",
    r#"<path d="M8 6h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z"></path><path d="M10 10h4v4h-4z"></path><path d="M9.5 3v3"></path><path d="M14.5 3v3"></path><path d="M9.5 18v3"></path><path d="M14.5 18v3"></path><path d="M3 9.5h3"></path><path d="M3 14.5h3"></path><path d="M18 9.5h3"></path><path d="M18 14.5h3"></path>"#
);
/// First drawn in DW-0790 (beside "Memory").
pub static MEMORY: Icon = line24!(
    "memory",
    r#"<path d="M3.5 7.5h17v8h-17z"></path><path d="M7.5 10.5v2"></path><path d="M12 10.5v2"></path><path d="M16.5 10.5v2"></path><path d="M6 15.5v3"></path><path d="M18 15.5v3"></path>"#
);
/// First drawn in DW-0790 (beside "Export a support bundle A fi").
pub static BOX: Icon = line24!(
    "box",
    r#"<path d="M4 7.5 12 3.5l8 4v9l-8 4-8-4z"></path><path d="M4 7.5l8 4 8-4"></path><path d="M12 11.5v9"></path>"#
);
