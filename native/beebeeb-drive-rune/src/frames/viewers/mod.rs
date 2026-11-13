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

mod archive;
mod audio;
mod chrome;
mod details;
mod dialogs;
mod fixture_media;
mod fixture_text;
pub mod icons;
pub mod kinds;
mod markdown;
mod md_source;
pub mod media;
mod media_icons;
pub mod new_file;
mod pdf;
mod photo;
pub mod source;
mod states;
mod style;
mod table;
mod text;

#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod new_file_tests;
#[cfg(test)]
mod tests;

use rune::{Cx, El, Key, KeyEvent, Tx};

use crate::State;
use crate::shell::app::{Event, Screen};
use crate::shell::host;
use crate::shell::registry::FrameGroup;
use crate::theme::tokens;
use source::{
    Body, Conflict, FixtureViewerSource, Keep, OpenError, OpenFile, SaveOutcome, TextDoc, ViewerSource, board,
};

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
    /// DW-1522's Create file: make `name` in `folder` (`None`: the top of Files), encrypted on
    /// this PC before it uploads, and open it in the editor (DW-1523). Nothing is made here.
    CreateFile { folder: Option<String>, name: String },
    /// DW-1521's Continue on an office type: New document with that app's templates (DW-1558).
    NewDocument(new_file::Kind),
    /// DW-1508: a CSV's Edit opens it in Sheets (saved back as CSV).
    EditInSheets,
    /// Move this file to Trash (a photo's Trash; Undo is the toast's, in Files).
    Trash,
}

/// What a control whose target is not built yet says (the shell's words for it).
pub const NOT_BUILT: &str = "Not built yet";

impl Intent {
    /// The name of where this intent goes, while nothing there answers it yet.
    fn not_built(&self) -> Option<&'static str> {
        Some(match self {
            Intent::Share => "Share",
            Intent::Trash => "Move to Trash",
            Intent::OpenInAnotherApp => "Open in another app",
            Intent::EditInSheets => "Edit in Sheets",
            Intent::VersionHistory => "Version history",
            _ => return None,
        })
    }
}

/// The file being edited (DW-1523): the text as typed, and the text it started from.
pub struct Editor {
    pub edit: rune::TextEdit,
    base: String,
}

impl Editor {
    /// The text differs from the version it started from.
    pub fn dirty(&self) -> bool {
        self.edit.text != self.base
    }

    /// The line the caret is on, from 0.
    pub fn caret_line(&self) -> usize {
        let at = self.edit.focus.min(self.edit.text.len());
        self.edit.text.as_bytes()[..at].iter().filter(|b| **b == b'\n').count()
    }

    /// "+1 line", "-2 lines", "same length": how the edit's length differs from the saved text.
    pub fn change_words(&self) -> String {
        let count = |text: &str| text.lines().count() as i64;
        match count(&self.edit.text) - count(&self.base) {
            0 => "same length".into(),
            1 => "+1 line".into(),
            -1 => "-1 line".into(),
            n if n > 0 => format!("+{n} lines"),
            n => format!("{n} lines"),
        }
    }
}

/// A question over the open file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// DW-1525: leaving with changes.
    Unsaved,
    /// DW-1526: saved elsewhere while editing.
    Conflict(Conflict),
}

/// The key of the editor's text field.
pub const EDITOR_KEY: &str = "editor-input";

pub struct Ui {
    source: Box<dyn ViewerSource>,
    pub file: OpenFile,
    /// The details panel (Alt+Shift+P).
    pub show_details: bool,
    /// Long lines wrap (DW-1512's Wrap / No wrap).
    pub wrap: bool,
    /// A Markdown note shows its text, not the rendered note (DW-1513's Source).
    pub md_source: bool,
    /// The editor, while the file is being edited.
    pub editor: Option<Editor>,
    pub dialog: Option<Dialog>,
    /// The message after a save ("Saved · encrypted · version 4"), until the next action.
    pub toast: Option<String>,
    /// When this session last saved the file ("09:41"): the status line says so.
    pub saved_at: Option<String>,
    /// What is open inside a photo, a recording, a PDF, a table or an archive (`media.rs`).
    pub media: media::Media,
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
            media: media::Media::of(&file),
            file,
            show_details: false,
            wrap: false,
            md_source: false,
            editor: None,
            dialog: None,
            toast: None,
            saved_at: None,
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
        // Nothing takes these yet (the shell's drain of this group's intents comes with the
        // engine's wiring): the press says so, in the shell's own words, never nothing.
        if let Some(what) = intent.not_built() {
            self.toast = Some(format!("{NOT_BUILT}: {what}"));
        }
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

    /// Edit (Ctrl+E): the editor on this file (DW-1523). A file over the editor's limit stays
    /// read-only (DW-1527), so there it does nothing.
    pub fn edit(&mut self) {
        let text = match &self.file.body {
            Body::Text(doc) if doc.truncated.is_none() => doc.plain(),
            Body::Markdown(doc) => doc.source.clone(),
            _ => return,
        };
        if self.editor.is_some() {
            return;
        }
        let mut edit = rune::TextEdit::new(text.clone());
        edit.set_caret(text.len());
        self.editor = Some(Editor { edit, base: text });
        self.toast = None;
        self.focus = Some(EDITOR_KEY);
    }

    /// The edit has changes that are not saved.
    pub fn dirty(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::dirty)
    }

    /// The lines of the text being edited, as the source colours them.
    pub fn editor_lines(&self) -> Vec<Vec<source::Run>> {
        match &self.editor {
            Some(editor) => self.source.colours(&self.file, &editor.edit.text),
            None => Vec::new(),
        }
    }

    fn outcome(&mut self, outcome: SaveOutcome) {
        match outcome {
            SaveOutcome::Saved { file, at } => {
                self.toast = Some(format!("Saved \u{00B7} encrypted \u{00B7} version {}", file.version));
                self.saved_at = Some(at);
                self.show(file);
            }
            SaveOutcome::Opened(file) => self.show(file),
            SaveOutcome::Conflict(other) => self.ask(Dialog::Conflict(other)),
            // Not saved: the edit stays as it is, and so does what the status line says.
            SaveOutcome::Failed => self.dialog = None,
        }
    }

    /// Leave the editor for the reader on `file`.
    fn show(&mut self, file: OpenFile) {
        self.file = file;
        self.editor = None;
        self.dialog = None;
        self.md_source = false;
        self.focus = Some("tb-btn-0");
    }

    /// Save (Ctrl+S): encrypted on this PC, a new version (DW-1524); or the two-versions
    /// question when someone saved it elsewhere meanwhile (DW-1526).
    pub fn save(&mut self) {
        let Some(editor) = &self.editor else { return };
        if !editor.dirty() {
            // Nothing to save: back to the reader, no new version.
            self.editor = None;
            self.dialog = None;
            return;
        }
        let outcome = self.source.save(&self.file.id, &editor.edit.text, self.file.version);
        self.outcome(outcome);
    }

    /// DW-1525's Discard changes: the edit is dropped, the saved version stays (DW-1513).
    pub fn discard(&mut self) {
        self.editor = None;
        self.dialog = None;
        self.focus = Some("tb-btn-0");
    }

    /// Keep editing / Cancel / Esc: back to the edit, still not saved.
    pub fn keep_editing(&mut self) {
        self.dialog = None;
        if self.editor.is_some() {
            self.focus = Some(EDITOR_KEY);
        }
    }

    /// DW-1526: which side stays. Either way the other version stays in version history.
    pub fn resolve(&mut self, keep: Keep) {
        let Some(editor) = &self.editor else { return };
        let outcome = self.source.resolve(&self.file.id, &editor.edit.text, keep);
        self.outcome(outcome);
    }

    /// DW-1519's Show it: the newer version replaces the one on screen, at the person's press.
    pub fn show_newer(&mut self) {
        if self.file.newer.is_none() || self.editor.is_some() {
            return;
        }
        if let Ok(file) = self.source.newer(&self.file.id) {
            self.file = file;
            self.focus = Some("tb-btn-0");
        }
    }

    /// Leaving the file was asked for. `true`: there are unsaved changes, so the question is
    /// up (DW-1525) and the file stays.
    pub fn hold_for_changes(&mut self) -> bool {
        if !self.dirty() {
            return false;
        }
        if self.dialog.is_none() {
            self.ask(Dialog::Unsaved);
        }
        true
    }

    /// Put a question over the edit: the keyboard is the dialog's until it is answered.
    fn ask(&mut self, dialog: Dialog) {
        if let Some(editor) = self.editor.as_mut() {
            editor.edit.focused = false;
        }
        self.dialog = Some(dialog);
        self.focus = Some("dlg-btn-0");
    }

    /// The engine's progress since the last call. `true`: the view must run again.
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
    s.viewer.ui.as_mut()
}

fn route_item(s: &State) -> Option<String> {
    match s.shell.app.screen() {
        Screen::Drive(route) => route.item,
        _ => None,
    }
}

/// What the window keeps for this group (`State::viewer`): the open file, and where the route's
/// files are read from.
#[derive(Default)]
pub struct Slot {
    /// The open file, while the route is on it.
    pub(crate) ui: Option<Ui>,
    /// Makes the source a file named by the route is asked of. `None` (a window on `--db`, and
    /// the engine's drive until its source is wired with [`wire`]): no file opens here, the
    /// route shows the shell's placeholder. The board's fixture is here only in the board's
    /// sample window ([`board_fixture`]), never as a fallback.
    route: Option<RouteSource>,
    /// The New file dialog over the Files window (DW-1521, DW-1522), while it is open.
    pub(crate) new_file: Option<new_file::NewFile>,
    /// What the New file dialog asked of the shell, oldest first ([`take_asked`]).
    pub(crate) asked: Vec<Intent>,
    /// The New file dialog closed by the person's press: the Files list takes the keyboard back on
    /// the next poll.
    pub(crate) refocus_list: bool,
    /// Where a key goes that reaches a text field of this group after the field's owner closed
    /// (two keys in one frame): a field nobody shows, never the Files window's search.
    pub(crate) stray: rune::TextEdit,
}

/// What the New file dialog asked for that is outside this group (make a file, New document),
/// oldest first. For the shell, as [`Ui::take_intents`].
#[cfg_attr(not(test), allow(dead_code))] // waits for its caller in `shell/host.rs` (PR body)
pub fn take_asked(s: &mut State) -> Vec<Intent> {
    std::mem::take(&mut s.viewer.asked)
}

/// Makes one source per opened file.
pub type RouteSource = Box<dyn Fn() -> Box<dyn ViewerSource>>;

/// Read the route's files from `route` from now on (the engine's source, once it exists).
pub fn wire(s: &mut State, route: RouteSource) {
    s.viewer.route = Some(route);
}

/// The board's sample window: its open files are the board's (`FixtureViewerSource`). What
/// headless renders, the gate and the tests start from, beside `frames::files::board_fixture`;
/// a window on any other drive is never given this.
pub fn board_fixture(mut s: State) -> State {
    wire(&mut s, Box::new(|| Box::new(FixtureViewerSource::new())));
    s
}

fn route_source(s: &State) -> Option<Box<dyn ViewerSource>> {
    s.viewer.route.as_ref().map(|make| make())
}

/// Open a file in the viewer: the route goes to it and its content is asked of the window's
/// source. `false`: there is no such file, or the window has no source (the route does not move).
pub fn open(s: &mut State, file_id: &str) -> bool {
    match route_source(s) {
        Some(source) => open_from(s, source, file_id),
        None => false,
    }
}

/// The router by file type, and the ONE way a file of a list opens here: a file whose type has
/// a viewer (`kinds::viewer_of`) and that the window's source has goes to the route
/// (`Event::OpenItem`, through [`open`]). `false`: not a file for this group; the caller opens
/// it as it did before (its own app).
pub fn open_item(s: &mut State, item: &crate::source::DriveItem) -> bool {
    !item.is_folder && kinds::viewer_of(&item.name).is_some() && open(s, &item.id)
}

/// [`open`] with the source to read from (the engine's, once it is wired).
pub fn open_from(s: &mut State, source: Box<dyn ViewerSource>, file_id: &str) -> bool {
    let Ok(ui) = Ui::open(source, file_id) else {
        return false;
    };
    sync(s);
    host::dispatch(s, Event::OpenItem(file_id.to_string()));
    s.viewer.ui = Some(ui);
    true
}

/// Back to the folder the file was opened from; the decrypted content is dropped.
pub fn close(s: &mut State) {
    if let Some(ui) = s.viewer.ui.as_mut() {
        // Find, or a folder inside an archive, closes before the file does.
        if ui.hold_for_changes() || ui.close_inner() {
            return;
        }
        ui.cancel();
    }
    s.viewer.ui = None;
    if route_item(s).is_some() {
        host::dispatch(s, Event::Escape);
    }
}

/// Drop the open file once the route left it (the shell's Esc and Back run before this group's
/// keys): a stopped decryption is cancelled, decrypted content leaves memory.
pub fn sync(s: &mut State) {
    let here = route_item(s);
    if s.viewer
        .ui
        .as_ref()
        .is_some_and(|ui| here.as_deref() != Some(ui.file.id.as_str()))
    {
        // Unsaved changes: the route comes back to the file and the question is asked
        // (DW-1525). An open question is answered first: Esc there is "keep editing".
        let still_drive = matches!(s.shell.app.screen(), Screen::Drive(_));
        let held = s.viewer.ui.as_mut().is_some_and(|ui| {
            if ui.dialog.is_some() {
                ui.keep_editing();
                true
            } else {
                // Esc and Back close find, or step out of an archive's folder, before the file.
                ui.hold_for_changes() || (still_drive && ui.close_inner())
            }
        });
        if held {
            let id = s.viewer.ui.as_ref().map(|ui| ui.file.id.clone()).unwrap_or_default();
            host::dispatch(s, Event::OpenItem(id));
            return;
        }
        close_stale(s);
    }
}

/// The text field the editor's keys go to. (With no editor open no key reaches this; the
/// window's search field is what the type asks for.)
pub fn editor_edit(s: &mut State) -> &mut rune::TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.editor.as_mut()) {
        Some(editor) => &mut editor.edit,
        None => &mut s.viewer.stray,
    }
}

fn close_stale(s: &mut State) {
    if let Some(mut ui) = s.viewer.ui.take() {
        ui.cancel();
    }
}

/// Copy: the open text file's text goes on the clipboard, at the person's own press.
pub fn copy_text(s: &mut State, tx: &mut Tx) {
    match s.viewer.ui.as_ref().map(|ui| &ui.file.body) {
        Some(Body::Text(doc)) => tx.copy(doc.plain()),
        Some(Body::Markdown(doc)) => tx.copy(doc.source.clone()),
        _ => {}
    }
}

/// The engine's answers that arrive between presses, and the route leaving the file. `true`:
/// the view must run again. For `shell::host::poll`.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let mut new_file = new_file_poll(s, tx);
    let had = s.viewer.ui.is_some();
    sync(s);
    // The route names a file that is not open yet (Files dispatched `Event::OpenItem`): ask the
    // window's source for it. No source, or a file it does not have: the shell's placeholder.
    if let (None, Some(id)) = (&s.viewer.ui, route_item(s)) {
        if let Some(Ok(ui)) = route_source(s).map(|source| Ui::open(source, &id)) {
            s.viewer.ui = Some(ui);
            return true;
        }
    }
    let Some(ui) = ui(s) else { return had || new_file };
    let changed = ui.pump() | ui.refind();
    if let Some(key) = ui.focus.take() {
        // Not after a key of this group (those take it in `on_key`): no ring is drawn.
        tx.focus(key, false);
    }
    new_file |= changed;
    new_file
}

/// The New file dialog between presses: Files' "New file…" and Ctrl+N name this group's frame
/// (`Intent::Frame("DW-1521")`): it opens here; the route leaving Files closes it.
fn new_file_poll(s: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    let asked = crate::frames::files::Intent::Frame("DW-1521");
    // Every ask since the last poll (two Ctrl+N in one frame are one dialog, opened once).
    let before = s.actions.intents.len();
    s.actions.intents.retain(|intent| *intent != asked);
    if s.actions.intents.len() != before {
        changed |= new_file::begin(s);
    }
    let open = s.viewer.new_file.is_some();
    new_file::sync(s);
    changed |= open && s.viewer.new_file.is_none();
    if let Some(key) = new_file::take_focus(s) {
        tx.focus(key, false);
        changed = true;
    }
    // Closed by a press: the keyboard goes back to the list, as after Files' own dialogs.
    if std::mem::take(&mut s.viewer.refocus_list) && s.viewer.new_file.is_none() && s.viewer.ui.is_none() {
        tx.focus(new_file::list_key(s), false);
        changed = true;
    }
    changed
}

/// The registry's line for `FrameGroup::Viewer`.
pub fn frame(s: &State, cx: &mut Cx<State>) -> El {
    let here = route_item(s);
    match &s.viewer.ui {
        // Only the file the route names is drawn: a stale one never shows under another's path.
        Some(ui) if here.as_deref() == Some(ui.file.id.as_str()) => view(s, ui, cx),
        _ => crate::shell::placeholder::window(s, cx, FrameGroup::Viewer),
    }
}

/// The first word of the file's facts: "Markdown" of "Markdown · 1.4 KB".
fn kind_word(file: &OpenFile) -> &str {
    file.facts.split(" \u{00B7} ").next().unwrap_or_default()
}

fn view(s: &State, ui: &Ui, cx: &mut Cx<State>) -> El {
    let t = tokens::theme(cx.system_dark());
    if let Some(editor) = &ui.editor {
        let dirty = editor.dirty();
        let unsaved = format!("Changes not saved yet \u{00B7} version {}", ui.file.version);
        let status = chrome::Status {
            dot: if dirty { chrome::Dot::Warn } else { chrome::Dot::Green },
            text: if dirty {
                unsaved.clone()
            } else {
                details::DECRYPTED_HERE.into()
            },
            details: format!(
                "{} \u{00B7} UTF-8 \u{00B7} line {}",
                kind_word(&ui.file),
                editor.caret_line() + 1
            ),
        };
        let head = chrome::Head {
            facts: dirty.then_some(unsaved),
            unsaved: dirty,
        };
        let body = text::editor(editor, &ui.editor_lines(), &t);
        return chrome::window(s, cx, ui, head, text::editor_tools(&t), None, body, &status, &t);
    }
    let head = chrome::Head::default();
    let reading = |doc_truncated: Option<source::Truncated>| match &ui.saved_at {
        // After a save the status line says what was saved, and when.
        Some(at) => chrome::Status {
            dot: chrome::Dot::Green,
            text: format!(
                "Saved \u{00B7} encrypted \u{00B7} version {} \u{00B7} {at}",
                ui.file.version
            ),
            details: format!("{} \u{00B7} UTF-8", kind_word(&ui.file)),
        },
        None => text::status(&TextDoc {
            lines: Vec::new(),
            truncated: doc_truncated,
        }),
    };
    let banner = text::newer_banner(&ui.file, &t);
    if let Body::Text(doc) = &ui.file.body {
        return chrome::window(
            s,
            cx,
            ui,
            head,
            text::tools(doc, ui.wrap, &t),
            banner.or_else(|| text::banner(doc, &t)),
            text::body(doc, ui.wrap, &t),
            &reading(doc.truncated),
            &t,
        );
    }
    if let Body::Markdown(doc) = &ui.file.body {
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
        return chrome::window(
            s,
            cx,
            ui,
            head,
            text::md_tools(ui.md_source, &t),
            banner,
            body,
            &reading(None),
            &t,
        );
    }
    match &ui.file.body {
        Body::Photo(doc) => {
            let (tools, body, status) = (photo::tools(doc, &t), photo::body(ui, doc, &t), photo::status(ui, doc));
            return chrome::window(s, cx, ui, head, tools, None, body, &status, &t);
        }
        Body::Audio(doc) => {
            let (tools, body) = (audio::tools(&t), audio::body(ui, doc, &t));
            return chrome::window(s, cx, ui, head, tools, None, body, &audio::status(doc), &t);
        }
        Body::Pdf(doc) => {
            let (tools, bar, body) = (pdf::tools(ui, &t), pdf::find_bar(ui, &t), pdf::body(ui, doc, &t));
            return chrome::window(s, cx, ui, pdf::head(ui, doc), tools, bar, body, &pdf::status(), &t);
        }
        Body::Table(doc) => {
            let (tools, body) = (table::tools(&t), table::body(ui, doc, &t));
            return chrome::window(s, cx, ui, head, tools, None, body, &table::status(), &t);
        }
        Body::Archive(doc) => {
            let (tools, note, body) = (archive::tools(&t), archive::banner(doc, &t), archive::body(ui, &t));
            return chrome::window(s, cx, ui, head, tools, Some(note), body, &archive::status(), &t);
        }
        _ => {}
    }
    let (body, status) = match (states::body(&ui.file, &t), states::status(&ui.file.body)) {
        (Some(body), Some(status)) => (body, status),
        _ => return crate::shell::placeholder::window(s, cx, FrameGroup::Viewer),
    };
    chrome::window(s, cx, ui, head, Vec::new(), None, body, &status, &t)
}

/// The viewer's keys. The shell's own ran first (`host::on_key`): Esc and Alt+Left already
/// left the file, and [`sync`] drops it here.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    sync(s);
    // A "not built yet" notice goes with the next key.
    if let Some(ui) = ui(s) {
        if ui.toast.as_deref().is_some_and(|words| words.starts_with(NOT_BUILT)) {
            ui.toast = None;
        }
    }
    if media::on_key(s, ev) {
        if let Some(key) = ui(s).and_then(|ui| ui.focus.take()) {
            tx.focus(key, true);
        }
        return true;
    }
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
        // Ctrl+S: save the edit as a new, encrypted version.
        Key::Command('s') if !ev.mods.alt && !ev.mods.shift && ui.editor.is_some() && ui.dialog.is_none() => {
            ui.save();
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
        "DW-1513" | "DW-1519" | "DW-1523" | "DW-1524" | "DW-1525" | "DW-1526" => board::NOTES,
        "DW-1501" | "DW-1502" => board::RAW,
        "DW-1503" => board::AUDIO,
        "DW-1505" => board::PDF,
        "DW-1506" => board::PDF_FIND,
        "DW-1514" => board::CSV,
        "DW-1515" => board::ZIP,
        _ => return None,
    })
}

/// The line the board's editor frames have typed at the end of the note.
pub const BOARD_TYPED: &str = "- Ask Priya about the invoices";

/// Put the window in a board frame's state, by the calls a person's presses make.
pub fn set_scene(s: &mut State, frame: &str) {
    let Some(file) = scene_file(frame) else { return };
    let source = FixtureViewerSource {
        newer: frame == "DW-1519",
        conflict: frame == "DW-1526",
        ..FixtureViewerSource::new()
    };
    open_from(s, Box::new(source), file);
    let Some(ui) = ui(s) else { return };
    // The board draws the note, its saved version and the RAW's facts with the details panel open.
    ui.show_details = matches!(frame, "DW-1513" | "DW-1524" | "DW-1502");
    match frame {
        // Ctrl+F, "terrace", Enter: the second of three.
        "DW-1506" => {
            ui.open_find();
            if let Some(find) = ui.media.find.as_mut() {
                find.edit.insert("terrace");
                find.edit.focused = true;
            }
            ui.find_step(1);
        }
        // A press on the cell of the third row's amount.
        "DW-1514" => ui.select_cell(2, 5),
        _ => {}
    }
    if matches!(frame, "DW-1523" | "DW-1524" | "DW-1525" | "DW-1526") {
        ui.edit();
        if let Some(editor) = ui.editor.as_mut() {
            let end = editor.edit.text.len();
            editor.edit.set_caret(end);
            editor.edit.insert(BOARD_TYPED);
            editor.edit.focused = true;
        }
    }
    match frame {
        "DW-1524" | "DW-1526" => ui.save(),
        "DW-1525" => {
            ui.hold_for_changes();
        }
        _ => {}
    }
}

/// DW-1520 to DW-1522: the Files window as the board draws it under New (Contract draft.pdf
/// selected, the details panel open), then New's menu on "New file…", or the dialog.
pub fn set_new_file_scene(s: &mut State, frame: &str) {
    s.select_only(6);
    match frame {
        "DW-1520" => {
            s.actions.menu = Some(crate::frames::files::OpenMenu {
                kind: crate::frames::files::MenuKind::New,
                at: (0.0, 0.0),
                hot: Some(1),
            });
        }
        "DW-1521" => {
            new_file::begin(s);
        }
        "DW-1522" => {
            // Text, Continue, "export_invoices" typed and selected, .py picked.
            new_file::begin(s);
            new_file::choose(s, new_file::Kind::Text);
            new_file::go_on(s);
            new_file::choose_ext(s, ".py");
            if let Some(nf) = s.viewer.new_file.as_mut() {
                nf.name = rune::TextEdit::new("export_invoices");
                nf.name.set_selection(0, nf.name.text.len());
                nf.name.focused = true;
            }
        }
        _ => {}
    }
}

/// The group's board frames and the scene that shows each (`frames::scenes`).
pub const FRAMES: &[crate::ui::gate::Scene<State>] = &[
    ("DW-1520", |s| set_new_file_scene(s, "DW-1520")),
    ("DW-1521", |s| set_new_file_scene(s, "DW-1521")),
    ("DW-1522", |s| set_new_file_scene(s, "DW-1522")),
    ("DW-1516", |s| set_scene(s, "DW-1516")),
    ("DW-1517", |s| set_scene(s, "DW-1517")),
    ("DW-1518", |s| set_scene(s, "DW-1518")),
    ("DW-1512", |s| set_scene(s, "DW-1512")),
    ("DW-1527", |s| set_scene(s, "DW-1527")),
    ("DW-1513", |s| set_scene(s, "DW-1513")),
    ("DW-1519", |s| set_scene(s, "DW-1519")),
    ("DW-1523", |s| set_scene(s, "DW-1523")),
    ("DW-1524", |s| set_scene(s, "DW-1524")),
    ("DW-1525", |s| set_scene(s, "DW-1525")),
    ("DW-1526", |s| set_scene(s, "DW-1526")),
    ("DW-1501", |s| set_scene(s, "DW-1501")),
    ("DW-1502", |s| set_scene(s, "DW-1502")),
    ("DW-1503", |s| set_scene(s, "DW-1503")),
    ("DW-1505", |s| set_scene(s, "DW-1505")),
    ("DW-1506", |s| set_scene(s, "DW-1506")),
    ("DW-1514", |s| set_scene(s, "DW-1514")),
    ("DW-1515", |s| set_scene(s, "DW-1515")),
];
