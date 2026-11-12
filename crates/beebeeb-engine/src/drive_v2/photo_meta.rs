//! Batch B02b: photo and video metadata read from the decrypted copy already on this PC (Photos, DW-06xx). A file
//! that is online only is never downloaded for this: its fields stay absent and the window shows the modified time.
//!
//! Only the head of the file is read (`HEAD_BYTES`): EXIF lives at the start of a JPEG, the dimensions in the image
//! header, and an MP4's `moov` box usually at the start of a file made for streaming. A field not found is `None`.

use serde::Serialize;

/// How much of a file is read for its metadata.
pub const HEAD_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoMeta {
    /// When it was taken (EXIF `DateTimeOriginal`, as written: local time, no zone), ISO 8601 without an offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taken_at: Option<String>,
    /// The camera: EXIF make and model ("Apple iPhone 15").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// A video's length in whole seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    /// The file is uploading from this PC now.
    pub uploading: bool,
    /// The device whose backup holds it: the device folder under the Backups root (LD-0113 lays Backups out one folder
    /// per device). Absent for a photo outside Backups: no command knows which device took it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

/// The device folder a state DB `path` sits in under one of the Backups `roots` (their state DB paths): the first
/// segment below the root, when the item is inside it (`Backups/Anna's iPhone/IMG_1.heic` → `Anna's iPhone`). A file
/// directly in the root, or outside every root, has none. Names compare exactly, as the state DB stores them.
pub fn backup_device(roots: &[String], path: &str) -> Option<String> {
    let path = path.trim_matches('/');
    roots.iter().find_map(|root| {
        let root = root.trim_matches('/');
        if root.is_empty() {
            return None;
        }
        let below = path.strip_prefix(root)?.strip_prefix('/')?;
        let (device, rest) = below.split_once('/')?;
        (!device.is_empty() && !rest.is_empty()).then(|| device.to_string())
    })
}

fn u16_at(b: &[u8], at: usize, le: bool) -> Option<u16> {
    let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
    Some(if le {
        u16::from_le_bytes(s)
    } else {
        u16::from_be_bytes(s)
    })
}

fn u32_at(b: &[u8], at: usize, le: bool) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(if le {
        u32::from_le_bytes(s)
    } else {
        u32::from_be_bytes(s)
    })
}

/// The TIFF block inside a JPEG's APP1 `Exif` segment.
fn jpeg_exif(b: &[u8]) -> Option<&[u8]> {
    if b.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    while at + 4 <= b.len() {
        if b[at] != 0xFF {
            return None;
        }
        let marker = b[at + 1];
        let len = u16_at(b, at + 2, false)? as usize;
        if marker == 0xE1 && b.get(at + 4..at + 10)? == b"Exif\0\0" {
            return b.get(at + 10..(at + 2 + len).min(b.len()));
        }
        if marker == 0xDA {
            return None;
        }
        at += 2 + len;
    }
    None
}

/// An ASCII tag's text, trimmed of its NUL and spaces.
fn ascii(tiff: &[u8], entry: usize, le: bool) -> Option<String> {
    let count = u32_at(tiff, entry + 4, le)? as usize;
    let bytes = if count <= 4 {
        tiff.get(entry + 8..entry + 8 + count)?
    } else {
        let off = u32_at(tiff, entry + 8, le)? as usize;
        tiff.get(off..off + count)?
    };
    let text = String::from_utf8_lossy(bytes)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// Make, model, `DateTimeOriginal` and the pixel size from a TIFF block (IFD0 and its Exif IFD).
pub fn parse_exif(tiff: &[u8], meta: &mut PhotoMeta) {
    let le = match tiff.get(0..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return,
    };
    let Some(ifd0) = u32_at(tiff, 4, le) else { return };
    let (mut make, mut model, mut exif_ifd) = (None, None, None);
    let walk = |ifd: usize, f: &mut dyn FnMut(u16, usize)| {
        let Some(n) = u16_at(tiff, ifd, le) else { return };
        for i in 0..n as usize {
            let entry = ifd + 2 + i * 12;
            if let Some(tag) = u16_at(tiff, entry, le) {
                f(tag, entry);
            }
        }
    };
    walk(ifd0 as usize, &mut |tag, entry| match tag {
        0x010F => make = ascii(tiff, entry, le),
        0x0110 => model = ascii(tiff, entry, le),
        0x8769 => exif_ifd = u32_at(tiff, entry + 8, le),
        _ => {}
    });
    meta.camera = match (make, model) {
        (Some(make), Some(model)) if model.starts_with(&make) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (make, model) => model.or(make),
    };
    if let Some(ifd) = exif_ifd {
        let (mut taken, mut w, mut h) = (None, None, None);
        walk(ifd as usize, &mut |tag, entry| {
            let num = || match u16_at(tiff, entry + 2, le) {
                Some(3) => u16_at(tiff, entry + 8, le).map(u32::from),
                _ => u32_at(tiff, entry + 8, le),
            };
            match tag {
                0x9003 => taken = ascii(tiff, entry, le),
                0xA002 => w = num(),
                0xA003 => h = num(),
                _ => {}
            }
        });
        meta.taken_at = taken.as_deref().and_then(exif_date);
        if meta.width.is_none() {
            meta.width = w.filter(|v| *v > 0);
            meta.height = h.filter(|v| *v > 0);
        }
    }
}

/// `2026:10:12 14:03:22` → `2026-10-12T14:03:22`; anything else (including `0000:00:00 00:00:00`) is `None`.
pub fn exif_date(raw: &str) -> Option<String> {
    let b = raw.as_bytes();
    if b.len() < 19 || b[4] != b':' || b[7] != b':' || b[10] != b' ' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| raw.get(r).filter(|s| s.bytes().all(|c| c.is_ascii_digit()));
    let (y, mo, d, hms) = (digits(0..4)?, digits(5..7)?, digits(8..10)?, raw.get(11..19)?);
    if y == "0000" || mo == "00" || d == "00" {
        return None;
    }
    Some(format!("{y}-{mo}-{d}T{hms}"))
}

/// An MP4 / MOV's length from `moov/mvhd` (time scale and duration), in whole seconds.
pub fn mp4_duration(b: &[u8]) -> Option<u64> {
    fn find<'a>(b: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
        let mut at = 0;
        while at + 8 <= b.len() {
            let size = u32_at(b, at, false)? as usize;
            let (head, size) = if size == 1 {
                (
                    16,
                    u64::from_be_bytes(b.get(at + 8..at + 16)?.try_into().ok()?) as usize,
                )
            } else {
                (8, size)
            };
            if size < head {
                return None;
            }
            if b.get(at + 4..at + 8)? == kind {
                return b.get(at + head..(at + size).min(b.len()));
            }
            at += size;
        }
        None
    }
    let mvhd = find(find(b, b"moov")?, b"mvhd")?;
    let (scale, duration) = if mvhd.first()? == &1 {
        (
            u32_at(mvhd, 20, false)? as u64,
            u64::from_be_bytes(mvhd.get(24..32)?.try_into().ok()?),
        )
    } else {
        (u32_at(mvhd, 12, false)? as u64, u32_at(mvhd, 16, false)? as u64)
    };
    (scale > 0).then(|| duration / scale)
}

/// What the head of a file says. `is_video` picks the MP4 reader; an image also gets its header's pixel size.
pub fn from_head(head: &[u8], is_video: bool) -> PhotoMeta {
    let mut meta = PhotoMeta::default();
    if is_video {
        meta.duration_seconds = mp4_duration(head);
        return meta;
    }
    if let Ok((w, h)) = image::ImageReader::new(std::io::Cursor::new(head))
        .with_guessed_format()
        .map_err(|_| ())
        .and_then(|r| r.into_dimensions().map_err(|_| ()))
    {
        meta.width = Some(w);
        meta.height = Some(h);
    }
    if let Some(tiff) = jpeg_exif(head) {
        parse_exif(tiff, &mut meta);
    }
    meta
}
