//! The commands of "Commands and keyboard shortcuts", list by list, as the board writes them
//! (DW-1232, DW-1233, DW-1234; the same table as WINDOWS.md's). Written from the board's
//! goldens by a script (the PR names it): do not edit a word here without the board.

use super::shortcuts::List;

/// DW-1232: Beebeeb, File, Edit.
pub const APP: &[List] = &[
    List {
        name: "Beebeeb",
        commands: &[
            ("About Beebeeb", "Settings \u{203A} General \u{203A} About"),
            ("Settings\u{2026}", "Ctrl+,"),
            ("Check for updates\u{2026}", "Settings \u{203A} General \u{203A} About"),
            ("Lock Beebeeb", "Settings \u{203A} Security"),
            ("Hide Beebeeb", "Minimize, or close the window: the b stays"),
            ("Hide others", "Windows has none (Win+D shows the desktop)"),
            ("Show all", "Windows has none"),
            ("Quit Beebeeb", "the b's menu"),
        ],
    },
    List {
        name: "File",
        commands: &[
            ("New folder", "Ctrl+Shift+N"),
            ("New file\u{2026}", "Ctrl+N"),
            ("New document\u{2026}", "New \u{25BE}"),
            ("Upload files\u{2026}", "Ctrl+O"),
            ("Upload a folder\u{2026}", "New \u{25BE}"),
            ("Open", "Enter"),
            ("Open With", "right-click \u{203A} Open with"),
            ("Preview", "Space"),
            ("Edit", "Ctrl+E"),
            ("Save", "Ctrl+S"),
            ("Share\u{2026}", "right-click, details panel"),
            ("Copy link", "right-click"),
            ("Make available offline", "right-click, details panel"),
            ("Show in File Explorer", "right-click"),
            ("Details", "Alt+Enter"),
            ("Rename", "F2"),
            ("Move\u{2026}", "right-click"),
            ("Duplicate", "right-click"),
            ("Version history\u{2026}", "right-click, details panel"),
            ("Move to Trash", "Delete"),
            ("Close window", "Ctrl+W"),
        ],
    },
    List {
        name: "Edit",
        commands: &[
            ("Undo Move to Trash", "Ctrl+Z"),
            ("Redo", "Ctrl+Y"),
            ("Cut", "Ctrl+X"),
            ("Copy", "Ctrl+C"),
            ("Paste", "Ctrl+V"),
            ("Select all", "Ctrl+A"),
            ("Search", "Ctrl+F"),
        ],
    },
];

/// DW-1233: View, Go, Window, Help.
pub const WINDOW: &[List] = &[
    List {
        name: "View",
        commands: &[
            ("as List", "toolbar: view switch"),
            ("as Grid", "toolbar: view switch"),
            ("Sort by", "toolbar: Sort"),
            ("Folders first", "toolbar: Sort"),
            ("Show details panel", "Alt+Shift+P"),
            ("Hide sidebar", "Ctrl+Shift+B"),
            ("Photos across", "Photos toolbar"),
            ("Enter full screen", "F11"),
        ],
    },
    List {
        name: "Go",
        commands: &[
            ("Back", "Alt+\u{2190}"),
            ("Forward", "Alt+\u{2192}"),
            ("Enclosing folder", "Alt+\u{2191}"),
            ("Files", "Ctrl+1"),
            ("Shared", "Ctrl+2"),
            ("Photos", "Ctrl+3"),
            ("Backups", "Ctrl+4"),
            ("Trash", "Ctrl+5"),
            ("Sync history", "Ctrl+6"),
            ("Available offline", "Ctrl+7"),
        ],
    },
    List {
        name: "Window",
        commands: &[
            ("Minimize", "the \u{2013} caption button (Win+\u{2193})"),
            ("Zoom", "the \u{25A1} caption button (Win+\u{2191})"),
            ("Bring all to front", "Windows has none: one window"),
            ("Beebeeb Drive", "the taskbar, Alt+Tab"),
        ],
    },
    List {
        name: "Help",
        commands: &[
            ("Search", "F1: Help has its own search"),
            ("Beebeeb Help", "F1"),
            ("Show the guide again", "Settings \u{203A} General \u{203A} Help"),
            ("Report a problem\u{2026}", "Settings \u{203A} General \u{203A} Help"),
            ("What's new in 0.9", "Settings \u{203A} General \u{203A} About"),
            ("Privacy policy", "Settings \u{203A} General \u{203A} About"),
        ],
    },
];

/// DW-1234: the editor's Edit, View, Format, Insert, while a document is open.
pub const EDITOR: &[List] = &[
    List {
        name: "Edit",
        commands: &[
            ("Undo Typing", "Ctrl+Z"),
            ("Redo", "Ctrl+Y"),
            ("Cut", "Ctrl+X"),
            ("Copy", "Ctrl+C"),
            ("Paste", "Ctrl+V"),
            ("Paste and match style", "Ctrl+Shift+V"),
            ("Select all", "Ctrl+A"),
            ("Find\u{2026}", "Ctrl+F"),
            ("Find next", "F3"),
            ("Search Drive", "Ctrl+Shift+F"),
            ("Commands\u{2026}", "Ctrl+K"),
        ],
    },
    List {
        name: "View",
        commands: &[
            ("Zoom in", "Ctrl+Plus"),
            ("Zoom out", "Ctrl\u{2212}"),
            ("Actual size", "Ctrl+0"),
            ("Show outline", "ribbon: Layout"),
            ("Show ribbon", "Ctrl+F1"),
            ("Present", "F5"),
            ("Enter full screen", "F11"),
        ],
    },
    List {
        name: "Format",
        commands: &[
            ("Bold", "Ctrl+B"),
            ("Italic", "Ctrl+I"),
            ("Underline", "Ctrl+U"),
            ("Strikethrough", "ribbon: Home"),
            ("Normal text", "ribbon: Home"),
            ("Heading 1", "ribbon: Home"),
            ("Heading 2", "ribbon: Home"),
            ("Heading 3", "ribbon: Home"),
            ("Align left", "Ctrl+L"),
            ("Align center", "Ctrl+E"),
            ("Align right", "Ctrl+R"),
            ("Justify", "Ctrl+J"),
            ("Bulleted list", "ribbon: Home"),
            ("Numbered list", "ribbon: Home"),
            ("Increase indent", "ribbon: Home"),
            ("Decrease indent", "ribbon: Home"),
        ],
    },
    List {
        name: "Insert",
        commands: &[
            ("Table\u{2026}", "ribbon: Insert"),
            ("Image\u{2026}", "ribbon: Insert"),
            ("Link\u{2026}", "ribbon: Insert"),
            ("Page break", "ribbon: Insert"),
            ("New slide", "Ctrl+M"),
            ("Duplicate slide", "right-click on a slide"),
        ],
    },
];
