//! `ViewerSource`: what an open file asks of the engine, and the board's fixture.
//!
//! The viewer never reads a file itself. The source hands it what is already decrypted IN MEMORY
//! (text, pages, pixels) or the reason there is nothing to show; nothing here writes a decrypted
//! byte to disk, and no content goes into a log or an accessibility label beyond what is on screen.
//!
//! The fixture holds exactly the board's samples (DW-1501 to DW-1527): invented data is forbidden
//! where the board shows data.
//!
//! An open file holds a decrypted name and decrypted content: a `{:?}` of [`OpenFile`], [`Body`],
//! [`Details`], [`Newer`], [`TextDoc`], [`MdDoc`] or [`Run`] prints counts and kinds, never a name, a
//! folder or a word of the file, and [`OpenFile::wipe`] overwrites the text, the name, the path and
//! the details it holds when the file is left.

use std::fmt;

use beebeeb_office_protocol::office::{EditKind, EditOp, Edited, HitMode, Template};
use zeroize::{Zeroize, Zeroizing};

/// One open file, as the viewer draws it.
#[derive(Clone, PartialEq)]
pub struct OpenFile {
    pub id: String,
    /// The folders above the file, outermost first ("Files", "Lisbon trip").
    pub path: Vec<String>,
    pub name: String,
    /// The line under the path: "Video · 120 MB".
    pub facts: String,
    /// Which version of the file this is (every save is a new one).
    pub version: u32,
    pub body: Body,
    /// What the details panel says about the file.
    pub details: Details,
    /// Another device saved a newer version while this one is open (DW-1519).
    pub newer: Option<Newer>,
}

/// The body's kind and the version, never the id, the name, the path or the details' values.
impl fmt::Debug for OpenFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenFile")
            .field("version", &self.version)
            .field("body", &self.body)
            .finish_non_exhaustive()
    }
}

impl OpenFile {
    /// Overwrite what this file holds that was decrypted (the note, the lines, the name, the folders above it,
    /// the details' values) before it is dropped: the file was closed, the route left it, or its session ended.
    /// Pictures are the source's to release.
    pub fn wipe(&mut self) {
        self.body.wipe();
        self.name.zeroize();
        self.facts.zeroize();
        for folder in &mut self.path {
            folder.zeroize();
        }
        for (_, value, _) in &mut self.details.facts {
            value.zeroize();
        }
        if let Some(newer) = self.newer.as_mut() {
            newer.from.zeroize();
        }
    }
}

/// The details panel's facts (`.mk-det`).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Details {
    /// "Markdown" of "Markdown · 1.4 KB".
    pub kind: String,
    /// "1.4 KB": a figure, drawn mono.
    pub size: String,
    /// Label, value, and whether the value is a figure (mono).
    pub facts: Vec<(&'static str, String, bool)>,
    /// The file is kept on this PC.
    pub offline: bool,
}

/// How many facts, never a value: the "Where" line is the decrypted folder path.
impl fmt::Debug for Details {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Details")
            .field("facts", &self.facts.len())
            .field("offline", &self.offline)
            .finish_non_exhaustive()
    }
}

/// A newer version saved elsewhere.
#[derive(Clone, PartialEq, Eq)]
pub struct Newer {
    /// "Anna's iPhone". Empty: no device is named (the engine does not say which device saved: decision 2505).
    pub from: String,
    /// "09:14".
    pub at: String,
}

/// That there is one, never the device's name (it holds a person's).
impl fmt::Debug for Newer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Newer")
    }
}

/// Why a file that is in Beebeeb is not shown here. Either way there is a way out: another app, or a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoPreview {
    /// This app has no viewer for this kind of file yet. Nothing was downloaded.
    NoViewer,
    /// A HEIC photo: this PC has no decoder for it. Said as that, with the system's app one press away. Nothing was downloaded.
    Heic,
    /// Larger than a preview holds in memory (`limit`). Nothing was downloaded.
    TooBig { size: u64, limit: u64 },
    /// It passed what a preview holds in memory (`limit`) while it was read, although it was said to be
    /// smaller: the read was given up and what was held was dropped. Its size is not known.
    OverLimit { limit: u64 },
    /// Decrypted, and not what its name says (text that is not text, a picture that does not decode).
    Unreadable,
    /// A picture that declares more pixels than a preview decodes. It was not decoded.
    PictureTooBig { width: u32, height: u32 },
    /// A PDF that asks for a password. It stays closed here.
    PdfLocked,
    /// A PDF this app cannot read: damaged, cut short, or protected in a way it does not open.
    PdfUnreadable,
    /// A PDF with more pages than the viewer lists.
    PdfTooLong { pages: u32 },
}

/// What there is of one page of the open PDF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageState {
    /// Its picture is there, or this source has none to wait for (the frame draws its plain page).
    Ready,
    /// It is being rendered on this PC: its picture comes.
    Rendering,
    /// It could not be rendered.
    Failed,
    /// Its picture is there; text whose font the PDF names and does not carry may be missing from it.
    LacksText,
}

/// Why opening stopped, when it is not a failed decryption (DW-1518 is that).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// Beebeeb could not be reached. Trying again may work.
    Offline,
    /// Beebeeb no longer accepts this session: the person signs in again first.
    SessionEnded,
    /// Beebeeb was locked, or the account changed, while the file was open or opening: its content left memory.
    SessionChanged,
    /// The file is no longer in Beebeeb.
    Gone,
    /// What this PC knows about the file could not be read at this moment. Trying again may work.
    NotNow,
    /// Nothing has synced on this PC yet, so it knows no file: this says nothing about the file, which may
    /// well be in Beebeeb. Trying again after the first sync works.
    NothingSynced,
    /// What this PC holds of Beebeeb is not provably the signed-in account's, so nothing of it is read. It
    /// says nothing about the file. Trying again may work (once sync has set this PC up for this account).
    NotThisAccounts,
    /// This PC holds a change to the file that is not in Beebeeb yet, and its copy could not be read just now.
    /// The older version Beebeeb has is not shown in its place. Trying again may work.
    CopyNotReadable,
    /// Beebeeb refused this account the file: it is no longer shared with it. Trying again does not change that.
    NoAccess,
    /// The file changed while it was opening, each time it was read. Trying again may work.
    Changed,
}

impl Stopped {
    /// Opening again may work as things are.
    pub fn can_retry(self) -> bool {
        matches!(
            self,
            Stopped::Offline
                | Stopped::SessionChanged
                | Stopped::NotNow
                | Stopped::CopyNotReadable
                | Stopped::Changed
                | Stopped::NothingSynced
                | Stopped::NotThisAccounts
        )
    }
}

/// Where the content on screen was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Downloaded from Beebeeb and decrypted in memory.
    Beebeeb,
    /// The copy kept on this PC. `unsent`: it has a change that is not in Beebeeb yet, so it is NOT the
    /// version Beebeeb has; the view says so.
    ThisPc { unsent: bool },
    /// The copy kept on this PC, read while sync is not running here: the local state calls it in sync, and
    /// nothing has checked that since sync stopped. Beebeeb may have a newer version; the view says so.
    ThisPcSyncOff,
    /// Downloaded from Beebeeb, for a file whose copy on this PC is not settled with it (two versions to choose
    /// from, or a sync of this file that failed): what is shown may not be what is on this PC; the view says so.
    BeebeebNotThisPcs,
    /// Downloaded from Beebeeb, for a file whose copy on this PC could not be confirmed as the version Beebeeb has
    /// (older, or of a version not known): the copy is not shown as it; the view says so.
    BeebeebUnconfirmedHere,
}

/// What the content area shows.
#[derive(Clone, PartialEq)]
pub enum Body {
    /// DW-1516: opening takes over about a second. Bytes done and in all.
    Decrypting { done: u64, total: u64 },
    /// DW-1517: the decrypted copy does not fit on this PC.
    TooLarge { size: u64, free: u64 },
    /// DW-1518: decrypting failed. The copy in Beebeeb is untouched.
    Failed,
    /// Opening stopped for a reason that is not a failed decryption (not a board frame: the engine's states).
    Stopped(Stopped),
    /// The file is in Beebeeb and is not shown here (not a board frame: the engine's states).
    NotPreviewable(NoPreview),
    /// DW-1512, DW-1527: text or code, decrypted in memory.
    Text(TextDoc),
    /// DW-1513: a Markdown note, decrypted in memory.
    Markdown(MdDoc),
    /// DW-1501, DW-1502: a photo (a RAW shows the preview the camera stored in it).
    Photo(PhotoDoc),
    /// DW-1503: a recording, with its folder as the queue.
    Audio(AudioDoc),
    /// A video in Beebeeb's player (decision D-MED-6; no board frame yet). Nothing makes one before it is bound.
    Video(VideoDoc),
    /// DW-1505, DW-1506: a PDF.
    Pdf(PdfDoc),
    /// DW-1514: a CSV or TSV as a table.
    Table(TableDoc),
    /// DW-1515: an archive's list.
    Archive(ArchiveDoc),
    /// DW-1507: a text document in the office editor's frame.
    Word(super::office::WordDoc),
    /// DW-1508: a spreadsheet, read-only (the Office engine's cells, already formatted).
    Sheet(super::super::office::sheets::SheetView),
    /// DW-1509, DW-1561: a presentation, read-only (the slide on the stage, and Present).
    Deck(super::super::office::slides::DeckBody),
    /// DW-1507, DW-1511: a Word document, read-only (the pages' pictures are the source's, asked by number).
    Doc(super::super::office::docs::DocBody),
    /// DW-1556: an office document, and the editor did not start on this PC. Nothing of the file
    /// is held here and nothing was changed.
    NoEditor,
    /// DW-1556, DW-1563, DW-1564: the Office viewer's state (a failure, opening, content missing).
    OfficeState(super::super::office::states::StateBody),
}

/// The kind and its counts, never a word of the content, a name or a title.
impl fmt::Debug for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Body::Decrypting { done, total } => write!(f, "Decrypting({done} of {total})"),
            Body::TooLarge { size, free } => write!(f, "TooLarge({size}, {free} free)"),
            Body::Failed => f.write_str("Failed"),
            Body::Stopped(why) => write!(f, "Stopped({why:?})"),
            Body::NotPreviewable(why) => write!(f, "NotPreviewable({why:?})"),
            Body::Text(doc) => write!(f, "Text({doc:?})"),
            Body::Markdown(doc) => write!(f, "Markdown({doc:?})"),
            Body::Photo(_) => f.write_str("Photo"),
            Body::Audio(_) => f.write_str("Audio"),
            Body::Video(doc) => write!(f, "Video({} s)", doc.length),
            Body::Word(doc) => write!(f, "Word({doc:?})"),
            Body::NoEditor => f.write_str("NoEditor"),
            Body::OfficeState(b) => write!(f, "OfficeState({:?})", b.state),
            Body::Sheet(v) => write!(f, "Sheet({} cells)", v.cells.len()),
            Body::Deck(d) => write!(f, "Deck({} slides)", d.view.slides.len()),
            Body::Doc(d) => fmt::Debug::fmt(d, f),
            Body::Pdf(doc) => write!(f, "Pdf({} pages)", doc.pages),
            Body::Table(doc) => write!(f, "Table({} rows)", doc.rows.len()),
            Body::Archive(doc) => write!(f, "Archive({} entries)", doc.entries.len()),
        }
    }
}

impl Body {
    /// Overwrite the decrypted text held here (see [`OpenFile::wipe`]).
    pub fn wipe(&mut self) {
        match self {
            Body::Text(doc) => doc.wipe(),
            Body::Markdown(doc) => doc.source.zeroize(),
            Body::Word(doc) => doc.wipe(),
            Body::Sheet(view) => view.wipe(),
            Body::Doc(doc) => doc.wipe(),
            _ => {}
        }
    }

    /// The person's place in an open Office document, carried from `old` (the body on screen) into this new body of
    /// the SAME open file (`Ui::pump`: a page, the outline or cells came): the page, the zoom and the outline pane of a
    /// document; the slide, the zoom and Present of a deck; the tab, the scroll, the selection of a sheet (same sheet
    /// only). Another pair of kinds carries nothing, and nothing else is ever carried (media, PDF, text: as handed).
    pub fn carry_view(&mut self, old: &Body) {
        match (self, old) {
            (Body::Doc(new), Body::Doc(old)) => {
                new.page = old.page.clamp(1, new.view.pages.max(1));
                new.at = old.at;
                new.view.zoom = old.view.zoom;
                new.outline_open = old.outline_open;
            }
            (Body::Deck(new), Body::Deck(old)) => {
                new.view.current = old.view.current.clamp(1, new.view.count.max(1));
                new.view.zoom = old.view.zoom;
                new.presenting = old.presenting;
            }
            (Body::Sheet(new), Body::Sheet(old)) if new.sheet == old.sheet => {
                new.active = old.active;
                new.top_left = old.top_left;
                new.selection = old.selection;
                new.current = old.current;
                new.formula.clone_from(&old.formula);
            }
            _ => {}
        }
    }
}

pub use super::kinds::{
    ArchiveDoc, AudioDoc, Column, Entry, EntryKind, Match, PdfDoc, PhotoDoc, Picture, StripItem, TableDoc, Track,
    VideoDoc,
};

/// A Markdown note: its text.
#[derive(Clone, PartialEq, Eq)]
pub struct MdDoc {
    pub source: String,
}

/// How long the note is, never a word of it.
impl fmt::Debug for MdDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MdDoc({} bytes)", self.source.len())
    }
}

/// Which side stays when a file was saved elsewhere while it was being edited (DW-1526).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Mine is saved next to theirs under its own name.
    Both,
    /// Mine becomes the newest version; theirs stays in the history.
    Mine,
    /// My edit is dropped; theirs opens.
    Theirs,
}

/// The other side of a conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// "Anna's iPhone".
    pub from: String,
    /// "09:14".
    pub at: String,
    pub version: u32,
}

/// What became of a save.
#[derive(Clone, Debug, PartialEq)]
pub enum SaveOutcome {
    /// Encrypted on this PC and stored as a new version: the file as it is now, and when.
    Saved { file: OpenFile, at: String },
    /// The edit was dropped and this file is open instead (Keep theirs).
    Opened(OpenFile),
    /// Someone saved the same file elsewhere meanwhile: nothing was written.
    Conflict(Conflict),
    /// Not saved, and nothing changed.
    Failed,
    /// Handed to the engine: what became of it comes by [`ViewerSource::settled`] (a save leaves the window's
    /// thread). Nothing is drawn as saved before then.
    Working,
}

/// What a change that left the window's thread became, read by [`ViewerSource::settled`].
pub enum Settled {
    /// Available offline was changed (`done`), or refused: `on` is what was asked.
    Offline { on: bool, done: bool },
    /// The edit is stored as the file's next version `version` (the file `id` under `name`: a Keep both is a new
    /// file next to the first), when `at`.
    Saved {
        id: String,
        name: String,
        body: Body,
        version: u32,
        at: String,
    },
    /// The edit is stored on this PC and sent as soon as Beebeeb can be reached; the version it becomes is not known
    /// yet. `version`: the one it was saved from, which the text on screen is based on (its label).
    Queued { body: Body, version: u32 },
    /// Keep theirs: the edit is dropped and the file as Beebeeb has it is shown.
    Opened { body: Body, version: u32 },
    /// The file is at a version past the one the edit began from (saved on another device, or by an earlier opening
    /// on this PC: who is not known); nothing was written.
    Conflict(Conflict),
    /// The local state holds a version of the open file that this view does not (DW-1519). Nothing on screen
    /// changed: the banner offers it.
    Newer(Newer),
    /// Not done, nothing changed.
    Failed,
    /// Not saved, and why (the words): nothing changed, the edit stays as it is (delta 3 of desktop#432: an earlier
    /// change to the file on this PC has not been sent yet).
    Refused(String),
    /// The revision of the editor's document that the save just reported (`Saved`, `Queued`) was made from: the
    /// editor is saved up to it, no further (typing during the upload stays unsaved). Comes just before its `Saved`/`Queued`.
    SavedRevision(u64),
    /// A change begun before the file was opened again (Show it, Try again): what it came to, said. Nothing on
    /// screen changes.
    Said(String),
}

/// Which of the theme's inks a run of text is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// Keywords, names, bold words.
    Strong,
    Plain,
    /// Comments, Markdown marks.
    Faint,
    /// Strings.
    Green,
    /// Numbers.
    Red,
}

/// A run of one line in one ink.
#[derive(Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub ink: Ink,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// The ink and the length, never the text.
impl fmt::Debug for Run {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Run({:?}, {} bytes)", self.ink, self.text.len())
    }
}

/// How many lines, never one of them.
impl fmt::Debug for TextDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TextDoc({} lines, {:?})", self.lines.len(), self.truncated)
    }
}

/// A text file as the viewer draws it: the lines it holds, coloured.
#[derive(Clone, PartialEq, Eq)]
pub struct TextDoc {
    pub lines: Vec<Vec<Run>>,
    /// The file is over the editor's limit (2 MB): read-only, the first lines only (DW-1527).
    pub truncated: Option<Truncated>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Truncated {
    /// Lines held (500).
    pub shown: u32,
    /// Lines in the file.
    pub total: u32,
    /// Why only the first lines are held: the banner says so.
    pub why: Cut,
}

/// Why a text is shown read-only, its first lines only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cut {
    /// DW-1527: the file is over the editor's limit (2 MB).
    #[default]
    OverTwoMb,
    /// Under 2 MB, and too many lines, or lines too long, for the reader to hold whole.
    TooLong,
}

impl TextDoc {
    /// Overwrite every line's text.
    pub fn wipe(&mut self) {
        for run in self.lines.iter_mut().flatten() {
            run.text.zeroize();
        }
    }

    /// The text as it is in the file: what Copy puts on the clipboard.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            for run in line {
                out.push_str(&run.text);
            }
            out.push('\n');
        }
        out
    }
}

/// A text with no colours: every line one plain run (what a file without a tokenizer shows).
pub fn plain_lines(text: &str) -> Vec<Vec<Run>> {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                Vec::new()
            } else {
                vec![Run {
                    text: line.to_string(),
                    ink: Ink::Plain,
                    bold: false,
                    italic: false,
                    underline: false,
                }]
            }
        })
        .collect()
}

/// Why a file could not be opened at all. Each is a state the window draws (`Ui::unopened`), except
/// [`OpenError::SignedOut`], which the shell's session gate draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The engine has no such item (deleted elsewhere).
    NotFound,
    /// Nothing has synced on this PC yet: it knows no file, this one included.
    NothingSynced,
    /// What this PC holds of Beebeeb is not provably this account's: nothing of it is read.
    NotThisAccounts,
    /// No session in memory (signed out, or locked): nothing is read.
    SignedOut,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    /// It could not be read at this moment (the local state was busy, or the session changed during the
    /// read). Asking again may work.
    NotNow,
}

impl OpenError {
    /// The state the window draws for a file that did not open. `None`: nothing of this group (the shell's
    /// gate says it).
    pub fn stopped(self) -> Option<Stopped> {
        match self {
            OpenError::NotFound => Some(Stopped::Gone),
            OpenError::NothingSynced => Some(Stopped::NothingSynced),
            OpenError::NotThisAccounts => Some(Stopped::NotThisAccounts),
            OpenError::SessionEnded => Some(Stopped::SessionEnded),
            OpenError::NotNow => Some(Stopped::NotNow),
            OpenError::SignedOut => None,
        }
    }
}

/// What became of "Open in another app…".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elsewhere {
    /// The system was asked to open the file with its own app.
    Opened,
    /// The file opens from the account's folder on this PC, and that folder is not in sync now.
    NeedsSync,
    /// It could not be opened there (the file is gone, the system refused, the kind is not opened for safety).
    Refused,
    /// This source has no way to another app: the shell is asked instead (`Intent::OpenInAnotherApp`).
    NotBound,
}

/// What the editor's helper answered since the last [`ViewerSource::poll_edit`].
#[derive(Debug, Clone, PartialEq)]
pub enum EditAnswer {
    /// An edit landed (also the answer to a hit and to an inserted image).
    Edited(Edited),
    /// The selection as text and, when it fit, html (answers `copy_selection`).
    Clip { text: String, html: Option<String> },
    /// The slide layouts' names (answers `layouts`).
    Layouts(Vec<String>),
    /// The editor stopped; the document on screen is no longer live.
    Stopped,
    /// A recovery snapshot was restored: the document is the one saved at `at`.
    Restored { at: String },
    /// The selection is too big to copy at all (not even as text): said as [`COPY_TOO_LARGE`].
    CopyTooLarge,
}

/// What is said when the helper cannot copy the selection (`Error{TooLarge}` to a `Copy`).
pub const COPY_TOO_LARGE: &str = "Too much to copy at once";

/// A snapshot of an earlier, unsaved editing of the file that this PC kept (the words say when: "09:41").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryOffer {
    pub at: String,
}

/// The editor's door to the open file's source (`frames/office/edit`): the `source` field is private to `viewers`.
impl super::Ui {
    /// The source and the open file's id, for the editor's calls.
    pub(crate) fn edit_source(&mut self) -> (&mut dyn ViewerSource, &str) {
        (self.source.as_mut(), self.file.id.as_str())
    }
}

pub trait ViewerSource {
    /// Start opening a file. The answer is what can be shown NOW; a `Decrypting` body is
    /// followed by [`ViewerSource::poll`].
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError>;
    /// Progress of the open file since the last call, if anything changed.
    #[cfg_attr(not(test), allow(dead_code))] // called by `viewers::poll`, which waits for the shell
    fn poll(&mut self, file_id: &str) -> Option<Body>;
    /// Stop decrypting. Nothing is kept half-decrypted.
    fn cancel(&mut self, file_id: &str);
    /// Keep the file on this PC (Available offline) or release the copy. `false`: not done.
    fn set_offline(&mut self, _file_id: &str, _on: bool) -> bool {
        false
    }
    /// Save `text` as a new, encrypted version of the file whose version `base` was edited.
    fn save(&mut self, _file_id: &str, _text: &str, _base: u32) -> SaveOutcome {
        SaveOutcome::Failed
    }
    /// Settle a conflict the last save reported.
    fn resolve(&mut self, _file_id: &str, _text: &str, _keep: Keep) -> SaveOutcome {
        SaveOutcome::Failed
    }
    /// Save the open office document's changes as a new, encrypted version of the file whose
    /// version `base` was opened (the editor holds the changes; nothing of them crosses here).
    fn save_document(&mut self, _file_id: &str, _base: u32) -> SaveOutcome {
        SaveOutcome::Failed
    }
    /// Settle a conflict the last [`ViewerSource::save_document`] reported (DW-1554).
    fn resolve_document(&mut self, _file_id: &str, _keep: Keep) -> SaveOutcome {
        SaveOutcome::Failed
    }
    /// What the open document's command search finds for `query` (styles, commands, files), from
    /// what is held in memory: it is asked at every draw. Nothing until an editor answers.
    fn commands(&self, _file_id: &str, _query: &str) -> Vec<super::office_over::CommandGroup> {
        Vec::new()
    }
    /// The newer version another device saved while this one is open (DW-1519's Show it).
    fn newer(&mut self, _file_id: &str) -> Result<OpenFile, OpenError> {
        Err(OpenError::NotFound)
    }
    /// The lines of `text` as the editor colours them. The default: Markdown's marks dimmed
    /// for a note, plain lines for any other text.
    fn colours(&self, file: &OpenFile, text: &str) -> Vec<Vec<Run>> {
        match file.body {
            Body::Markdown(_) => text.split('\n').map(super::md_source::line).collect(),
            _ => text
                .split('\n')
                .map(|line| plain_lines(line).pop().unwrap_or_default())
                .collect(),
        }
    }
    /// Open the file with the app the system has for it, from the place the engine keeps the account's files on
    /// this PC (never a decrypted copy made for the occasion).
    fn open_elsewhere(&mut self, _file_id: &str) -> Elsewhere {
        Elsewhere::NotBound
    }
    /// Whether a file of this name is shown HERE (decrypted and drawn). `false`: a list's Open leaves it to its
    /// own app, as before (`viewers::open_item`); a route that names it anyway gets the source's honest state.
    /// The default: every kind but those only a player shows (`kinds::needs_a_player`), which a source without
    /// one (the board's fixture, a test's) does not show.
    fn previews(&self, name: &str) -> bool {
        !super::kinds::needs_a_player(name)
    }
    /// Whether the file's bytes are kept on this PC, where the system can open them with the file's own app.
    /// `true`: a list's Open hands the file to that app (`viewers::open_item`), as it did before there was a
    /// viewer here; the viewer is for files that are not on this PC, where it saves a download to disk.
    #[allow(dead_code)]
    fn kept_on_this_pc(&self, _file_id: &str) -> bool {
        false
    }
    /// Whether a recording or a video named `name` that is kept on this PC still opens HERE, in Beebeeb's player,
    /// from a list's Open (decision D-MED-13; "Open in another app…" stays one press away). `false` (the default):
    /// it goes to its own app, as before (`viewers::open_item`).
    #[allow(dead_code)]
    fn opens_here_when_kept(&self, _name: &str) -> bool {
        false
    }
    /// Where the content on screen was read from. Asked when it is drawn.
    fn origin(&self, _file_id: &str) -> Origin {
        Origin::Beebeeb
    }
    /// What there is of page `page` (from 1) of the open PDF, for the page the frame draws without a picture.
    fn page_state(&self, _file_id: &str, _page: u32) -> PageState {
        PageState::Ready
    }
    /// Whether [`ViewerSource::find`] searches the open PDF's text. `false`: Find says it is not built, never
    /// "No matches" for words that were not looked for.
    fn finds(&self) -> bool {
        true
    }
    /// Whether "Save a copy…" has somewhere to go. `false`: the button is not drawn.
    fn saves_copies(&self) -> bool {
        true
    }
    /// Whether [`ViewerSource::save`] can store an edit. `false`: the editor does not open (an edit that cannot
    /// be saved is not offered), and the press says so.
    fn can_save(&self) -> bool {
        true
    }
    /// Whether `Details::offline` was read and [`ViewerSource::set_offline`] can change it. `false`: the
    /// details panel draws no Available offline switch (a state that was not read is not shown).
    fn knows_offline(&self) -> bool {
        true
    }
    /// What a change handed over by `set_offline` (when [`ViewerSource::answers_later`]), `save` or `resolve`
    /// became. `None`: nothing new.
    fn settled(&mut self, _file_id: &str) -> Option<Settled> {
        None
    }
    /// Whether `set_offline` answers by [`ViewerSource::settled`]: the switch moves when the engine did, not at the
    /// press.
    fn answers_later(&self) -> bool {
        false
    }
    /// A change of the open file is on its way (a save, a keep, the switch): Show it waits until it answered.
    fn saving(&self) -> bool {
        false
    }
    /// Whether the open file takes editor calls (`edit`, `hit`, ...). Default: not available.
    fn can_edit(&self, _file_id: &str) -> bool {
        false
    }
    /// What the editor's guard said editing the open file would drop: it opened read-only in the viewer, which says
    /// so ([`beebeeb_office_protocol::office::DroppedPart::notice`]). Default: nothing (not guarded).
    fn edit_guarded(&self, _file_id: &str) -> Vec<beebeeb_office_protocol::office::DroppedPart> {
        Vec::new()
    }
    /// The friendly name of the app the system opens the open file's type with ("PowerPoint"), once known. Never
    /// waits; `None`: none registered, not known yet, or this system cannot say ("Open in another app…" then).
    fn default_app(&self, _file_id: &str) -> Option<String> {
        None
    }
    /// Hand one edit to the editor. `false`: not available or not taken. The answer comes by `poll_edit`.
    fn edit(&mut self, _file_id: &str, _op: EditOp) -> bool {
        false
    }
    /// What is under a point of a page (the caret goes there, or the selection grows). `false`: not taken.
    fn hit(&mut self, _file_id: &str, _page: u32, _x_pt: f32, _y_pt: f32, _mode: HitMode) -> bool {
        false
    }
    /// Ask for the selection as text (answered `EditAnswer::Clip`). `false`: not taken.
    fn copy_selection(&mut self, _file_id: &str) -> bool {
        false
    }
    /// Ask for the slide layouts' names (answered `EditAnswer::Layouts`). `false`: not taken.
    fn layouts(&mut self, _file_id: &str) -> bool {
        false
    }
    /// Insert an image (PNG or JPEG bytes, wiped when dropped). `false`: not taken.
    fn insert_image(&mut self, _file_id: &str, _bytes: Zeroizing<Vec<u8>>, _png: bool) -> bool {
        false
    }
    /// The editor's next answer, if any arrived.
    fn poll_edit(&mut self, _file_id: &str) -> Option<EditAnswer> {
        None
    }
    /// Make a new document of `kind` from `template` in `folder` (`None`: the top). The answer is `settled`.
    fn create_document(&mut self, _folder: Option<&str>, _name: &str, _kind: EditKind, _template: Template) -> bool {
        false
    }
    /// An unsaved editing of this file that this PC kept, if any.
    fn recovery(&self, _file_id: &str) -> Option<RecoveryOffer> {
        None
    }
    /// Open the kept editing (answered `EditAnswer::Restored`). `false`: not taken.
    fn restore(&mut self, _file_id: &str) -> bool {
        false
    }
    /// Drop the kept editing.
    fn discard_recovery(&mut self, _file_id: &str) {}
    /// The account's files with a kept editing, newest first: the file's id and the editing's time ("14:32"). For the
    /// notice after unlock.
    fn recovery_pending(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// How long until this source next reads whether a newer version arrived (DW-1519), while it reads one: the
    /// window asks to be woken then, so the banner comes without a press. `None`: it does not read.
    fn next_check(&self) -> Option<std::time::Duration> {
        None
    }
    /// Download and decrypt again after a failure.
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError>;
    /// A picture of the open file as decoded pixels (the photo, a strip thumbnail, the artwork,
    /// a PDF page), rendered on this PC from the decrypted file. `None`: there is none (yet);
    /// the frame draws its plain tile.
    fn picture(&self, _file_id: &str, _which: Picture) -> Option<rune::Pixels> {
        None
    }
    /// A picture of an open Office file: the one held, else asked of its renderer once and `None` until it comes (as
    /// [`ViewerSource::picture`]).
    fn office_picture(&self, _file_id: &str, _which: super::kinds::OfficePicture) -> Option<rune::Pixels> {
        None
    }
    /// Whether this source holds the open Office file in a renderer: the scenes ask it for pictures and cells. `false`
    /// (the default, the fixture source): they draw the fixture pictures. Not told by the answers, which are `None`
    /// both before a live picture comes and always from a fixture.
    fn office_live(&self) -> bool {
        false
    }
    /// The view shows these cells of `sheet`: asked if not held; the cells come later through
    /// [`ViewerSource::poll`] as a new `Body::Sheet`.
    fn office_cells(&self, _file_id: &str, _sheet: u32, _first_row: u32, _first_col: u32, _rows: u32, _cols: u32) {}
    /// Where `words` are in the open PDF, in reading order. Empty: nowhere, or no text to search.
    fn find(&mut self, _file_id: &str, _words: &str) -> Vec<Match> {
        Vec::new()
    }
    /// `true`: this source can play a recording. `false` (the default): the player draws no Play
    /// button and offers "Open in another app" instead.
    fn can_play(&self) -> bool {
        false
    }
    /// Play or pause the open recording. `false`: not done.
    fn play(&mut self, _file_id: &str, _on: bool) -> bool {
        false
    }
    /// The shell's player, when this source plays recordings and videos (`media::player`). `None` (the default):
    /// the views draw what they drew before there was a player.
    fn player(&self) -> Option<crate::media::player::PlayerHandle> {
        None
    }
}

const MB: u64 = 1_000_000;
const GB: u64 = 1_000_000_000;

/// The ids of the board's sample files.
pub mod board {
    /// DW-1516.
    pub const DECRYPTING: &str = "tram-28.mov";
    /// DW-1517.
    pub const TOO_LARGE: &str = "studio-walkthrough-4k.mov";
    /// DW-1518.
    pub const FAILED: &str = "invoice-0139.pdf";
    /// DW-1512.
    pub const CODE: &str = "export_invoices.py";
    /// DW-1527.
    pub const LOG: &str = "server.log";
    /// DW-1513.
    pub const NOTES: &str = "meeting-notes-4-nov.md";
    /// DW-1501, DW-1502.
    pub const RAW: &str = "dsc04182.arw";
    /// DW-1503.
    pub const AUDIO: &str = "harbour-sessions-take-3.m4a";
    /// DW-1505.
    pub const PDF: &str = "studio-plan.pdf";
    /// DW-1506.
    pub const PDF_FIND: &str = "service-agreement.pdf";
    /// DW-1514.
    pub const CSV: &str = "rates-2026.csv";
    /// DW-1515.
    pub const ZIP: &str = "shoot-selects-export.zip";
    /// DW-1507.
    pub const DOCX: &str = "service-agreement.docx";
    /// DW-1511: an older Word file.
    pub const DOC: &str = "proposal-2012.doc";
    /// DW-1508.
    pub const SHEET: &str = "budget-2026.xlsx";
    /// DW-1509, DW-1561 (the id of `office::slides::slides_fixture::file()`).
    pub const DECK: &str = "lisbon-trip-recap.pptx";
    /// DW-1507, DW-1511 (the ids of `office::docs::docs_fixture::file`): a Word file in the viewer.
    pub const DOC_VIEW: &str = super::super::super::office::docs::docs_fixture::DOCX_ID;
    pub const DOC_OLD_VIEW: &str = super::super::super::office::docs::docs_fixture::DOC_ID;
    /// DW-1513's text, as the note's source (DW-1523 shows it with one more line being typed).
    pub const NOTES_TEXT: &str = "# Meeting notes, 4 Nov\n\nPresent: Anna, Lotte, Jonas\n\n## Harbourlight shoot\n- Final selects due **Friday**\n- Colour grade in `Shoot selects/graded`\n- Send the agreement: [Service agreement](Service agreement.docx)\n\n## Studio\n- Rent goes up in January (see Studio rent 2026.pdf)\n- Book the Lisbon follow-up\n";
}

/// The board's samples. `calls` records what the viewer asked, for tests.
#[derive(Default)]
pub struct FixtureViewerSource {
    pub calls: Vec<String>,
    /// DW-1519: another device saved a newer version of the note while it is open.
    pub newer: bool,
    /// DW-1526, DW-1554: the next save finds that newer version in its way.
    pub conflict: bool,
    /// Which board frame's office document is handed over.
    pub office: super::office::board::Case,
    /// The editor's answers not yet polled, and the revision they reached.
    pub edit_answers: Vec<EditAnswer>,
    pub edit_revision: u64,
    /// The revision of the last save that landed (a save becomes the next version).
    pub saved_revision: u64,
    /// A kept editing the board's recovery offer shows.
    pub offer: Option<RecoveryOffer>,
}

impl FixtureViewerSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// The device of the board's DW-1519 and DW-1526.
    fn other_side() -> Newer {
        Newer {
            from: "Anna's iPhone".into(),
            at: "09:14".into(),
        }
    }

    /// An `Edited` with a caret on page one at `revision`.
    fn edited(revision: u64, can_redo: bool) -> Edited {
        use beebeeb_office_protocol::office::{Dirty, ListKind, Marks, PageRect, ParaStyle, RectPt, TextAlign};
        Edited {
            revision,
            caret: Some(PageRect {
                page: 0,
                rect: RectPt {
                    x: 72.0,
                    y: 96.0,
                    w: 1.0,
                    h: 14.0,
                },
            }),
            selection: Vec::new(),
            can_undo: revision > 0,
            can_redo,
            dirty: Dirty::All,
            pages: None,
            cells: Vec::new(),
            slides: None,
            marks: Marks {
                bold: false,
                italic: false,
                underline: false,
                strike: false,
                style: ParaStyle::Normal,
                align: TextAlign::Left,
                list: ListKind::None,
            },
        }
    }

    /// The file after a save of `text` as `version`.
    fn saved(id: &str, text: &str, version: u32) -> Option<OpenFile> {
        let mut file = Self::file(id)?;
        file.version = version;
        match &mut file.body {
            Body::Markdown(doc) => {
                doc.source = text.to_string();
                file.facts = "Markdown \u{00B7} 1.5 KB".into();
                for fact in &mut file.details.facts {
                    if fact.0 == "Version history" {
                        fact.1 = format!("{version} versions");
                    }
                }
            }
            Body::Text(doc) if doc.truncated.is_none() => doc.lines = plain_lines(text),
            _ => return None,
        }
        Some(file)
    }

    /// The agreement as stored by a save at 09:41, as `version`, under `name` if given.
    fn agreement_saved(version: u32, name: Option<&str>) -> SaveOutcome {
        let Some(mut file) = Self::file(board::DOCX) else {
            return SaveOutcome::Failed;
        };
        file.version = version;
        file.body = Body::Word(super::office::board::service_agreement_saved());
        if let Some(name) = name {
            file.name = name.to_string();
        }
        SaveOutcome::Saved {
            file,
            at: "09:41".into(),
        }
    }

    fn file(id: &str) -> Option<OpenFile> {
        let f = |path: &[&str], name: &str, facts: &str, body: Body| OpenFile {
            id: id.to_string(),
            path: path.iter().map(|p| p.to_string()).collect(),
            name: name.to_string(),
            facts: facts.to_string(),
            version: 1,
            body,
            details: Details::default(),
            newer: None,
        };
        Some(match id {
            board::DECRYPTING => f(
                &["Files", "Lisbon trip"],
                "Tram 28.mov",
                "Video \u{00B7} 120 MB",
                Body::Decrypting {
                    done: 46 * MB,
                    total: 120 * MB,
                },
            ),
            board::TOO_LARGE => f(
                &["Files", "Documents"],
                "Studio walkthrough 4K.mov",
                "Video \u{00B7} 18.2 GB",
                Body::TooLarge {
                    size: 18_200 * MB,
                    free: 12_400 * MB,
                },
            ),
            board::DOCX => OpenFile {
                version: 7,
                ..f(
                    &["Files"],
                    "Service agreement.docx",
                    "Word document \u{00B7} 38 KB \u{00B7} opens in the editor",
                    Body::Word(super::office::board::service_agreement()),
                )
            },
            board::DOC => OpenFile {
                version: 2,
                ..f(
                    &["Files", "Documents"],
                    "Proposal 2012.doc",
                    "Word 97\u{2013}2004 document \u{00B7} 220 KB",
                    Body::Word(super::office::board::proposal()),
                )
            },
            board::SHEET => OpenFile {
                version: 12,
                ..f(
                    &["Files"],
                    super::super::office::sheets::sheets_fixture::FILE_NAME,
                    "Excel workbook \u{00B7} 84 KB",
                    Body::Sheet(super::super::office::sheets::sheets_fixture::board()),
                )
            },
            board::DECK => super::super::office::slides::slides_fixture::file(),
            board::DOC_VIEW => super::super::office::docs::docs_fixture::file(false),
            board::DOC_OLD_VIEW => super::super::office::docs::docs_fixture::file(true),
            board::FAILED => f(
                &["Files", "Invoices 2026"],
                "Invoice 0139.pdf",
                "PDF \u{00B7} 220 KB",
                Body::Failed,
            ),
            board::CODE => f(
                &["Files", "Documents"],
                "export_invoices.py",
                "Python \u{00B7} 25 lines \u{00B7} UTF-8",
                Body::Text(TextDoc {
                    lines: super::fixture_text::python(),
                    truncated: None,
                }),
            ),
            board::LOG => f(
                &["Files", "Documents"],
                "server.log",
                "Log \u{00B7} 6.4 MB \u{00B7} read-only",
                Body::Text(TextDoc {
                    lines: super::fixture_text::log(),
                    truncated: Some(Truncated {
                        shown: 500,
                        total: 41_206,
                        why: Cut::OverTwoMb,
                    }),
                }),
            ),
            board::NOTES => {
                let mut file = f(
                    &["Files", "Documents"],
                    "Meeting notes 4 Nov.md",
                    "Markdown \u{00B7} 1.4 KB",
                    Body::Markdown(MdDoc {
                        source: board::NOTES_TEXT.to_string(),
                    }),
                );
                file.version = 3;
                file.details = Details {
                    kind: "Markdown".into(),
                    size: "1.4 KB".into(),
                    facts: vec![
                        ("Changed", "Today, 09:20".into(), true),
                        ("Changed on", "This PC".into(), false),
                        ("Where", "Files \u{203A} Documents".into(), false),
                        ("Version history", "3 versions".into(), true),
                        ("Encryption", "AES-256-GCM".into(), true),
                        ("Key", "256-bit, this file only".into(), false),
                    ],
                    offline: true,
                };
                file
            }
            _ => return super::fixture_media::file(id),
        })
    }
}

impl ViewerSource for FixtureViewerSource {
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.calls.push(format!("open {file_id}"));
        let mut file = Self::file(file_id).ok_or(OpenError::NotFound)?;
        if self.newer && file_id == board::NOTES {
            file.newer = Some(Self::other_side());
        }
        if file_id == board::DOCX {
            use super::office::board::{Case, service_agreement_as};
            if self.office == Case::NoEditor {
                file.body = Body::NoEditor;
                file.facts = "Word document \u{00B7} 38 KB".into();
            } else {
                file.body = Body::Word(service_agreement_as(self.office));
            }
        }
        Ok(file)
    }

    fn can_edit(&self, file_id: &str) -> bool {
        file_id == board::DOCX
    }

    /// The fixture's edit: the next revision, the caret after the typed text on page one.
    fn edit(&mut self, file_id: &str, op: EditOp) -> bool {
        self.calls.push(format!("edit {file_id}"));
        if file_id != board::DOCX {
            return false;
        }
        self.edit_revision += 1;
        let can_redo = matches!(op, EditOp::Undo);
        let answer = Self::edited(self.edit_revision, can_redo);
        self.edit_answers.push(EditAnswer::Edited(answer));
        true
    }

    fn hit(&mut self, file_id: &str, _page: u32, _x_pt: f32, _y_pt: f32, _mode: HitMode) -> bool {
        self.calls.push(format!("hit {file_id}"));
        file_id == board::DOCX && {
            self.edit_answers
                .push(EditAnswer::Edited(Self::edited(self.edit_revision, false)));
            true
        }
    }

    fn copy_selection(&mut self, file_id: &str) -> bool {
        self.calls.push(format!("copy {file_id}"));
        file_id == board::DOCX && {
            self.edit_answers.push(EditAnswer::Clip {
                text: "Service agreement".into(),
                html: None,
            });
            true
        }
    }

    fn poll_edit(&mut self, _file_id: &str) -> Option<EditAnswer> {
        (!self.edit_answers.is_empty()).then(|| self.edit_answers.remove(0))
    }

    fn recovery(&self, _file_id: &str) -> Option<RecoveryOffer> {
        self.offer.clone()
    }

    fn restore(&mut self, file_id: &str) -> bool {
        self.calls.push(format!("restore {file_id}"));
        match self.offer.take() {
            Some(offer) => {
                self.edit_answers.push(EditAnswer::Restored { at: offer.at });
                true
            }
            None => false,
        }
    }

    fn discard_recovery(&mut self, file_id: &str) {
        self.calls.push(format!("discard recovery {file_id}"));
        self.offer = None;
    }

    /// The fixture saves the agreement as the board's DW-1553 shows it: the next version, at 09:41.
    fn save_document(&mut self, file_id: &str, base: u32) -> SaveOutcome {
        self.calls.push(format!("save document {file_id} from version {base}"));
        if file_id != board::DOCX {
            return SaveOutcome::Failed;
        }
        if self.conflict {
            let other = Self::other_side();
            return SaveOutcome::Conflict(Conflict {
                from: other.from,
                at: other.at,
                version: base + 1,
            });
        }
        self.saved_revision = self.edit_revision;
        let table = self.office == super::office::board::Case::Table;
        self.office = super::office::board::Case::Selected;
        let mut outcome = Self::agreement_saved(base + 1, None);
        // What was saved is what was on screen: the inserted table stays.
        if let (true, SaveOutcome::Saved { file, .. }) = (table, &mut outcome)
            && let Body::Word(doc) = &mut file.body
        {
            doc.sections = super::office::board::service_agreement_as(super::office::board::Case::Table).sections;
        }
        outcome
    }

    fn resolve_document(&mut self, file_id: &str, keep: Keep) -> SaveOutcome {
        self.calls.push(format!("resolve document {file_id} {keep:?}"));
        let Some(theirs) = Self::file(file_id).filter(|_| file_id == board::DOCX) else {
            return SaveOutcome::Failed;
        };
        self.conflict = false;
        self.office = super::office::board::Case::Selected;
        match keep {
            // Mine next to theirs, under this PC's name.
            Keep::Both => Self::agreement_saved(1, Some("Service agreement (Anna-Laptop).docx")),
            Keep::Mine => Self::agreement_saved(theirs.version + 2, None),
            Keep::Theirs => SaveOutcome::Opened(OpenFile {
                version: theirs.version + 1,
                body: Body::Word(super::office::board::service_agreement_saved()),
                ..theirs
            }),
        }
    }

    fn commands(&self, _file_id: &str, query: &str) -> Vec<super::office_over::CommandGroup> {
        super::office::board::commands(query)
    }

    /// The fixture saves the note as the board's DW-1524 shows it: version 4 at 09:41, 1.5 KB
    /// in the toolbar, "4 versions" in the panel. (The board's panel still says 1.4 KB and
    /// "Changed Today, 09:20" there: its own slip, kept as drawn and named in the PR body.)
    fn save(&mut self, file_id: &str, text: &str, base: u32) -> SaveOutcome {
        self.calls.push(format!("save {file_id} from version {base}"));
        if self.conflict {
            let other = Self::other_side();
            return SaveOutcome::Conflict(Conflict {
                from: other.from,
                at: other.at,
                version: base + 1,
            });
        }
        match Self::saved(file_id, text, base + 1) {
            Some(file) => SaveOutcome::Saved {
                file,
                at: "09:41".into(),
            },
            None => SaveOutcome::Failed,
        }
    }

    fn resolve(&mut self, file_id: &str, text: &str, keep: Keep) -> SaveOutcome {
        self.calls.push(format!("resolve {file_id} {keep:?}"));
        self.conflict = false;
        let Some(theirs) = Self::file(file_id) else {
            return SaveOutcome::Failed;
        };
        let saved = |version: u32, name: Option<&str>| match Self::saved(file_id, text, version) {
            Some(mut file) => {
                if let Some(name) = name {
                    file.name = name.to_string();
                }
                SaveOutcome::Saved {
                    file,
                    at: "09:41".into(),
                }
            }
            None => SaveOutcome::Failed,
        };
        match keep {
            // Mine next to theirs, under this PC's name (the board's words for it).
            Keep::Both => saved(1, Some("Meeting notes 4 Nov (Anna-Laptop).md")),
            Keep::Mine => saved(theirs.version + 2, None),
            Keep::Theirs => SaveOutcome::Opened(OpenFile {
                version: theirs.version + 1,
                ..theirs
            }),
        }
    }

    fn newer(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.calls.push(format!("newer {file_id}"));
        self.newer = false;
        let file = Self::file(file_id).ok_or(OpenError::NotFound)?;
        Ok(OpenFile {
            version: file.version + 1,
            ..file
        })
    }

    /// The board colours its samples by hand: a line of the board's own text keeps the board's
    /// runs, any other line is coloured by rule.
    fn colours(&self, file: &OpenFile, text: &str) -> Vec<Vec<Run>> {
        let board = super::fixture_text::notes_source();
        text.split('\n')
            .map(|line| {
                let drawn = board
                    .iter()
                    .find(|runs| runs.iter().map(|r| r.text.as_str()).collect::<String>() == line);
                match (drawn, &file.body) {
                    (Some(runs), Body::Markdown(_)) => runs.clone(),
                    (_, Body::Markdown(_)) => super::md_source::line(line),
                    _ => plain_lines(line).pop().unwrap_or_default(),
                }
            })
            .collect()
    }

    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }

    fn cancel(&mut self, file_id: &str) {
        self.calls.push(format!("cancel {file_id}"));
    }

    fn set_offline(&mut self, file_id: &str, on: bool) -> bool {
        self.calls.push(format!("offline {file_id} {on}"));
        true
    }

    fn picture(&self, file_id: &str, which: Picture) -> Option<rune::Pixels> {
        super::fixture_media::picture(file_id, which)
    }

    fn find(&mut self, file_id: &str, words: &str) -> Vec<Match> {
        self.calls.push(format!("find {file_id} {words}"));
        super::fixture_media::find(file_id, words)
    }

    fn can_play(&self) -> bool {
        true
    }

    fn play(&mut self, file_id: &str, on: bool) -> bool {
        self.calls.push(format!("play {file_id} {on}"));
        true
    }

    /// The fixture's second try gets as far as the board's DW-1516 draws.
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.calls.push(format!("retry {file_id}"));
        // The editor starts at the second try (DW-1556's Try again).
        if file_id == board::DOCX && self.office == super::office::board::Case::NoEditor {
            self.office = super::office::board::Case::Plain;
            return self.open(file_id);
        }
        let mut file = Self::file(file_id).ok_or(OpenError::NotFound)?;
        file.body = Body::Decrypting {
            done: 0,
            total: 220_000,
        };
        Ok(file)
    }
}

/// "18.2 GB", "120 MB", "220 KB": decimal units, one decimal from a gigabyte up, as the board writes sizes.
pub fn size_words(bytes: u64) -> String {
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else {
        format!("{} KB", bytes / 1000)
    }
}

/// "46 of 120 MB": the part done in the unit of the whole.
pub fn progress_words(done: u64, total: u64) -> String {
    let whole = size_words(total);
    let unit = if total >= GB {
        GB
    } else if total >= MB {
        MB
    } else {
        1000
    };
    let part = if total >= GB {
        format!("{:.1}", done as f64 / unit as f64)
    } else {
        (done / unit).to_string()
    };
    format!("{part} of {whole}")
}

/// "41,206": a count with thousands separators.
pub fn count_words(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The whole percent done, as the bar draws it.
pub fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    (done.min(total) * 100 / total) as u32
}
