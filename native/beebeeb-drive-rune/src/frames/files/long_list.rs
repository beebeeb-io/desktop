//! Long content and thumbnails in the Files list (DW-0420b, DW-0421): a folder of 212 items, names
//! longer than a row, the letters that jump to a name, and the pictures of the board's fixture.
//!
//! - **Names** longer than [`NAME_MAX`] characters are shortened in the middle, so the start, the
//!   number and the extension stay ([`shortened`], the board's rule); the pointer over one shows
//!   the whole name. A name that still does not fit its column is cut in the middle too.
//! - **Letters** typed in the list jump to the first name that starts with them ([`State::type_to`]).
//! - **Order**: names are compared without their accents and without case ([`sort_key`]), so
//!   "Sé Cathedral" stands before "Sintra".
//! - **The list stays one lazy list**: `list.rs` realises only the rows in view (`virtual_rows`);
//!   nothing here builds a row that is not shown.
//! - **Pictures**: a thumbnail's pixels are the engine's. The board's six stand-in pictures come
//!   from a FIXTURE `ThumbSource` only ([`TripThumbs`]); `ThumbSource::sample`'s default says none.

use std::time::{Duration, Instant};

use super::source::BoardDrive;
use crate::State;
use crate::frames::global::grid::ThumbSource;
use crate::source::{DriveItem, DriveSource, Listing, SourceError, StorageSummary};
use crate::theme::tokens::{Theme, type_};
use crate::ui::{global_icons, grid_tiles, kit};
use rune::{Color, El, Tx, el};

/// Names up to this many characters are written whole (the board's `midCut`).
pub const NAME_MAX: usize = 58;

/// A name longer than [`NAME_MAX`] characters, shortened in the middle: its first 32 characters,
/// an ellipsis, its last 23. `None`: it is written whole.
pub fn shortened(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= NAME_MAX {
        return None;
    }
    // ceil(58 x 0.55) and floor(58 x 0.4)
    let (head, tail) = (32, 23);
    let mut out: String = chars[..head].iter().collect();
    out.push('\u{2026}');
    out.extend(&chars[chars.len() - tail..]);
    Some(out)
}

/// A row's name (`.mk-row`'s name): shortened in the middle when it is long, the whole name under
/// the pointer.
pub fn name_label(k: &str, name: &str, t: &Theme) -> El {
    let short = shortened(name);
    let label = kit::label(
        format!("{k}-name"),
        short.clone().unwrap_or_else(|| name.to_string()),
        type_::LIST_ITEM,
        t.text,
    )
    .truncate_middle()
    .min_w(0.0);
    match short {
        Some(_) => label.help(name),
        None => label.help_when_truncated(name),
    }
}

/// A file's size in the list: one decimal under 10 ("4.0 MB"), and no ".0" from 10 up ("112 MB").
pub fn size_words(bytes: i64) -> String {
    let words = crate::source::format_size(bytes);
    match words.split_once(".0 ") {
        Some((whole, unit)) if whole.len() >= 2 => format!("{whole} {unit}"),
        _ => words,
    }
}

/// What names are ordered by: lower case, accents left out.
pub fn sort_key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

/// The letters typed in the list, and when the last one came.
#[derive(Default)]
pub struct Typed {
    letters: String,
    at: Option<Instant>,
}

/// Letters typed further apart than this start a new word.
const TYPING_PAUSE: Duration = Duration::from_millis(1000);

impl State {
    /// Letters typed in the list: the first name that starts with them is selected and shown.
    /// `false`: nothing was typed that a name could start with.
    pub fn type_to(&mut self, typed: &str, tx: &mut Tx) -> bool {
        if typed.trim().is_empty() && self.actions.typed.letters.is_empty() {
            return false;
        }
        let now = Instant::now();
        let typing = &mut self.actions.typed;
        if typing.at.is_none_or(|at| now.duration_since(at) > TYPING_PAUSE) {
            typing.letters.clear();
        }
        typing.at = Some(now);
        typing.letters.push_str(&sort_key(typed));
        let wanted = typing.letters.clone();
        let Some(row) = self
            .listing
            .items
            .iter()
            .position(|item| sort_key(&item.name).starts_with(&wanted))
        else {
            return true;
        };
        self.select_only(row);
        tx.scroll_into_view(format!("row-{}", row.wrapping_sub(self.actions.board_first_row)));
        true
    }
}

/// A row's tile when the source has one of the board's stand-in pictures for it: the picture,
/// and a video's mark. : the list's own tile.
pub fn picture_tile(s: &State, k: &str, item: &DriveItem, t: &Theme) -> Option<El> {
    if !item.has_thumbnail {
        return None;
    }
    let thumbs = s.global.grid.thumbs.as_ref();
    let sample = thumbs.sample(&item.id)?;
    let _ = t;
    let mut tile = grid_tiles::sample(
        el().key(format!("{k}-tile"))
            .size(22.0, 22.0)
            .radius(6.0)
            .clip_rounded(6.0)
            .center(),
        sample,
    );
    if thumbs.video_length(&item.id).is_some() {
        tile = tile.child(kit::glyph(format!("{k}-icon"), &global_icons::PLAY, 10.0, Color::WHITE));
    }
    Some(tile)
}

// ---- the board's fixture ----

/// Rows 139 to 170 of Lisbon trip's 212, as the board draws them (DW-0420b): name, which of the
/// six stand-in pictures (0: a document), changed, size in KB, on this PC, a video's length.
const BOARD_ROWS: [(&str, u8, &str, i64, bool, &str); 32] = [
    (
        "Miradouro da Graça 0139.heic",
        2,
        "2026-10-11T10:00:00Z",
        3686,
        false,
        "",
    ),
    (
        "Miradouro de Santa Luzia, the tiled terrace above Alfama at golden hour 0140.heic",
        5,
        "2026-10-11T10:00:00Z",
        4301,
        false,
        "",
    ),
    (
        "Miradouro de Santa Luzia 0141.heic",
        1,
        "2026-10-11T10:00:00Z",
        3994,
        false,
        "",
    ),
    (
        "Museu Nacional do Azulejo — opening hours, tickets and the floor plan we used.pdf",
        0,
        "2026-10-11T10:00:00Z",
        1126,
        true,
        "",
    ),
    (
        "Padrão dos Descobrimentos 0143.heic",
        6,
        "2026-10-11T10:00:00Z",
        3379,
        false,
        "",
    ),
    ("Pastéis de Belém 0144.heic", 4, "2026-10-11T10:00:00Z", 2970, false, ""),
    (
        "Ponte 25 de Abril from the ferry 0145.mov",
        3,
        "2026-10-12T10:00:00Z",
        114688,
        false,
        "1:12",
    ),
    (
        "Praça do Comércio 0146.heic",
        2,
        "2026-10-12T10:00:00Z",
        3789,
        false,
        "",
    ),
    ("Rossio station 0147.heic", 1, "2026-10-12T10:00:00Z", 3174, false, ""),
    ("Rua Augusta arch 0148.heic", 5, "2026-10-12T10:00:00Z", 4096, false, ""),
    ("Sé Cathedral 0149.heic", 6, "2026-10-12T10:00:00Z", 3584, false, ""),
    ("Sé Cathedral 0150.heic", 2, "2026-10-12T10:00:00Z", 3482, false, ""),
    (
        "Sintra, Pena Palace 0151.heic",
        5,
        "2026-10-12T10:00:00Z",
        4198,
        false,
        "",
    ),
    (
        "Sintra, Pena Palace 0152.heic",
        1,
        "2026-10-12T10:00:00Z",
        3891,
        false,
        "",
    ),
    (
        "Sintra, the train back 0153.mov",
        3,
        "2026-10-12T10:00:00Z",
        98304,
        false,
        "0:48",
    ),
    (
        "Sunset at Cais do Sodré 0154.heic",
        6,
        "2026-10-12T10:00:00Z",
        3277,
        false,
        "",
    ),
    (
        "Sunset at Cais do Sodré 0155.heic",
        4,
        "2026-10-12T10:00:00Z",
        3686,
        false,
        "",
    ),
    (
        "Tagus from the ferry 0156.heic",
        2,
        "2026-10-12T10:00:00Z",
        4506,
        false,
        "",
    ),
    ("Tasca do Chico 0157.heic", 1, "2026-10-12T10:00:00Z", 2867, false, ""),
    ("Tickets and receipts.pdf", 0, "2026-10-12T10:00:00Z", 640, true, ""),
    ("Time Out Market 0158.heic", 5, "2026-10-13T10:00:00Z", 3379, false, ""),
    ("Time Out Market 0159.heic", 6, "2026-10-13T10:00:00Z", 3994, false, ""),
    ("Torre de Belém 0160.heic", 4, "2026-10-13T10:00:00Z", 4096, false, ""),
    ("Torre de Belém 0161.heic", 2, "2026-10-13T10:00:00Z", 3584, false, ""),
    ("Tram 28 at Graça 0162.heic", 3, "2026-10-13T10:00:00Z", 3789, false, ""),
    (
        "Tram 28 at Graça 0163.mov",
        1,
        "2026-10-13T10:00:00Z",
        59392,
        false,
        "0:31",
    ),
    ("Tram 28 climbing 0164.heic", 5, "2026-10-13T10:00:00Z", 3174, false, ""),
    ("Tram 28 climbing 0165.heic", 6, "2026-10-13T10:00:00Z", 3072, false, ""),
    (
        "Viewpoint at dusk 0166.heic",
        4,
        "2026-10-13T10:00:00Z",
        4403,
        false,
        "",
    ),
    (
        "Viewpoint at dusk 0167.heic",
        2,
        "2026-10-13T10:00:00Z",
        3994,
        false,
        "",
    ),
    (
        "Wall tiles in Alfama 0168.heic",
        1,
        "2026-10-13T10:00:00Z",
        2765,
        false,
        "",
    ),
    (
        "Wall tiles in Alfama 0169.heic",
        5,
        "2026-10-13T10:00:00Z",
        3277,
        false,
        "",
    ),
];

/// The row DW-0420b starts at: 138 items stand before it.
pub const BOARD_FIRST_ROW: usize = 138;
const TRIP: &str = "f-lisbon";

fn id_of(name: &str) -> String {
    let digits: String = name.chars().filter(char::is_ascii_digit).collect();
    match digits.len() {
        4 => format!("l-{digits}"),
        _ => format!(
            "l-{}",
            sort_key(name).replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        ),
    }
}

fn trip_file(name: &str, picture: u8, modified: &str, kb: i64, on_pc: bool) -> DriveItem {
    let video = name.ends_with(".mov");
    DriveItem {
        id: id_of(name),
        parent_id: Some(TRIP.into()),
        name: name.into(),
        is_folder: false,
        size: Some(kb * 1024),
        child_count: None,
        modified_at: modified.into(),
        mime: Some(
            match (picture, video) {
                (0, _) => "application/pdf",
                (_, true) => "video/quicktime",
                _ => "image/heic",
            }
            .into(),
        ),
        has_thumbnail: picture > 0,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: Some(if on_pc { "on_this_pc" } else { "online_only" }.into()),
        is_protected: false,
    }
}

/// The 180 items around the board's 32: the same kind of names, numbered, so that 138 stand
/// before "Miradouro" and 42 after "Wall tiles" when sorted by name. Ten of them are on this PC
/// (the board's status line: "12 on this PC").
fn around() -> Vec<DriveItem> {
    const BEFORE: [&str; 6] = [
        "Alfama streets",
        "Bairro Alto at night",
        "Castelo de S\u{e3}o Jorge",
        "Elevador de Santa Justa",
        "Feira da Ladra",
        "LX Factory",
    ];
    const AFTER: [&str; 2] = ["Xabregas riverside", "Zambujeira day trip"];
    let numbered = |place: &str, n: usize| {
        let on_pc = n % 14 == 5 && n <= BOARD_FIRST_ROW;
        trip_file(
            &format!("{place} {n:04}.heic"),
            (n % 6) as u8 + 1,
            "2026-10-09T10:00:00Z",
            3_200 + (n as i64 % 9) * 130,
            on_pc,
        )
    };
    let before = (1..=BOARD_FIRST_ROW).map(|n| numbered(BEFORE[(n - 1) * BEFORE.len() / BOARD_FIRST_ROW], n));
    let after = (170..212).map(|n| numbered(AFTER[(n - 170) * AFTER.len() / 42], n));
    before.chain(after).collect()
}

/// The board's drive with Lisbon trip as DW-0420b has it: 212 items.
pub struct LongTrip(BoardDrive);

impl LongTrip {
    pub fn new() -> Self {
        LongTrip(BoardDrive::new())
    }
}

impl Default for LongTrip {
    fn default() -> Self {
        Self::new()
    }
}

impl DriveSource for LongTrip {
    fn list(&self, folder_id: Option<&str>) -> Result<Listing, SourceError> {
        let mut listing = self.0.list(folder_id)?;
        if folder_id == Some(TRIP) {
            listing.items = around();
            listing.items.extend(
                BOARD_ROWS
                    .iter()
                    .map(|&(name, picture, modified, kb, on_pc, _)| trip_file(name, picture, modified, kb, on_pc)),
            );
        }
        Ok(listing)
    }

    fn storage(&self) -> Option<StorageSummary> {
        self.0.storage()
    }
}

/// The board's pictures of Lisbon trip: every thumbnail has arrived, each is one of the board's
/// six stand-ins, and a video says how long it is. A fixture: an engine hands over pixels.
pub struct TripThumbs;

impl ThumbSource for TripThumbs {
    fn arrived(&self, _: &str) -> bool {
        true
    }
    fn video_length(&self, id: &str) -> Option<String> {
        if id == "tram28" {
            return Some("0:41".into());
        }
        BOARD_ROWS
            .iter()
            .find(|row| !row.5.is_empty() && id_of(row.0) == id)
            .map(|row| row.5.to_string())
    }
    fn sample(&self, id: &str) -> Option<u8> {
        match id {
            // DW-0421's four.
            "alfama" => Some(1),
            "belem" => Some(2),
            "tram28" => Some(3),
            "sunset" => Some(4),
            _ => BOARD_ROWS
                .iter()
                .find(|row| row.1 > 0 && id_of(row.0) == id)
                .map(|row| row.1)
                .or_else(|| {
                    // The items around the board's rows: by their number.
                    let n: usize = id.strip_prefix("l-")?.parse().ok()?;
                    Some((n % 6) as u8 + 1)
                }),
        }
    }
}

/// The board frames of this file, by frame id.
pub const SCENES: &[crate::ui::gate::Scene<State>] = &[
    // DW-0421: Lisbon trip as a grid, its thumbnails arrived, Alfama.heic selected.
    ("DW-0421", |s| {
        use crate::frames::global::samples::TripByDate;
        s.show_detail_pane = false;
        s.source = Box::new(TripByDate::new());
        (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
        s.actions = super::Actions::new(Box::new(TripByDate::files()));
        s.open_folder(Some(TripByDate::FOLDER.to_string()));
        s.global = crate::frames::global::Ui::default();
        s.global.grid.thumbs = Box::new(TripThumbs);
        crate::frames::global::grid::set(s, true);
        if let Some(row) = s.listing.items.iter().position(|i| i.id == "alfama") {
            s.select_only(row);
        }
    }),
    // DW-0420b: Lisbon trip's 212 items from row 139 on, the long name selected. The gate draws
    // one frame and cannot scroll, so the frame is drawn from its first row (`board_first_row`).
    ("DW-0420b", |s| {
        s.show_detail_pane = false;
        s.source = Box::new(LongTrip::new());
        (s.listing, s.list_error) = crate::load(s.source.as_ref(), None);
        s.actions = super::Actions::new(Box::new(super::source::FilesFixture::default()));
        s.open_folder(Some(TRIP.to_string()));
        s.global = crate::frames::global::Ui::default();
        s.global.grid.thumbs = Box::new(TripThumbs);
        if let Some(row) = s.listing.items.iter().position(|i| i.id == "l-0140") {
            s.select_only(row);
        }
        s.actions.board_first_row = BOARD_FIRST_ROW;
    }),
];
