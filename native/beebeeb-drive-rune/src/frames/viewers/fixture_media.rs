//! The board's samples of DW-1501 to DW-1506, DW-1514 and DW-1515, for `FixtureViewerSource`: the
//! files, and the pictures a real source renders from the decrypted file (the photo and its strip
//! are the board's `.mk-ph` samples as the Photos group exports them; the two PDF pages are cut
//! from the board by `scripts/export-viewer-pages.py`). Only the board's sample window has this.

use std::sync::OnceLock;

use rune::Pixels;

use super::kinds::clock;
use super::source::{
    ArchiveDoc, AudioDoc, Body, Column, Details, Entry, EntryKind, Match, OpenFile, PdfDoc, PhotoDoc, Picture,
    StripItem, TableDoc, Track, board,
};

/// The strip of DW-1501: which of the board's sample photos each place shows; the open one is place 4.
const STRIP: [u8; 7] = [3, 4, 6, 1, 4, 2, 3];
const STRIP_OPEN: usize = 4;

/// The recordings of "Studio demos", in the folder's order: id, title, seconds.
const TRACKS: [(&str, &str, u32); 5] = [
    (board::AUDIO, "Harbour sessions, take 3", 242),
    ("harbour-sessions-take-4.m4a", "Harbour sessions, take 4", 227),
    ("room-tone-studio-b.m4a", "Room tone, Studio B", 75),
    ("client-call-30-oct.m4a", "Client call, 30 Oct", 1720),
    ("voice-over-draft.m4a", "Voice-over draft", 138),
];

/// Rows of the Files window's own fixture that this group has a viewer for: the row keeps its
/// name; what it shows is the board's sample of its type.
const FILES_ROWS: [(&str, &str, &str); 4] = [
    ("contract", "Contract draft.pdf", board::PDF),
    ("brand", "Northwind brand guide.pdf", board::PDF),
    ("rent", "Studio rent 2026.pdf", board::PDF),
    ("harbour", "Harbour.heic", board::RAW),
];

fn strip_id(place: usize) -> String {
    if place == STRIP_OPEN {
        board::RAW.to_string()
    } else {
        format!("dsc0417{}.arw", 8 + place)
    }
}

fn strip_place(id: &str) -> Option<usize> {
    (0..STRIP.len()).find(|place| strip_id(*place) == id)
}

/// The sample a Files row stands on, if `id` is one.
fn base(id: &str) -> &str {
    FILES_ROWS.iter().find(|row| row.0 == id).map_or(id, |row| row.2)
}

fn fact(label: &'static str, value: &str, mono: bool) -> (&'static str, String, bool) {
    (label, value.to_string(), mono)
}

fn open_file(id: &str, path: &[&str], name: &str, facts: &str, body: Body) -> OpenFile {
    OpenFile {
        id: id.to_string(),
        path: path.iter().map(|p| p.to_string()).collect(),
        name: name.to_string(),
        facts: facts.to_string(),
        version: 1,
        body,
        details: Details::default(),
        newer: None,
    }
}

fn photo(id: &str, place: usize) -> OpenFile {
    let strip = (0..STRIP.len())
        .map(|i| StripItem {
            id: strip_id(i),
            name: format!("DSC0417{}.ARW", 8 + i),
            current: i == place,
        })
        .collect();
    let body = Body::Photo(PhotoDoc {
        pill: Some("Camera preview \u{00B7} Sony ARW".into()),
        note: "RAW \u{00B7} the camera's own preview, not a developed RAW".into(),
        strip,
    });
    let name = format!("DSC0417{}.ARW", 8 + place);
    if place != STRIP_OPEN {
        // The board draws one photo of the folder; of the others the fixture knows the name only.
        return open_file(id, &["Files", "Shoot selects"], &name, "RAW", body);
    }
    let mut file = open_file(
        id,
        &["Files", "Shoot selects"],
        &name,
        "Sunday 14:20 \u{00B7} RAW \u{00B7} 61.4 MB",
        body,
    );
    file.details = Details {
        kind: "Sony ARW (RAW)".into(),
        size: "61.4 MB".into(),
        facts: vec![
            fact("Taken", "Sunday, 14:20", true),
            fact("From", "This PC", false),
            fact("Camera", "Sony ILCE-7RM5", false),
            fact("Lens", "FE 35mm F1.4 GM", false),
            fact("Exposure", "ISO 200 \u{00B7} 1/250 s \u{00B7} f/2.8", true),
            fact("Focal length", "35 mm", true),
            fact("Size", "9504 \u{00D7} 6336", true),
            fact("Preview", "Camera's own, 1616 \u{00D7} 1080", true),
            fact("Where", "Files \u{203A} Shoot selects", false),
            fact("Version history", "1 version", true),
            fact("Encryption", "AES-256-GCM", true),
            fact("Key", "256-bit, this file only", false),
        ],
        offline: false,
    };
    file
}

fn audio(id: &str, index: usize) -> OpenFile {
    let (_, title, length) = TRACKS[index];
    let first = index == 0;
    let queue = TRACKS
        .iter()
        .enumerate()
        .map(|(i, (id, title, length))| Track {
            id: id.to_string(),
            title: title.to_string(),
            length: *length,
            current: i == index,
        })
        .collect();
    let mut facts = format!("Audio \u{00B7} {}", clock(length));
    if first {
        facts.push_str(" \u{00B7} 7.8 MB");
    }
    open_file(
        id,
        &["Files", "Documents", "Studio demos"],
        &format!("{title}.m4a"),
        &facts,
        Body::Audio(AudioDoc {
            title: title.to_string(),
            // The board shows the tags of the first recording only.
            artist: if first {
                "Northwind Studio".into()
            } else {
                String::new()
            },
            length,
            at: if first { 72 } else { 0 },
            playing: true,
            folder: "Studio demos".into(),
            queue,
        }),
    )
}

fn table() -> Body {
    let column = |name: &str, numeric: bool| Column {
        name: name.to_string(),
        numeric,
    };
    let rows: [[&str; 6]; 8] = [
        ["2026-10-02", "Harbourlight", "Autumn shoot", "8.0", "95.00", "760.00"],
        ["2026-10-03", "Harbourlight", "Autumn shoot", "6.5", "95.00", "617.50"],
        ["2026-10-09", "Northwind", "Brand guide", "4.0", "110.00", "440.00"],
        ["2026-10-12", "Lisbon tourism", "City walk", "7.0", "95.00", "665.00"],
        ["2026-10-15", "Northwind", "Q4 plan", "3.5", "110.00", "385.00"],
        ["2026-10-21", "Harbourlight", "Selects", "5.0", "95.00", "475.00"],
        ["2026-10-24", "Studio", "Admin", "2.0", "0.00", "0.00"],
        ["2026-10-28", "Northwind", "Proposal", "6.0", "110.00", "660.00"],
    ];
    Body::Table(TableDoc {
        separator: "Comma-separated".into(),
        columns: vec![
            column("Date", false),
            column("Client", false),
            column("Project", false),
            column("Hours", true),
            column("Rate", true),
            column("Amount", true),
        ],
        rows: rows
            .iter()
            .map(|row| row.iter().map(|v| v.to_string()).collect())
            .collect(),
        total_rows: 48,
    })
}

fn archive() -> Body {
    let entry = |path: &str, kind: EntryKind, contains: &str, size: &str| Entry {
        path: path.to_string(),
        kind,
        contains: contains.to_string(),
        size: size.to_string(),
    };
    Body::Archive(ArchiveDoc {
        note: "214 files \u{00B7} 3.8 GB unpacked \u{00B7} files inside can't be opened here.".into(),
        entries: vec![
            entry("Shoot selects/", EntryKind::Folder, "212 files", ""),
            entry("Shoot selects/Day 1/", EntryKind::Folder, "96 files", ""),
            entry("contact-sheet.pdf", EntryKind::Text, "", "4.1 MB"),
            entry("selects-list.csv", EntryKind::Sheet, "", "18 KB"),
            entry("README.txt", EntryKind::Text, "", "1 KB"),
        ],
    })
}

/// The board's sample file with this id.
pub fn file(id: &str) -> Option<OpenFile> {
    if let Some(row) = FILES_ROWS.iter().find(|row| row.0 == id) {
        let mut file = file(row.2)?;
        file.id = id.to_string();
        file.path = vec!["Files".into()];
        file.name = row.1.to_string();
        if let Body::Photo(doc) = &mut file.body {
            // A photo that is not a RAW: no pill, and of its folder the fixture knows only itself.
            *doc = PhotoDoc {
                pill: None,
                note: "Space for Preview".into(),
                strip: Vec::new(),
            };
            file.facts = "HEIC".into();
            file.details = Details::default();
        }
        return Some(file);
    }
    if let Some(place) = strip_place(id) {
        return Some(photo(id, place));
    }
    if let Some(index) = TRACKS.iter().position(|track| track.0 == id) {
        return Some(audio(id, index));
    }
    Some(match id {
        board::PDF => open_file(
            id,
            &["Files", "Documents"],
            "Studio plan.pdf",
            "PDF \u{00B7} 2.3 MB",
            Body::Pdf(PdfDoc {
                pages: 6,
                page: 2,
                size: "2.3 MB".into(),
            }),
        ),
        board::PDF_FIND => open_file(
            id,
            &["Files", "Documents"],
            "Service agreement.pdf",
            "PDF \u{00B7} 210 KB",
            Body::Pdf(PdfDoc {
                pages: 4,
                page: 1,
                size: "210 KB".into(),
            }),
        ),
        board::CSV => open_file(
            id,
            &["Files", "Documents"],
            "Rates 2026.csv",
            "Comma-separated \u{00B7} 6 columns \u{00B7} 48 rows",
            table(),
        ),
        board::ZIP => open_file(
            id,
            &["Files", "Shoot selects"],
            "Shoot selects export.zip",
            "ZIP archive \u{00B7} 3.2 GB",
            archive(),
        ),
        _ => return None,
    })
}

/// Where the board marks "terrace" in Service agreement.pdf: twice on page 1 (DW-1506), once on page 3.
pub fn find(id: &str, words: &str) -> Vec<Match> {
    if base(id) != board::PDF_FIND || !words.trim().eq_ignore_ascii_case("terrace") {
        return Vec::new();
    }
    let at = |page: u32, x: f32, y: f32| Match {
        page,
        x,
        y,
        w: 44.31,
        h: 15.0,
    };
    vec![at(1, 298.17, 161.5), at(1, 122.91, 266.5), at(3, 122.91, 161.5)]
}

fn decode(file: &[u8]) -> Pixels {
    let mut reader = png::Decoder::new(std::io::Cursor::new(file))
        .read_info()
        .expect("a bundled sample picture is a PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("a bundled sample picture is small")];
    let info = reader.next_frame(&mut buf).expect("a bundled sample picture decodes");
    let count = (info.width * info.height) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..count * 4].to_vec(),
        png::ColorType::Rgb => buf[..count * 3]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => panic!("a bundled sample picture is RGB or RGBA, not {other:?}"),
    };
    let pixels = Pixels::new();
    pixels.set(info.width, info.height, rgba);
    pixels
}

macro_rules! sample {
    ($n:literal) => {
        [
            include_bytes!(concat!("../photos/assets/photo-", $n, "-preview.png")).as_slice(),
            include_bytes!(concat!("../photos/assets/photo-", $n, "-strip.png")).as_slice(),
            include_bytes!(concat!("../photos/assets/photo-", $n, "-square.png")).as_slice(),
        ]
    };
}

/// The board's sample photos 1 to 6, each as preview, strip and square.
static PHOTOS: [[&[u8]; 3]; 6] = [
    sample!("1"),
    sample!("2"),
    sample!("3"),
    sample!("4"),
    sample!("5"),
    sample!("6"),
];
static PAGES: [&[u8]; 2] = [
    include_bytes!("assets/page-studio-plan-2.png"),
    include_bytes!("assets/page-service-agreement-1.png"),
];

fn sample(number: u8, shape: usize) -> Pixels {
    static DECODED: OnceLock<Vec<[Pixels; 3]>> = OnceLock::new();
    let all = DECODED.get_or_init(|| PHOTOS.iter().map(|files| files.map(decode)).collect());
    all[usize::from(number - 1) % all.len()][shape].clone()
}

fn page(index: usize) -> Pixels {
    static DECODED: OnceLock<Vec<Pixels>> = OnceLock::new();
    DECODED.get_or_init(|| PAGES.iter().map(|file| decode(file)).collect())[index].clone()
}

/// The board's picture for a sample file.
pub fn picture(id: &str, which: Picture) -> Option<Pixels> {
    let id = base(id);
    match which {
        Picture::Photo => strip_place(id).map(|place| sample(STRIP[place], 0)),
        Picture::Strip(i) => {
            let open = strip_place(id)?;
            let number = *STRIP.get(i)?;
            Some(sample(number, if i == open { 2 } else { 1 }))
        }
        // The board draws artwork for the first recording; the others have none in their tags.
        Picture::Artwork => (id == board::AUDIO).then(|| sample(6, 2)),
        Picture::Page(2) if id == board::PDF => Some(page(0)),
        Picture::Page(1) if id == board::PDF_FIND => Some(page(1)),
        Picture::Page(_) => None,
    }
}
