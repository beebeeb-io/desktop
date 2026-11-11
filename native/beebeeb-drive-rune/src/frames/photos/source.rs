//! `PhotosSource`: what the Photos frames ask of the engine, and the board's fixture.
//!
//! The frames draw only what a source hands them: days and their photos, thumbnails as decoded
//! pixels, the devices and folders Photos shows, and what one open photo is. The fixture holds
//! exactly the board's sample content (DW-0601 to DW-0611, DW-0116); the engine lane writes the
//! real one (the PR body lists the engine call behind each method).

use std::sync::OnceLock;

use rune::Pixels;

/// One photo or video of the library. Stable for as long as the item exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhotoId(pub u64);

/// The shape a thumbnail is asked in: the frames never crop or scale a photo themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A grid tile, the open photo's place in the strip, a dialog's tile.
    Square,
    /// A neighbour in the strip (36 x 52).
    Strip,
    /// The stored preview, at the photo's own proportions.
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Photo,
    /// A video and its length in seconds.
    Video(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Photo {
    pub id: PhotoId,
    pub kind: Kind,
    /// Still uploading from its device (the arrow on the tile).
    pub uploading: bool,
    pub bytes: u64,
}

/// One day of the timeline: its title ("Today", "12 October · Lisbon") and how many items it has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Day {
    pub title: String,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Phone,
    Tablet,
}

/// A device whose backup Photos can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub kind: DeviceKind,
    pub photos: u64,
    /// Whether Photos shows it (the Photo sources dialog).
    pub shown: bool,
}

/// A folder from Files that Photos shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    /// "212 items, with its subfolders".
    pub detail: String,
    pub shown: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub photos: u64,
    pub videos: u64,
    pub bytes: u64,
}

/// What the devices' backups are doing: "Anna's iPhone is backing up 12 photos" and
/// "last backup 09:12".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupStatus {
    pub now: String,
    pub last: String,
}

/// One thumbnail of the strip under an open photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripItem {
    pub id: PhotoId,
    pub current: bool,
}

/// One open photo or video, as the viewer shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewing {
    pub id: PhotoId,
    pub kind: Kind,
    /// "Today 09:12".
    pub taken: String,
    /// "Anna's iPhone".
    pub device: String,
    /// "Harbour.heic"; a video shows its length instead.
    pub name: String,
    /// "4K HEIC".
    pub preview_format: String,
    pub original_bytes: u64,
    /// Its place in the library, from 1, and the library's size.
    pub position: (u64, u64),
    /// The neighbours in reach, in order, the open one among them.
    pub strip: Vec<StripItem>,
    /// Milliseconds played, for a video.
    pub played_ms: u32,
}

/// One line of the Details dialog. `mono`: a figure, drawn in the machine face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    pub label: &'static str,
    pub value: String,
    pub mono: bool,
    /// Opens something (Version history).
    pub opens: bool,
}

/// Which source a change in the Photo sources dialog is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceRef {
    Device(usize),
    Folder(usize),
}

pub trait PhotosSource {
    fn devices(&self) -> Vec<Device>;
    fn folders(&self) -> Vec<Folder>;
    /// Shows or hides a source (Photo sources, Done).
    fn set_shown(&mut self, source: SourceRef, shown: bool);
    /// The totals of what shows, or of one device (`devices()` index).
    fn totals(&self, device: Option<usize>) -> Totals;
    fn backup_status(&self) -> Option<BackupStatus>;
    /// The status line's words: where the photos come from.
    fn origin(&self) -> String;
    /// The days, newest first.
    fn days(&self, device: Option<usize>) -> Vec<Day>;
    /// Item `index` of day `day`.
    fn photo(&self, device: Option<usize>, day: usize, index: usize) -> Option<Photo>;
    /// A thumbnail as decoded pixels. `None`: not there yet (the tile shows its placeholder).
    fn thumbnail(&self, id: PhotoId, shape: Shape) -> Option<Pixels>;
    fn viewing(&self, id: PhotoId) -> Option<Viewing>;
    fn details(&self, id: PhotoId) -> Vec<Detail>;
    /// Whether the original is kept on this PC.
    fn kept_offline(&self, id: PhotoId) -> bool;
    fn keep_offline(&mut self, id: PhotoId, keep: bool);
    /// Moves items to Beebeeb's Trash; `restore` brings them back (Undo).
    fn trash(&mut self, ids: &[PhotoId]);
    fn restore(&mut self, ids: &[PhotoId]);
}

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

const MB: u64 = 1_000_000;
const GB: u64 = 1_000 * MB;

/// The board's six sample photos (`.mk-ph1` to `.mk-ph6`), exported from the board by
/// `scripts/export-photos.mjs` in the three shapes the board draws them.
static SAMPLE_FILES: [[&[u8]; 3]; 6] = [
    [
        include_bytes!("assets/photo-1-square.png"),
        include_bytes!("assets/photo-1-strip.png"),
        include_bytes!("assets/photo-1-preview.png"),
    ],
    [
        include_bytes!("assets/photo-2-square.png"),
        include_bytes!("assets/photo-2-strip.png"),
        include_bytes!("assets/photo-2-preview.png"),
    ],
    [
        include_bytes!("assets/photo-3-square.png"),
        include_bytes!("assets/photo-3-strip.png"),
        include_bytes!("assets/photo-3-preview.png"),
    ],
    [
        include_bytes!("assets/photo-4-square.png"),
        include_bytes!("assets/photo-4-strip.png"),
        include_bytes!("assets/photo-4-preview.png"),
    ],
    [
        include_bytes!("assets/photo-5-square.png"),
        include_bytes!("assets/photo-5-strip.png"),
        include_bytes!("assets/photo-5-preview.png"),
    ],
    [
        include_bytes!("assets/photo-6-square.png"),
        include_bytes!("assets/photo-6-strip.png"),
        include_bytes!("assets/photo-6-preview.png"),
    ],
];

fn decode(file: &[u8]) -> Pixels {
    let mut reader = png::Decoder::new(std::io::Cursor::new(file))
        .read_info()
        .expect("a bundled sample photo is a PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("a bundled sample photo is small")];
    let info = reader.next_frame(&mut buf).expect("a bundled sample photo decodes");
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "a bundled sample photo is 8 bit");
    let count = (info.width * info.height) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..count * 4].to_vec(),
        png::ColorType::Rgb => buf[..count * 3]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => panic!("a bundled sample photo is RGB or RGBA, not {other:?}"),
    };
    let pixels = Pixels::new();
    pixels.set(info.width, info.height, rgba);
    pixels
}

/// The decoded samples, shared by every fixture of the process.
fn sample(number: u8, shape: Shape) -> Pixels {
    static SAMPLES: OnceLock<Vec<[Pixels; 3]>> = OnceLock::new();
    let all = SAMPLES.get_or_init(|| SAMPLE_FILES.iter().map(|files| files.map(decode)).collect());
    let column = match shape {
        Shape::Square => 0,
        Shape::Strip => 1,
        Shape::Preview => 2,
    };
    all[usize::from(number - 1) % all.len()][column].clone()
}

/// The board's names, in one place: the fixture and the tests read them from here.
pub mod board {
    pub const DAYS: [&str; 3] = ["Today", "Sunday", "12 October \u{00B7} Lisbon"];
    /// The sample photo of each tile of DW-0601, by day.
    pub const TILES: [[u8; 7]; 3] = [[1, 2, 3, 4, 5, 6, 1], [4, 5, 6, 1, 2, 3, 4], [2, 3, 4, 5, 6, 1, 2]];
    /// The strip of DW-0605 as (day, index) of DW-0601's tiles, the open photo fifth.
    pub const STRIP: [(usize, usize); 9] = [(0, 2), (0, 3), (0, 5), (0, 6), (0, 4), (0, 1), (1, 5), (1, 2), (1, 0)];
    /// Harbour.heic, the photo DW-0605 opens.
    pub const HARBOUR: (usize, usize) = (0, 4);
    /// The video DW-0611 plays.
    pub const VIDEO: (usize, usize) = (1, 4);
    /// The three photos DW-0602 has checked.
    pub const CHECKED: [(usize, usize); 3] = [(0, 0), (0, 3), (1, 2)];
    pub const LIBRARY: u64 = 12_430;
}

fn id_of(day: usize, index: usize) -> PhotoId {
    PhotoId((day * 1000 + index) as u64)
}

fn place_of(id: PhotoId) -> (usize, usize) {
    ((id.0 / 1000) as usize, (id.0 % 1000) as usize)
}

/// The board's library: three days of seven, three devices, one folder from Files.
pub struct FixturePhotos {
    empty: bool,
    devices: Vec<Device>,
    folders: Vec<Folder>,
    trashed: Vec<PhotoId>,
    kept: Vec<PhotoId>,
}

impl FixturePhotos {
    pub fn new() -> FixturePhotos {
        let device = |name: &str, kind, photos| Device {
            name: name.into(),
            kind,
            photos,
            shown: true,
        };
        FixturePhotos {
            empty: false,
            // In the order of the pills (DW-0601); the dialog lists them in its own order.
            devices: vec![
                device("Anna's iPhone", DeviceKind::Phone, 12_418),
                device("Old iPhone", DeviceKind::Phone, 8_204),
                device("iPad", DeviceKind::Tablet, 4_120),
            ],
            folders: vec![Folder {
                name: "Lisbon trip".into(),
                detail: "212 items, with its subfolders".into(),
                shown: true,
            }],
            trashed: Vec::new(),
            kept: Vec::new(),
        }
    }

    /// DW-0116: the same account with nothing backed up yet.
    pub fn empty() -> FixturePhotos {
        FixturePhotos {
            empty: true,
            ..FixturePhotos::new()
        }
    }

    /// Every drawn photo comes from the first device: the board names no other for any of them.
    fn shows(&self, device: Option<usize>) -> bool {
        !self.empty && device.is_none_or(|d| d == 0) && self.devices[0].shown
    }

    fn day_items(&self, day: usize) -> Vec<PhotoId> {
        (0..board::TILES[day].len())
            .map(|i| id_of(day, i))
            .filter(|id| !self.trashed.contains(id))
            .collect()
    }

    fn sample_number(id: PhotoId) -> u8 {
        let (day, index) = place_of(id);
        board::TILES[day % 3][index % 7]
    }
}

impl PhotosSource for FixturePhotos {
    fn devices(&self) -> Vec<Device> {
        self.devices.clone()
    }

    fn folders(&self) -> Vec<Folder> {
        self.folders.clone()
    }

    fn set_shown(&mut self, source: SourceRef, shown: bool) {
        match source {
            SourceRef::Device(i) => {
                if let Some(device) = self.devices.get_mut(i) {
                    device.shown = shown;
                }
            }
            SourceRef::Folder(i) => {
                if let Some(folder) = self.folders.get_mut(i) {
                    folder.shown = shown;
                }
            }
        }
    }

    fn totals(&self, device: Option<usize>) -> Totals {
        if self.empty {
            return Totals::default();
        }
        match device {
            // The board's figures for the whole library (DW-0601).
            None => Totals {
                photos: board::LIBRARY,
                videos: 842,
                bytes: 216 * GB,
            },
            // For one device the board gives its photo count only (DW-0604).
            Some(i) => Totals {
                photos: self.devices.get(i).map_or(0, |d| d.photos),
                videos: 0,
                bytes: 0,
            },
        }
    }

    fn backup_status(&self) -> Option<BackupStatus> {
        (!self.empty).then(|| BackupStatus {
            now: "Anna's iPhone is backing up 12 photos".into(),
            last: "last backup 09:12".into(),
        })
    }

    fn origin(&self) -> String {
        "Photos from Anna's iPhone, Old iPhone and iPad \u{00B7} none from this PC".into()
    }

    fn days(&self, device: Option<usize>) -> Vec<Day> {
        if !self.shows(device) {
            return Vec::new();
        }
        (0..board::DAYS.len())
            .map(|day| Day {
                title: board::DAYS[day].into(),
                len: self.day_items(day).len(),
            })
            .filter(|day| day.len > 0)
            .collect()
    }

    fn photo(&self, device: Option<usize>, day: usize, index: usize) -> Option<Photo> {
        if !self.shows(device) {
            return None;
        }
        // `day` counts the days that still have items.
        let day = (0..board::DAYS.len())
            .filter(|&d| !self.day_items(d).is_empty())
            .nth(day)?;
        let id = *self.day_items(day).get(index)?;
        let (_, tile) = place_of(id);
        let place = (day, tile);
        Some(Photo {
            id,
            kind: if place == (0, 2) || place == board::VIDEO {
                Kind::Video(24)
            } else {
                Kind::Photo
            },
            uploading: place == (0, 0) || place == (0, 1),
            // DW-0602: three photos are 42 MB. Harbour.heic is 30.9 MB (DW-0605).
            bytes: if place == board::HARBOUR { 30_900_000 } else { 14 * MB },
        })
    }

    fn thumbnail(&self, id: PhotoId, shape: Shape) -> Option<Pixels> {
        // DW-0611 draws the playing video with the third sample.
        let number = if place_of(id) == board::VIDEO && shape == Shape::Preview {
            3
        } else {
            FixturePhotos::sample_number(id)
        };
        Some(sample(number, shape))
    }

    fn viewing(&self, id: PhotoId) -> Option<Viewing> {
        let (day, index) = place_of(id);
        if day >= board::DAYS.len() || index >= 7 || self.trashed.contains(&id) {
            return None;
        }
        let place = (day, index);
        let strip = board::STRIP
            .iter()
            .map(|&(d, i)| id_of(d, i))
            .filter(|other| !self.trashed.contains(other));
        // The board draws ONE strip, around Harbour.heic, in every viewer frame; a photo that
        // is not in it is shown alone.
        let in_strip = board::STRIP.contains(&place);
        let strip: Vec<StripItem> = if in_strip || place == board::VIDEO {
            let current = if in_strip {
                id
            } else {
                id_of(board::HARBOUR.0, board::HARBOUR.1)
            };
            strip
                .map(|other| StripItem {
                    id: other,
                    current: other == current,
                })
                .collect()
        } else {
            vec![StripItem { id, current: true }]
        };
        let video = place == board::VIDEO || place == (0, 2);
        Some(Viewing {
            id,
            kind: if video { Kind::Video(24) } else { Kind::Photo },
            // The board gives a time for two items; the others show their day.
            taken: if place == board::HARBOUR {
                "Today 09:12".into()
            } else if place == board::VIDEO {
                "Sunday 14:20".into()
            } else {
                board::DAYS[day].into()
            },
            device: "Anna's iPhone".into(),
            name: if place == board::HARBOUR {
                "Harbour.heic".into()
            } else {
                String::new()
            },
            preview_format: "4K HEIC".into(),
            original_bytes: if place == board::HARBOUR { 30_900_000 } else { 14 * MB },
            // DW-0605: Harbour.heic is "Photo 1 of 12,430".
            position: (
                if place == board::HARBOUR {
                    1
                } else {
                    (day * 7 + index + 1) as u64
                },
                board::LIBRARY,
            ),
            strip,
            played_ms: if place == board::VIDEO { 9_120 } else { 0 },
        })
    }

    fn details(&self, id: PhotoId) -> Vec<Detail> {
        if place_of(id) != board::HARBOUR {
            return Vec::new();
        }
        let line = |label, value: &str, mono, opens| Detail {
            label,
            value: value.into(),
            mono,
            opens,
        };
        vec![
            line("Taken", "Today, 09:12", true, false),
            line("From", "Anna's iPhone", false, false),
            line("Camera", "Sony A7R V, 35 mm", false, false),
            line("Size", "9504 \u{00D7} 6336, 30.9 MB", true, false),
            line("Preview", "4K HEIC, 1.0 MB", false, false),
            line("Where", "Backups / Anna's iPhone", false, false),
            line("Version history", "1 version", false, true),
        ]
    }

    fn kept_offline(&self, id: PhotoId) -> bool {
        self.kept.contains(&id)
    }

    fn keep_offline(&mut self, id: PhotoId, keep: bool) {
        self.kept.retain(|other| *other != id);
        if keep {
            self.kept.push(id);
        }
    }

    fn trash(&mut self, ids: &[PhotoId]) {
        for id in ids {
            if !self.trashed.contains(id) {
                self.trashed.push(*id);
            }
        }
    }

    fn restore(&mut self, ids: &[PhotoId]) {
        self.trashed.retain(|id| !ids.contains(id));
    }
}

/// A library of `count` photos, 40 to a day: what the grid's virtual rows are measured with.
/// Not the board's content: it is never drawn in a gated frame.
pub struct LargePhotos {
    count: usize,
}

impl LargePhotos {
    pub const PER_DAY: usize = 40;

    pub fn new(count: usize) -> LargePhotos {
        LargePhotos { count }
    }
}

impl PhotosSource for LargePhotos {
    fn devices(&self) -> Vec<Device> {
        Vec::new()
    }
    fn folders(&self) -> Vec<Folder> {
        Vec::new()
    }
    fn set_shown(&mut self, _: SourceRef, _: bool) {}
    fn totals(&self, _: Option<usize>) -> Totals {
        Totals {
            photos: self.count as u64,
            videos: 0,
            bytes: 0,
        }
    }
    fn backup_status(&self) -> Option<BackupStatus> {
        None
    }
    fn origin(&self) -> String {
        String::new()
    }
    fn days(&self, _: Option<usize>) -> Vec<Day> {
        (0..self.count.div_ceil(LargePhotos::PER_DAY))
            .map(|day| Day {
                title: format!("Day {}", day + 1),
                len: (self.count - day * LargePhotos::PER_DAY).min(LargePhotos::PER_DAY),
            })
            .collect()
    }
    fn photo(&self, _: Option<usize>, day: usize, index: usize) -> Option<Photo> {
        let n = day * LargePhotos::PER_DAY + index;
        (index < LargePhotos::PER_DAY && n < self.count).then_some(Photo {
            id: PhotoId(n as u64),
            kind: Kind::Photo,
            uploading: false,
            bytes: MB,
        })
    }
    fn thumbnail(&self, id: PhotoId, shape: Shape) -> Option<Pixels> {
        Some(sample((id.0 % 6) as u8 + 1, shape))
    }
    fn viewing(&self, _: PhotoId) -> Option<Viewing> {
        None
    }
    fn details(&self, _: PhotoId) -> Vec<Detail> {
        Vec::new()
    }
    fn kept_offline(&self, _: PhotoId) -> bool {
        false
    }
    fn keep_offline(&mut self, _: PhotoId, _: bool) {}
    fn trash(&mut self, _: &[PhotoId]) {}
    fn restore(&mut self, _: &[PhotoId]) {}
}
