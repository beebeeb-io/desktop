//! A folder as a grid, and its thumbnails arriving one by one (DW-0119): the names and the sizes
//! are there at once, because the list is already on this PC; a picture's tile is plain until its
//! thumbnail arrives. No spinner, no loading state.
//!
//! The Files window asks at two places: its view switch ([`on`], [`set`]) and its body
//! ([`grid_body`], in the list's place). "Each folder remembers its own view" (DW-1102): the
//! folders seen as a grid are kept here by id, for as long as the app runs.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Mutex;

use rune::{El, Key, KeyEvent, PointerPress, Tx, col, row};

use crate::State;
use crate::source::DriveItem;
use crate::theme::tokens::Theme;
use crate::theme::file_icons::icon_for_item;
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
    /// The arrived thumbnail as pixels. Only a source of real thumbnails answers; a board fixture names a sample.
    fn pixels(&self, _id: &str) -> Option<rune::Pixels> {
        None
    }
    /// The tile of this item scrolled far out of view: what is held of it may go, so the tiles in view can be fetched.
    fn release(&self, _id: &str) {}
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

impl GridUi {
    /// The lines in view, as the list last reported them (before the first layout: the first screenful).
    fn near_lines(&self) -> std::ops::Range<usize> {
        self.visible.lock().unwrap().clone()
    }

    /// The lines in view changed: true when the view must be built again (new lines ask for their pictures).
    fn set_visible(&self, r: std::ops::Range<usize>) -> bool {
        let mut v = self.visible.lock().unwrap();
        let changed = *v != r;
        *v = r;
        changed
    }

    /// A picture asked for earlier whose tile is now more than [`grid_tiles::KEEP_ROWS`] lines out of view is let go.
    fn release_far(&self, items: &[DriveItem]) {
        let mut asked = self.asked.lock().unwrap();
        if asked.is_empty() {
            return;
        }
        let near = self.near_lines();
        let columns = self.columns.max(1);
        let band = grid_tiles::KEEP_ROWS * columns;
        let (from, to) = ((near.start * columns).saturating_sub(band), (near.end * columns + band).min(items.len()));
        for item in items[..from.min(items.len())].iter().chain(&items[to..]) {
            if asked.remove(&item.id) {
                self.thumbs.release(&item.id);
            }
        }
    }
}

/// What the grid keeps (`global::Ui::grid`).
pub struct GridUi {
    pub thumbs: Box<dyn ThumbSource>,
    /// The folders shown as a grid, by id (`None`: the root).
    folders: HashSet<Option<String>>,
    /// How many tiles stand side by side at the window's width (set when the grid is laid out).
    pub(crate) columns: usize,
    /// The lines in view (set by the virtual list after each layout).
    visible: Mutex<std::ops::Range<usize>>,
    /// The pictures this grid asked its source for, not yet released.
    asked: Mutex<HashSet<String>>,
}

impl Default for GridUi {
    fn default() -> Self {
        GridUi {
            thumbs: Box::new(NoThumbs),
            folders: HashSet::new(),
            columns: 1,
            visible: Mutex::new(0..5),
            asked: Mutex::new(HashSet::new()),
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
    Thumb::Type(icon_for_item(item), if item.is_folder { 44.0 } else { 34.0 })
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

/// The arrows inside the grid: Left and Right one tile, Up and Down one row of tiles, PageUp and PageDown one screen. Every other
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
        // PageUp / PageDown: one screen of tile lines, keeping the column.
        (Key::PageUp | Key::PageDown, sel) => {
            let r = s.global.grid.near_lines();
            let page = r.end.saturating_sub(r.start).max(1) * columns;
            let i = sel.unwrap_or(0);
            if ev.key == Key::PageUp { i.saturating_sub(page) } else { (i + page).min(count - 1) }
        }
        _ => return false,
    };
    if m.shift {
        s.extend_to(to);
    } else {
        s.select_only(to);
    }
    tx.scroll_into_view(format!("grid-line-{}", s.listing.items[to - to % columns].id));
    true
}

/// Where an item stands now, found by its id (a row's handlers never trust the index it was built at).
fn index_of(s: &State, id: &str) -> Option<usize> {
    s.listing.items.iter().position(|it| it.id == id)
}

/// One line of the grid: up to `columns` tiles side by side, built only while the line is in the mounted window.
/// Pictures are asked for only by the lines in view and one line above and below.
fn line(s: &State, i: usize, t: &Theme) -> El {
    let g = &s.global.grid;
    let thumbs = g.thumbs.as_ref();
    let columns = g.columns.max(1);
    let count = s.listing.items.len();
    let (from, to) = (i * columns, ((i + 1) * columns).min(count));
    let near = g.near_lines();
    let line_near = i + 1 >= near.start && i < near.end + 1;
    let first = s.listing.items.get(from).map_or("none", |it| it.id.as_str());
    let tiles: Vec<El> = (from..to.max(from))
        .filter_map(|n| s.listing.items.get(n).map(|item| (n, item)))
        .map(|(n, item)| {
            let sub = sub_of(item);
            let asks = pictured(item) && line_near;
            if asks {
                g.asked.lock().unwrap().insert(item.id.clone());
            }
            let length = asks.then(|| thumbs.video_length(&item.id)).flatten();
            let mut tile = grid_tiles::tile(
                n,
                &Tile {
                    name: &item.name,
                    sub: &sub,
                    thumb: if pictured(item) && !asks {
                        Thumb::Plain
                    } else {
                        thumb_of(item, thumbs)
                    },
                    pixels: asks.then(|| thumbs.pixels(&item.id)).flatten(),
                    length: length.as_deref(),
                    selected: s.is_selected(n),
                },
                t,
            )
            .role("option")
            .label(tile_name(item, &sub, length.as_deref()))
            .position_in_set(n + 1, count);
            let id = item.id.clone();
            if item.is_folder {
                // "Drop on a folder: uploads into that folder", as in the list.
                let folder = id.clone();
                tile = tile.on_drop_files(move |s: &mut State, files: &[std::path::PathBuf], tx: &mut Tx| {
                    s.upload(Some(folder.clone()), files, tx);
                });
            }
            let (a, b, c) = (id.clone(), id.clone(), id);
            tile.on_press_tx(move |s: &mut State, tx: &mut Tx| {
                let Some(i) = index_of(s, &a) else { return };
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
                if let Some(i) = index_of(s, &b) {
                    s.open_row(i, tx);
                }
            })
            .on_secondary_press(move |s: &mut State, at: PointerPress, tx: &mut Tx| {
                let Some(i) = index_of(s, &c) else { return };
                tx.focus("rows", false);
                if !s.is_selected(i) {
                    s.select_only(i);
                }
                s.open_file_menu((at.x, at.y));
            })
        })
        .collect();
    row()
        .key(format!("grid-line-{first}"))
        .w_full()
        .items_start()
        .gap(grid_tiles::COL_GAP)
        .pb(grid_tiles::ROW_GAP)
        .children(tiles)
}

/// The folder as a grid, in the list's place. `None`: this folder is a list, or could not be read.
/// Only the lines in view (and what the window mounts around them) are realised, keyed by their first item's id.
pub fn grid_body(s: &State, t: &Theme) -> Option<El> {
    if !on(s) || s.list_error.is_some() {
        return None;
    }
    let g = &s.global.grid;
    let count = s.listing.items.len();
    let columns = g.columns.max(1);
    let lines = count.div_ceil(columns);
    g.release_far(&s.listing.items);
    let t = *t;
    // The first id and the last id stand for the keys: a reordered or replaced folder re-reads them.
    let version = (
        lines,
        columns,
        s.listing.items.first().map(|i| i.id.clone()),
        s.listing.items.last().map(|i| i.id.clone()),
        s.listing.folder.id.clone(),
    );
    let mut rows = col()
        .role("listbox")
        .label(s.listing.folder.name.clone())
        // The grid's one Tab stop: it holds keyboard focus, the selected tile is its active one.
        .focusable(true)
        .no_focus_ring()
        .on_key(on_key)
        .on_layout(|s: &mut State, [_, _, w, _]| {
            s.global.grid.columns = grid_tiles::columns(w + 2.0 * grid_tiles::PAD_X);
        });
    if let Some(i) = s.selected.filter(|&i| i < count) {
        rows = rows.active_descendant(format!("grid-{i}"));
    }
    let rows = rows.child(
        col()
            .key("grid-lines")
            .w_full()
            .growth(rune::GrowthSpec::default())
            .virtual_rows(
                lines,
                grid_tiles::CELL_H + grid_tiles::ROW_GAP,
                move |s: &State, i| {
                    let first = s.listing.items.get(i * s.global.grid.columns.max(1));
                    Cow::Owned(format!("grid-line-{}", first.map_or("none", |it| it.id.as_str())))
                },
                move |s: &State, i| line(s, i, &t),
            )
            .virtual_rows_version(version)
            .on_visible_rows(|s: &mut State, r: std::ops::Range<usize>| s.global.grid.set_visible(r)),
    );
    Some(
        grid_tiles::grid(rows)
            .on_drop_files(|s: &mut State, files: &[std::path::PathBuf], tx: &mut Tx| {
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use rune::{Env, Runtime};

    use super::*;
    use crate::frames::shared::tests::settle;
    use crate::view;

    /// Which items the grid asked for a picture, and which it let go.
    #[derive(Default)]
    struct Log {
        asked: HashSet<String>,
        released: HashSet<String>,
    }

    struct Counting(Arc<Mutex<Log>>);

    impl ThumbSource for Counting {
        fn arrived(&self, id: &str) -> bool {
            self.0.lock().unwrap().asked.insert(id.to_string());
            false
        }
        fn video_length(&self, _: &str) -> Option<String> {
            None
        }
        fn pixels(&self, id: &str) -> Option<rune::Pixels> {
            self.0.lock().unwrap().asked.insert(id.to_string());
            None
        }
        fn release(&self, id: &str) {
            self.0.lock().unwrap().released.insert(id.to_string());
        }
    }

    fn picture(i: usize) -> DriveItem {
        DriveItem {
            id: format!("p{i}"),
            parent_id: None,
            name: format!("IMG_{i:04}.jpg"),
            is_folder: false,
            size: Some(2_000_000),
            child_count: None,
            modified_at: String::new(),
            mime: Some("image/jpeg".to_string()),
            has_thumbnail: true,
            shared_type: None,
            shared_badges: Vec::new(),
            pc_status: None,
            is_protected: false,
        }
    }

    #[test]
    fn folder_grid_fetches_pictures_for_tiles_scrolled_into_view() {
        const N: usize = 400;
        let log = Arc::new(Mutex::new(Log::default()));
        let mut state = crate::shell::state::init();
        state.listing.items = (0..N).map(picture).collect();
        state.list_error = None;
        state.show_detail_pane = false;
        state.clear_selection();
        state.global.grid.thumbs = Box::new(Counting(log.clone()));
        set(&mut state, true);
        let env = Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = Runtime::new(state, view, (1278.0, 798.0), env);
        settle(&mut rt);
        {
            let log = log.lock().unwrap();
            assert!(log.asked.contains("p0"), "the first tile asks");
            // About five rows of a handful of tiles are in view; the grid never asks for all 400.
            assert!(log.asked.len() < N / 4, "asked for {} of {N}", log.asked.len());
            assert!(!log.asked.contains("p399"), "a tile far below asks for nothing");
        }
        // Scroll to the end: the tiles there now ask, and the first ones, far out of view, are let go.
        rt.update(|s, tx| tx.scroll_into_view(format!("grid-line-{}", s.listing.items[N - 1 - (N - 1) % s.global.grid.columns].id)));
        settle(&mut rt);
        settle(&mut rt);
        let log = log.lock().unwrap();
        assert!(log.asked.contains("p399"), "the tiles scrolled into view ask");
        assert!(
            log.released.contains("p0"),
            "the pictures scrolled far out of view are let go"
        );
        assert!(!log.released.contains("p399"));
        assert!(log.asked.len() < N, "still not every tile");
    }
}
