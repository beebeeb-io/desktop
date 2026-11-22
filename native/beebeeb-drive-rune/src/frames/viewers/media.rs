//! What is open INSIDE a photo, a recording, a PDF, a table or an archive (the page, the zoom,
//! find, the selected cell, the sort, the archive's folder), every control's method, and the keys.
//! The views are `photo.rs`, `audio.rs`, `pdf.rs`, `table.rs` and `archive.rs`.

use rune::{Key, KeyEvent, TextEdit};

use super::source::{Body, Entry, Match, OpenFile};
use super::{Intent, Ui};
use crate::State;
use crate::media::player::PlayerHandle;
use crate::ui::list_keys::{ListAction, ListCursor, ListKeys, on_list_key};

/// How far Up / Down move the volume of a recording or a video, in percent.
pub const VOLUME_STEP: i32 = 10;
/// How many rows Page Up / Page Down move in the archive's list.
pub const ARCHIVE_PAGE: usize = 10;

/// The zoom steps of a PDF, in percent.
pub const ZOOMS: [u32; 8] = [50, 75, 100, 125, 150, 200, 300, 400];
/// The key of the find bar's field.
pub const FIND_KEY: &str = "find-input";
/// What Find says on a PDF whose session ended (CP2 row 10.2): the true state, never "Not built yet" (Find is built)
/// and never "No matches" for words nobody looked for.
pub const FIND_SESSION_ENDED: &str =
    "Can't search this file: Beebeeb was locked or signed out, or another account signed in.";
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
    /// The archive list's keyboard (`ui::list_keys`): its type-ahead buffer; its focus follows `entry`.
    pub list: ListCursor,
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
        // A source that cannot search the text says why: a find bar that answers "No matches" for words nobody
        // looked for would be a wrong answer. The engine's source searches every PDF it opened while that opening is
        // the session's, so one it cannot search is a PDF whose session ended (locked, signed out, another account):
        // the press says so, and the view shows that state now (the source's poll), never "Not built yet".
        if !self.source.finds() {
            self.pump();
            self.toast = Some(FIND_SESSION_ENDED.to_string());
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

    /// The entries on screen, in the list's order: the whole list at the archive's top (as the
    /// board draws it, folders by their full path), else what is under the folder stepped into.
    pub fn entries(&self) -> Vec<&Entry> {
        let dir = &self.media.dir;
        match &self.file.body {
            Body::Archive(doc) => doc
                .entries
                .iter()
                .filter(|e| e.path.starts_with(dir.as_str()) && e.path != *dir)
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn select_entry(&mut self, row: usize) {
        if row < self.entries().len() {
            self.media.entry = Some(row);
        }
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

    /// Whether the player draws Play (the source can play); else "Open in another app".
    pub fn can_play(&self) -> bool {
        self.source.can_play()
    }

    /// Space and the round button of a recording, Space and the bar's button of a video: play or pause THIS file.
    /// Whether it plays is the player's word for this file only (`PlayerHandle::state_of`): with another file playing,
    /// or this one refused, the press is a Play, and the source opens this file for the press's session (its first
    /// Play too: a Files video opens here). A source with no player: the recording's own doc says.
    pub fn toggle_play(&mut self) {
        if !self.source.can_play() {
            return;
        }
        let live = self
            .source
            .player()
            .map(|player| player.state_of(&self.file.id).is_some_and(|state| state.playing));
        let on = match &self.file.body {
            Body::Audio(doc) => !live.unwrap_or(doc.playing),
            Body::Video(_) => !live.unwrap_or(false),
            _ => return,
        };
        if self.source.play(&self.file.id, on)
            && let Body::Audio(doc) = &mut self.file.body
        {
            doc.playing = on;
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

/// The volume in percent, on the 10 % steps (0 to 100).
pub fn volume_percent(player: &PlayerHandle) -> i32 {
    (player.volume() * 100.0).round() as i32
}

/// Up / Down on a recording or a video: one step of the volume, stopping at 0 and 100 %. A volume change while muted
/// is heard: the sound comes back at the new volume. What the toast says.
pub fn step_volume(player: &PlayerHandle, steps: i32) -> String {
    let at = volume_percent(player);
    // Back on the grid first (a volume set elsewhere), then the step.
    let snapped = (at as f32 / VOLUME_STEP as f32).round() as i32 * VOLUME_STEP;
    let to = (snapped + steps * VOLUME_STEP).clamp(0, 100);
    player.set_volume(to as f32 / 100.0);
    if player.muted() {
        player.set_muted(false);
    }
    format!("Volume {to} %")
}

/// M on a recording or a video: muted, or the sound back at its volume. What the toast says.
pub fn toggle_mute(player: &PlayerHandle) -> &'static str {
    let muted = !player.muted();
    player.set_muted(muted);
    if muted { "Muted" } else { "Sound on" }
}

/// Home / End on a photo: the strip's first / last photo.
pub fn jump(s: &mut State, last: bool) -> bool {
    let Some(id) = s.viewer.ui.as_ref().and_then(|ui| match &ui.file.body {
        Body::Photo(doc) => {
            let to = if last { doc.strip.last() } else { doc.strip.first() };
            to.map(|item| (item.id.clone(), item.current))
        }
        _ => None,
    }) else {
        return false;
    };
    match id {
        // Already there: the key is still the strip's.
        (_, true) => true,
        (id, false) => switch_to(s, &id),
    }
}

/// A monotonic clock for the archive list's type-ahead.
fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// The keys of the archive's list on the one list model (`ui::list_keys`): Up/Down, Home/End, PageUp/PageDown,
/// Enter (a folder's list), Alt+Up (one folder up) and type-ahead on the entries' names. A read-only, single-select
/// list: Shift/Ctrl moves are plain moves, Space, F2 and Delete are not its keys, and no entry has a menu.
fn archive_key(ui: &mut Ui, ev: &KeyEvent, now: u64) -> bool {
    let m = &ev.mods;
    let ctrl = m.ctrl || m.cmd;
    let ours = match &ev.key {
        Key::Up => true,
        Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown => !m.alt,
        Key::Enter => !m.alt && !ctrl && !m.shift,
        // Printable text types ahead (AltGr, Ctrl+Alt, still types); Space is not this list's.
        Key::Text(t) => t != " " && ctrl == m.alt,
        _ => false,
    };
    if !ours {
        return false;
    }
    let names: Vec<String> = ui
        .entries()
        .iter()
        .map(|e| {
            e.path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let keys = ListKeys {
        len: names.len(),
        page: ARCHIVE_PAGE,
        multi: false,
    };
    ui.media.list.focus = ui.media.entry;
    ui.media.list.anchor = ui.media.entry;
    let mut cursor = std::mem::take(&mut ui.media.list);
    let action = on_list_key(ev, &keys, &mut cursor, now, |i| names[i].clone());
    ui.media.list = cursor;
    match action {
        ListAction::Move { to, .. } => {
            ui.media.entry = Some(to);
            true
        }
        ListAction::Open(row) => ui.enter_entry(row),
        ListAction::Parent => ui.archive_up(),
        // Typed text with no match is still the list's: it went to the type-ahead buffer.
        ListAction::None => matches!(ev.key, Key::Text(_)),
        ListAction::Toggle(_) | ListAction::Rename(_) | ListAction::Delete | ListAction::Menu(_) => false,
    }
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
    if matches!(ui.file.body, Body::Archive(_)) && archive_key(ui, ev, now_ms()) {
        return true;
    }
    match (&ui.file.body, &ev.key) {
        (Body::Photo(_), Key::Left) if plain => step(s, -1),
        (Body::Photo(_), Key::Right) if plain => step(s, 1),
        (Body::Photo(_), Key::Home) if plain => jump(s, false),
        (Body::Photo(_), Key::End) if plain => jump(s, true),
        // Up / Down: the volume in 10 % steps; M: mute. Each change is said in the toast (a polite live region).
        (Body::Audio(_) | Body::Video(_), Key::Up | Key::Down) if plain => match ui.source.player() {
            Some(handle) => {
                ui.toast = Some(step_volume(&handle, if ev.key == Key::Up { 1 } else { -1 }));
                true
            }
            None => false,
        },
        (Body::Audio(_) | Body::Video(_), Key::Text(key)) if plain && key.eq_ignore_ascii_case("m") => {
            match ui.source.player() {
                Some(handle) => {
                    ui.toast = Some(toggle_mute(&handle).to_string());
                    true
                }
                None => false,
            }
        }
        // Left / Right seek 5 s in a recording or a video (Ctrl+Left / Right: the neighbouring recording).
        (Body::Audio(_) | Body::Video(_), Key::Left | Key::Right) if plain => {
            let delta = if ev.key == Key::Left { -1 } else { 1 };
            match ui.source.player() {
                Some(handle) => {
                    super::video::step_seek(&handle, Some(&ui.file.id), delta);
                    true
                }
                None => false,
            }
        }
        (Body::Audio(_), Key::Left) if ctrl => step(s, -1),
        (Body::Audio(_), Key::Right) if ctrl => step(s, 1),
        (Body::Audio(_), Key::Text(key)) if plain && key == " " => {
            ui.toggle_play();
            true
        }
        // Space plays and pauses a video (not while a text field has the keys: this view has none), the same press as
        // the bar's button: the first one opens the video through the source. Esc is the viewer's own: it closes the
        // viewer, and closing stops the video (`closing_a_video_stops_it`).
        (Body::Video(_), Key::Text(key)) if plain && key == " " && super::video::space_toggles(false) => {
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
        (Body::Archive(_), Key::Backspace) if plain => ui.archive_up(),
        _ => false,
    }
}
