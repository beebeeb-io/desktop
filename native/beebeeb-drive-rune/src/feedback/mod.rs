//! Live feedback mode: click any element of the running app, type a note, press Enter. Each
//! note lands in a LOCAL inbox folder with the window's own render, the element's crop and its
//! place in Rune's element tree; a panel lists the notes with what happened to each.
//!
//! Compiled in only with the cargo feature `feedback` (test builds). A release build has none
//! of it: `scripts/check-feedback-absent.sh`. Schema, keys and limits: `README.md` beside this file.
//!
//! This module has no network code (a test greps for it): notes never leave this PC.

pub mod capture;
pub mod model;
pub mod paths;
mod ui;

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
    /// Which of `chain` is outlined: Alt+Up and Alt+Down step it.
    pub level: usize,
    pub note: TextEdit,
    pub tags: [bool; model::TAGS.len()],
    pub panel_open: bool,
    /// The note open in the panel, by id.
    pub opened: Option<String>,
    pub notes: Vec<NoteEntry>,
    pub inbox: Option<PathBuf>,
    /// One line under the hint bar or in the panel: "Note saved", or why one was not.
    pub message: Option<String>,
    pub secure_on_screen: bool,
    dark: Cell<bool>,
    last_scan: Option<f64>,
    thumbs: RefCell<HashMap<String, Pixels>>,
    opened_image: Option<Pixels>,
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
            thumbs: RefCell::new(HashMap::new()),
            opened_image: None,
        };
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
        self.level = 0;
        self.message = None;
        tx.blur();
    }

    fn leave(&mut self) {
        self.mode = Mode::Off;
        self.chain.clear();
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
        if self.panel_open {
            self.rescan();
        }
    }

    fn open_note(&mut self, id: &str) {
        self.opened_image = self
            .notes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| load_pixels(&n.dir.join("window.png"), 1400));
        self.opened = Some(id.to_string());
    }

    /// Reads the inbox again. True when the list or a status changed.
    pub fn rescan(&mut self) -> bool {
        let notes = self.inbox.as_deref().map(model::scan).unwrap_or_default();
        let mut thumbs = self.thumbs.borrow_mut();
        thumbs.retain(|id, _| notes.iter().any(|n| &n.id == id));
        for n in &notes {
            if !thumbs.contains_key(&n.id) {
                if let Some(p) = load_pixels(&n.dir.join("window.png"), 256) {
                    thumbs.insert(n.id.clone(), p);
                }
            }
        }
        drop(thumbs);
        let changed = notes != self.notes;
        self.notes = notes;
        changed
    }
}

/// A PNG as an image the panel can show, at most `max` pixels on its longer side.
fn load_pixels(path: &std::path::Path, max: u32) -> Option<Pixels> {
    let img = Rgba::read_png(path).ok()?;
    let k = (img.w.max(img.h)).div_ceil(max).max(1);
    let (w, h) = ((img.w / k).max(1), (img.h / k).max(1));
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * k * img.w + x * k) * 4) as usize;
            px.extend_from_slice(&img.px[i..i + 4]);
        }
    }
    let p = Pixels::new();
    p.set(w, h, px);
    Some(p)
}

// ---------------------------------------------------------------------------
// Hook-up: the app's own view, key and poll handlers, wrapped
// ---------------------------------------------------------------------------

struct Inner {
    view: fn(&State, &mut Cx<State>) -> El,
    on_key: Option<fn(&mut State, &KeyEvent, &mut Tx) -> bool>,
    poll: Option<fn(&mut State, f64, &mut Tx) -> bool>,
}

static INNER: OnceLock<Inner> = OnceLock::new();

fn inner() -> &'static Inner {
    INNER.get().expect("feedback::hook_spec ran before the first frame")
}

/// Wraps the app's `view`, `on_key` and `poll` with comment mode. The first call wins.
pub fn hook_spec(mut spec: AppSpec<State>) -> AppSpec<State> {
    INNER.get_or_init(|| Inner {
        view: spec.view,
        on_key: spec.on_key,
        poll: spec.poll,
    });
    spec.view = view;
    spec.on_key = Some(on_key);
    spec.poll = Some(poll);
    spec
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

/// F8, or Ctrl+Shift+/ (which a US layout sends as Ctrl+?).
fn is_comment_key(ev: &KeyEvent) -> bool {
    match ev.key {
        Key::Function(8) => !ev.mods.shift,
        Key::Command('/') | Key::Command('?') => ev.mods.shift,
        _ => false,
    }
}

fn is_panel_key(ev: &KeyEvent) -> bool {
    ev.key == Key::Function(8) && ev.mods.shift
}

pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let fb = &mut s.feedback;
    if is_comment_key(ev) {
        match fb.mode {
            Mode::Off => fb.enter(tx),
            Mode::Capturing => {}
            Mode::Picking | Mode::Writing => fb.leave(),
        }
        return true;
    }
    if is_panel_key(ev) {
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
            if fb.panel_open && ev.key == Key::Escape {
                fb.toggle_panel();
                return true;
            }
            inner().on_key.is_some_and(|f| f(s, ev, tx))
        }
    }
}

pub fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    let mut changed = inner().poll.is_some_and(|f| f(s, clock, tx));
    let fb = &mut s.feedback;
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
/// has moved) with Rune's own hit test, restricted to the app's tree. True when the outline has
/// to be drawn again.
pub fn sync(rt: &mut Runtime<State>) -> bool {
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
    for i in model::ancestors(&tree, picked) {
        chain.push(tree[i].clone());
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
    changed
}

pub fn capture_due(s: &State) -> bool {
    s.feedback.mode == Mode::Capturing
}

/// What the note says about where the app was. Reads `State` only: whichever `DriveSource` is
/// behind it (board fixture, large fixture, the engine's database) makes no difference.
fn frame_summary(s: &State) -> Value {
    json!({
        "id": "DW-0401",
        "route": format!("files/{}", s.listing.path.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join("/")),
        "state": {
            "sidebar_section": s.sidebar_section.label(),
            "folder": s.listing.folder.name,
            "item_count": s.listing.items.len(),
            "selected_index": s.selected,
            "selected_name": s.selected.and_then(|i| s.listing.items.get(i)).map(|i| i.name.clone()),
            "detail_pane": s.show_detail_pane,
            "search_active": !s.search.text.is_empty(),
        },
    })
}

/// Writes the note from the frame just drawn (`img` is that frame, without feedback chrome).
/// Comment mode stays on for the next element; a failure reopens the popover with its text.
pub fn finish_capture(rt: &mut Runtime<State>, mut img: Rgba) {
    let (tree, rigid) = capture::snapshot(rt);
    let (vw, vh) = rt.window_viewport();
    let scale = img.w as f32 / vw.max(1.0);
    let result = (|| {
        let fb = &rt.state.feedback;
        let inbox = fb
            .inbox
            .as_deref()
            .ok_or("no inbox folder: this PC has no local app data folder")?;
        let target = fb.target().ok_or("no element is picked")?;
        let picked = model::index_of(&tree, &target.path).ok_or("the picked element is no longer on screen")?;
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
            comment: fb.note.text.trim().to_string(),
            tags: model::TAGS
                .iter()
                .zip(fb.tags)
                .filter(|(_, on)| *on)
                .map(|(t, _)| t.to_string())
                .collect(),
            scale,
            dark: fb.dark.get(),
            window_size: [vw, vh],
            frame: frame_summary(&rt.state),
        };
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
            fb.message = Some(format!("Not saved: {e}"));
        }
    }
}
