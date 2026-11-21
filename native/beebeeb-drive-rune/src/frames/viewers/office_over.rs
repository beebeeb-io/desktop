//! What sits over, or in place of, a document in the office editor's frame: the question after a
//! save that found a newer version (DW-1554), the command search (DW-1551), the body when the
//! editor did not start (DW-1556), Insert's tools and a table on the page (DW-1550).
//!
//! The command search lists our operations for the editor's kind (`office::edit::palette`) with the styles and
//! file names [`ViewerSource::commands`] finds; Enter runs the highlighted one through the source (`edit`), a
//! file name opens that file (`Intent::OpenInDrive`). A save is asked of [`ViewerSource::save_document`].
//! Without a live editor the source's answer alone is shown, and a press on it says "Not built yet". Nothing is drawn as saved, kept or found before the
//! source says so.
//!
//! [`ViewerSource::save_document`]: super::source::ViewerSource::save_document
//! [`ViewerSource::commands`]: super::source::ViewerSource::commands

use std::fmt;

use beebeeb_office_protocol::office::EditOp;

use rune::{Align, Color, El, Icon, Key, KeyEvent, TextEdit, TextStyle, Tx, col, el, row, text_input};

use crate::frames::office::edit::palette::{self, Run};
use crate::frames::office::edit::save::{self, Seen, Sent};
use crate::frames::office::edit::word_insert::{self, Insert, LinkForm, Picking, TableForm};

use crate::ui::button::{ButtonState, Size, Variant, button, keys};
use super::dialogs::{self, Look};
use super::office::{self, WordDoc};
use super::office_icons as oi;
use super::source::Body;
use super::source::{Conflict, Keep, SaveOutcome};
use super::{Intent, Ui, icons, style};
use crate::State;
use crate::theme::tokens::{Theme, type_};
use crate::ui::field::{self, Bare, caret_mark};
use crate::ui::kit;
use crate::ui::dialog::{self, Sub};
use crate::ui::settings_dialog::{DIALOG_TITLE, Weight};

/// 13px on 17px (`.t-body`): a command's name, and what is typed.
const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(17.0);
/// 10px/600 on 13px, tracked 0.7px, capitals (`.t-label`).
const GROUP: TextStyle = TextStyle::new(10.0, 600.0).lh(13.0).tracking(0.7);
/// 11px/500 on 14px (`.t-cap`).
const CAP: TextStyle = TextStyle::new(11.0, 500.0).lh(14.0);
/// 12px on 15px: a tool's word.
const TOOL_WORD: TextStyle = TextStyle::new(12.0, 400.0).lh(15.0);
/// A table's cell on the page.
const CELL: TextStyle = TextStyle::new(11.0, 400.0).lh(17.0);
const CELL_HEAD: TextStyle = TextStyle::new(11.0, 600.0).lh(17.0);

pub const SAVED_FIRST: &str = "Someone else saved first";
pub const NOT_SAVED: &str = "Not saved. Nothing changed; your edit is still here.";
pub const NO_EDITOR_TITLE: &str = "We couldn't prepare the editor on this device.";
pub const NO_EDITOR_WORDS: &str = "Nothing was changed. The file still opens in Preview.";

/// One command the search found.
#[derive(Clone, PartialEq, Eq)]
pub struct Command {
    /// "Heading 1", "Bold", or a file's name.
    pub name: String,
    /// "Ctrl+B".
    pub shortcut: Option<String>,
}

/// The commands of one kind ("Styles", "Format", "Files").
#[derive(Clone, PartialEq, Eq)]
pub struct CommandGroup {
    pub title: String,
    pub commands: Vec<Command>,
}

/// A file's name can be among them: the count only.
impl fmt::Debug for CommandGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommandGroup({} commands)", self.commands.len())
    }
}

/// The command search while it is open (Ctrl+K).
pub struct Palette {
    pub query: TextEdit,
    /// Which result the arrow keys are on, counted over all groups.
    pub hot: usize,
}

impl Palette {
    fn new() -> Self {
        let mut query = TextEdit::new("");
        query.focused = true;
        Palette { query, hot: 0 }
    }
}

/// What is open over the document.
pub enum Over {
    /// DW-1554: the last save found this newer version in its way.
    Conflict(Conflict),
    /// DW-1551.
    Palette(Palette),
    /// DW-1557: the licences of the Office engine, from the status line's About.
    Licence,
    /// DW-1550: Insert's Table...
    Table(TableForm),
    /// DW-1550: Insert's Link...
    Link(LinkForm),
    /// DW-1550: Insert's Image...: the platform's file picker is open, and answers on its own worker.
    Image(Picking),
    /// DW-1525 for the office editor: Back with changes no version holds.
    Leave,
}

impl Ui {
    /// The open document, if the editor's frame shows one.
    fn word(&self) -> Option<&WordDoc> {
        match &self.file.body {
            super::source::Body::Word(doc) => Some(doc),
            _ => None,
        }
    }

    /// The status line's About: the licences of the Office engine, over the document.
    pub fn office_about(&mut self) {
        if self.office.over.is_none() {
            self.office.over = Some(Over::Licence);
        }
    }

    /// Save (the ribbon's Save, Ctrl+S): asked of the source, and drawn as what it answers. A
    /// document with nothing to save, or one whose save is on its way, asks nothing.
    pub fn office_save(&mut self) {
        if self.office.edit.is_some() {
            self.office_ask(None);
            return;
        }
        if !self.word().is_some_and(|doc| doc.kept == office::Kept::Unsaved) {
            return;
        }
        let outcome = self.source.save_document(&self.file.id, self.file.version);
        self.office_outcome(outcome);
    }

    /// DW-1554's three ways to keep the edit.
    pub fn office_resolve(&mut self, keep: Keep) {
        if !matches!(self.office.over, Some(Over::Conflict(_))) {
            return;
        }
        if self.office.edit.is_some() {
            self.office_ask(Some(keep));
            return;
        }
        let outcome = self.source.resolve_document(&self.file.id, keep);
        self.office_outcome(outcome);
    }

    /// A live editor's save (`keep`: the answer to a conflict, else a plain save): the revision it holds is kept
    /// until the answer comes, so what is typed meanwhile stays unsaved.
    fn office_ask(&mut self, keep: Option<Keep>) {
        let Some(edit) = self.office.edit.as_ref() else { return };
        // A conflict's answer is asked of an edit that is unsaved by definition; a second save is never asked.
        if edit.saving || edit.stopped || (keep.is_none() && (!save::can_begin(edit) || self.office.over.is_some())) {
            return;
        }
        let sent = Sent {
            revision: edit.revision,
            id: self.file.id.clone(),
            version: self.file.version,
            answer: None,
        };
        let version = self.file.version;
        let (source, id) = self.edit_source();
        let id = id.to_string();
        let outcome = match keep {
            None => source.save_document(&id, version),
            Some(keep) => source.resolve_document(&id, keep),
        };
        self.office.save = Some(sent);
        self.office_outcome(outcome);
    }

    /// The save's answer, from the press (`Saved`, `Opened`, `Conflict`, `Failed`) or later ([`Ui::office_settle_save`]).
    fn office_outcome(&mut self, outcome: SaveOutcome) {
        let sent = self.office.save.take();
        match outcome {
            SaveOutcome::Saved { file, .. } => {
                self.toast = Some(format!("Saved \u{00B7} encrypted \u{00B7} version {}", file.version));
                self.office.over = None;
                self.replace(file);
                if let (Some(edit), Some(sent)) = (self.office.edit.as_mut(), sent.as_ref()) {
                    save::landed(edit, sent);
                }
            }
            SaveOutcome::Opened(file) => {
                self.office.over = None;
                self.replace(file);
                if let Some(edit) = self.office.edit.as_mut() {
                    save::dropped(edit);
                }
            }
            // Nothing was written: the edit stays unsaved, the question opens over it.
            SaveOutcome::Conflict(other) => self.office.over = Some(Over::Conflict(other)),
            // Not saved: the edit and the question stay as they are, and the person is told.
            SaveOutcome::Failed => self.toast = Some(NOT_SAVED.to_string()),
            // On its way to the engine: nothing is drawn as saved before it answers; the question and file stay.
            SaveOutcome::Working => {
                self.toast = Some(save::SAVING.to_string());
                if let Some(edit) = self.office.edit.as_mut() {
                    edit.saving = true;
                    self.office.save = sent;
                }
            }
        }
    }

    /// The engine's answer for the Office save on its way, kept on it for [`Ui::office_settle_save`].
    pub(super) fn office_answer(&mut self, answer: Seen) {
        if let Some(sent) = self.office.save.as_mut() {
            sent.answer = Some(answer);
        }
    }

    /// What a save that left the window's thread became, read from the window's file and toast (`viewers::settle`
    /// applied it). `true`: something changed, the view must run again.
    pub fn office_settle_save(&mut self) -> bool {
        let Some(sent) = self.office.save.clone() else { return false };
        // A newer version in the way: `settle` asked the text editor's question; the office editor's is this one.
        if let Some(super::Dialog::Conflict(other)) = &self.dialog {
            let other = other.clone();
            self.dialog = None;
            self.office.save = None;
            if let Some(edit) = self.office.edit.as_mut() {
                edit.saving = false;
            }
            self.office.over = Some(Over::Conflict(other));
            return true;
        }
        match save::seen(&sent, &self.file.id, self.file.version) {
            Seen::Waiting => false,
            Seen::Landed => {
                self.office.save = None;
                self.office.over = None;
                if let Some(edit) = self.office.edit.as_mut() {
                    save::landed(edit, &sent);
                }
                // Keep both: the window is on the copy now, another file. The editor was the first file's.
                if sent.id != self.file.id {
                    self.office.edit = None;
                    self.office.recovery = None;
                }
                true
            }
            Seen::Queued => {
                self.office.save = None;
                self.office.over = None;
                if let Some(edit) = self.office.edit.as_mut() {
                    save::queued(edit, &sent);
                }
                true
            }
            Seen::Failed => {
                self.office.save = None;
                if let Some(edit) = self.office.edit.as_mut() {
                    edit.saving = false;
                }
                self.toast = Some(NOT_SAVED.to_string());
                true
            }
        }
    }

    /// The open document's editing session is up (the source takes edits): the editor starts on it, once. A Word
    /// document's page takes the keyboard when it is first drawn, and a kept editing of the file is read now, before
    /// this session's own snapshots replace it, to be offered. `true`: the editor started.
    pub fn office_begin_edit(&mut self) -> bool {
        use super::source::Body;
        use beebeeb_office_protocol::office::EditKind;
        if self.office.edit.is_some() {
            return false;
        }
        // What the session edits is what it shows (an older format has no editing session: its source says no).
        let kind = match self.file.body {
            Body::Doc(_) => EditKind::Docx,
            Body::Sheet(_) => EditKind::Xlsx,
            Body::Deck(_) => EditKind::Pptx,
            _ => return false,
        };
        if !self.source.can_edit(&self.file.id) {
            return false;
        }
        self.office.edit = Some(crate::frames::office::edit::EditUi::new(kind));
        self.office.recovery = self.source.recovery(&self.file.id);
        if kind == EditKind::Docx {
            crate::frames::office::edit::word::arm_open_focus();
        }
        true
    }

    /// Leaving the document was asked for (Back, Esc): `true` while changes no version holds are there, so the
    /// question is up (DW-1525) and the document stays. For `viewers::hold_for_changes`.
    pub fn office_hold_for_changes(&mut self) -> bool {
        let Some(edit) = self.office.edit.as_ref() else { return false };
        // A lock or sign-out closed the document (its pane says so, and its text was sealed as a recovery snapshot
        // or left with the key): nothing is on screen to keep, and a question about it would be drawn nowhere, so
        // Files and the gear would answer nothing.
        if edit.stopped || !edit.unsaved() || matches!(self.file.body, Body::Stopped(_)) {
            return false;
        }
        if self.office.over.is_none() {
            self.office.over = Some(Over::Leave);
        }
        true
    }

    /// Cancel and Esc: back to the document, still not saved.
    pub fn office_close_over(&mut self) {
        self.office.over = None;
        self.focus = Some("rib-find");
    }

    /// Ctrl+K, or a press on the ribbon's search (both are the editor's frame's only: [`on_key`],
    /// and the ribbon takes no press without an editor).
    pub fn office_palette(&mut self) {
        if self.office.over.is_none() {
            self.office.over = Some(Over::Palette(Palette::new()));
            self.focus = Some("pal-input");
        }
    }

    /// What the source finds for the typed words. Asked at every draw: a source answers from
    /// what it holds in memory.
    fn office_found(&self) -> Vec<CommandGroup> {
        match &self.office.over {
            Some(Over::Palette(palette)) => {
                let kind = self.office.edit.as_ref().filter(|e| !e.stopped).map(|e| e.kind);
                let typed = palette.query.text.as_str();
                palette::found(kind, typed, self.source.commands(&self.file.id, typed))
            }
            _ => Vec::new(),
        }
    }

    fn office_step(&mut self, by: isize) {
        let count: usize = self.office_found().iter().map(|g| g.commands.len()).sum();
        if let Some(Over::Palette(palette)) = &mut self.office.over
            && count > 0
        {
            // From where the mark is drawn (fewer results than before keep it on the list).
            let at = palette.hot.min(count - 1) as isize;
            palette.hot = (at + by).rem_euclid(count as isize) as usize;
        }
    }

    /// Enter or a press on a result: a file name opens that file, an operation goes to the editor through the
    /// source; what nothing answers to says so.
    fn office_run(&mut self) {
        let found = self.office_found();
        let count: usize = found.iter().map(|group| group.commands.len()).sum();
        // Nothing found: nothing to run.
        if count == 0 {
            return;
        }
        let hot = match &self.office.over {
            Some(Over::Palette(palette)) => palette.hot.min(count - 1),
            _ => return,
        };
        let Some((group, command)) = found
            .iter()
            .flat_map(|group| group.commands.iter().map(move |command| (group, command)))
            .nth(hot)
        else {
            return;
        };
        let edit = self.office.edit.as_ref().filter(|e| !e.stopped);
        let live = edit.is_some();
        let run = palette::resolve(edit.map(|e| e.kind), &group.title, &command.name, edit.and_then(|e| e.marks.as_ref()));
        self.office.over = None;
        match run {
            Run::File(name) => self.intent(Intent::OpenInDrive(name)),
            Run::Op(op) if live => self.office_edit(op),
            Run::Insert(tool) if live => self.office_insert(tool),
            _ => self.intent(Intent::Office("this command")),
        }
    }
}

/// Insert's tools (DW-1550): live only while the editor is (`office.edit`), else the press says "Not built yet".
impl Ui {
    /// The editor is live on this document (its helper has not stopped).
    fn insert_live(&self) -> bool {
        self.office.edit.as_ref().is_some_and(|edit| !edit.stopped)
    }

    /// A press on one of Insert's tools.
    pub fn office_insert(&mut self, tool: Insert) {
        if self.office.over.is_some() {
            return;
        }
        match tool {
            Insert::Table => {
                self.office.over = Some(Over::Table(TableForm::new()));
                self.focus = Some(ROWS);
            }
            Insert::Link => {
                self.office.over = Some(Over::Link(LinkForm::new()));
                self.focus = Some(LINK_TEXT);
            }
            Insert::PageBreak => self.office_edit(EditOp::PageBreak),
            Insert::Image => {
                // The platform's own file picker, on a worker of its own: it blocks until it is closed.
                let platform = crate::shell::platform::current();
                self.office_pick_image(move || platform.pick_photo("Insert a picture", std::path::Path::new("")));
            }
        }
    }

    /// Open the picker `pick` stands for. `false`: it did not open (said in a toast).
    pub fn office_pick_image(
        &mut self,
        pick: impl FnOnce() -> Result<Option<std::path::PathBuf>, crate::shell::platform::PlatformError> + Send + 'static,
    ) -> bool {
        match Picking::start(pick) {
            Some(picking) => {
                self.office.over = Some(Over::Image(picking));
                true
            }
            None => {
                self.toast = Some(word_insert::NO_PICKER.to_string());
                false
            }
        }
    }

    /// Whether a picker is open and its answer is awaited: the shell's poll looks again soon.
    pub fn office_image_waiting(&self) -> bool {
        matches!(self.office.over, Some(Over::Image(_)))
    }

    /// The picker's answer, if it came (the shell's poll calls this while [`Ui::office_image_waiting`]). The picture
    /// goes to the source; a refusal is said in words. `true`: something changed, the view must run again.
    pub fn office_image_poll(&mut self) -> bool {
        let Some(Over::Image(picking)) = &self.office.over else {
            return false;
        };
        let word_insert::Looked::Got(answer) = picking.look() else {
            return false;
        };
        self.office.over = None;
        self.office_image_answer(answer);
        true
    }

    /// The one place a picked picture comes in.
    pub(crate) fn office_image_answer(&mut self, answer: word_insert::Answer) {
        match answer {
            word_insert::Answer::Closed => {}
            word_insert::Answer::Refused(words) => self.toast = Some(words),
            word_insert::Answer::Image(image) => {
                let (source, id) = self.edit_source();
                let id = id.to_string();
                if !source.insert_image(&id, image.bytes, image.png) {
                    self.toast = Some(word_insert::NOT_ADDED.to_string());
                }
            }
        }
    }

    /// One edit to the editor. A source that does not take it says so.
    fn office_edit(&mut self, op: EditOp) {
        let (source, id) = self.edit_source();
        let id = id.to_string();
        if !source.edit(&id, op) {
            self.toast = Some(word_insert::NOT_DONE.to_string());
        }
    }

    /// The table dialog's Insert (Enter): what is typed is checked first, in words.
    pub fn office_table_ok(&mut self) {
        let Some(Over::Table(form)) = &mut self.office.over else { return };
        match word_insert::table_op(&form.rows.text, &form.cols.text) {
            Ok(op) => {
                self.office.over = None;
                self.office_edit(op);
            }
            Err((which, words)) => {
                form.error = Some((which, words));
                self.focus = Some(if which == 0 { ROWS } else { COLS });
            }
        }
    }

    /// The link dialog's Insert (Enter).
    pub fn office_link_ok(&mut self) {
        let Some(Over::Link(form)) = &mut self.office.over else { return };
        match word_insert::link_op(&form.text.text, &form.url.text) {
            Ok(op) => {
                self.office.over = None;
                self.office_edit(op);
            }
            Err((which, words)) => {
                form.error = Some((which, words));
                self.focus = Some(if which == 0 { LINK_TEXT } else { LINK_URL });
            }
        }
    }
}

/// A board frame's state on the open document, by the calls a person's presses make.
pub fn set_scene(ui: &mut Ui, frame: &str) {
    match frame {
        // The Insert tab, pressed.
        "DW-1550" => ui.office.tab = office::Tab::Insert,
        // Ctrl+K, "head".
        "DW-1551" => {
            ui.office_palette();
            if let Some(Over::Palette(palette)) = ui.office.over.as_mut() {
                palette.query.insert(office::board::QUERY);
            }
        }
        // Ctrl+S: saved (DW-1553), or the newer version in its way (DW-1554).
        "DW-1553" | "DW-1554" => ui.office_save(),
        _ => {}
    }
}

/// The keys of what is open over the document. `true`: the key was taken.
pub fn on_key(s: &mut State, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // A presentation's keys (Present's arrows and Space) before the viewer's own.
    if super::office_glue::on_key(s, ev, tx) {
        return true;
    }
    let Some(ui) = super::ui(s) else { return false };
    // Only the editor's frame has these keys.
    if ui.word().is_none() && ui.office.edit.is_none() && ui.office.over.is_none() {
        return false;
    }
    match (&ui.office.over, &ev.key) {
        (Some(_), Key::Escape) => ui.office_close_over(),
        (Some(Over::Palette(_)), Key::Down) => ui.office_step(1),
        (Some(Over::Palette(_)), Key::Up) => ui.office_step(-1),
        (Some(Over::Palette(_)), Key::Enter) => ui.office_run(),
        (None, Key::Command('k')) if !ev.mods.alt && !ev.mods.shift => ui.office_palette(),
        (None, Key::Command('s')) if !ev.mods.alt && !ev.mods.shift => ui.office_save(),
        _ => return false,
    }
    true
}

fn close(s: &mut State) {
    if let Some(ui) = super::ui(s) {
        ui.office_close_over();
    }
}

/// DW-1554: saved elsewhere while this document was edited.
fn conflict(opened: u32, other: &Conflict, t: &Theme) -> El {
    let words = format!(
        "This file was saved as version {} while you were editing (you opened version {opened}). Office documents can't be diffed line by line \u{2014} choose how to keep your edit.",
        other.version
    );
    let head = row()
        .key("dlg-head")
        .w_full()
        .pt(20.0)
        .px(24.0)
        .pb(16.0)
        .gap(14.0)
        .items_start()
        .child(
            el().key("dlg-tile")
                .size(36.0, 36.0)
                .radius(10.0)
                .fill(style::tile_red(t))
                .center()
                .pixel_snap(false)
                .child(kit::glyph("dlg-tile-icon", &icons::REFRESH, 18.0, style::red(t))),
        )
        .child(
            col()
                .key("dlg-words")
                .grow(1.0)
                .min_w(0.0)
                .gap(6.0)
                .child(
                    kit::label("dlg-title", SAVED_FIRST, DIALOG_TITLE, t.text)
                        .role("heading")
                        .level(1)
                        .w_full(),
                )
                .child(
                    kit::label("dlg-sub", words, style::SUB, t.text_tertiary)
                        .wrap()
                        .w_full(),
                ),
        );
    let keep = |keep: Keep| {
        move |s: &mut State| {
            if let Some(ui) = super::ui(s) {
                ui.office_resolve(keep);
            }
        }
    };
    let wide = |key: &str, label: String, look: Look| {
        button(keys(key), &label, None, look.variant(), Size::Md, ButtonState::Enabled, t)
            .pixel_snap(false)
            .w_full()
    };
    let foot = col()
            .key("dlg-stack")
            .w_full()
            .gap(8.0)
            .child(wide("btn-0", "Save as a new version anyway".into(), Look::Primary).on_press(keep(Keep::Mine)))
            .child(
                wide(
                    "btn-1",
                    "Keep both \u{2014} save mine as a separate file".into(),
                    Look::Outline,
                )
                .on_press(keep(Keep::Both)),
            )
            .child(
                wide(
                    "btn-2",
                    format!("Discard my changes and reopen version {}", other.version),
                    Look::Danger,
                )
                .on_press(keep(Keep::Theirs)),
            )
            .child(wide("btn-3", "Cancel".into(), Look::Quiet).on_press(close));
    dialogs::shell_closing(SAVED_FIRST, "alertdialog", 420.0, head, Vec::new(), foot, close, t)
}

/// DW-1525 in the office editor: Back with changes no version holds. Save keeps the document open while it saves.
fn leave(name: &str, t: &Theme) -> El {
    let foot = row()
        .center_y()
        .justify_end()
        .gap(8.0)
        .child(
            crate::ui::settings_dialog::button(0, "Save", Weight::Primary, t).on_press(|s: &mut State| {
                if let Some(ui) = super::ui(s) {
                    ui.office.over = None;
                    ui.office_save();
                }
            }),
        )
        .child(
            crate::ui::settings_dialog::button_danger(1, "Discard changes", t).on_press(|s: &mut State| {
                if let Some(ui) = super::ui(s)
                    && let Some(edit) = ui.office.edit.as_mut()
                {
                    save::dropped(edit);
                    ui.office.over = None;
                }
                super::close(s);
            }),
        )
        .child(crate::ui::settings_dialog::button(2, "Keep editing", Weight::Quiet, t).on_press(close));
    let words = format!("Your edits to {name} aren't saved. Save them as a new, encrypted version, or discard them.");
    insert_card(
        dialog::card(
            "You have unsaved changes",
            420.0,
            "You have unsaved changes",
            Some(Sub::Wrap(&words)),
            Vec::new(),
            foot,
            t,
        ),
        t,
    )
}

/// The editor's answers that arrive between presses (the shell's poll calls this each frame): the picked picture,
/// and what a save that left the window's thread became. While one is awaited it wakes the window again soon, so
/// neither waits for input. `true`: something changed, the view must run again.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let Some(ui) = super::ui(s) else { return false };
    let changed = ui.office_begin_edit() | ui.office_image_poll() | ui.office_settle_save();
    if ui.office_image_waiting() || ui.office.save.is_some() {
        tx.wake_after(std::time::Duration::from_millis(100));
    }
    // The editor's own answers (an edit's, a copy's, a restart's), and whether it has the keyboard.
    changed | crate::frames::office::edit::poll(s, tx)
}

/// The account's files with a kept editing (id, time), asked of the window's source: for the notice after unlock
/// (`office::edit::recovery::on_unlocked`). Empty: none, or the window has no source.
pub(crate) fn recovery_pending(s: &State) -> Vec<(String, String)> {
    super::route_source(s)
        .map(|source| source.recovery_pending())
        .unwrap_or_default()
}

/// A `.kit-kbd` as wide as its letters. Keys: `<key>-box`, `<key>`.
pub(super) fn kbd(key: &str, words: &str, t: &Theme) -> El {
    el().key(format!("{key}-box"))
        .h(20.0)
        .px(6.0)
        .radius(6.0)
        .fill(t.card)
        .box_shadow(kit::ring_inset(t.separator))
        .box_shadow(kit::drop(1.0, 0.0, t.separator))
        .center()
        .child(style::fitted(key.to_string(), words, type_::KBD, t.text_tertiary))
}

/// DW-1551: the command search over the document.
fn palette(ui: &Ui, palette: &Palette, t: &Theme) -> El {
    let typed = palette.query.text.as_str();
    let typed_w = rune::text::advance(typed, BODY);
    if !typed.is_empty() {
        kit::note_text("pal-query", "pal-query-box", typed, BODY, t.text);
    }
    let input = text_input(&palette.query, BODY)
        .key("pal-input")
        .label("Search commands, styles, files")
        .single_line()
        .color(t.text)
        .caret(Color::TRANSPARENT, t.selection)
        // The palette's own words; a key that arrives after it closed goes nowhere.
        .on_edit(
            |s: &mut State| match s.viewer.ui.as_mut().map(|ui| &mut ui.office.over) {
                Some(Some(Over::Palette(palette))) => &mut palette.query,
                _ => &mut s.viewer.stray,
            },
        )
        .on_input_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| {
            matches!(ev.key, Key::Escape | Key::Down | Key::Up | Key::Enter) && on_key(s, ev, tx)
        })
        .w_full();
    let field = el()
        .key("pal-field")
        .grow(1.0)
        .min_w(0.0)
        .h(17.0)
        .child(input)
        // Where the typed words end, for the gate and for the caret.
        .child(el().key("pal-query-box").absolute(0.0, 0.5).w(typed_w).h(15.5))
        .child(
            el().key("pal-caret-at")
                .absolute(typed_w + 1.0, 1.0)
                .child(caret_mark("pal-caret".into(), 15.0, t)),
        );
    let head = row()
        .key("pal-head")
        .w_full()
        .h(48.0)
        .px(16.0)
        .gap(8.0)
        .center_y()
        .border_sides([0.0, 0.0, kit::HAIR, 0.0], t.separator)
        .border_layout()
        .child(kit::glyph("pal-find-icon", &oi::FIND, 15.0, t.text))
        .child(field)
        .child(kbd("pal-esc", "esc", t));
    let mut list = col().key("pal-list").role("listbox").label("Results").w_full().py(8.0);
    let found = ui.office_found();
    // Fewer results than before: the mark stays on the list.
    let last = found.iter().map(|g| g.commands.len()).sum::<usize>().saturating_sub(1);
    let marked = palette.hot.min(last);
    let mut n = 0;
    for (g, group) in found.iter().enumerate() {
        let title = group.title.to_uppercase();
        kit::note_text(
            &format!("pal-g{g}"),
            &format!("pal-g{g}-box"),
            &title,
            GROUP,
            t.text_tertiary,
        );
        list = list.child(
            el().key(format!("pal-g{g}-box"))
                .w_full()
                .pt(8.0)
                .px(16.0)
                .pb(4.0)
                .child(kit::label(format!("pal-g{g}-t"), title, GROUP, t.text_tertiary)),
        );
        for command in &group.commands {
            let key = format!("pal-{n}");
            let hot = n == marked;
            let mut line = row()
                .key(format!("{key}-row"))
                .role("option")
                .label(command.name.clone())
                .checked(hot)
                .mx(8.0)
                .h(30.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(7.0)
                .fill(if hot { t.selection } else { Color::TRANSPARENT })
                .hover(t.hover)
                .on_press(|s: &mut State| {
                    if let Some(ui) = super::ui(s) {
                        ui.office_run();
                    }
                })
                .child(
                    kit::label(key.clone(), command.name.clone(), BODY, t.text)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0)
                        .a11y_hidden(),
                );
            if let Some(shortcut) = &command.shortcut {
                line = line
                    .shortcut_hint(shortcut.clone())
                    .child(kbd(&format!("{key}-kbd"), shortcut, t));
            }
            list = list.child(line);
            n += 1;
        }
    }
    let foot = row()
        .key("pal-foot")
        .w_full()
        .h(34.0)
        .px(16.0)
        .gap(8.0)
        .center_y()
        .border_sides([kit::HAIR, 0.0, 0.0, 0.0], t.separator)
        .border_layout()
        .child(kit::glyph("pal-foot-icon", &oi::PADLOCK, 11.0, t.text_tertiary))
        .child(style::fitted(
            "pal-foot-text",
            "Runs entirely on this device",
            CAP,
            t.text_tertiary,
        ));
    let card = col()
        .key("pal")
        .role("dialog")
        .label("Search commands, styles, files")
        .absolute(390.0, 70.0)
        .w(520.0)
        .radius(14.0)
        .fill(t.card)
        .box_shadow(kit::drop(30.0, 80.0, Color::BLACK.alpha(0.30)))
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key("pal-clip")
                .w_full()
                .clip_rounded(14.0)
                .child(head)
                .child(list)
                .child(foot),
        );
    el().key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(crate::ui::settings_dialog::scrim_color(t))
        .child(card)
        .modal(false, close)
}

/// DW-1557: the "About this viewer" card at the bottom right, no dimming; Esc or a press outside closes it.
fn licence(t: &Theme) -> El {
    col()
        .key("scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(super::office_glue::licence(t).pin(None, None, Some(16.0), Some(36.0)))
        .modal(false, close)
}

/// What is open over the document, if anything.
pub fn over(ui: &Ui, t: &Theme) -> Option<El> {
    Some(match ui.office.over.as_ref()? {
        Over::Conflict(other) => conflict(ui.file.version, other, t),
        Over::Palette(p) => palette(ui, p, t),
        Over::Licence => licence(t),
        Over::Table(form) => table_dialog(form, t),
        Over::Link(form) => link_dialog(form, t),
        Over::Image(_) => image_dialog(t),
        Over::Leave => leave(ui.file.name.as_str(), t),
    })
}

/// DW-1556: the editor did not start. Nothing was changed; the file still opens read-only.
pub fn no_editor(t: &Theme) -> El {
    let width = 30.0 * rune::text::advance("0", style::SUB);
    let content = col()
        .key("empty")
        .role("alert")
        .py(28.0)
        .px(24.0)
        .gap(8.0)
        .items_center()
        .child(
            el().key("empty-tile")
                .size(52.0, 52.0)
                .radius(15.0)
                .fill(style::tile_red(t))
                .center()
                .child(kit::glyph("empty-icon", &icons::ALERT, 26.0, style::red(t))),
        )
        .child(
            kit::label("empty-title", NO_EDITOR_TITLE, style::TITLE, t.text)
                .role("heading")
                .level(1),
        )
        .child(
            kit::label("empty-text", NO_EDITOR_WORDS, style::SUB, t.text_secondary)
                .wrap()
                .w(width)
                .align(Align::Center),
        )
        .child(
            row()
                .key("empty-actions")
                .gap(8.0)
                .mt(8.0)
                .child(
                    button(keys("empty-add"), "Try again", None, Variant::Secondary, Size::Md, ButtonState::Enabled, t).on_press(|s: &mut State| {
                        if let Some(ui) = super::ui(s) {
                            ui.retry();
                        }
                    }),
                )
                .child(
                    button(keys("empty-btn-1"), "Open in another app\u{2026}", None, Variant::Ghost, Size::Md, ButtonState::Enabled, t)
                    .on_press(|s: &mut State| {
                        if let Some(ui) = super::ui(s) {
                            ui.intent(Intent::OpenInAnotherApp);
                        }
                    }),
                ),
        );
    col()
        .key("stage")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .center()
        .fill(style::stage(t))
        .child(content)
}

/// A tool with its word (Insert's): 28 high, the glyph, the word. Keys: `<key>`, `-icon`, `-label`.
/// A press does `tool` while the editor is live, else asks the shell's "Not built yet".
fn worded(key: &str, word: &'static str, icon: &'static Icon, tool: Insert, t: &Theme) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(word)
        .h(28.0)
        .px(6.0)
        .gap(4.0)
        .center_y()
        .radius(7.0)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .on_press(move |s: &mut State| {
            if let Some(ui) = super::ui(s) {
                if ui.insert_live() {
                    ui.office_insert(tool);
                } else {
                    ui.intent(Intent::Office(word));
                }
            }
        })
        .child(kit::glyph(format!("{key}-icon"), icon, 15.0, t.text_secondary))
        .child(style::fitted(format!("{key}-label"), word, TOOL_WORD, t.text).a11y_hidden())
}

/// Insert's tools, in the board's order (DW-1550).
pub fn insert_tools(t: &Theme) -> Vec<El> {
    vec![
        worded("rt-0", "Table\u{2026}", &oi::TABLE, Insert::Table, t),
        worded("rt-1", "Image\u{2026}", &oi::IMAGE, Insert::Image, t),
        worded("rt-2", "Link\u{2026}", &oi::LINK, Insert::Link, t),
        office::hairline("rsep-0".into(), t),
        worded("rt-3", "Page break", &super::media_icons::FILE_TEXT, Insert::PageBreak, t),
    ]
}

// ---------------------------------------------------------------------------------------------------------------
// The dialogs of Insert's tools. Their fields' keys do not start with `edit-` (that is the editing surface's).

pub const ROWS: &str = "ins-rows";
pub const COLS: &str = "ins-cols";
pub const LINK_TEXT: &str = "ins-text";
pub const LINK_URL: &str = "ins-url";

/// A field's keys before the field edits: Esc closes; what a one-line field edits, Tab and Enter (the dialog's
/// default button) go on; anything else stops here, so it never reaches the document behind.
fn field_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    match &ev.key {
        Key::Escape => {
            close(s);
            true
        }
        Key::Text(_)
        | Key::Left
        | Key::Right
        | Key::Home
        | Key::End
        | Key::Backspace
        | Key::Delete
        | Key::Tab
        | Key::Enter => false,
        Key::Command(c) => !matches!(c, 'a' | 'c' | 'x' | 'v' | 'z' | 'y'),
        _ => true,
    }
}

/// A field's text for an edit that arrives while its dialog is gone (two keys in one frame): a field nobody
/// shows. Typing takes the dialog's error away.
fn rows_edit(s: &mut State) -> &mut TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.office.over.as_mut()) {
        Some(Over::Table(form)) => {
            form.error = None;
            &mut form.rows
        }
        _ => &mut s.viewer.stray,
    }
}

fn cols_edit(s: &mut State) -> &mut TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.office.over.as_mut()) {
        Some(Over::Table(form)) => {
            form.error = None;
            &mut form.cols
        }
        _ => &mut s.viewer.stray,
    }
}

fn link_text_edit(s: &mut State) -> &mut TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.office.over.as_mut()) {
        Some(Over::Link(form)) => {
            form.error = None;
            &mut form.text
        }
        _ => &mut s.viewer.stray,
    }
}

fn link_url_edit(s: &mut State) -> &mut TextEdit {
    match s.viewer.ui.as_mut().and_then(|ui| ui.office.over.as_mut()) {
        Some(Over::Link(form)) => {
            form.error = None;
            &mut form.url
        }
        _ => &mut s.viewer.stray,
    }
}

/// A labelled field of a dialog. `error`: the words, when this field is the one at fault.
fn labelled(
    key: &'static str,
    label: &str,
    edit: &TextEdit,
    on_edit: impl for<'a> Fn(&'a mut State) -> &'a mut TextEdit + 'static,
    error: Option<&str>,
    t: &Theme,
) -> El {
    let field = field::bare_field(
        Bare {
            key: &format!("{key}-f"),
            name: label,
            message: error,
            error: error.is_some(),
        },
        edit,
        key,
        on_edit,
        |input| input.on_input_key(field_key),
        t,
    );
    col()
        .key(format!("{key}-col"))
        .w_full()
        .gap(6.0)
        .child(kit::label(format!("{key}-label"), label, CAP, t.text_tertiary))
        .child(field)
}

fn error_of(error: &Option<word_insert::Why>, field: usize) -> Option<&str> {
    error.as_ref().filter(|(at, _)| *at == field).map(|(_, words)| words.as_str())
}

/// The scrim and the card of one of Insert's dialogs: Esc and a press outside close it.
fn insert_card(card: El, t: &Theme) -> El {
    dialog::scrim(0.0, card, t).modal(false, close)
}

fn insert_foot(ok: &'static str, on_ok: fn(&mut State), t: &Theme) -> El {
    row()
        .center_y()
        .justify_end()
        .child(
            crate::ui::settings_dialog::button(0, ok, Weight::Primary, t)
                .default_button()
                .on_press(on_ok),
        )
        .child(crate::ui::settings_dialog::button(1, "Cancel", Weight::Quiet, t).on_press(close))
}

/// DW-1550: Table... asks for its rows and columns.
fn table_dialog(form: &TableForm, t: &Theme) -> El {
    let fields = row()
        .key("ins-fields")
        .w_full()
        .gap(12.0)
        .child(labelled(ROWS, "Rows", &form.rows, rows_edit, error_of(&form.error, 0), t))
        .child(labelled(
            COLS,
            "Columns",
            &form.cols,
            cols_edit,
            error_of(&form.error, 1),
            t,
        ));
    let foot = insert_foot("Insert", |s| {
        if let Some(ui) = super::ui(s) {
            ui.office_table_ok();
        }
    }, t);
    insert_card(
        dialog::card(
            "Insert table",
            420.0,
            "Insert table",
            Some(Sub::Wrap("At the cursor. Fill in the cells after.")),
            vec![fields],
            foot,
            t,
        ),
        t,
    )
}

/// DW-1550: Link... asks for the words and the address. The address is only text in the document.
fn link_dialog(form: &LinkForm, t: &Theme) -> El {
    let fields = col()
        .key("ins-fields")
        .w_full()
        .gap(12.0)
        .child(labelled(
            LINK_TEXT,
            "Text to show",
            &form.text,
            link_text_edit,
            error_of(&form.error, 0),
            t,
        ))
        .child(labelled(
            LINK_URL,
            "Address",
            &form.url,
            link_url_edit,
            error_of(&form.error, 1),
            t,
        ));
    let foot = insert_foot("Insert", |s| {
        if let Some(ui) = super::ui(s) {
            ui.office_link_ok();
        }
    }, t);
    insert_card(
        dialog::card(
            "Insert link",
            460.0,
            "Insert link",
            Some(Sub::Wrap("The address is kept as text in the document. It is never opened from here.")),
            vec![fields],
            foot,
            t,
        ),
        t,
    )
}

/// DW-1550: Image... while the platform's file picker is open.
fn image_dialog(t: &Theme) -> El {
    let foot = row()
        .center_y()
        .justify_end()
        .child(crate::ui::settings_dialog::button(0, "Cancel", Weight::Quiet, t).on_press(close));
    insert_card(
        dialog::card(
            "Insert picture",
            420.0,
            "Insert picture",
            Some(Sub::Wrap(
                "Choose a PNG or JPEG file in the window that opened. It is encrypted together with the document.",
            )),
            Vec::new(),
            foot,
            t,
        ),
        t,
    )
}

/// A table on the page: the first row is its headings. Keys: `tbl`, `tbl-<row>-<column>`.
pub fn table(rows: &[Vec<String>], width: f32, ink: Color) -> El {
    let mut grid = col()
        .key("tbl")
        .role("table")
        .w_full()
        .box_shadow(kit::ring(1.0, Color::BLACK.alpha(0.30)));
    for (r, cells) in rows.iter().enumerate() {
        let text_style = if r == 0 { CELL_HEAD } else { CELL };
        let mut line = row().key(format!("tbl-{r}")).role("row").w_full();
        for (c, cell) in cells.iter().enumerate() {
            let key = format!("tbl-{r}-{c}");
            kit::note_text(&key, &format!("{key}-cell"), cell, text_style, ink);
            line = line.child(
                el().key(format!("{key}-cell"))
                    .role(if r == 0 { "columnheader" } else { "cell" })
                    .label(cell.clone())
                    .w(width / cells.len().max(1) as f32)
                    .px(8.0)
                    .py(4.0)
                    .pixel_snap(false)
                    .box_shadow(kit::ring(kit::HAIR, Color::BLACK.alpha(0.20)))
                    .child(
                        kit::label(format!("{key}-t"), cell.clone(), text_style, ink)
                            .truncate()
                            .a11y_hidden(),
                    ),
            );
        }
        grid = grid.child(line);
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::viewers::source::{FixtureViewerSource, board};

    /// A save handed to the engine draws nothing as saved: the toast says it is on its way, the file's version and
    /// the question over the document stay as they were.
    #[test]
    fn office_save_handed_to_the_engine_draws_nothing_as_saved() {
        let mut ui = Ui::open(Box::new(FixtureViewerSource::new()), board::DOCX).expect("the fixture opens it");
        let version = ui.file.version;
        ui.office.over = Some(Over::Licence);
        ui.office_outcome(SaveOutcome::Working);
        assert_eq!(ui.toast.as_deref(), Some("Saving\u{2026}"));
        assert_eq!(ui.file.version, version, "the version did not move");
        assert!(matches!(ui.office.over, Some(Over::Licence)), "the question stayed");
    }

    use crate::frames::office::edit::EditUi;
    use crate::frames::viewers::source::{Body, OpenError, OpenFile, Settled, ViewerSource};
    use beebeeb_office_protocol::office::EditKind;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// The fixture whose save is handed to the engine and answers later, as the test queues it.
    struct Later {
        inner: FixtureViewerSource,
        saves: Rc<RefCell<u32>>,
        settled: Rc<RefCell<VecDeque<Settled>>>,
    }

    impl ViewerSource for Later {
        fn open(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.inner.open(id)
        }
        fn poll(&mut self, id: &str) -> Option<Body> {
            self.inner.poll(id)
        }
        fn cancel(&mut self, _: &str) {}
        fn retry(&mut self, id: &str) -> Result<OpenFile, OpenError> {
            self.inner.retry(id)
        }
        fn save_document(&mut self, _: &str, _: u32) -> SaveOutcome {
            *self.saves.borrow_mut() += 1;
            SaveOutcome::Working
        }
        fn settled(&mut self, _: &str) -> Option<Settled> {
            self.settled.borrow_mut().pop_front()
        }
    }

    type Saves = Rc<RefCell<u32>>;
    type Answers = Rc<RefCell<VecDeque<Settled>>>;

    /// The agreement live in the editor, one change typed (revision 1, nothing saved).
    fn live() -> (Ui, Saves, Answers) {
        let (saves, settled) = (Saves::default(), Answers::default());
        let source = Later {
            inner: FixtureViewerSource::new(),
            saves: saves.clone(),
            settled: settled.clone(),
        };
        let mut ui = Ui::open(Box::new(source), board::DOCX).expect("the fixture opens it");
        let mut edit = EditUi::new(EditKind::Docx);
        edit.revision = 1;
        ui.office.edit = Some(edit);
        (ui, saves, settled)
    }

    fn unsaved(ui: &Ui) -> bool {
        ui.office.edit.as_ref().is_some_and(EditUi::unsaved)
    }

    fn saved_as(ui: &Ui, version: u32) -> Settled {
        Settled::Saved {
            id: ui.file.id.clone(),
            name: ui.file.name.clone(),
            body: ui.file.body.clone(),
            version,
            at: "09:41".into(),
        }
    }

    #[test]
    fn a_save_in_progress_refuses_a_second_one() {
        let (mut ui, saves, _) = live();
        ui.office_save();
        ui.office_save();
        assert_eq!(*saves.borrow(), 1);
        assert!(ui.office.edit.as_ref().is_some_and(|e| e.saving));
        assert_eq!(ui.toast.as_deref(), Some(save::SAVING));
        assert!(unsaved(&ui), "nothing is drawn as saved before it lands");
    }

    #[test]
    fn an_edit_typed_during_a_save_stays_unsaved_after_it_lands() {
        let (mut ui, _, settled) = live();
        ui.office_save();
        ui.office.edit.as_mut().expect("edit").revision = 2;
        settled.borrow_mut().push_back(saved_as(&ui, 8));
        assert!(ui.pump());
        assert!(ui.office_settle_save());
        let edit = ui.office.edit.as_ref().expect("edit");
        assert!(!edit.saving);
        assert_eq!(edit.saved_revision, 1, "saved up to the revision that was sent");
        assert!(edit.unsaved());
        assert_eq!(ui.file.version, 8);
        assert_eq!(ui.toast.as_deref(), Some("Saved \u{00B7} encrypted \u{00B7} version 8"));
    }

    #[test]
    fn a_save_with_nothing_typed_during_it_lands_as_saved() {
        let (mut ui, _, settled) = live();
        ui.office_save();
        settled.borrow_mut().push_back(saved_as(&ui, 8));
        assert!(ui.pump() && ui.office_settle_save());
        assert!(!unsaved(&ui));
        assert!(ui.office.save.is_none());
    }

    #[test]
    fn a_failed_save_says_so_and_keeps_the_edit() {
        let (mut ui, _, settled) = live();
        ui.office_save();
        settled.borrow_mut().push_back(Settled::Failed);
        assert!(ui.pump() && ui.office_settle_save());
        assert_eq!(ui.toast.as_deref(), Some(NOT_SAVED));
        assert!(unsaved(&ui) && !ui.office.edit.as_ref().is_some_and(|e| e.saving));
    }

    #[test]
    fn a_saved_a_copy_toast_during_a_save_is_not_its_landing() {
        let (mut ui, _, settled) = live();
        ui.office_save();
        // Save a copy finishes while the Office save is on its way: it says so in the toast, nothing more.
        ui.toast = Some("Saved a copy".to_string());
        assert!(!ui.office_settle_save());
        assert!(unsaved(&ui) && ui.office.edit.as_ref().is_some_and(|e| e.saving));
        // The real save then fails: still unsaved, and told.
        settled.borrow_mut().push_back(Settled::Failed);
        assert!(ui.pump() && ui.office_settle_save());
        assert!(unsaved(&ui) && !ui.office.edit.as_ref().is_some_and(|e| e.saving));
        assert_eq!(ui.toast.as_deref(), Some(NOT_SAVED));
    }

    #[test]
    fn a_not_saved_toast_does_not_fail_a_save_that_lands() {
        let (mut ui, _, settled) = live();
        ui.office_save();
        ui.toast = Some("Not saved".to_string());
        assert!(!ui.office_settle_save());
        settled.borrow_mut().push_back(saved_as(&ui, 8));
        assert!(ui.pump() && ui.office_settle_save());
        assert!(!unsaved(&ui));
    }

    #[test]
    fn a_conflict_opens_the_question_and_cancel_saves_nothing() {
        let (mut ui, saves, settled) = live();
        ui.office_save();
        let other = Conflict {
            from: "Anna's iPhone".into(),
            at: "09:14".into(),
            version: 8,
        };
        settled.borrow_mut().push_back(Settled::Conflict(other));
        assert!(ui.pump());
        assert!(matches!(ui.office.over, Some(Over::Conflict(_))));
        assert!(ui.dialog.is_none(), "the text editor's question is not asked");
        ui.office_close_over();
        assert!(ui.office.over.is_none());
        assert_eq!(*saves.borrow(), 1, "Cancel asked nothing of the source");
        assert!(unsaved(&ui));
    }

    #[test]
    fn back_with_unsaved_changes_asks_first() {
        let (mut ui, _, _) = live();
        assert!(ui.office_hold_for_changes());
        assert!(matches!(ui.office.over, Some(Over::Leave)));
        ui.office_close_over();
        assert!(unsaved(&ui), "Keep editing keeps the edit");
        ui.office.edit.as_mut().expect("edit").saved_revision = 1;
        assert!(!ui.office_hold_for_changes(), "nothing unsaved: Back leaves");
    }
}
