//! What the source hands the viewer for a photo, a recording, a PDF, a table and an archive
//! (DW-1501 to DW-1506, DW-1514, DW-1515), and the router that says which viewer a file name gets.
//!
//! None of these holds pixels: a picture (the photo, a strip thumbnail, the artwork, a PDF page) is
//! asked of the source when it is drawn ([`Picture`]), and a source that has none answers `None`;
//! the frame then draws its plain tile, never a sample.

/// A photo (DW-1501). The camera's own preview of a RAW says so in the pill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhotoDoc {
    /// "Camera preview · Sony ARW": the photo on screen is the preview a RAW carries. `None`:
    /// the photo itself.
    pub pill: Option<String>,
    /// What the status line says at the right ("RAW · the camera's own preview, not a developed RAW").
    pub note: String,
    /// The photos of the folder around this one, in the folder's order.
    pub strip: Vec<StripItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StripItem {
    pub id: String,
    pub name: String,
    pub current: bool,
}

/// A recording (DW-1503): what plays, and the folder as the queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDoc {
    /// The title from the file's tags, else the file's name.
    pub title: String,
    /// The artist from the tags. Empty: none.
    pub artist: String,
    /// Seconds in all, and where playback is.
    pub length: u32,
    pub at: u32,
    pub playing: bool,
    /// The folder the queue is ("Studio demos").
    pub folder: String,
    /// The folder's audio files, in its sort order.
    pub queue: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub length: u32,
    pub current: bool,
}

/// A PDF (DW-1505): how many pages, the page it opens on, and its size in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfDoc {
    pub pages: u32,
    /// From 1.
    pub page: u32,
    /// "2.3 MB".
    pub size: String,
}

/// One place the words looked for are, on a page (DW-1506): a box in the page's own points at 100 %.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Match {
    /// From 1.
    pub page: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A CSV or TSV (DW-1514): the first row as the columns' names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDoc {
    /// "Comma-separated", "Semicolon-separated", "Tab-separated": the separator found.
    pub separator: String,
    pub columns: Vec<Column>,
    /// The rows held.
    pub rows: Vec<Vec<String>>,
    /// The rows in the file.
    pub total_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// Every value is a number: right-aligned, tabular figures.
    pub numeric: bool,
}

/// An archive's list (DW-1515). Only the list is read: nothing is unpacked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveDoc {
    /// "214 files · 3.8 GB unpacked · files inside can't be opened here."
    pub note: String,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    Text,
    Sheet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The path inside the archive; a folder's ends in "/".
    pub path: String,
    pub kind: EntryKind,
    /// "212 files" of a folder. Empty: a file.
    pub contains: String,
    /// "4.1 MB" of a file. Empty: a folder.
    pub size: String,
}

/// A picture the viewer draws from the source's decoded pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    /// The open photo, at the size the stage shows it.
    Photo,
    /// Item `0..` of the photo's strip.
    Strip(usize),
    /// The artwork in a recording's tags.
    Artwork,
    /// A PDF's page, from 1.
    Page(u32),
}

/// Which viewer a file gets, by its name's extension. `None`: this group has no viewer for it
/// (the file goes to its own app, as before).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Viewer {
    Photo,
    Audio,
    Pdf,
    Table,
    Archive,
    Text,
    Markdown,
}

/// The router by file type.
pub fn viewer_of(name: &str) -> Option<Viewer> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "arw" | "cr2" | "cr3" | "nef" | "raf" | "orf" | "rw2" | "dng" | "jpg" | "jpeg" | "png" | "heic" | "webp"
        | "gif" => Viewer::Photo,
        "m4a" | "mp3" | "wav" | "flac" | "aac" | "ogg" | "opus" => Viewer::Audio,
        "pdf" => Viewer::Pdf,
        "csv" | "tsv" => Viewer::Table,
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" => Viewer::Archive,
        "md" | "markdown" => Viewer::Markdown,
        "txt" | "log" | "py" | "rs" | "js" | "ts" | "json" | "toml" | "yaml" | "yml" | "sh" | "css" | "html"
        | "xml" => Viewer::Text,
        _ => return None,
    })
}

/// Whether the file's own app SHOWS it: a document, a picture, a table, a note. `false` for source code,
/// scripts, web pages and data files: the system's Open would run a script with its interpreter, load a web
/// page as a local file in the browser, or find no app at all; reading one is what the window's viewer is for.
/// For a list's Open of a file that is kept on this PC (`viewers::open_item`).
pub fn its_app_shows_it(name: &str) -> bool {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match viewer_of(name) {
        Some(Viewer::Text) => matches!(ext.as_str(), "txt" | "log"),
        Some(_) => true,
        None => true,
    }
}

/// "4:02", "28:40": minutes and seconds.
pub fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
