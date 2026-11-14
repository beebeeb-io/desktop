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
//! [`TextDoc`], [`MdDoc`] or [`Run`] prints counts and kinds, never a name or a word of the file,
//! and [`OpenFile::wipe`] overwrites the text it holds when the file is left.

use std::fmt;

use zeroize::Zeroize;

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
    /// Overwrite the decrypted text this file holds (the note, the lines) before it is dropped: the file was
    /// closed, the route left it, or its session ended. Pictures are the source's to release.
    pub fn wipe(&mut self) {
        self.body.wipe();
    }
}

/// The details panel's facts (`.mk-det`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

/// A newer version saved elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Newer {
    /// "Anna's iPhone".
    pub from: String,
    /// "09:14".
    pub at: String,
}

/// Why a file that is in Beebeeb is not shown here. Either way there is a way out: another app, or a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoPreview {
    /// This app has no viewer for this kind of file yet. Nothing was downloaded.
    NoViewer,
    /// Larger than a preview holds in memory (`limit`). Nothing was downloaded.
    TooBig { size: u64, limit: u64 },
    /// Decrypted, and not what its name says (text that is not text, a picture that does not decode).
    Unreadable,
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
}

impl Stopped {
    /// Opening again may work as things are.
    pub fn can_retry(self) -> bool {
        matches!(self, Stopped::Offline | Stopped::SessionChanged)
    }
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
    /// DW-1505, DW-1506: a PDF.
    Pdf(PdfDoc),
    /// DW-1514: a CSV or TSV as a table.
    Table(TableDoc),
    /// DW-1515: an archive's list.
    Archive(ArchiveDoc),
    /// DW-1507: a text document in the office editor's frame.
    Word(super::office::WordDoc),
    /// DW-1556: an office document, and the editor did not start on this PC. Nothing of the file
    /// is held here and nothing was changed.
    NoEditor,
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
            Body::Word(doc) => write!(f, "Word({doc:?})"),
            Body::NoEditor => f.write_str("NoEditor"),
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
            _ => {}
        }
    }
}

pub use super::kinds::{
    ArchiveDoc, AudioDoc, Column, Entry, EntryKind, Match, PdfDoc, PhotoDoc, Picture, StripItem, TableDoc, Track,
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

/// Why a file could not be opened at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The engine has no such item (deleted elsewhere, or not this account's).
    NotFound,
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
    fn previews(&self, _name: &str) -> bool {
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
    /// Download and decrypt again after a failure.
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError>;
    /// A picture of the open file as decoded pixels (the photo, a strip thumbnail, the artwork,
    /// a PDF page), rendered on this PC from the decrypted file. `None`: there is none (yet);
    /// the frame draws its plain tile.
    fn picture(&self, _file_id: &str, _which: Picture) -> Option<rune::Pixels> {
        None
    }
    /// Where `words` are in the open PDF, in reading order. Empty: nowhere, or no text to search.
    fn find(&mut self, _file_id: &str, _words: &str) -> Vec<Match> {
        Vec::new()
    }
    /// Play or pause the open recording. `false`: not done.
    fn play(&mut self, _file_id: &str, _on: bool) -> bool {
        false
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
        self.office = super::office::board::Case::Selected;
        Self::agreement_saved(base + 1, None)
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
        if i > 0 && (digits.len() - i) % 3 == 0 {
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
