//! Photos (the board's group "Photos", DW-0601 to DW-0611 and DW-0116): the timeline by date,
//! selecting, one photo open, and the menus and dialogs over them. `FRAMES.md` is the checklist.
//!
//! [`Ui`] is everything the group keeps; every press and key goes through one of its methods,
//! and a scene (`scene`) reaches a board frame through those same methods. What the group
//! cannot do itself (another lane's dialog, an engine call that is not wired yet) it records as
//! an [`Intent`]: the shell and the engine lane take them from [`Ui::take_intents`].

mod chrome;
mod grid;
mod icons;
mod overlay;
pub mod source;
#[cfg(test)]
mod tests;
mod viewer;

use std::collections::BTreeMap;

use rune::{Cx, El, Key, KeyEvent, Tx};

use crate::State;
use crate::shell::app::{Event, Route, Section};
use crate::theme::tokens;
use source::{FixturePhotos, Kind, Photo, PhotoId, PhotosSource, SourceRef, Viewing};

/// The choices of "Photos across", largest tiles first.
pub const ACROSS: [usize; 4] = [3, 4, 5, 7];

/// Something this group asks of the shell or the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// New: "adds to Files" (DW-0601), the Files lane's New menu.
    NewInFiles,
    /// Search names, submitted: the Search frames (DW-0801).
    Search(String),
    /// Add a folder… in Photo sources: the folder picker of Files.
    AddFolder,
    /// Share… on a selection: the share dialog of Files, for several items.
    ShareSeveral(Vec<PhotoId>),
    /// Make available offline, on a selection.
    KeepSeveral(Vec<PhotoId>),
    /// Move… (DW-0409).
    Move(Vec<PhotoId>),
    /// Save to PC…: a save panel, then a normal decrypted copy.
    SaveToPc(PhotoId),
    Rename(PhotoId),
    Duplicate(PhotoId),
    /// Copy link in Share (DW-0107 says so afterwards).
    CopyLink {
        id: PhotoId,
        can_edit: bool,
        days: Option<u32>,
        password: bool,
    },
    /// Link expires, pressed: the choices are the Files share dialog's.
    ChooseExpiry(PhotoId),
    /// Send a copy…: decrypts, then the Windows Share menu.
    SendCopy(PhotoId),
    /// Original asked for: download the full file. `StopOriginal` when Preview is chosen meanwhile.
    DownloadOriginal(PhotoId),
    StopOriginal(PhotoId),
    /// "On this PC", pressed: offer Remove from this PC (DW-0608).
    OfferRemoveFromPc(PhotoId),
    /// Version history, in Details.
    VersionHistory(PhotoId),
    /// The choice of photos across changed: "remembered on this PC" (DW-0603).
    RememberAcross(usize),
    /// Play or pause the video (DW-0611).
    PlayPause(PhotoId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// Photos across (DW-0603).
    Across,
    /// The ⋯ menu of an open photo (DW-0606).
    More,
}

/// An open menu and the row under the pointer or the arrow keys (rows count the heading).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenMenu {
    pub which: Menu,
    pub hot: Option<usize>,
}

/// What Photo sources will apply on Done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcesDraft {
    pub devices: Vec<bool>,
    pub folders: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareDraft {
    pub can_edit: bool,
    /// Days until the link expires; `None`: never.
    pub days: Option<u32>,
    pub password: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    Sources(SourcesDraft),
    Share(ShareDraft),
    Details,
}

/// Which rendition of an open photo shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Original {
    /// The stored preview.
    Preview,
    /// The original is downloading; the preview stays on screen (DW-0605b).
    Downloading {
        done: u64,
    },
    Whole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer {
    pub id: PhotoId,
    pub original: Original,
    pub playing: bool,
    /// The photo just moved to Trash, as it was: while Undo is offered the top bar and the
    /// strip still show it and the next photo is already on the stage (DW-0610).
    pub gone: Option<Viewing>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Toast {
    /// "Keeping the original on this PC" (DW-0608).
    Kept,
    /// "Moved to Trash", with Undo (DW-0610).
    Trashed(Vec<PhotoId>),
}

/// One row of the grid: a day's heading, if this is its first row, and up to `across` tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub day: usize,
    pub start: usize,
    pub len: usize,
    pub head: bool,
}

pub struct Ui {
    pub(crate) source: Box<dyn PhotosSource>,
    pub across: usize,
    /// The device whose photos show (`devices()` index); `None`: All.
    pub filter: Option<usize>,
    pub selecting: bool,
    /// The checked photos and their sizes.
    pub selected: BTreeMap<PhotoId, u64>,
    /// The tile the arrow keys are on: (day, index).
    pub cursor: Option<(usize, usize)>,
    pub menu: Option<OpenMenu>,
    pub dialog: Option<Dialog>,
    pub viewer: Option<Viewer>,
    pub toast: Option<Toast>,
    intents: Vec<Intent>,
    pub(crate) days: Vec<source::Day>,
    pub(crate) lines: Vec<Line>,
    /// The grid's width in points, as last laid out.
    width: f32,
}

impl Ui {
    pub fn new(source: Box<dyn PhotosSource>) -> Ui {
        let mut ui = Ui {
            source,
            across: 7,
            filter: None,
            selecting: false,
            selected: BTreeMap::new(),
            cursor: None,
            menu: None,
            dialog: None,
            viewer: None,
            toast: None,
            intents: Vec::new(),
            days: Vec::new(),
            lines: Vec::new(),
            // The content area of the 1280 x 800 window; the first layout corrects it.
            width: 1058.0,
        };
        ui.relayout();
        ui
    }

    /// Reads the days again and cuts them into rows. After anything that changes what shows.
    pub fn relayout(&mut self) {
        self.days = self.source.days(self.filter);
        self.lines.clear();
        for (day, d) in self.days.iter().enumerate() {
            let mut start = 0;
            while start < d.len {
                let len = (d.len - start).min(self.across);
                self.lines.push(Line {
                    day,
                    start,
                    len,
                    head: start == 0,
                });
                start += len;
            }
        }
        if let Some((day, index)) = self.cursor
            && self.days.get(day).is_none_or(|d| index >= d.len)
        {
            self.cursor = None;
        }
    }

    pub fn set_width(&mut self, width: f32) {
        if (width - self.width).abs() > 0.25 && width > 0.0 {
            self.width = width;
        }
    }

    /// A tile's side: the grid's width less its 16 pt margins and the 3 pt gaps, by the number across.
    pub fn tile_side(&self) -> f32 {
        let n = self.across as f32;
        ((self.width - 32.0 - (n - 1.0) * 3.0) / n).max(8.0)
    }

    pub fn photo_at(&self, day: usize, index: usize) -> Option<Photo> {
        self.source.photo(self.filter, day, index)
    }

    pub fn ask(&mut self, intent: Intent) {
        self.intents.push(intent);
    }

    /// What the group asked for since the last call, in order.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    // ---- the grid ----

    pub fn set_filter(&mut self, device: Option<usize>) {
        self.filter = device;
        self.selected.clear();
        self.relayout();
    }

    pub fn set_across(&mut self, across: usize) {
        if ACROSS.contains(&across) && across != self.across {
            self.across = across;
            self.ask(Intent::RememberAcross(across));
            self.relayout();
        }
        self.menu = None;
    }

    /// Ctrl+Plus (`larger`) and Ctrl+−: the next choice of photos across, or the same at an end.
    pub fn across_after(&self, larger: bool) -> usize {
        let at = ACROSS
            .iter()
            .position(|&n| n == self.across)
            .unwrap_or(ACROSS.len() - 1);
        ACROSS[if larger {
            at.saturating_sub(1)
        } else {
            (at + 1).min(ACROSS.len() - 1)
        }]
    }

    pub fn start_selecting(&mut self) {
        self.selecting = true;
        self.menu = None;
    }

    pub fn end_selecting(&mut self) {
        self.selecting = false;
        self.selected.clear();
    }

    pub fn toggle(&mut self, photo: Photo) {
        self.selecting = true;
        if self.selected.remove(&photo.id).is_none() {
            self.selected.insert(photo.id, photo.bytes);
        }
    }

    pub fn select_all(&mut self) {
        self.selecting = true;
        for (day, d) in self.days.iter().enumerate() {
            for index in 0..d.len {
                if let Some(photo) = self.source.photo(self.filter, day, index) {
                    self.selected.insert(photo.id, photo.bytes);
                }
            }
        }
    }

    fn selected_ids(&self) -> Vec<PhotoId> {
        self.selected.keys().copied().collect()
    }

    /// A press on a tile: checks it while selecting, otherwise puts the keyboard there.
    pub fn press_tile(&mut self, day: usize, index: usize) {
        self.cursor = Some((day, index));
        if self.selecting
            && let Some(photo) = self.photo_at(day, index)
        {
            self.toggle(photo);
        }
    }

    /// Ctrl+click on a tile: checks or unchecks it; the first one starts selecting.
    pub fn check_tile(&mut self, day: usize, index: usize) {
        self.cursor = Some((day, index));
        if let Some(photo) = self.photo_at(day, index) {
            self.toggle(photo);
        }
    }

    /// A double press, Enter or Space on a tile: opens it. Not while selecting: there a press
    /// checks, and the second press of a double press has already unchecked.
    pub fn open_tile(&mut self, day: usize, index: usize) {
        if self.selecting {
            return;
        }
        if let Some(photo) = self.photo_at(day, index) {
            self.cursor = Some((day, index));
            self.open(photo.id);
        }
    }

    fn move_cursor(&mut self, dx: isize, dy: isize) {
        let Some((day, index)) = self.cursor else {
            if !self.days.is_empty() {
                self.cursor = Some((0, 0));
            }
            return;
        };
        // The cursor's row among the lines, and its column.
        let Some(mut line) = self
            .lines
            .iter()
            .position(|l| l.day == day && l.start <= index && index < l.start + l.len)
        else {
            return;
        };
        let mut column = index - self.lines[line].start;
        if dy != 0 {
            let next = line as isize + dy;
            if next < 0 || next as usize >= self.lines.len() {
                return;
            }
            line = next as usize;
            column = column.min(self.lines[line].len - 1);
        } else if dx < 0 {
            if column > 0 {
                column -= 1;
            } else if line > 0 {
                line -= 1;
                column = self.lines[line].len - 1;
            }
        } else if column + 1 < self.lines[line].len {
            column += 1;
        } else if line + 1 < self.lines.len() {
            line += 1;
            column = 0;
        }
        self.cursor = Some((self.lines[line].day, self.lines[line].start + column));
    }

    // ---- menus ----

    pub fn toggle_menu(&mut self, which: Menu) {
        self.menu = match self.menu {
            Some(open) if open.which == which => None,
            _ => Some(OpenMenu { which, hot: None }),
        };
    }

    /// The rows of a menu that can be chosen (the heading of Photos across cannot).
    fn menu_rows(which: Menu) -> std::ops::Range<usize> {
        match which {
            Menu::Across => 1..ACROSS.len() + 1,
            Menu::More => 0..4,
        }
    }

    pub fn pick(&mut self, which: Menu, row: usize) {
        self.menu = None;
        match which {
            Menu::Across => {
                if let Some(&across) = row.checked_sub(1).and_then(|i| ACROSS.get(i)) {
                    self.set_across(across);
                }
            }
            Menu::More => {
                let Some(id) = self.viewer.as_ref().map(|v| v.id) else {
                    return;
                };
                match row {
                    0 => self.ask(Intent::SaveToPc(id)),
                    1 => self.ask(Intent::Move(vec![id])),
                    2 => self.ask(Intent::Rename(id)),
                    3 => self.ask(Intent::Duplicate(id)),
                    _ => {}
                }
            }
        }
    }

    // ---- dialogs ----

    pub fn open_sources(&mut self) {
        self.menu = None;
        self.dialog = Some(Dialog::Sources(SourcesDraft {
            devices: self.source.devices().iter().map(|d| d.shown).collect(),
            folders: self.source.folders().iter().map(|f| f.shown).collect(),
        }));
    }

    pub fn toggle_source(&mut self, source: SourceRef) {
        if let Some(Dialog::Sources(draft)) = &mut self.dialog {
            let slot = match source {
                SourceRef::Device(i) => draft.devices.get_mut(i),
                SourceRef::Folder(i) => draft.folders.get_mut(i),
            };
            if let Some(shown) = slot {
                *shown = !*shown;
            }
        }
    }

    pub fn open_share(&mut self) {
        if self.viewer.is_some() {
            self.menu = None;
            self.dialog = Some(Dialog::Share(ShareDraft {
                can_edit: false,
                days: Some(30),
                password: false,
            }));
        }
    }

    pub fn open_details(&mut self) {
        if self.viewer.is_some() {
            self.menu = None;
            self.dialog = Some(Dialog::Details);
        }
    }

    /// Cancel, Esc, or Done where Done changes nothing (Details).
    pub fn close_dialog(&mut self) {
        self.dialog = None;
    }

    /// The dialog's first button: Done in Photo sources and Details, Copy link in Share.
    pub fn confirm_dialog(&mut self) {
        match self.dialog.take() {
            Some(Dialog::Sources(draft)) => {
                for (i, shown) in draft.devices.into_iter().enumerate() {
                    self.source.set_shown(SourceRef::Device(i), shown);
                }
                for (i, shown) in draft.folders.into_iter().enumerate() {
                    self.source.set_shown(SourceRef::Folder(i), shown);
                }
                // A pill that went away cannot stay chosen.
                if let Some(device) = self.filter
                    && !self.source.devices().get(device).is_some_and(|d| d.shown)
                {
                    self.filter = None;
                }
                self.relayout();
            }
            Some(Dialog::Share(draft)) => {
                if let Some(viewer) = &self.viewer {
                    let id = viewer.id;
                    self.ask(Intent::CopyLink {
                        id,
                        can_edit: draft.can_edit,
                        days: draft.days,
                        password: draft.password,
                    });
                }
            }
            Some(Dialog::Details) | None => {}
        }
    }

    // ---- one photo open ----

    pub fn open(&mut self, id: PhotoId) {
        if self.source.viewing(id).is_some() {
            self.viewer = Some(Viewer {
                id,
                original: Original::Preview,
                playing: true,
                gone: None,
            });
            self.menu = None;
            self.toast = None;
        }
    }

    pub fn close_viewer(&mut self) {
        if let Some(viewer) = self.viewer.take()
            && matches!(viewer.original, Original::Downloading { .. })
        {
            self.ask(Intent::StopOriginal(viewer.id));
        }
        self.menu = None;
        self.dialog = None;
        self.toast = None;
        self.relayout();
    }

    /// What the top bar and the strip show: the open photo, or the one just trashed while
    /// Undo is offered.
    pub fn viewing(&self) -> Option<Viewing> {
        let viewer = self.viewer.as_ref()?;
        viewer.gone.clone().or_else(|| self.source.viewing(viewer.id))
    }

    /// ← and →, and a press on a thumbnail of the strip.
    pub fn step(&mut self, by: isize) {
        let Some(viewing) = self.viewer.as_ref().and_then(|v| self.source.viewing(v.id)) else {
            return;
        };
        let Some(at) = viewing.strip.iter().position(|item| item.current) else {
            return;
        };
        let next = at as isize + by;
        if next >= 0
            && let Some(item) = viewing.strip.get(next as usize)
        {
            self.jump(item.id);
        }
    }

    pub fn jump(&mut self, id: PhotoId) {
        let Some(viewer) = &mut self.viewer else { return };
        if viewer.id == id && viewer.gone.is_none() {
            return;
        }
        // A download carries on in the background (DW-0605b); the next photo shows its preview.
        viewer.id = id;
        viewer.original = Original::Preview;
        viewer.playing = true;
        viewer.gone = None;
        self.toast = None;
        self.menu = None;
    }

    pub fn show_preview(&mut self) {
        let Some(viewer) = &mut self.viewer else { return };
        let was = std::mem::replace(&mut viewer.original, Original::Preview);
        if matches!(was, Original::Downloading { .. }) {
            let id = viewer.id;
            self.ask(Intent::StopOriginal(id));
        }
    }

    pub fn show_original(&mut self) {
        let Some(viewer) = &mut self.viewer else { return };
        if viewer.original == Original::Preview {
            viewer.original = Original::Downloading { done: 0 };
            let id = viewer.id;
            self.ask(Intent::DownloadOriginal(id));
        }
    }

    /// The engine reports how much of the original has arrived.
    pub fn original_progress(&mut self, id: PhotoId, done: u64, total: u64) {
        if let Some(viewer) = &mut self.viewer
            && viewer.id == id
            && matches!(viewer.original, Original::Downloading { .. })
        {
            viewer.original = if done >= total {
                Original::Whole
            } else {
                Original::Downloading { done }
            };
        }
    }

    /// Offline: keeps the original on this PC; pressed again, asks to offer its removal.
    pub fn press_offline(&mut self) {
        let Some(id) = self.viewer.as_ref().map(|v| v.id) else {
            return;
        };
        if self.source.kept_offline(id) {
            self.ask(Intent::OfferRemoveFromPc(id));
        } else {
            self.source.keep_offline(id, true);
            self.toast = Some(Toast::Kept);
        }
    }

    /// Trash on an open photo: no question, the next one comes in, Undo is offered.
    pub fn trash_open(&mut self) {
        let Some(viewer) = &self.viewer else { return };
        if viewer.gone.is_some() {
            return;
        }
        let Some(was) = self.source.viewing(viewer.id) else {
            return;
        };
        let at = was.strip.iter().position(|item| item.current).unwrap_or(0);
        let next = was
            .strip
            .get(at + 1)
            .or_else(|| at.checked_sub(1).and_then(|i| was.strip.get(i)));
        let (id, next) = (was.id, next.map(|item| item.id));
        self.source.trash(&[id]);
        self.selected.remove(&id);
        self.toast = Some(Toast::Trashed(vec![id]));
        self.menu = None;
        match (next, &mut self.viewer) {
            (Some(next), Some(viewer)) => {
                viewer.id = next;
                viewer.original = Original::Preview;
                viewer.gone = Some(was);
            }
            // The last one: back to the grid, where Undo is still offered.
            _ => {
                self.viewer = None;
            }
        }
        self.relayout();
    }

    /// Move to Trash on a selection.
    pub fn trash_selected(&mut self) {
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        self.source.trash(&ids);
        self.end_selecting();
        self.toast = Some(Toast::Trashed(ids));
        self.relayout();
    }

    /// Undo, Ctrl+Z: brings back what the message is about.
    pub fn undo(&mut self) -> bool {
        let Some(Toast::Trashed(ids)) = self.toast.clone() else {
            return false;
        };
        self.source.restore(&ids);
        self.toast = None;
        if let Some(viewer) = &mut self.viewer
            && let Some(was) = viewer.gone.take()
        {
            viewer.id = was.id;
        }
        self.relayout();
        true
    }

    pub fn play_pause(&mut self) {
        if let Some(viewer) = &mut self.viewer {
            viewer.playing = !viewer.playing;
            let id = viewer.id;
            self.ask(Intent::PlayPause(id));
        }
    }
}

/// The group's state in a new window: the board's library until the engine's source is wired.
pub fn start() -> Ui {
    Ui::new(Box::new(FixturePhotos::new()))
}

// ---------------------------------------------------------------------------
// The frame and its keys
// ---------------------------------------------------------------------------

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    let ui = &s.photos;
    let mut front = Vec::new();
    if let Some(open) = ui.menu {
        front.push(overlay::menu(ui, open, &t));
    }
    if let Some(dialog) = &ui.dialog {
        front.push(overlay::scrim(&t));
        front.push(overlay::dialog(ui, dialog, cx.env().scale, &t));
    }
    match ui.viewing() {
        Some(viewing) => chrome::window(
            s,
            cx,
            &t,
            viewer::bar(ui, &viewing, &t),
            None,
            viewer::stage(ui, &viewing, &t),
            &viewer::status(&viewing),
            front,
        ),
        None => chrome::window(
            s,
            cx,
            &t,
            grid::bar(ui, &t),
            ui.selecting.then(|| grid::bulk(ui, &t)),
            grid::body(ui, &t),
            &ui.source.origin(),
            front,
        ),
    }
}

/// The group's keys. The shell's own shortcuts ran first (`host::on_key`).
pub fn on_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let ui = &mut s.photos;
    let plain = !ev.mods.cmd && !ev.mods.alt;
    let space = matches!(&ev.key, Key::Text(t) if t == " ");

    if ui.dialog.is_some() {
        return match ev.key {
            Key::Escape => {
                ui.close_dialog();
                true
            }
            Key::Enter if plain => {
                ui.confirm_dialog();
                true
            }
            _ => false,
        };
    }
    if let Some(open) = ui.menu {
        let rows = Ui::menu_rows(open.which);
        return match ev.key {
            Key::Escape => {
                ui.menu = None;
                true
            }
            Key::Down | Key::Up => {
                let down = ev.key == Key::Down;
                let hot = match open.hot {
                    None => {
                        if down {
                            rows.start
                        } else {
                            rows.end - 1
                        }
                    }
                    Some(hot) if down => (hot + 1).min(rows.end - 1),
                    Some(hot) => hot.saturating_sub(1).max(rows.start),
                };
                ui.menu = Some(OpenMenu { hot: Some(hot), ..open });
                true
            }
            Key::Enter => {
                match open.hot {
                    Some(hot) => ui.pick(open.which, hot),
                    None => ui.menu = None,
                }
                true
            }
            _ => false,
        };
    }
    if ev.key == Key::Command('z') && ui.undo() {
        return true;
    }
    if ui.viewer.is_some() {
        let video = ui.viewing().is_some_and(|v| matches!(v.kind, Kind::Video(_)));
        return match ev.key {
            Key::Escape => {
                ui.close_viewer();
                true
            }
            Key::Left if plain => {
                ui.step(-1);
                true
            }
            Key::Right if plain => {
                ui.step(1);
                true
            }
            Key::Delete => {
                ui.trash_open();
                true
            }
            // Space closes a photo and plays or pauses a video (DW-0605, DW-0611).
            _ if space && video => {
                ui.play_pause();
                true
            }
            _ if space => {
                ui.close_viewer();
                true
            }
            _ => false,
        };
    }
    match ev.key {
        Key::Escape if ui.selecting => {
            ui.end_selecting();
            true
        }
        Key::Command('+') | Key::Command('=') => {
            ui.set_across(ui.across_after(true));
            true
        }
        Key::Command('-') => {
            ui.set_across(ui.across_after(false));
            true
        }
        Key::Command('a') if ui.selecting => {
            ui.select_all();
            true
        }
        Key::Left if plain => {
            ui.move_cursor(-1, 0);
            true
        }
        Key::Right if plain => {
            ui.move_cursor(1, 0);
            true
        }
        Key::Up if plain => {
            ui.move_cursor(0, -1);
            true
        }
        Key::Down if plain => {
            ui.move_cursor(0, 1);
            true
        }
        Key::Delete if ui.selecting => {
            ui.trash_selected();
            true
        }
        Key::Enter if plain => match ui.cursor {
            Some((day, index)) if ui.selecting => {
                ui.press_tile(day, index);
                true
            }
            Some((day, index)) => {
                ui.open_tile(day, index);
                true
            }
            None => false,
        },
        _ if space => match ui.cursor {
            Some((day, index)) if ui.selecting => {
                ui.press_tile(day, index);
                true
            }
            Some((day, index)) => {
                ui.open_tile(day, index);
                true
            }
            None => false,
        },
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Scenes: each board frame, reached through the same calls a person's presses make
// ---------------------------------------------------------------------------

/// The state of one board frame, by its id. `None`: not a frame of this group (or not built).
pub fn scene(frame: &str) -> Option<Ui> {
    use source::board;
    let id = |(day, index): (usize, usize)| PhotoId((day * 1000 + index) as u64);
    let mut ui = start();
    let open_harbour = |ui: &mut Ui| ui.open_tile(board::HARBOUR.0, board::HARBOUR.1);
    match frame {
        "DW-0601" => {}
        "DW-0116" => ui = Ui::new(Box::new(FixturePhotos::empty())),
        "DW-0602" => {
            ui.start_selecting();
            for (day, index) in board::CHECKED {
                ui.press_tile(day, index);
            }
            ui.cursor = None;
        }
        "DW-0603" => {
            ui.toggle_menu(Menu::Across);
            // The board draws the pointer on "5".
            ui.menu = Some(OpenMenu {
                which: Menu::Across,
                hot: Some(3),
            });
        }
        "DW-0604" => ui.open_sources(),
        "DW-0605" => open_harbour(&mut ui),
        "DW-0605b" => {
            open_harbour(&mut ui);
            ui.show_original();
            // 40 % of 30.9 MB, as the board's bar is; it reads "12.4 of 30.9 MB".
            ui.original_progress(id(board::HARBOUR), 12_360_000, 30_900_000);
        }
        "DW-0606" => {
            open_harbour(&mut ui);
            ui.toggle_menu(Menu::More);
            ui.menu = Some(OpenMenu {
                which: Menu::More,
                hot: Some(0),
            });
        }
        "DW-0607" => {
            open_harbour(&mut ui);
            ui.open_share();
        }
        "DW-0608" => {
            open_harbour(&mut ui);
            ui.press_offline();
        }
        "DW-0609" => {
            open_harbour(&mut ui);
            ui.open_details();
        }
        "DW-0610" => {
            open_harbour(&mut ui);
            ui.trash_open();
        }
        "DW-0611" => ui.open_tile(board::VIDEO.0, board::VIDEO.1),
        _ => return None,
    }
    ui.take_intents();
    Some(ui)
}

/// Put the window in a board frame's state (`shell/mod.rs` `SCENES`).
pub fn set_scene(s: &mut State, frame: &str) {
    crate::shell::host::dispatch(s, Event::Navigate(Route::section(Section::Photos)));
    if let Some(ui) = scene(frame) {
        s.photos = ui;
    }
}
