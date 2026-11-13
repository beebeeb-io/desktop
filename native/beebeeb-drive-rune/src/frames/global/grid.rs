//! A folder as a grid, and its thumbnails arriving one by one (DW-0119): the names and the sizes
//! are there at once, because the list is already on this PC; a picture's tile is plain until its
//! thumbnail arrives. No spinner, no loading state.
//!
//! The Files window asks at two places: its view switch ([`on`], [`set`]) and its body
//! ([`grid_body`], in the list's place). "Each folder remembers its own view" (DW-1102): the
//! folders seen as a grid are kept here by id, for as long as the app runs.

use std::collections::HashSet;

use rune::{El, Key, KeyEvent, PointerPress, Tx, row};

use crate::State;
use crate::source::DriveItem;
use crate::theme::tokens::Theme;
use crate::ui::file_list::file_type_icon;
use crate::ui::grid_tiles::{self, Thumb, Tile};

/// What the grid knows about a picture or a video beyond the list's row.
pub trait ThumbSource: Send + 'static {
    /// Has the thumbnail of this item arrived on this PC?
    fn arrived(&self, id: &str) -> bool;
    /// A video's length, as it is written on its tile ("0:41").
    fn video_length(&self, id: &str) -> Option<String>;
    /// Which of the board's six stand-in pictures this item shows (1 to 6). Only a board fixture
    /// answers; a source of real thumbnails hands over pixels, and says `None` here.
    fn sample(&self, _id: &str) -> Option<u8> {
        None
    }
}

/// Nothing is connected that hands over thumbnails: every picture stays a plain tile.
pub struct NoThumbs;

impl ThumbSource for NoThumbs {
    fn arrived(&self, _: &str) -> bool {
        false
    }
    fn video_length(&self, _: &str) -> Option<String> {
        None
    }
}

/// The board's Lisbon trip (DW-0119): no thumbnail has arrived yet; Tram 28.mov is 0:41 long.
/// `arrived` holds the ids whose thumbnail came in since (the tests let them arrive one by one).
#[derive(Default)]
pub struct BoardThumbs {
    pub arrived: Vec<String>,
}

impl ThumbSource for BoardThumbs {
    fn arrived(&self, id: &str) -> bool {
        self.arrived.iter().any(|a| a == id)
    }
    fn video_length(&self, id: &str) -> Option<String> {
        (id == "tram28").then(|| "0:41".to_string())
    }
}

/// What the grid keeps (`global::Ui::grid`).
pub struct GridUi {
    pub thumbs: Box<dyn ThumbSource>,
    /// The folders shown as a grid, by id (`None`: the root).
    folders: HashSet<Option<String>>,
    /// How many tiles stand side by side at the window's width (set when the grid is laid out).
    columns: usize,
}

impl Default for GridUi {
    fn default() -> Self {
        GridUi {
            thumbs: Box::new(NoThumbs),
            folders: HashSet::new(),
            columns: 1,
        }
    }
}

/// Is the folder shown a grid?
pub fn on(s: &State) -> bool {
    s.global.grid.folders.contains(&s.listing.folder.id)
}

/// The view switch: this folder as a grid, or as a list. Other folders keep theirs.
pub fn set(s: &mut State, grid: bool) {
    let folder = s.listing.folder.id.clone();
    if grid {
        s.global.grid.folders.insert(folder);
    } else {
        s.global.grid.folders.remove(&folder);
    }
}

/// The view switch from the keyboard or a click, keeping the keyboard in the folder's items.
pub fn switch(s: &mut State, grid: bool, tx: &mut Tx) {
    set(s, grid);
    if !s.listing.items.is_empty() {
        tx.focus("rows", false);
    }
}

/// A size as a tile writes it: "220 KB", "3.4 MB", "82 MB".
pub fn tile_size(bytes: i64) -> String {
    crate::source::format_size(bytes).replace(".0 ", " ")
}

/// The line under a tile's name: how many items a folder holds, or a file's size.
pub fn sub_of(item: &DriveItem) -> String {
    if item.is_folder {
        return match item.child_count {
            Some(1) => "1 item".to_string(),
            Some(n) => format!("{n} items"),
            None => String::new(),
        };
    }
    item.size.map(tile_size).unwrap_or_default()
}

fn pictured(item: &DriveItem) -> bool {
    !item.is_folder
        && item
            .mime
            .as_deref()
            .is_some_and(|m| m.starts_with("image/") || m.starts_with("video/"))
}

/// What a tile shows of this item now.
pub fn thumb_of(item: &DriveItem, thumbs: &dyn ThumbSource) -> Thumb {
    if pictured(item) {
        return if thumbs.arrived(&item.id) {
            thumbs.sample(&item.id).map_or(Thumb::Picture, Thumb::Sample)
        } else {
            Thumb::Plain
        };
    }
    Thumb::Glyph(file_type_icon(item), if item.is_folder { 44.0 } else { 34.0 })
}

/// What a screen reader says of a tile: "Tram 28.mov, video, 0:41, 82 MB".
pub fn tile_name(item: &DriveItem, sub: &str, length: Option<&str>) -> String {
    let mut parts = vec![item.name.clone()];
    if item.is_folder {
        parts.push("folder".to_string());
    }
    if let Some(length) = length {
        parts.push(format!("video, {length}"));
    }
    if !sub.is_empty() {
        parts.push(sub.to_string());
    }
    parts.join(", ")
}

/// The arrows inside the grid: Left and Right one tile, Up and Down one row of tiles. Every other
/// key is the Files window's (Enter, Space, Delete, the menu key, Home and End).
fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    if m.cmd || m.ctrl || m.alt || m.logo || s.actions.menu.is_some() {
        return false;
    }
    let count = s.listing.items.len();
    if count == 0 {
        return false;
    }
    let columns = s.global.grid.columns.max(1);
    let to = match (&ev.key, s.selected) {
        (Key::Left | Key::Right | Key::Up | Key::Down, None) => 0,
        (Key::Left, Some(i)) => i.saturating_sub(1),
        (Key::Right, Some(i)) => (i + 1).min(count - 1),
        (Key::Up, Some(i)) if i >= columns => i - columns,
        (Key::Down, Some(i)) if i + columns < count => i + columns,
        (Key::Up | Key::Down, Some(i)) => i,
        (Key::Home, _) => 0,
        (Key::End, _) => count - 1,
        _ => return false,
    };
    if m.shift {
        s.extend_to(to);
    } else {
        s.select_only(to);
    }
    tx.scroll_into_view(format!("grid-{to}"));
    true
}

/// The folder as a grid, in the list's place. `None`: this folder is a list, or could not be read.
pub fn grid_body(s: &State, t: &Theme) -> Option<El> {
    if !on(s) || s.list_error.is_some() {
        return None;
    }
    let thumbs = s.global.grid.thumbs.as_ref();
    let count = s.listing.items.len();
    let tiles = s
        .listing
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let sub = sub_of(item);
            let length = pictured(item).then(|| thumbs.video_length(&item.id)).flatten();
            let mut tile = grid_tiles::tile(
                i,
                &Tile {
                    name: &item.name,
                    sub: &sub,
                    thumb: thumb_of(item, thumbs),
                    length: length.as_deref(),
                    selected: s.is_selected(i),
                },
                t,
            )
            .role("option")
            .label(tile_name(item, &sub, length.as_deref()))
            .position_in_set(i + 1, count);
            if item.is_folder {
                // "Drop on a folder: uploads into that folder", as in the list.
                let id = item.id.clone();
                tile = tile.on_drop_files(move |s: &mut State, files: &[std::path::PathBuf], tx: &mut Tx| {
                    s.upload(Some(id.clone()), files, tx);
                });
            }
            tile.on_press_tx(move |s: &mut State, tx: &mut Tx| {
                let mods = tx.mods();
                tx.focus("rows", false);
                if mods.shift {
                    s.extend_to(i);
                } else if mods.cmd || mods.ctrl {
                    s.toggle_row(i);
                } else {
                    s.select_only(i);
                }
            })
            .on_double_press_tx(move |s: &mut State, tx: &mut Tx| {
                s.open_row(i, tx);
            })
            .on_secondary_press(move |s: &mut State, at: PointerPress, tx: &mut Tx| {
                tx.focus("rows", false);
                if !s.is_selected(i) {
                    s.select_only(i);
                }
                s.open_file_menu((at.x, at.y));
            })
        })
        .collect();
    let mut rows = row()
        .role("listbox")
        .label(s.listing.folder.name.clone())
        // The grid's one Tab stop: it holds keyboard focus, the selected tile is its active one.
        .focusable(true)
        .no_focus_ring()
        .on_key(on_key)
        .on_layout(|s: &mut State, [_, _, w, _]| {
            s.global.grid.columns = grid_tiles::columns(w + 2.0 * grid_tiles::PAD_X);
        });
    if let Some(i) = s.selected.filter(|&i| i < s.listing.items.len()) {
        rows = rows.active_descendant(format!("grid-{i}"));
    }
    Some(
        grid_tiles::grid(rows, tiles).on_drop_files(|s: &mut State, files: &[std::path::PathBuf], tx: &mut Tx| {
            let folder = s.listing.folder.id.clone();
            s.upload(folder, files, tx);
        }),
    )
}

/// DW-0119: Lisbon trip as a grid, before any thumbnail has arrived.
pub fn scene(s: &mut State) {
    use super::samples::TripByDate;
    s.show_detail_pane = false;
    s.source = Box::new(TripByDate::new());
    (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
    s.actions = crate::frames::files::Actions::new(Box::new(TripByDate::files()));
    s.open_folder(Some(TripByDate::FOLDER.to_string()));
    s.clear_selection();
    s.global = super::Ui::default();
    s.global.grid.thumbs = Box::new(BoardThumbs::default());
    s.global.board_crop_bottom = true;
    // Under the crop the board's status line repeats the root's words (DW-0401) for this folder;
    // DW-0420 counts the folder itself ("1 on this PC ..."), which is what the app says.
    s.global.board_status_words = Some("4 on this PC \u{00B7} 3 available offline \u{00B7} 4 online only".to_string());
    set(s, true);
}
