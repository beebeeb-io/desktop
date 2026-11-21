//! What the source hands the viewer for a photo, a recording, a PDF, a table and an archive
//! (DW-1501 to DW-1506, DW-1514, DW-1515), and the router that says which viewer a file name gets.
//!
//! None of these holds pixels: a picture (the photo, a strip thumbnail, the artwork, a PDF page) is
//! asked of the source when it is drawn ([`Picture`]), and a source that has none answers `None`;
//! the frame then draws its plain tile, never a sample.

/// Office files open in the viewer (on since O-CP3: the viewer's rungs ran on Windows; which types show is [`OFFICE_SHIPS`]).
pub const OFFICE_VIEWER_ON: bool = true;

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

/// A video in Beebeeb's player (decision D-MED-6: a video opened from a list plays here; the board has no frame for
/// it yet, one is requested). What plays, and where, comes from the player (`media::player::PlayState`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoDoc {
    /// Seconds in all, as the window knows it before the player opened the file.
    pub length: u32,
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
    /// Beebeeb's player (`viewers::video`).
    Video,
    Pdf,
    Table,
    Archive,
    Text,
    Markdown,
}

/// The audio formats added with the player (decision D-MED-13): only a source with a player shows them
/// ([`needs_a_player`]).
const PLAYER_ONLY_AUDIO: [&str; 8] = ["oga", "aif", "aiff", "caf", "mka", "ac3", "mp2", "wma"];

/// The router by file type.
pub fn viewer_of(name: &str) -> Option<Viewer> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        // A HEIC/HEIF and a RAW are decoded by the confined helper (095), else the honest refusal.
        "arw" | "cr2" | "cr3" | "nef" | "raf" | "orf" | "rw2" | "dng" | "jpg" | "jpeg" | "png" | "heic" | "heif"
        | "webp" | "gif" => Viewer::Photo,
        "m4a" | "mp3" | "wav" | "flac" | "aac" | "ogg" | "opus" => Viewer::Audio,
        ext if PLAYER_ONLY_AUDIO.contains(&ext) => Viewer::Audio,
        // Not `.ts`: that stays a text file (TypeScript, below). A transport stream plays as `.m2ts` or `.mts`.
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "m2ts" | "mts" | "mpg" | "mpeg" | "vob" | "ogv" | "wmv"
        | "3gp" => Viewer::Video,
        "pdf" => Viewer::Pdf,
        "csv" | "tsv" => Viewer::Table,
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" => Viewer::Archive,
        "md" | "markdown" => Viewer::Markdown,
        "txt" | "log" | "py" | "rs" | "js" | "ts" | "json" | "toml" | "yaml" | "yml" | "sh" | "css" | "html"
        | "xml" | "svg" => Viewer::Text,
        _ => return None,
    })
}

/// Whether only the player shows this kind of file: a video, or an audio format added with the player (decisions
/// D-MED-6, D-MED-13). A source without a player does not show these (`ViewerSource::previews`' default), so a list's
/// Open leaves them to their own app, as before there was a player.
pub fn needs_a_player(name: &str) -> bool {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    viewer_of(name) == Some(Viewer::Video) || PLAYER_ONLY_AUDIO.contains(&ext.as_str())
}

/// "4:02", "28:40": minutes and seconds.
pub fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Which Office types ship (2482 Q7: ≥ 90 % of a type's corpus pages, measured on Windows). O-CP3: all five met it.
/// `.doc` counts Word 97 and later and `.xls` Excel 97 and later; older and password-protected files show their honest
/// state instead (the release notes say so).
pub const OFFICE_SHIPS: OfficeShips = OfficeShips {
    docx: true,
    doc: true,
    xlsx: true,
    xls: true,
    pptx: true,
};

/// One switch per Office type the viewer may show ([`OFFICE_SHIPS`]). `.ppt` has none: it is never shown here (2482 Q5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfficeShips {
    pub docx: bool,
    pub doc: bool,
    pub xlsx: bool,
    pub xls: bool,
    pub pptx: bool,
}

/// An Office file's type, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OfficeKind {
    Docx,
    Doc,
    Xlsx,
    Xls,
    Pptx,
    Ppt,
}

/// Where an Office file goes from a list's Open, while the viewer is on ([`OFFICE_VIEWER_ON`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficeRoute {
    /// Beebeeb's own viewer shows it.
    Shows(OfficeKind),
    /// DW-1556: no preview for this type here (not shipped yet, or `.ppt`). Nothing is fetched.
    NoPreview(OfficeKind),
}

/// The Office type of a file, by its name's extension. `None`: not an Office file, or one of the types O-CP3 did not
/// measure (`.docm .dotx .dotm .dot .xlsm .xltx .pptm .ppsx`): those stay their own app's, as before (a measuring row
/// lists them).
pub fn office_of(name: &str) -> Option<OfficeKind> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "docx" => OfficeKind::Docx,
        "doc" => OfficeKind::Doc,
        "xlsx" => OfficeKind::Xlsx,
        "xls" => OfficeKind::Xls,
        "pptx" => OfficeKind::Pptx,
        "ppt" | "pps" => OfficeKind::Ppt,
        _ => return None,
    })
}

/// Where an Office file goes, by the switches as they ship. `None`: not an Office file, or the viewer is off (its own
/// app, as before).
pub fn office_route(name: &str) -> Option<OfficeRoute> {
    office_route_with(name, OFFICE_VIEWER_ON, OFFICE_SHIPS)
}

/// [`office_route`] under the switches `on` and `ships`.
pub fn office_route_with(name: &str, on: bool, ships: OfficeShips) -> Option<OfficeRoute> {
    if !on {
        return None;
    }
    let kind = office_of(name)?;
    let shipped = match kind {
        OfficeKind::Docx => ships.docx,
        OfficeKind::Doc => ships.doc,
        OfficeKind::Xlsx => ships.xlsx,
        OfficeKind::Xls => ships.xls,
        OfficeKind::Pptx => ships.pptx,
        OfficeKind::Ppt => false,
    };
    Some(if shipped {
        OfficeRoute::Shows(kind)
    } else {
        OfficeRoute::NoPreview(kind)
    })
}

/// A picture of an open Office file. Its own type: `Picture` has exhaustive matches outside the Office viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OfficePicture {
    /// A whole page or slide, from 0.
    Page { page: u32, zoom_pct: u16, dpi: u16 },
    /// A [`TILE_PX`] square of that page at that scale.
    Tile {
        page: u32,
        zoom_pct: u16,
        dpi: u16,
        col: u32,
        row: u32,
    },
    /// A filmstrip slide, [`THUMB_W_PT`] wide.
    Thumb { slide: u32, dpi: u16 },
    /// A sheet's chart or picture.
    Anchor {
        sheet: u32,
        anchor: u32,
        zoom_pct: u16,
        dpi: u16,
    },
}

/// A tile's side, in pixels.
pub const TILE_PX: u32 = 512;
/// A filmstrip slide's width, in points.
pub const THUMB_W_PT: f32 = 140.0;

/// Tiles instead of whole pages when zoom × dpi / 96 ≥ 1.5 (O-CP2 L2: a whole page at 150 % missed 16 ms under load;
/// tiles met it).
pub fn uses_tiles(zoom_pct: u16, dpi: u16) -> bool {
    // In whole numbers: zoom_pct / 100 × dpi / 96 ≥ 1.5 ⇔ zoom_pct × dpi ≥ 14 400.
    u32::from(zoom_pct) * u32::from(dpi) >= 14_400
}
