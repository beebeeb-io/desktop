//! Live feedback mode: click any element of the running app, type a note, press Enter. Each
//! note lands in a LOCAL inbox folder with the window's own render, the element's crop and its
//! place in Rune's element tree; a panel lists the notes with what happened to each.
//!
//! Compiled in only with the cargo feature `feedback` (test builds). A release build has none
//! of it: `scripts/check-feedback-absent.sh`. Schema, keys and limits: `README.md` beside this file.
//!
//! This module has no network code (a test greps for it): the app itself sends a note nowhere.

pub mod capture;
pub mod frame;
pub mod model;
pub mod paths;
mod ui;

#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use rune::{App, AppSpec, Cx, El, Key, KeyEvent, Pixels, Runtime, TextEdit, Tx};
use serde_json::{Value, json};

use crate::State;
use model::{ElementSnap, NoteEntry, NoteMeta, Rgba};

/// Seconds between looks at the inbox for new notes and `status.json` changes. Never faster.
pub const SCAN_EVERY: f64 = 2.0;
const ROOT_KEY: &str = "fb-root";

/// The hotkeys, in one place: `(key, with Shift)` and the words the UI shows for them.
pub const COMMENT_KEY: (Key, bool) = (Key::Function(8), false);
pub const COMMENT_KEY_LABEL: &str = "F8";
pub const PANEL_KEY: (Key, bool) = (Key::Function(8), true);
pub const PANEL_KEY_LABEL: &str = "Shift+F8";

/// At most this many thumbnails are decoded for the panel (newest notes first).
const MAX_THUMBS: usize = 40;

/// Whether comment mode may run on a frame: what the frame's group declares in its ONE line of
/// the shell's frame registry (`shell::registry::FRAMES`). A frame that says nothing is denied.
pub use crate::shell::registry::FeedbackPolicy as Policy;

/// The frame on screen, as the shell's route machine and frame registry report it (`frame.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub id: String,
    pub route: String,
    pub feedback: Policy,
}

#[cfg(test)]
impl FrameInfo {
    pub fn allowed(id: &str, route: &str) -> FrameInfo {
        FrameInfo {
            id: id.into(),
            route: route.into(),
            feedback: Policy::Allowed,
        }
    }

    pub fn denied(id: &str, route: &str, reason: &'static str) -> FrameInfo {
        FrameInfo {
            id: id.into(),
            route: route.into(),
            feedback: Policy::Denied(reason),
        }
    }
}

pub const DENIED_LINE: &str = "Comment mode is off on this screen";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    /// Comment mode: the element under the pointer is outlined; a click or Enter picks it.
    Picking,
    /// The note popover is open on the picked element.
    Writing,
    /// The one frame drawn without any feedback chrome, which the capture reads.
    Capturing,
}

/// What the app's `State` carries for this mode.
pub struct Feedback {
    pub mode: Mode,
    pub pointer: Option<(f32, f32)>,
    resolved_for: Option<(f32, f32)>,
    /// The element under the pointer and its ancestors up to the app's root, nearest first.
    pub chain: Vec<ElementSnap>,
    /// What each of `chain` and its subtree said when it was outlined (`model::fingerprint`).
    prints: Vec<String>,
    /// Which of `chain` is outlined: Alt+Up and Alt+Down step it.
    pub level: usize,
    pub note: TextEdit,
    pub tags: [bool; model::TAGS.len()],
    pub panel_open: bool,
    /// The note open in the panel, by id.
    pub opened: Option<String>,
    pub notes: Vec<NoteEntry>,
    pub inbox: Option<PathBuf>,
    /// One line in the hint bar or the panel: "Note saved", or why one was not.
    pub message: Option<String>,
    pub secure_on_screen: bool,
    dark: Cell<bool>,
    last_scan: Option<f64>,
    /// A failed save puts the keyboard back in the note field on the next frame.
    refocus: bool,
    thumbs: RefCell<HashMap<String, Pixels>>,
    opened_image: Option<Pixels>,
    #[cfg(test)]
    pub frame_override: Option<FrameInfo>,
}

impl Feedback {
    pub fn new() -> Feedback {
        Feedback::with_inbox(paths::inbox())
    }

    pub fn with_inbox(inbox: Option<PathBuf>) -> Feedback {
        let mut fb = Feedback {
            mode: Mode::Off,
            pointer: None,
            resolved_for: None,
            chain: Vec::new(),
            prints: Vec::new(),
            level: 0,
            note: TextEdit::new(""),
            tags: [false; model::TAGS.len()],
            panel_open: false,
            opened: None,
            notes: Vec::new(),
            inbox,
            message: None,
            secure_on_screen: false,
            dark: Cell::new(false),
            last_scan: None,
            refocus: false,
            thumbs: RefCell::new(HashMap::new()),
            opened_image: None,
            #[cfg(test)]
            frame_override: None,
        };
        // Start-up reads the small JSON files only; no picture is decoded until the panel opens.
        if let Some(inbox) = &fb.inbox {
            model::sweep_temp(inbox);
        }
        fb.rescan();
        fb
    }

    pub fn target(&self) -> Option<&ElementSnap> {
        self.chain.get(self.level)
    }

    fn enter(&mut self, tx: &mut Tx) {
        self.mode = Mode::Picking;
        self.panel_open = false;
        self.resolved_for = None;
        self.chain.clear();
        self.prints.clear();
        self.level = 0;
        self.message = None;
        tx.blur();
    }

    fn leave(&mut self) {
        self.mode = Mode::Off;
        self.chain.clear();
        self.prints.clear();
        self.note.clear();
        self.tags = [false; model::TAGS.len()];
    }

    fn pick(&mut self, tx: &mut Tx) {
        if self.mode == Mode::Picking && self.target().is_some() {
            self.mode = Mode::Writing;
            self.message = None;
            tx.focus(ui::NOTE_INPUT, false);
        }
    }

    /// Esc: the popover closes back to picking; picking leaves comment mode.
    fn back(&mut self) {
        match self.mode {
            Mode::Writing => {
                self.mode = Mode::Picking;
                self.note.clear();
                self.tags = [false; model::TAGS.len()];
            }
            Mode::Picking => self.leave(),
            Mode::Off | Mode::Capturing => {}
        }
    }

    /// Enter in the popover: the next frame draws without feedback chrome and is captured.
    /// `tx.settle()` unmounts the chrome at once: no exit transition is left in the picture.
    fn save(&mut self, tx: &mut Tx) {
        if self.mode != Mode::Writing {
            return;
        }
        if self.note.text.trim().is_empty() {
            self.message = Some("Type a note first.".into());
            return;
        }
        self.mode = Mode::Capturing;
        tx.settle();
    }

    fn step(&mut self, up: bool) {
        let top = self.chain.len().saturating_sub(1);
        self.level = if up {
            (self.level + 1).min(top)
        } else {
            self.level.saturating_sub(1)
        };
    }

    pub fn toggle_panel(&mut self) {
        self.panel_open = !self.panel_open;
        self.opened = None;
        self.opened_image = None;
        self.last_scan = None;
        self.message = None;
        if self.panel_open {
            self.rescan();
            self.load_thumbs();
        }
    }

    fn open_note(&mut self, id: &str) {
        self.opened_image = self
            .notes
            .iter()
            .find(|n| n.id == id)
            .map(|n| decode_later(n.dir.join("window.png"), 1400));
        self.opened = Some(id.to_string());
    }

    /// Reads the inbox again (only the JSON of notes whose files changed). True when the list or
    /// a status changed.
    pub fn rescan(&mut self) -> bool {
        let notes = self
            .inbox
            .as_deref()
            .map(|i| model::scan(i, &self.notes))
            .unwrap_or_default();
        self.thumbs
            .borrow_mut()
            .retain(|id, _| notes.iter().any(|n| &n.id == id));
        let changed = notes != self.notes;
        self.notes = notes;
        if self.panel_open {
            self.load_thumbs();
        }
        changed
    }

    /// Thumbnails for the newest notes, decoded off the UI thread and only while the panel is open.
    fn load_thumbs(&mut self) {
        let mut thumbs = self.thumbs.borrow_mut();
        for n in self.notes.iter().take(MAX_THUMBS) {
            if !thumbs.contains_key(&n.id) {
                // The whole window, small: an element's own crop is often a sliver.
                thumbs.insert(n.id.clone(), decode_later(n.dir.join("window.png"), 256));
            }
        }
    }

    #[cfg(test)]
    pub fn thumb_size(&self, id: &str) -> Option<(u32, u32)> {
        self.thumbs.borrow().get(id).map(Pixels::size)
    }
}

/// An image the panel can show at once and that fills in when a worker thread has decoded the
/// PNG (size-capped, at most `max` pixels on its longer side). A file that cannot be decoded
/// leaves it empty.
fn decode_later(path: PathBuf, max: u32) -> Pixels {
    let pixels = Pixels::new();
    let fill = pixels.clone();
    std::thread::spawn(move || {
        if let Ok(img) = Rgba::read_png(&path) {
            let small = img.shrink(max);
            fill.set(small.w, small.h, small.px);
        }
    });
    pixels
}

// ---------------------------------------------------------------------------
// Hook-up: the app's own view, key and poll handlers, wrapped
// ---------------------------------------------------------------------------

struct Inner {
    view: fn(&State, &mut Cx<State>) -> El,
    poll: Option<fn(&mut State, f64, &mut Tx) -> bool>,
}

/// The app's own key handler: what a key reaches when the feedback mode does not take it.
pub type AppKeys = fn(&mut State, &KeyEvent, &mut Tx) -> bool;

static INNER: OnceLock<Inner> = OnceLock::new();

fn inner() -> &'static Inner {
    INNER.get().expect("feedback::hook_spec ran before the first frame")
}

/// Wraps the main window's `view` and `poll` with comment mode (`shell::run` calls it; the
/// keys come through the shell's `host::on_key`, which hands them to [`on_key`]). The first
/// call wins.
pub fn hook_spec(mut spec: AppSpec<State>) -> AppSpec<State> {
    INNER.get_or_init(|| Inner {
        view: spec.view,
        poll: spec.poll,
    });
    spec.view = view;
    spec.poll = Some(poll);
    spec
}

/// The frame on screen: where the shell's route machine is, and what that frame group's line
/// in the frame registry says about comment mode. No line, or no opt-in: denied.
pub fn frame_of(s: &State) -> FrameInfo {
    #[cfg(test)]
    if let Some(f) = &s.feedback.frame_override {
        return f.clone();
    }
    frame::on_screen(&s.shell.app)
}

/// Attaches the compositor that reads the live window's tree and pixels.
pub fn install(app: App<State>) -> App<State> {
    app.compositor(capture::FeedbackCompositor::default())
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    s.feedback.dark.set(cx.system_dark());
    let app = (inner().view)(s, cx);
    ui::wrap(s, cx, app)
}

fn is_key(ev: &KeyEvent, key: &(Key, bool)) -> bool {
    ev.key == key.0 && ev.mods.shift == key.1
}

/// Every key of the main window, before the shell's own shortcuts (`shell::host::on_key`):
/// the two hotkeys, and everything while comment mode is on. Any other key goes on to `app`.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx, app: AppKeys) -> bool {
    if is_key(ev, &COMMENT_KEY) {
        let frame = frame_of(s);
        let fb = &mut s.feedback;
        match (fb.mode, frame.feedback) {
            (Mode::Off, Policy::Allowed) => fb.enter(tx),
            (Mode::Off, Policy::Denied(why)) => fb.message = Some(format!("{DENIED_LINE}: {why}.")),
            (Mode::Capturing, _) => {}
            (Mode::Picking | Mode::Writing, _) => fb.leave(),
        }
        return true;
    }
    let fb = &mut s.feedback;
    if is_key(ev, &PANEL_KEY) {
        if fb.mode != Mode::Off {
            fb.leave();
        }
        fb.toggle_panel();
        return true;
    }
    match fb.mode {
        Mode::Picking => {
            match ev.key {
                Key::Escape => fb.back(),
                Key::Enter => fb.pick(tx),
                Key::Up if ev.mods.alt => fb.step(true),
                Key::Down if ev.mods.alt => fb.step(false),
                _ => {}
            }
            // Comment mode owns the keyboard: nothing reaches the app under it.
            true
        }
        Mode::Writing => {
            if ev.key == Key::Escape {
                fb.back();
            }
            true
        }
        Mode::Capturing => true,
        Mode::Off => {
            // A notice ("off on this screen", "saved") goes with the next key.
            let had_notice = fb.message.take().is_some();
            if fb.panel_open && ev.key == Key::Escape {
                fb.toggle_panel();
                return true;
            }
            // Short-circuit order matters: the app's handler always runs.
            app(s, ev, tx) | had_notice
        }
    }
}

pub fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    let mut changed = inner().poll.is_some_and(|f| f(s, clock, tx));
    let fb = &mut s.feedback;
    if fb.refocus {
        fb.refocus = false;
        tx.focus(ui::NOTE_INPUT, false);
        changed = true;
    }
    if fb.panel_open {
        match fb.last_scan {
            // Opening the panel read the folder: the 2 s start now.
            None => fb.last_scan = Some(clock),
            Some(t) if clock - t >= SCAN_EVERY => {
                fb.last_scan = Some(clock);
                changed |= fb.rescan();
            }
            Some(_) => {}
        }
        tx.wake_after(std::time::Duration::from_secs_f64(SCAN_EVERY));
    }
    changed
}

// ---------------------------------------------------------------------------
// After a frame: the pick under the pointer, and the capture
// ---------------------------------------------------------------------------

/// The app's own root: the first child of the wrapper `ui::wrap` puts around it.
fn app_root(tree: &[ElementSnap]) -> Option<usize> {
    let root = tree.iter().position(|e| e.key.as_deref() == Some(ROOT_KEY))?;
    tree.iter().position(|e| e.parent == Some(root))
}

/// An element's path as notes show it: from the app's root, without the feedback wrapper.
pub fn app_path(path: &str) -> &str {
    path.split_once(&format!("{ROOT_KEY}/")).map_or(path, |(_, rest)| rest)
}

/// In comment mode, finds the element under the pointer (the window's centre before the pointer
/// has moved) with Rune's own hit test, restricted to the app's tree. True when the overlay has
/// to be drawn again. A frame that is (or became) denied ends comment mode.
pub fn sync(rt: &mut Runtime<State>) -> bool {
    if rt.state.feedback.mode == Mode::Off {
        return false;
    }
    if let Policy::Denied(why) = frame_of(&rt.state).feedback {
        let fb = &mut rt.state.feedback;
        fb.leave();
        fb.message = Some(format!("{DENIED_LINE}: {why}."));
        return true;
    }
    if rt.state.feedback.mode != Mode::Picking {
        return false;
    }
    let (vw, vh) = rt.window_viewport();
    let at = rt.state.feedback.pointer.unwrap_or((vw / 2.0, vh / 2.0));
    if rt.state.feedback.resolved_for == Some(at) {
        return false;
    }
    let (tree, _) = capture::snapshot(rt);
    let Some(root) = app_root(&tree) else { return false };
    let picked = rt
        .element_at_in(&tree[root].path, at.0, at.1)
        .and_then(|p| model::index_of(&tree, p.path()))
        .unwrap_or(root);
    let mut chain: Vec<ElementSnap> = Vec::new();
    let mut prints = Vec::new();
    for i in model::ancestors(&tree, picked) {
        chain.push(tree[i].clone());
        prints.push(model::fingerprint(&tree, i));
        if i == root {
            break;
        }
    }
    let fb = &mut rt.state.feedback;
    fb.resolved_for = Some(at);
    fb.secure_on_screen = tree.iter().any(|e| e.secure);
    if fb.chain.first().map(|e| &e.path) != chain.first().map(|e| &e.path) {
        fb.level = 0;
    }
    let changed = fb.chain != chain;
    fb.chain = chain;
    fb.prints = prints;
    changed
}

pub fn capture_due(s: &State) -> bool {
    s.feedback.mode == Mode::Capturing
}

/// What the note says about where the app was: the frame and route the app reports, and what
/// `State` holds. Whichever `DriveSource` is behind it makes no difference.
fn frame_summary(s: &State, frame: &FrameInfo) -> Value {
    json!({
        "id": frame.id,
        "route": frame.route,
        "state": {
            "group": frame::group_name(&s.shell.app),
            "sidebar_section": s.sidebar_section.label(),
            "folder": s.listing.folder.name,
            "path": s.listing.path.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            "item_count": s.listing.items.len(),
            "selected_index": s.selected,
            "selected_name": s.selected.and_then(|i| s.listing.items.get(i)).map(|i| i.name.clone()),
            "detail_pane": s.show_detail_pane,
            "search_active": !s.search.text.is_empty(),
        },
    })
}

/// Writes the note from the frame just drawn (`img` is that frame, without feedback chrome).
/// Comment mode stays on for the next element; a failure reopens the popover with its text and
/// puts the keyboard back in it.
pub fn finish_capture(rt: &mut Runtime<State>, img: Result<Rgba, String>) {
    let (tree, rigid) = capture::snapshot(rt);
    let (vw, vh) = rt.window_viewport();
    let result = (|| {
        let mut img = img?;
        let scale = img.w as f32 / vw.max(1.0);
        let frame = frame_of(&rt.state);
        if let Policy::Denied(why) = frame.feedback {
            return Err(format!("{DENIED_LINE}: {why}"));
        }
        let fb = &rt.state.feedback;
        let inbox = fb
            .inbox
            .as_deref()
            .ok_or("no inbox folder: this PC has no local app data folder")?;
        let target = fb.target().ok_or("no element is picked")?;
        let picked =
            model::index_of(&tree, &target.path).ok_or("the picked element is no longer on screen; pick it again")?;
        // Rows are keyed by position: the same key may hold another item by now.
        if fb.prints.get(fb.level) != Some(&model::fingerprint(&tree, picked)) {
            return Err("the element changed while you were writing; press Esc and pick it again".to_string());
        }
        model::still(&tree)?;
        model::redact_pixels(&mut img, &tree, scale, rigid)?;
        // The note's tree is the app's own: paths start at its root, and the root has no parent.
        let root = app_root(&tree);
        let tree: Vec<ElementSnap> = tree
            .iter()
            .enumerate()
            .map(|(i, e)| ElementSnap {
                path: app_path(&e.path).to_string(),
                parent: e.parent.filter(|_| Some(i) != root),
                ..e.clone()
            })
            .collect();
        let meta = NoteMeta {
            comment: model::clean_text(fb.note.text.trim(), model::MAX_COMMENT_CHARS),
            tags: model::TAGS
                .iter()
                .zip(fb.tags)
                .filter(|(_, on)| *on)
                .map(|(t, _)| t.to_string())
                .collect(),
            scale,
            dark: fb.dark.get(),
            window_size: [vw, vh],
            frame: frame_summary(&rt.state, &frame),
        };
        model::make_room(&model::scan(inbox, &fb.notes), model::MAX_NOTES, model::MAX_INBOX_BYTES)?;
        model::write_note(inbox, &meta, &tree, picked, &img)
    })();
    let fb = &mut rt.state.feedback;
    match result {
        Ok(id) => {
            fb.mode = Mode::Picking;
            fb.note.clear();
            fb.tags = [false; model::TAGS.len()];
            fb.resolved_for = None;
            fb.message = Some(format!("Note saved: {id}"));
            fb.rescan();
        }
        Err(e) => {
            fb.mode = Mode::Writing;
            fb.refocus = true;
            fb.message = Some(format!("Not saved: {e}"));
        }
    }
}
