//! What the Files menus hold: the file menu (DW-0402, DW-1104), Sort (DW-0404), New (DW-0414).

use super::{Sort, SortBy};
use crate::State;
use crate::ui::menu::Entry;

/// `.wn-menu` of the file menu and of New.
pub const FILE_MENU_W: f32 = 236.0;
pub const NEW_MENU_W: f32 = 236.0;
pub const SORT_MENU_W: f32 = 190.0;

/// The file menu for the selection, by the board's rules (DW-0402):
/// - Backups and service folders offer Open, Preview, Show in File Explorer and Details only;
/// - several selected items leave out Rename, Version history… and Details;
/// - Make available offline reads Remove from this PC when the item already is;
/// - Free up space follows it when something selected has a copy on this PC.
///
/// A folder's Preview stays in the menu, dimmed: it cannot be picked ("Open goes into it and Preview is dimmed").
pub fn file_menu(s: &State) -> Vec<Entry> {
    let rows = s.selected_rows();
    let items: Vec<_> = rows.iter().filter_map(|&i| s.listing.items.get(i)).collect();
    let several = items.len() > 1;
    let open = Entry::keyed("open", "Open", "Enter");
    let preview = Entry::keyed("preview", "Preview", "Space").dimmed(!s.can_preview());
    let explorer = Entry::item("show-in-explorer", "Show in File Explorer");
    let details = Entry::keyed("details", "Details", "Alt+Enter");
    if items.iter().any(|i| i.is_protected) || s.protects_selection() {
        return vec![open, preview, Entry::Sep, explorer, details];
    }
    let keep = if s.selection_is_kept() {
        Entry::item("keep-offline", "Remove from this PC")
    } else {
        Entry::item("keep-offline", "Make available offline")
    };
    let mut entries = vec![
        open,
        preview,
        Entry::Sep,
        Entry::item("share", "Share\u{2026}"),
        Entry::item("copy-link", "Copy link"),
        Entry::Sep,
        keep,
    ];
    // Free up space, as File Explorer offers it: where something selected has a copy on this PC
    // (downloaded, or kept). Not on the board yet (PR 305, Decision: owner request 2026-11-12).
    if s.selection_is_on_this_pc() {
        entries.push(Entry::item("free-up-space", "Free up space"));
    }
    entries.push(explorer);
    if !several {
        entries.push(details);
    }
    entries.push(Entry::Sep);
    if !several {
        entries.push(Entry::keyed("rename", "Rename", "F2"));
    }
    entries.push(Entry::item("move", "Move\u{2026}"));
    entries.push(Entry::item("duplicate", "Duplicate"));
    if !several {
        entries.push(Entry::item("versions", "Version history\u{2026}"));
    }
    entries.push(Entry::Sep);
    entries.push(Entry::danger("trash", "Move to Trash", "Delete"));
    entries
}

/// Sort: by what, in which order, folders first. The Order labels follow the key (DW-0404).
pub fn sort_menu(sort: Sort) -> Vec<Entry> {
    let (forward, reversed) = match sort.by {
        SortBy::Name | SortBy::Type => ("A to Z", "Z to A"),
        SortBy::DateChanged => ("Newest first", "Oldest first"),
        SortBy::Size => ("Largest first", "Smallest first"),
    };
    vec![
        Entry::Head("Sort by"),
        Entry::check("sort-name", "Name", sort.by == SortBy::Name),
        Entry::check("sort-date", "Date changed", sort.by == SortBy::DateChanged),
        Entry::check("sort-size", "Size", sort.by == SortBy::Size),
        Entry::check("sort-type", "Type", sort.by == SortBy::Type),
        Entry::Sep,
        Entry::Head("Order"),
        Entry::check("order-forward", forward, !sort.reversed),
        Entry::check("order-reversed", reversed, sort.reversed),
        Entry::Sep,
        Entry::check("folders-first", "Folders first", sort.folders_first),
    ]
}

/// New: everything that adds to the folder you're in.
pub fn new_menu() -> Vec<Entry> {
    vec![
        Entry::keyed("new-folder", "New folder", "Ctrl+Shift+N"),
        Entry::keyed("new-file", "New file\u{2026}", "Ctrl+N"),
        Entry::item("new-document", "New document\u{2026}"),
        Entry::Sep,
        Entry::keyed("upload-files", "Upload files\u{2026}", "Ctrl+O"),
        Entry::item("upload-folder", "Upload a folder\u{2026}"),
    ]
}
