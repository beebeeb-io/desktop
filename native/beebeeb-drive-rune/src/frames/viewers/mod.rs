//! Open files (the board's groups "Open files: the viewers", "Open files: states" and "Text and
//! Markdown editing"): a file rendered inside the window's content area, under the window's
//! toolbar. `FRAMES.md` beside this file lists every frame of the group and which are built.
//!
//! [`Ui`] is the open file's state; every control changes it through one of its methods. What
//! leaves the group (the Free-up-space dialog, a save panel, the report dialog) is an [`Intent`]
//! the shell takes with [`Ui::take_intents`].
//!
//! **Decrypted content is sensitive.** It lives in [`Ui`] (memory) and nowhere else: this group
//! writes no file, logs no content, and drops the [`Ui`] when the file is closed ([`close`]) or
//! the route left it ([`sync`]).
//!
//! The shell registers the group with one line (`shell/registry.rs`: [`frame`], [`on_key`]) and
//! opens a file with [`open`].

mod chrome;
mod details;
mod fixture_text;
pub mod icons;
mod markdown;
mod md_source;
pub mod source;
mod states;
mod style;
mod text;

#[cfg(test)]
mod tests;

use rune::{Cx, El, Key, KeyEvent, Tx};

use crate::State;
use crate::shell::app::{Event, Screen};
use crate::shell::host;
use crate::shell::registry::FrameGroup;
use crate::theme::tokens;
use source::{Body, FixtureViewerSource, OpenError, OpenFile, TextDoc, ViewerSource, board};

/// Where the person goes when an action leaves this group. The shell opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// DW-0780: remove downloaded copies; files stay in Beebeeb.
    FreeUpSpace,
    /// A save panel that lists only disks with room; the decrypted copy goes there.
    DownloadTo,
    /// DW-0782: the report dialog. The report carries the code, never the file's name.
    ReportProblem,
    /// DW-1520: New's menu.
    NewMenu,
    /// DW-1523: the editor on this file.
    Edit,
    /// The system's "Open with" for this file, decrypted for the app the person picks.
    OpenInAnotherApp,
    /// DW-0407: share this file.
    Share,
    /// DW-0413: this file's versions.
    VersionHistory,
    /// A link in a note to a file in Drive: open it (the path as the note wrote it).
    OpenInDrive(String),
    /// A link in a note to the web: the browser opens it, never this window.
    OpenInBrowser(String),
}

pub struct Ui {
    source: Box<dyn ViewerSource>,
    pub file: OpenFile,
    /// The details panel (Alt+Shift+P).
    pub show_details: bool,
    /// Long lines wrap (DW-1512's Wrap / No wrap).
    pub wrap: bool,
    /// A Markdown note shows its text, not the rendered note (DW-1513's Source).
    pub md_source: bool,
    /// The element that takes keyboard focus once this change is on screen.
    focus: Option<&'static str>,
    intents: Vec<Intent>,
}

impl Ui {
    /// Open `file_id` from `source`.
    pub fn open(mut source: Box<dyn ViewerSource>, file_id: &str) -> Result<Ui, OpenError> {
        let file = source.open(file_id)?;
        Ok(Ui {
            source,
            file,
            show_details: false,
            wrap: false,
            md_source: false,
            focus: None,
            intents: Vec::new(),
        })
    }

    pub fn toggle_details(&mut self) {
        self.show_details = !self.show_details;
    }

    /// Available offline, on or off: the engine keeps or releases the copy on this PC.
    pub fn toggle_offline(&mut self) {
        let on = !self.file.details.offline;
        if self.source.set_offline(&self.file.id, on) {
            self.file.details.offline = on;
        }
    }

    /// A link in a note: to the web in the browser, else a file in Drive.
    pub fn follow_link(&mut self, dest: &str) {
        self.intent(if markdown::is_web(dest) {
            Intent::OpenInBrowser(dest.to_string())
        } else {
            Intent::OpenInDrive(dest.to_string())
        });
    }

    pub fn intent(&mut self, intent: Intent) {
        self.intents.push(intent);
    }

    /// What the person asked for that is outside this group, oldest first.
    #[cfg_attr(not(test), allow(dead_code))] // waits for its caller in `shell/host.rs` (PR body)
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// Stop decrypting (DW-1516's Cancel, Esc). Nothing is kept half-decrypted.
    pub fn cancel(&mut self) {
        if matches!(self.file.body, Body::Decrypting { .. }) {
            self.source.cancel(&self.file.id);
        }
    }

    /// DW-1518's Try again: download and decrypt again (DW-1516).
    pub fn retry(&mut self) {
        if self.file.body != Body::Failed {
            return;
        }
        if let Ok(file) = self.source.retry(&self.file.id) {
            self.file = file;
            self.focus = Some(states::TITLE_KEY);
        }
    }

    /// Edit (Ctrl+E): the editor, DW-1523 (the shell opens it). A file over the editor's limit
    /// stays read-only (DW-1527), so there it does nothing.
    pub fn edit(&mut self) {
        let editable = match &self.file.body {
            Body::Text(doc) => doc.truncated.is_none(),
            Body::Markdown(_) => true,
            _ => false,
        };
        if editable {
            self.intent(Intent::Edit);
        }
    }

    /// The engine's progress since the last call. `true`: the view must run again.
    #[cfg_attr(not(test), allow(dead_code))] // called by `poll`, which waits for the shell
    pub fn pump(&mut self) -> bool {
        match self.source.poll(&self.file.id) {
            Some(body) => {
                if std::mem::discriminant(&body) != std::mem::discriminant(&self.file.body) {
                    self.focus = Some(states::TITLE_KEY);
                }
                self.file.body = body;
                true
            }
            None => false,
        }
    }
}

/// The open file, while the route is on it.
pub fn ui(s: &mut State) -> Option<&mut Ui> {
    s.viewer.as_mut()
}

fn route_item(s: &State) -> Option<String> {
    match s.shell.app.screen() {
        Screen::Drive(route) => route.item,
        _ => None,
    }
}

/// Open a file in the viewer: the route goes to it and its content is asked of the source.
/// `false`: there is no such file (the route does not move).
pub fn open(s: &mut State, file_id: &str) -> bool {
    open_from(s, Box::new(FixtureViewerSource::new()), file_id)
}

/// [`open`] with the source to read from (the engine's, once it is wired).
pub fn open_from(s: &mut State, source: Box<dyn ViewerSource>, file_id: &str) -> bool {
    let Ok(ui) = Ui::open(source, file_id) else {
        return false;
    };
    sync(s);
    host::dispatch(s, Event::OpenItem(file_id.to_string()));
    s.viewer = Some(ui);
    true
}

/// Back to the folder the file was opened from; the decrypted content is dropped.
pub fn close(s: &mut State) {
    if let Some(ui) = s.viewer.as_mut() {
        ui.cancel();
    }
    s.viewer = None;
    if route_item(s).is_some() {
        host::dispatch(s, Event::Escape);
    }
}

/// Drop the open file once the route left it (the shell's Esc and Back run before this group's
/// keys): a stopped decryption is cancelled, decrypted content leaves memory.
pub fn sync(s: &mut State) {
    let here = route_item(s);
    if s.viewer
        .as_ref()
        .is_some_and(|ui| here.as_deref() != Some(ui.file.id.as_str()))
    {
        close_stale(s);
    }
}

fn close_stale(s: &mut State) {
    if let Some(mut ui) = s.viewer.take() {
        ui.cancel();
    }
}

/// Copy: the open text file's text goes on the clipboard, at the person's own press.
pub fn copy_text(s: &mut State, tx: &mut Tx) {
    match s.viewer.as_ref().map(|ui| &ui.file.body) {
        Some(Body::Text(doc)) => tx.copy(doc.plain()),
        Some(Body::Markdown(doc)) => tx.copy(doc.source.clone()),
        _ => {}
    }
}

/// The engine's answers that arrive between presses, and the route leaving the file. `true`:
/// the view must run again. For `shell::host::poll`.
#[allow(dead_code)] // waits for its one line in `shell/host.rs` (PR body)
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let had = s.viewer.is_some();
    sync(s);
    let Some(ui) = ui(s) else { return had };
    let changed = ui.pump();
    if let Some(key) = ui.focus.take() {
        tx.focus(key, true);
    }
    changed
}

/// The registry's line for `FrameGroup::Viewer`.
pub fn frame(s: &State, cx: &mut Cx<State>) -> El {
    let here = route_item(s);
    match &s.viewer {
        // Only the file the route names is drawn: a stale one never shows under another's path.
        Some(ui) if here.as_deref() == Some(ui.file.id.as_str()) => view(s, ui, cx),
        _ => crate::shell::placeholder::window(s, cx, FrameGroup::Viewer),
    }
}

fn view(s: &State, ui: &Ui, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    if let Body::Text(doc) = &ui.file.body {
        return chrome::window(
            s,
            cx,
            ui,
            text::tools(doc, ui.wrap, &t),
            text::banner(doc, &t),
            text::body(doc, ui.wrap, &t),
            &text::status(doc),
            &t,
        );
    }
    if let Body::Markdown(doc) = &ui.file.body {
        let plain = text::status(&TextDoc {
            lines: Vec::new(),
            truncated: None,
        });
        let body = if ui.md_source {
            text::body(
                &TextDoc {
                    lines: md_source::lines(&doc.source),
                    truncated: None,
                },
                true,
                &t,
            )
        } else {
            markdown::body(&markdown::parse(&doc.source), &t)
        };
        return chrome::window(s, cx, ui, text::md_tools(ui.md_source, &t), None, body, &plain, &t);
    }
    let (body, status) = match (states::body(&ui.file, &t), states::status(&ui.file.body)) {
        (Some(body), Some(status)) => (body, status),
        _ => return crate::shell::placeholder::window(s, cx, FrameGroup::Viewer),
    };
    chrome::window(s, cx, ui, Vec::new(), None, body, &status, &t)
}

/// The viewer's keys. The shell's own ran first (`host::on_key`): Esc and Alt+Left already
/// left the file, and [`sync`] drops it here.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    sync(s);
    let Some(ui) = ui(s) else { return false };
    let handled = match &ev.key {
        // Alt+Shift+P: the details panel.
        Key::Text(key) if key.eq_ignore_ascii_case("p") && ev.mods.alt && ev.mods.shift && !ev.mods.cmd => {
            ui.toggle_details();
            true
        }
        // Ctrl+E: edit.
        Key::Command('e') if !ev.mods.alt && !ev.mods.shift => {
            ui.edit();
            true
        }
        _ => false,
    };
    if let Some(key) = ui.focus.take() {
        tx.focus(key, true);
    }
    handled
}

// ---------------------------------------------------------------------------
// Scenes: each board frame, reached through the same calls a person's presses make
// ---------------------------------------------------------------------------

/// The file a board frame has open. `None`: not a frame of this group (or not built).
pub fn scene_file(frame: &str) -> Option<&'static str> {
    Some(match frame {
        "DW-1516" => board::DECRYPTING,
        "DW-1517" => board::TOO_LARGE,
        "DW-1518" => board::FAILED,
        "DW-1512" => board::CODE,
        "DW-1527" => board::LOG,
        "DW-1513" => board::NOTES,
        _ => return None,
    })
}

/// Put the window in a board frame's state.
pub fn set_scene(s: &mut State, frame: &str) {
    if let Some(file) = scene_file(frame) {
        open(s, file);
    }
    if let Some(ui) = ui(s) {
        // The board draws the note with its details panel open.
        ui.show_details = frame == "DW-1513";
    }
}

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1516", |s| set_scene(s, "DW-1516")),
    ("DW-1517", |s| set_scene(s, "DW-1517")),
    ("DW-1518", |s| set_scene(s, "DW-1518")),
    ("DW-1512", |s| set_scene(s, "DW-1512")),
    ("DW-1527", |s| set_scene(s, "DW-1527")),
    ("DW-1513", |s| set_scene(s, "DW-1513")),
];
