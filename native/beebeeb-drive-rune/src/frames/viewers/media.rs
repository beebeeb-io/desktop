//! What is open INSIDE a photo, a recording, a PDF, a table or an archive (the page, the zoom,
//! find, the selected cell, the sort, the archive's folder), every control's method, and the keys.
//! The views are `photo.rs`, `audio.rs`, `pdf.rs`, `table.rs` and `archive.rs`.

use rune::{Key, KeyEvent, TextEdit};

use super::source::{Body, Entry, Match, OpenFile};
use super::{Intent, Ui};
use crate::State;

/// The zoom steps of a PDF, in percent.
pub const ZOOMS: [u32; 8] = [50, 75, 100, 125, 150, 200, 300, 400];
/// The key of the find bar's field.
pub const FIND_KEY: &str = "find-input";
/// The keys of the table and of the archive's list, which take the arrows.
pub const TABLE_KEY: &str = "csv-table";
pub const LIST_KEY: &str = "rows";

/// Find in a PDF (DW-1506).
pub struct Find {
    pub edit: TextEdit,
    /// The words the matches are of.
    asked: String,
    pub matches: Vec<Match>,
    /// Which match is the current one, from 0.
    pub current: usize,
}

#[derive(Default)]
pub struct Media {
    /// The PDF's page on screen, from 1, and its zoom in percent.
    pub page: u32,
    pub zoom: u32,
    pub find: Option<Find>,
    /// The table's selected cell: row (as sorted) and column.
    pub cell: Option<(usize, usize)>,
    /// The column the table is sorted by, and ascending or not. `None`: the file's order.
    pub sort: Option<(usize, bool)>,
    /// The archive's folder on screen ("" is the top, else it ends in "/"), and the selected row.
    pub dir: String,
    pub entry: Option<usize>,
}

impl Media {
    pub fn of(file: &OpenFile) -> Media {
        Media {
            page: match &file.body {
                Body::Pdf(doc) => doc.page.clamp(1, doc.pages.max(1)),
                _ => 0,
            },
            zoom: 100,
            ..Media::default()
        }
    }
}

impl Ui {
    // ---- PDF ----

    fn pages(&self) -> u32 {
        match &self.file.body {
            Body::Pdf(doc) => doc.pages,
            _ => 0,
        }
    }

    /// A thumbnail's press, Page Up / Page Down: that page.
    pub fn go_page(&mut self, page: u32) {
        if self.pages() > 0 {
            self.media.page = page.clamp(1, self.pages());
        }
    }

    /// Ctrl+Plus / Ctrl+Minus and the two buttons: one step, stopping at the ends.
    pub fn zoom_by(&mut self, steps: i32) {
        if self.pages() == 0 {
            return;
        }
        let at = ZOOMS
            .iter()
            .position(|z| *z >= self.media.zoom)
            .unwrap_or(ZOOMS.len() - 1);
        let to = at.saturating_add_signed(steps as isize).min(ZOOMS.len() - 1);
        self.media.zoom = ZOOMS[to];
    }

    /// Ctrl+F and the Find button: the find bar, with the keyboard in its field.
    pub fn open_find(&mut self) {
        if self.pages() == 0 {
            return;
        }
        if self.media.find.is_none() {
            self.media.find = Some(Find {
                edit: TextEdit::new(String::new()),
                asked: String::new(),
                matches: Vec::new(),
                current: 0,
            });
        }
        self.focus = Some(FIND_KEY);
    }

    /// Done, Esc, and the Find button again.
    pub fn close_find(&mut self) {
        if self.media.find.take().is_some() {
            self.focus = Some("tool-3");
        }
    }

    /// Ask the source where the words in the field are, when they changed. `true`: they had.
    pub fn refind(&mut self) -> bool {
        let Some(find) = self.media.find.as_mut() else {
            return false;
        };
        if find.edit.text == find.asked {
            return false;
        }
        find.asked = find.edit.text.clone();
        find.matches = if find.asked.trim().is_empty() {
            Vec::new()
        } else {
            self.source.find(&self.file.id, &find.asked)
        };
        // The first match on or after the page on screen.
        find.current = find.matches.iter().position(|m| m.page >= self.media.page).unwrap_or(0);
        if let Some(page) = find.matches.get(find.current).map(|m| m.page) {
            self.media.page = page;
        }
        true
    }

    /// Enter / Shift+Enter and the two chevrons: the next or the previous match, around the ends;
    /// its page comes on screen.
    pub fn find_step(&mut self, delta: i32) {
        self.refind();
        let Some(find) = self.media.find.as_mut() else { return };
        let count = find.matches.len() as i32;
        if count == 0 {
            return;
        }
        find.current = (find.current as i32 + delta).rem_euclid(count) as usize;
        self.media.page = find.matches[find.current].page;
    }

    /// "2 of 3", "No matches"; empty while nothing is looked for.
    pub fn find_count(&self) -> String {
        match &self.media.find {
            Some(find) if find.asked.trim().is_empty() => String::new(),
            Some(find) if find.matches.is_empty() => "No matches".into(),
            Some(find) => format!("{} of {}", find.current + 1, find.matches.len()),
            None => String::new(),
        }
    }

    // ---- Table ----

    fn table_size(&self) -> (usize, usize) {
        match &self.file.body {
            Body::Table(doc) => (doc.rows.len(), doc.columns.len()),
            _ => (0, 0),
        }
    }

    /// The table's rows as they are shown: the file's order, or sorted by the chosen column
    /// (numbers by value, words by letter).
    pub fn table_rows(&self) -> Vec<&Vec<String>> {
        let Body::Table(doc) = &self.file.body else {
            return Vec::new();
        };
        let mut rows: Vec<&Vec<String>> = doc.rows.iter().collect();
        if let Some((column, ascending)) = self.media.sort {
            let numeric = doc.columns.get(column).is_some_and(|c| c.numeric);
            let cell = |row: &Vec<String>| row.get(column).cloned().unwrap_or_default();
            rows.sort_by(|a, b| {
                let order = if numeric {
                    let n = |row: &Vec<String>| cell(row).parse::<f64>().unwrap_or(f64::MIN);
                    n(a).total_cmp(&n(b))
                } else {
                    cell(a).to_lowercase().cmp(&cell(b).to_lowercase())
                };
                if ascending { order } else { order.reverse() }
            });
        }
        rows
    }

    /// A press on a cell: it is the selected one.
    pub fn select_cell(&mut self, row: usize, column: usize) {
        let (rows, columns) = self.table_size();
        if row < rows && column < columns {
            self.media.cell = Some((row, column));
        }
    }

    /// The arrows: the selection one cell on, stopping at the edges. With none, the first cell.
    pub fn move_cell(&mut self, rows: isize, columns: isize) {
        let (row_count, column_count) = self.table_size();
        if row_count == 0 || column_count == 0 {
            return;
        }
        self.media.cell = Some(match self.media.cell {
            Some((r, c)) => (
                r.saturating_add_signed(rows).min(row_count - 1),
                c.saturating_add_signed(columns).min(column_count - 1),
            ),
            None => (0, 0),
        });
    }

    /// A press on a column's name: sorted by it, ascending; again: descending; a third time: the
    /// file's order. The selection is of the old order, so it goes.
    pub fn sort_by(&mut self, column: usize) {
        if column >= self.table_size().1 {
            return;
        }
        self.media.sort = match self.media.sort {
            Some((c, true)) if c == column => Some((column, false)),
            Some((c, false)) if c == column => None,
            _ => Some((column, true)),
        };
        self.media.cell = None;
    }

    // ---- Archive ----

    /// The entries of the archive's folder on screen, in the list's order.
    pub fn entries(&self) -> Vec<&Entry> {
        match &self.file.body {
            Body::Archive(doc) => doc.entries.iter().filter(|e| e.parent() == self.media.dir).collect(),
            _ => Vec::new(),
        }
    }

    pub fn select_entry(&mut self, row: usize) {
        if row < self.entries().len() {
            self.media.entry = Some(row);
        }
    }

    pub fn move_entry(&mut self, delta: isize) {
        let count = self.entries().len();
        if count == 0 {
            return;
        }
        self.media.entry = Some(match self.media.entry {
            Some(i) => i.saturating_add_signed(delta).min(count - 1),
            None => 0,
        });
    }

    /// Enter or a double press on a row: a folder's list comes on screen. A file inside an
    /// archive cannot be opened here (the banner says so): nothing happens.
    pub fn enter_entry(&mut self, row: usize) -> bool {
        let Some(path) = self
            .entries()
            .get(row)
            .filter(|e| e.path.ends_with('/'))
            .map(|e| e.path.clone())
        else {
            return false;
        };
        self.media.dir = path;
        self.media.entry = None;
        self.focus = Some(LIST_KEY);
        true
    }

    /// Backspace, Back: one folder up inside the archive. `false`: already at its top.
    pub fn archive_up(&mut self) -> bool {
        if self.media.dir.is_empty() {
            return false;
        }
        let inner = self.media.dir.trim_end_matches('/');
        self.media.dir = match inner.rfind('/') {
            Some(at) => inner[..=at].to_string(),
            None => String::new(),
        };
        self.media.entry = None;
        true
    }

    // ---- Audio ----

    /// Space and the round button: play or pause.
    pub fn toggle_play(&mut self) {
        let Body::Audio(doc) = &self.file.body else { return };
        let on = !doc.playing;
        if self.source.play(&self.file.id, on) {
            if let Body::Audio(doc) = &mut self.file.body {
                doc.playing = on;
            }
        }
    }

    /// Something inside the file is open that Esc or Back closes before the file itself: find,
    /// then the archive's folder. `true`: it was closed, the file stays.
    pub fn close_inner(&mut self) -> bool {
        if self.media.find.is_some() {
            self.close_find();
            return true;
        }
        self.archive_up()
    }
}

/// Show another file in this one's place (the next photo, a track of the queue): Back still
/// returns to the folder, not to the file before.
pub fn switch_to(s: &mut State, id: &str) -> bool {
    let Some(source) = super::route_source(s) else {
        return false;
    };
    let Ok(ui) = Ui::open(source, id) else { return false };
    if let Some(mut old) = s.viewer.ui.take() {
        old.cancel();
    }
    crate::shell::host::dispatch(s, crate::shell::app::Event::Escape);
    crate::shell::host::dispatch(s, crate::shell::app::Event::OpenItem(id.to_string()));
    s.viewer.ui = Some(ui);
    true
}

/// The id `delta` places from the current one in the photo's strip or the recording's queue.
fn neighbour(file: &OpenFile, delta: isize) -> Option<String> {
    let (ids, at): (Vec<&String>, Option<usize>) = match &file.body {
        Body::Photo(doc) => (
            doc.strip.iter().map(|i| &i.id).collect(),
            doc.strip.iter().position(|i| i.current),
        ),
        Body::Audio(doc) => (
            doc.queue.iter().map(|t| &t.id).collect(),
            doc.queue.iter().position(|t| t.current),
        ),
        _ => return None,
    };
    let to = at?.checked_add_signed(delta)?;
    ids.get(to).map(|id| id.to_string())
}

/// Left / Right on a photo, Ctrl+Left / Ctrl+Right and the two buttons on a recording.
pub fn step(s: &mut State, delta: isize) -> bool {
    let Some(id) = s.viewer.ui.as_ref().and_then(|ui| neighbour(&ui.file, delta)) else {
        return false;
    };
    switch_to(s, &id)
}

/// The keys of these viewers. `true`: the key was theirs.
pub fn on_key(s: &mut State, ev: &KeyEvent) -> bool {
    let Some(ui) = s.viewer.ui.as_mut() else { return false };
    if ui.editor.is_some() || ui.dialog.is_some() {
        return false;
    }
    let m = &ev.mods;
    let plain = !m.alt && !m.shift && !m.cmd && !m.ctrl;
    let ctrl = (m.cmd || m.ctrl) && !m.alt && !m.shift;
    match (&ui.file.body, &ev.key) {
        (Body::Photo(_), Key::Left) if plain => step(s, -1),
        (Body::Photo(_), Key::Right) if plain => step(s, 1),
        (Body::Audio(_), Key::Left) if ctrl => step(s, -1),
        (Body::Audio(_), Key::Right) if ctrl => step(s, 1),
        (Body::Audio(_), Key::Text(key)) if plain && key == " " => {
            ui.toggle_play();
            true
        }
        (Body::Pdf(_), Key::Command('f')) if !m.alt && !m.shift => {
            ui.open_find();
            true
        }
        (Body::Pdf(_), Key::Command('=' | '+')) => {
            ui.zoom_by(1);
            true
        }
        (Body::Pdf(_), Key::Command('-')) => {
            ui.zoom_by(-1);
            true
        }
        (Body::Pdf(_), Key::Enter) if ui.media.find.is_some() && !m.alt && !m.cmd && !m.ctrl => {
            ui.find_step(if m.shift { -1 } else { 1 });
            true
        }
        (Body::Pdf(_), Key::PageDown) if plain => {
            ui.go_page(ui.media.page + 1);
            true
        }
        (Body::Pdf(_), Key::PageUp) if plain => {
            ui.go_page(ui.media.page.saturating_sub(1));
            true
        }
        (Body::Table(_), Key::Command('e')) if !m.alt && !m.shift => {
            ui.intent(Intent::EditInSheets);
            true
        }
        (Body::Table(_), Key::Up | Key::Down | Key::Left | Key::Right) if plain => {
            let (rows, columns) = match ev.key {
                Key::Up => (-1, 0),
                Key::Down => (1, 0),
                Key::Left => (0, -1),
                _ => (0, 1),
            };
            ui.move_cell(rows, columns);
            true
        }
        (Body::Archive(_), Key::Up) if plain => {
            ui.move_entry(-1);
            true
        }
        (Body::Archive(_), Key::Down) if plain => {
            ui.move_entry(1);
            true
        }
        (Body::Archive(_), Key::Enter) if plain => match ui.media.entry {
            Some(row) => ui.enter_entry(row),
            None => false,
        },
        (Body::Archive(_), Key::Backspace) if plain => ui.archive_up(),
        _ => false,
    }
}
