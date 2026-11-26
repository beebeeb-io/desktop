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
#[cfg(test)]
pub use chrome::{FROM_THIS_PC, FROM_THIS_PC_UNSENT};
#[cfg(test)]
pub use pdf::{PAGE_FAILED, PAGE_LACKS_TEXT, PAGE_RENDERING};
pub(crate) mod details;
mod editor_view;
mod editor_minimap;
pub(crate) mod dialogs;
mod fixture_media;
mod fixture_text;
pub mod icons;
pub mod kinds;
mod markdown;
mod local_file;
mod md_source;
pub mod media;
mod media_icons;
pub mod new_file;
mod new_file_scene;
pub use new_file_scene::set_new_file_scene;
pub mod office;
pub mod office_board;
mod office_glue;
pub use office_glue::{deck_mut, doc_mut, leave_present, office_try_again, open_deck, open_doc, scene_licence};
mod office_icons;
pub mod office_over;
mod open_route;
mod pdf;
mod more_menu;
mod photo;
mod settle;
pub mod source;
mod states;
pub mod unsupported;
mod style;
mod table;
mod text;
pub(crate) mod unsaved_guard;
pub mod refusal_text;
pub mod video;
use open_route::is_this_groups;

#[cfg(test)]
#[path = "editor_keys_tests.rs"]
mod editor_keys_tests;
#[cfg(test)]
mod media_tests;
#[cfg(test)]
mod new_file_tests;
#[cfg(test)]
mod office_tests;
#[cfg(test)]
mod route_tests;
#[cfg(test)]
mod tests;

use rune::{Cx, El, Key, KeyEvent, Tx};

use crate::State;
use crate::shell::app::{Event, Screen};
use crate::shell::host;
use crate::shell::registry::FrameGroup;
use crate::theme::tokens;
use source::{Body, Conflict, Elsewhere, FixtureViewerSource, Keep, OpenError, OpenFile, TextDoc, ViewerSource, board};

/// Where the person goes when an action leaves this group. The shell opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// DW-0780: remove downloaded copies; files stay in Beebeeb.
    FreeUpSpace,
    /// A save panel that lists only disks with room; the decrypted copy goes there.
    DownloadTo,
    /// "Save a copy…" of a file that is not shown here: a save panel, and the decrypted copy goes where the
    /// person says.
    SaveCopy,
    /// Edit, where the source cannot save an edit.
    Edit,
    /// Available offline, where the source cannot change it.
    KeepOffline,
    /// DW-0782: the report dialog. The report carries the code, never the file's name.
    ReportProblem,
    /// DW-1520: New's menu.
    NewMenu,
    /// The system's "Open with" for this file, decrypted for the app the person picks.
    OpenInAnotherApp,
    /// The unsupported page's "Open with another app…": the system's app chooser, then the file opens in the app picked.
    ChooseApp,
    /// The unsupported page's "Show in Finder".
    ShowInFolder,
    /// DW-0407: share this file.
    Share,
    /// DW-0413: this file's versions.
    VersionHistory,
    /// A link in a note to a file in Drive: open it (the path as the note wrote it).
    OpenInDrive(String),
    /// A link in a note to the web: the browser opens it, never this window.
    OpenInBrowser(String),
    /// A slide command from the editor's command search (Duplicate slide, Delete slide): the host applies it to the
    /// deck with its `State` (`office::edit::slide::run_command`).
    Slide(crate::frames::office::edit::palette::SlideCmd),
    /// DW-1522's Create file: make `name` in `folder` (`None`: the top of Files), encrypted on this PC before it
    /// uploads. Files makes it (`frames::files::create_file`); opening it in the editor (DW-1523) is not bound yet.
    CreateFile { folder: Option<String>, name: String },
    /// DW-1521's Continue on an office type: New document with that app's templates (DW-1558).
    NewDocument(new_file::Kind),
    /// DW-1508: a CSV's Edit opens it in Sheets (saved back as CSV).
    EditInSheets,
    /// Move this file to Trash (a photo's Trash; Undo is the toast's, in Files).
    Trash,
    /// Rename this file: Files' own rename of its row.
    Rename,
    /// A command of the office editor (its name, as the control says it): the editor takes it.
    Office(&'static str),
}

/// What a control whose target is not built yet says (the shell's words for it).
pub const NOT_BUILT: &str = "Not built yet";

/// What Try again says when the file could not even be asked for again (signed out, gone from this PC's list).
pub const NOT_REOPENED: &str = "Couldn't open this file again. Go back to Files and open it from there.";

/// What Show it says while a change of the open file is still on its way (a save, or the Available offline switch):
/// nothing is opened meanwhile.
pub const NOT_WHILE_SAVING: &str = "Newer version not opened yet: your last change is still on its way";

/// What Show it says when the newer version did not open: the version on screen stays, and why.
pub fn not_shown(why: OpenError) -> String {
    let reason = match why.stopped() {
        Some(source::Stopped::Gone) => "the file is gone",
        Some(source::Stopped::SessionEnded) => "the session ended",
        Some(source::Stopped::NothingSynced) => concat!("nothing has synced on ", beebeeb_engine::device_word!(), " yet"),
        Some(source::Stopped::NotThisAccounts) => concat!(beebeeb_engine::device_word!(), "'s data couldn't be confirmed as this account's"),
        // Signed out or locked: the shell's own gate says it.
        None => "Beebeeb is locked or signed out",
        Some(_) => "couldn't read it right now",
    };
    format!("Newer version not opened: {reason}")
}

/// What "Open in another app…" says when the account's folder on this PC is not in sync now.
pub const NEEDS_SYNC_ELSEWHERE: &str =
    concat!("Couldn't open it in another app. It opens from your Beebeeb folder, so sync has to be on for ", beebeeb_engine::device_word!(), ".");

/// What "Open in another app…" says when the file could not be opened there for another reason.
pub const NOT_OPENED_ELSEWHERE: &str = "Couldn't open this file in another app.";

impl Intent {
    /// The name of where this intent goes, while nothing there answers it yet.
    pub(crate) fn not_built(&self) -> Option<&'static str> {
        Some(match self {
            Intent::Share => "Share",
            Intent::Trash => "Move to Trash",
            Intent::OpenInAnotherApp => "Open in another app",
            Intent::SaveCopy => "Save a copy",
            Intent::Edit => "Edit and save from here",
            Intent::KeepOffline => "Available offline from here",
            Intent::EditInSheets => "Edit in Sheets",
            Intent::VersionHistory => "Version history",
            Intent::Rename => "Rename",
            Intent::Office(command) => command,
            _ => return None,
        })
    }
}

/// The file being edited (DW-1523): the text as typed, and the text it started from.
pub struct Editor {
    pub edit: rune::TextEdit,
    base: String,
    /// Why the last save was refused behind an earlier change to the file on this PC (the engine's words), while it
    /// is the last answer: its banner offers Save a copy (task 2530, item 1). It goes with the edit.
    refused: Option<String>,
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
    pub editor_view: editor_view::EditorView,
    pub dialog: Option<Dialog>,
    /// The message after a save ("Saved · encrypted · version 4"), until the next action.
    pub toast: Option<String>,
    /// When this session last saved the file ("09:41"): the status line says so.
    pub saved_at: Option<String>,
    /// What is open inside a photo, a recording, a PDF, a table or an archive (`media.rs`).
    pub media: media::Media,
    /// What is open inside the office editor's frame (`office.rs`).
    pub office: office::OfficeUi,
    /// The element that takes keyboard focus once this change is on screen.
    focus: Option<&'static str>,
    /// The toolbar's "…" menu, while it is open (`more_menu.rs`).
    pub more: Option<more_menu::Open>,
    intents: Vec<Intent>,
}

impl Ui {
    /// Open `file_id` from `source`.
    pub fn open(mut source: Box<dyn ViewerSource>, file_id: &str) -> Result<Ui, OpenError> {
        let file = source.open(file_id)?;
        Ok(Self::on(source, file))
    }

    /// The file the ROUTE names (Search's Open, Back and Forward, the reopen after an unlock), opened from
    /// `source`; or, when it cannot be opened, the window saying why (it is gone, the session ended, or it could
    /// not be read right now, with Try again), never the shell's "not built yet" frame and never nothing. Such a
    /// window knows no name: what this PC knows of the file was not read. `Err`: nothing of this group to draw
    /// (no session: the shell's gate says it).
    pub fn open_or_say_why(mut source: Box<dyn ViewerSource>, file_id: &str) -> Result<Ui, OpenError> {
        match source.open(file_id) {
            Ok(file) => Ok(Self::on(source, file)),
            Err(why) => {
                let stopped = why.stopped().ok_or(why)?;
                Ok(Self::on(source, unopened(file_id, stopped)))
            }
        }
    }

    fn on(source: Box<dyn ViewerSource>, file: OpenFile) -> Ui {
        Ui {
            source,
            media: media::Media::of(&file),
            office: office::OfficeUi::default(),
            file,
            show_details: false,
            wrap: false,
            md_source: false,
            editor: None,
            editor_view: editor_view::EditorView::default(),
            dialog: None,
            toast: None,
            saved_at: None,
            focus: None,
            more: None,
            intents: Vec::new(),
        }
    }

    /// Where the content on screen was read from (`ViewerSource::origin`).
    pub(crate) fn origin(&self) -> source::Origin {
        self.source.origin(&self.file.id)
    }

    /// Whether the source can store an edit (`ViewerSource::can_save`).
    pub(crate) fn can_save(&self) -> bool {
        self.source.can_save()
    }

    /// What there is of page `page` of the open PDF (`ViewerSource::page_state`).
    pub(crate) fn page_state(&self, page: u32) -> source::PageState {
        self.source.page_state(&self.file.id, page)
    }

    /// Whether "Save a copy…" has somewhere to go (`ViewerSource::saves_copies`).
    pub(crate) fn saves_copies(&self) -> bool {
        self.source.saves_copies()
    }

    pub fn toggle_details(&mut self) {
        self.show_details = !self.show_details;
    }

    /// Available offline, on or off: the engine keeps or releases the copy on this PC. A source that cannot
    /// change it says so (never a press that does nothing).
    pub fn toggle_offline(&mut self) {
        if !self.source.knows_offline() {
            self.intent(Intent::KeepOffline);
            return;
        }
        let on = !self.file.details.offline;
        if self.source.set_offline(&self.file.id, on) && !self.source.answers_later() {
            self.file.details.offline = on;
        }
    }

    /// Whether the details panel shows the Available offline switch: only a state that was read.
    pub(crate) fn knows_offline(&self) -> bool {
        self.source.knows_offline()
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
        // "Open in another app…" is the source's when it has a way there: the system opens the file from
        // the place the engine keeps the account's files on this PC. A refusal says so.
        if intent == Intent::OpenInAnotherApp {
            match self.source.open_elsewhere(&self.file.id) {
                Elsewhere::Opened => {
                    self.toast = None;
                    return;
                }
                Elsewhere::NeedsSync => {
                    self.toast = Some(NEEDS_SYNC_ELSEWHERE.to_string());
                    return;
                }
                Elsewhere::Refused => {
                    self.toast = Some(NOT_OPENED_ELSEWHERE.to_string());
                    return;
                }
                Elsewhere::NotBound => {}
            }
        }
        if matches!(intent, Intent::ChooseApp | Intent::ShowInFolder) {
            let id = self.file.id.clone();
            let answer = if intent == Intent::ChooseApp {
                self.source.choose_elsewhere(&id)
            } else {
                self.source.reveal_elsewhere(&id)
            };
            self.toast = match answer {
                Elsewhere::Opened => None,
                Elsewhere::NeedsSync => Some(NEEDS_SYNC_ELSEWHERE.to_string()),
                Elsewhere::Refused | Elsewhere::NotBound => Some(NOT_OPENED_ELSEWHERE.to_string()),
            };
            return;
        }
        // The shell's router takes it (`shell::intents::drain_viewer_intents`); while nothing there answers it,
        // the router's fallback says "Not built yet: <what>".
        self.intents.push(intent);
    }

    /// What the person asked for that is outside this group, oldest first.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// Stop decrypting (DW-1516's Cancel, Esc). Nothing is kept half-decrypted.
    pub fn cancel(&mut self) {
        if matches!(self.file.body, Body::Decrypting { .. }) {
            self.source.cancel(&self.file.id);
        }
    }

    /// DW-1518's Try again: download and decrypt again (DW-1516). Also after a stop that trying again may
    /// get past (offline, a lock that ended).
    pub fn retry(&mut self) {
        let may = match &self.file.body {
            Body::Failed | Body::NoEditor => true,
            Body::Stopped(why) => why.can_retry(),
            _ => false,
        };
        if !may {
            return;
        }
        match self.source.retry(&self.file.id) {
            Ok(file) => {
                self.replace(file);
                self.toast = None;
                self.focus = Some(states::TITLE_KEY);
            }
            // Never a press that does nothing. A refusal the window can say is said in its place (the file is
            // gone now, the session ended); the same one again, or one it cannot say, is the toast.
            Err(why) => match why.stopped() {
                // The file as a route to a file that did not open draws it: its name and folders are overwritten
                // and gone, so nothing of them is drawn empty.
                Some(stopped) if Body::Stopped(stopped) != self.file.body => {
                    let id = self.file.id.clone();
                    self.file.wipe();
                    self.file = unopened(&id, stopped);
                    self.toast = None;
                    self.focus = Some(states::TITLE_KEY);
                }
                _ => self.toast = Some(NOT_REOPENED.to_string()),
            },
        }
    }

    /// Put `file` in place of the open one, whose decrypted text is overwritten first.
    fn replace(&mut self, file: OpenFile) {
        self.file.wipe();
        // What was open over a document goes with the document (its keys with it).
        if !matches!(file.body, Body::Word(_)) {
            self.office.over = None;
        }
        self.file = file;
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
        // An edit that cannot be saved is not offered: the press says so.
        if !self.source.can_save() {
            self.intent(Intent::Edit);
            return;
        }
        let mut edit = rune::TextEdit::new(text.clone());
        edit.set_caret(text.len());
        self.editor = Some(Editor {
            edit,
            base: text,
            refused: None,
        });
        self.toast = None;
        self.focus = Some(EDITOR_KEY);
    }

    /// The edit has changes that are not saved.
    pub fn dirty(&self) -> bool {
        self.editor.as_ref().is_some_and(Editor::dirty)
    }

    /// Leave the editor for the reader on `file`.
    fn show(&mut self, file: OpenFile) {
        self.replace(file);
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
        // A change still on its way belongs to what is on screen: the newer version waits until it answered.
        if self.source.saving() {
            self.toast = Some(NOT_WHILE_SAVING.to_string());
            return;
        }
        match self.source.newer(&self.file.id) {
            Ok(file) => {
                self.replace(file);
                self.focus = Some("tb-btn-0");
            }
            // The version on screen stays, still open; the press says why the newer one did not open.
            Err(why) => self.toast = Some(not_shown(why)),
        }
    }

    /// Leaving the file was asked for. `true`: there are unsaved changes, so the question is
    /// up (DW-1525) and the file stays.
    pub fn hold_for_changes(&mut self) -> bool {
        if self.office_hold_for_changes() {
            return true;
        }
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
        if let Some(settled) = self.source.settled(&self.file.id) {
            self.settle(settled);
            return true;
        }
        match self.source.poll(&self.file.id) {
            Some(mut body) => {
                body.carry_view(&self.file.body);
                let another_kind = std::mem::discriminant(&body) != std::mem::discriminant(&self.file.body);
                if another_kind {
                    self.focus = Some(states::TITLE_KEY);
                }
                // What was shown until now leaves memory overwritten (a session that ended takes the text).
                self.file.body.wipe();
                self.file.body = body;
                // Content that arrives after "Decrypting" starts where its kind starts (a PDF on its first page):
                // what was open inside the body before belongs to that body. The same kind again (a page of the
                // open PDF arrived) keeps the page, the zoom and the find bar where the person has them.
                if another_kind {
                    self.media = media::Media::of(&self.file);
                    if !matches!(self.file.body, Body::Word(_)) {
                        self.office.over = None;
                    }
                }
                true
            }
            None => false,
        }
    }
}

/// The open file is left (closed, the route moved on, the account left): the decrypted text it holds is
/// overwritten, not only freed, and so is an edit of it. The source drops what it holds itself.
impl Drop for Ui {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.file.wipe();
        if let Some(editor) = self.editor.as_mut() {
            editor.base.zeroize();
            editor.edit.text.zeroize();
        }
    }
}

/// The open file, while the route is on it.
pub fn ui(s: &mut State) -> Option<&mut Ui> {
    s.viewer.ui.as_mut()
}

pub(crate) fn route_item(s: &State) -> Option<String> {
    match s.shell.app.screen() {
        Screen::Drive(route) => route.item,
        _ => None,
    }
}

/// The window of a file that could not be opened at all: no name, no path beyond "Files", and why.
fn unopened(file_id: &str, why: source::Stopped) -> OpenFile {
    OpenFile {
        id: file_id.to_string(),
        path: vec!["Files".to_string()],
        name: String::new(),
        facts: String::new(),
        version: 0,
        body: Body::Stopped(why),
        details: source::Details::default(),
        newer: None,
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
    /// The file the route names for which there is nothing of this group to draw (no session in memory): it is not
    /// asked for again on every poll, only once the route names another file or the session gate closed and opened
    /// again. A file that did not open for a reason the window can say is an open `Ui` saying it instead
    /// (`Ui::open_or_say_why`), with Try again where trying again may work.
    unopened: Option<String>,
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
    /// The unsaved-changes question a quit or the gate's sign-out raised ([`unsaved_guard`]).
    pub(crate) leave: Option<unsaved_guard::Leave>,
}

#[cfg(test)]
impl Slot {
    /// The file the route names that did not open (tests).
    pub(crate) fn unopened_for_test(&self) -> Option<String> {
        self.unopened.clone()
    }
}

/// What the New file dialog asked for that is outside this group (make a file, New document),
/// oldest first. The shell's poll hands them to Files (`frames::files::create_file::poll`).
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
#[allow(dead_code)]
pub fn open(s: &mut State, file_id: &str) -> bool {
    match route_source(s) {
        Some(source) => open_from(s, source, file_id),
        None => false,
    }
}

/// The router by file type, and the ONE way a file of a list opens here: a file whose type has
/// a viewer (`kinds::viewer_of`) and that the window's source has goes to the route
/// (`Event::OpenItem`), and so does every other file (the "No preview" frame). `false`: a folder, or the window has
/// no source: the caller never hands a file to the system's app (w29).
///
/// `true` is "this group's", NOT "it opened": a file this group reads that cannot be opened right now is shown
/// as that (it is gone, the session ended, or it could not be read, with Try again), and with no session
/// nothing is shown (the shell's gate says it). It is never handed on to the system's Open because the viewer
/// could not open it: for a script or a web page that would run it.
pub fn open_item(s: &mut State, item: &crate::source::DriveItem) -> bool {
    if !is_this_groups(s, item) {
        return false;
    }
    if crate::shell::session_gate::refuse(s) {
        return true;
    }
    if let Some(Ok(ui)) = route_source(s).map(|source| Ui::open_or_say_why(source, &item.id)) {
        sync(s);
        host::dispatch(s, Event::OpenItem(item.id.clone()));
        s.viewer.ui = Some(ui);
    }
    true
}

/// Whether the window's viewer source shows a file of this name (`ViewerSource::previews`); `false` with no source.
pub fn previews(s: &State, name: &str) -> bool {
    route_source(s).is_some_and(|source| source.previews(name))
}

/// Open a text or Markdown file that is not in the drive in the viewer (`frames::open_file`): the licence notice.
/// `false`: it could not be read, or the session gate is closed.
pub fn open_local(s: &mut State, path: &std::path::Path) -> bool {
    open_from(s, Box::new(local_file::LocalFile), &path.to_string_lossy())
}

/// [`open`] with the source to read from (the engine's, once it is wired).
pub fn open_from(s: &mut State, source: Box<dyn ViewerSource>, file_id: &str) -> bool {
    // Not while the shell's session gate is closed: the source is not asked for the file.
    if crate::shell::session_gate::refuse(s) {
        return false;
    }
    let Ok(ui) = Ui::open(source, file_id) else {
        return false;
    };
    sync(s);
    // The route machine refuses OpenItem while Settings is in front (About's licence row opens from there): leave
    // Settings first, to the Drive route it came from, or the viewer's file is never the route's item and `sync` drops it.
    let here = s.shell.app.route().clone();
    host::dispatch(s, Event::Navigate(here));
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
            } else if ui.office.over.is_some() {
                // Back under the question or the command search closes that, not the document.
                ui.office_close_over();
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

/// The session ended under the window (`shell::host`'s poll, with the Files window's reset). The
/// account left (a sign-out, another account): the open file is dropped whatever it holds, also
/// unsaved changes and an open question, and so is a New file dialog with its typed name; nothing
/// of it is drawn or asked for again under the next account. A lock is the same account coming
/// back: [`sync`] decides, as for any route that left the file.
pub fn end_session(s: &mut State, account_left: bool) {
    if !account_left {
        return;
    }
    close_stale(s);
    s.viewer.new_file = None;
    s.viewer.refocus_list = false;
}

/// The text field the editor's keys go to. (With no editor open no key reaches this; the
/// window's search field is what the type asks for.)
pub fn editor_edit(s: &mut State) -> &mut rune::TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.editor.as_mut()) {
        Some(editor) => &mut editor.edit,
        None => &mut s.viewer.stray,
    }
}

/// Enter in the editor starts a new line (w36-txt-save-enter). Rune's field reads a plain Return as "send" (a
/// chat's composer) and breaks the line only with Shift; the editor has nothing to send, so the plain key did
/// nothing. It is handed to the field as the line break, so undo and a held key work as for any typing.
pub fn editor_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let m = ev.mods;
    // F6 / Shift+F6: the find bar, the live preview (a focused field's keys reach no pane's handler).
    if let Some(back) = crate::ui::focus::f6(ev) {
        editor_f6(s, editor_view::FIELD_PANE, back, tx);
        return true;
    }
    // The field reads Esc as "blur", so it is taken here while the find bar is open.
    if ev.key == Key::Escape && !m.alt && !m.cmd && !m.ctrl && !m.shift && close_find(s) {
        return true;
    }
    if ev.key == Key::Tab && !m.alt && !m.cmd && !m.ctrl {
        let Some(editor) = s.viewer.ui.as_mut().and_then(|ui| ui.editor.as_mut()) else {
            return false;
        };
        editor_tab(&mut editor.edit, m.shift);
        tx.scroll_into_view(format!("caret-{}", editor.caret_line()));
        return true;
    }
    if ev.key != Key::Enter || m.shift || m.alt || m.cmd || m.ctrl {
        return false;
    }
    let Some(editor) = s.viewer.ui.as_mut().and_then(|ui| ui.editor.as_mut()) else {
        return false;
    };
    let line_break = KeyEvent {
        key: Key::Enter,
        mods: rune::Mods {
            shift: true,
            ..rune::Mods::default()
        },
    };
    editor.edit.key(&line_break, None);
    tx.scroll_into_view(format!("caret-{}", editor.caret_line()));
    true
}

/// F6 from the editor stop `from`: focus goes to the next (`back`: previous) one, with the ring. The fields' own
/// focus marks follow, so the caret is drawn in the field that has the keyboard.
pub fn editor_f6(s: &mut State, from: &str, back: bool, tx: &mut Tx) {
    let Some(ui) = ui(s) else { return };
    let Some(to) = editor_view::f6_stop(ui, from, back) else { return };
    if let Some(editor) = ui.editor.as_mut() {
        editor.edit.focused = to == editor_view::FIELD_PANE;
    }
    ui.editor_view.find.needle.focused = to == editor_view::FIND_PANE;
    ui.editor_view.find.replacement.focused = false;
    tx.focus(to, true);
}

/// The find keys while the editor is open. `true`: the key was used.
fn editor_find_key(ui: &mut Ui, ev: &KeyEvent) -> bool {
    use editor_view::editor_find_bar as bar;
    let m = ev.mods;
    let Some(editor) = ui.editor.as_mut() else { return false };
    let view = &mut ui.editor_view;
    match &ev.key {
        Key::Command(c) if (*c == 'f' || *c == 'h') && !m.alt && !m.shift => {
            bar::open(view, &mut editor.edit, *c == 'h');
            true
        }
        Key::Function(3) if !m.alt && !m.cmd => {
            bar::step(view, &mut editor.edit, m.shift);
            true
        }
        Key::Enter if view.find.open && view.find.needle.focused && !m.alt && !m.cmd => {
            bar::step(view, &mut editor.edit, m.shift);
            true
        }
        Key::Escape if view.find.open => {
            bar::close(view, &mut editor.edit);
            true
        }
        _ => false,
    }
}

/// Esc with the editor's find bar open closes the bar, not the file (the shell asks first). `true`: taken.
pub fn close_find(s: &mut State) -> bool {
    let Some(ui) = ui(s) else { return false };
    match ui.editor.as_mut() {
        Some(editor) if ui.editor_view.find.open => {
            editor_view::editor_find_bar::close(&mut ui.editor_view, &mut editor.edit);
            ui.focus = Some(EDITOR_KEY);
            true
        }
        _ => false,
    }
}

/// Replace `edit`'s text with `new` through the field's own edit (so undo and the unsaved mark work), then select `sel`.
fn apply_edit(edit: &mut rune::TextEdit, new: &str, sel: std::ops::Range<usize>) {
    let old = edit.text.clone();
    let mut pre = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(pre) || !new.is_char_boundary(pre) {
        pre -= 1;
    }
    let max_suf = old.len().min(new.len()) - pre;
    let mut suf = old.bytes().rev().zip(new.bytes().rev()).take(max_suf).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(old.len() - suf) || !new.is_char_boundary(new.len() - suf) {
        suf -= 1;
    }
    edit.set_selection(pre, old.len() - suf);
    edit.insert(&new[pre..new.len() - suf]);
    edit.set_selection(sel.start, sel.end);
}

/// Tab and Shift+Tab in the editor: indent or outdent the selected lines, or put one indent unit at the caret.
fn editor_tab(edit: &mut rune::TextEdit, outdent: bool) {
    use crate::text_kit::indent::{detect, indent_lines, outdent_lines, unit_str};
    let unit = detect(&edit.text);
    let (a, b) = edit.range();
    let multi = edit.text[a..b].contains('\n');
    if outdent {
        let (new, sel) = outdent_lines(&edit.text, a..b, unit);
        apply_edit(edit, &new, sel);
    } else if multi {
        let (new, sel) = indent_lines(&edit.text, a..b, unit);
        apply_edit(edit, &new, sel);
    } else {
        edit.insert(&unit_str(unit));
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
    // Never while the shell's session gate is closed: the route may name a file, nothing reads it.
    let (here, gate_open) = (route_item(s), crate::shell::session_gate::open(s));
    // A file that did not open is asked for again only for another route or another session.
    if s.viewer.unopened.is_some() && (s.viewer.unopened != here || !gate_open) {
        s.viewer.unopened = None;
    }
    if let (None, Some(id), true) = (&s.viewer.ui, here, gate_open)
        && s.viewer.unopened.as_deref() != Some(id.as_str()) {
            match route_source(s).map(|source| Ui::open_or_say_why(source, &id)) {
                Some(Ok(ui)) => {
                    s.viewer.ui = Some(ui);
                    return true;
                }
                Some(Err(_)) => s.viewer.unopened = Some(id),
                None => {}
            }
        }
    let Some(ui) = ui(s) else { return had || new_file };
    let changed = ui.pump() | ui.refind();
    // An open text or note whose version is read: woken when the next read is due, not only by a press.
    if let Some(after) = ui.source.next_check() {
        tx.wake_after(after);
    }
    if let Some(key) = ui.focus.take() {
        // Not after a key of this group (those take it in `on_key`): no ring is drawn.
        tx.focus(key, false);
    }
    crate::frames::office::edit::sheet::take_focus(s, tx);
    // The office editor between presses: its answers, a save's end, the picked picture (`office_over::poll`).
    new_file |= changed | office_over::poll(s, tx);
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
        Some(ui) if here.as_deref() == Some(ui.file.id.as_str()) => {
            let page = view(s, ui, cx);
            // Share and Version history of the open file are Files' dialogs: they stand over the viewer.
            let theme = tokens::theme(cx.system_dark());
            match crate::frames::files::window::dialog_over_viewer(s, &theme) {
                Some(dialog) => rune::el().key("viewer-root").w_full().h_full().child(page).child(dialog),
                None => page,
            }
        }
        _ => crate::shell::placeholder::window(s, cx, FrameGroup::Viewer),
    }
}

/// The first word of the file's facts: "Markdown" of "Markdown · 1.4 KB".
fn kind_word(file: &OpenFile) -> &str {
    file.facts.split(" \u{00B7} ").next().unwrap_or_default()
}

fn view(s: &State, ui: &Ui, cx: &mut Cx<State>) -> El {
    match &ui.file.body {
        Body::Word(doc) => return office::window(s, cx, ui, Some(doc)),
        Body::NoEditor => return office::window(s, cx, ui, None),
        Body::Sheet(_) | Body::Deck(_) | Body::Doc(_) | Body::OfficeState(_) => return office_glue::window(s, cx, ui),
        _ => {}
    }
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
        let (tools, banner, body) = editor_view::parts(ui, editor, &t);
        return chrome::window(s, cx, ui, head, tools, banner, body, &status, &t);
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
        None => {
            let mut status = text::status(&TextDoc {
                lines: Vec::new(),
                truncated: doc_truncated,
            });
            // An edit that cannot be saved is not offered: the status line does not name its key either.
            if doc_truncated.is_none() && !ui.can_save() {
                status.details = text::READ_ONLY_HINT.into();
            }
            status
        }
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
            let can_play = ui.can_play();
            let (tools, body) = (audio::tools(can_play, &t), audio::body(ui, doc, &t));
            return chrome::window(s, cx, ui, head, tools, None, body, &audio::status(doc, can_play), &t);
        }
        Body::Video(_) => {
            let (body, status) = (video::view(ui, &t), video::status(ui));
            return chrome::window(s, cx, ui, head, Vec::new(), None, body, &status, &t);
        }
        Body::Pdf(doc) => {
            let (tools, bar, body) = (pdf::tools(ui, &t), pdf::find_bar(ui, &t), pdf::body(ui, doc, &t));
            return chrome::window(s, cx, ui, pdf::head(ui, doc), tools, bar, body, &pdf::status(ui), &t);
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
    let (body, status) = match (
        states::body(&ui.file, ui.saves_copies(), ui.origin(), &t),
        states::status(&ui.file.body, ui.origin()),
    ) {
        (Some(body), Some(status)) => (body, status),
        _ => return crate::shell::placeholder::window(s, cx, FrameGroup::Viewer),
    };
    chrome::window(s, cx, ui, head, Vec::new(), None, body, &status, &t)
}

/// The viewer's keys. The shell's own ran first (`host::on_key`): Esc and Alt+Left already
/// left the file, and [`sync`] drops it here.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    sync(s);
    // The toolbar's "…" menu is modal: while it is open it takes every key.
    if ui(s).is_some_and(|ui| ui.more.is_some()) {
        return more_menu::on_key(s, ev, tx);
    }
    // A "not built yet" notice goes with the next key.
    if let Some(ui) = ui(s)
        && ui.toast.as_deref().is_some_and(|words| words.starts_with(NOT_BUILT)) {
            ui.toast = None;
        }
    // The editor's frame: the question, the command search, Ctrl+K and Ctrl+S.
    if office_over::on_key(s, ev, tx) {
        if let Some(key) = ui(s).and_then(|ui| ui.focus.take()) {
            tx.focus(key, true);
        }
        crate::frames::office::edit::sheet::take_focus(s, tx);
        return true;
    }
    let page_before = ui(s).map(|ui| ui.media.page);
    if media::on_key(s, ev) {
        // A page turned by a key: its thumbnail comes into view, and the page is read from its top.
        if let Some(ui) = ui(s).filter(|ui| matches!(ui.file.body, Body::Pdf(_)))
            && Some(ui.media.page) != page_before {
                tx.scroll_into_view(pdf::thumb_key(ui.media.page));
                pdf::show_page_from_its_top(tx);
            }
        if let Some(key) = ui(s).and_then(|ui| ui.focus.take()) {
            tx.focus(key, true);
        }
        crate::frames::office::edit::sheet::take_focus(s, tx);
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
        // The editor's find keys (ux-15): Ctrl+F, Ctrl+H, F3, Shift+F3, Enter in the find field, Esc.
        _ if ui.editor.is_some() && ui.dialog.is_none() && editor_find_key(ui, ev) => true,
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
    crate::frames::office::edit::sheet::take_focus(s, tx);
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
        "DW-1234" | "DW-1507e" | "DW-1550" | "DW-1551" | "DW-1552" | "DW-1553" | "DW-1554" | "DW-1556" => board::DOCX,
        "DW-1511" => board::DOC,
        "DW-1508" => board::SHEET,
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
        conflict: matches!(frame, "DW-1526" | "DW-1554"),
        office: office::board::Case::of(frame),
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
        "DW-1550" | "DW-1551" | "DW-1553" | "DW-1554" => office_over::set_scene(ui, frame),
        "DW-1524" | "DW-1526" => ui.save(),
        "DW-1525" => {
            ui.hold_for_changes();
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
    ("DW-1507e", |s| set_scene(s, "DW-1507e")),
    ("DW-1511", crate::frames::office::docs::scene_older),
    ("DW-1550", |s| set_scene(s, "DW-1550")),
    ("DW-1551", |s| set_scene(s, "DW-1551")),
    ("DW-1552", |s| set_scene(s, "DW-1552")),
    ("DW-1553", |s| set_scene(s, "DW-1553")),
    ("DW-1554", |s| set_scene(s, "DW-1554")),
    ("DW-1556", crate::frames::office::states::scene_failed),
];
