//! A picture's own limits, asked of its header BEFORE anything decodes it.
//!
//! A picture file of a few kilobytes can declare a canvas of hundreds of megabytes once decoded, and an animation
//! multiplies that by its frames. The toolkit's decoders have limits of their own, but they are the toolkit's
//! (larger, and not the same on every path). So the viewer reads what the file declares, from the same header
//! fields the decoder sizes its buffers from, and refuses before a decoder is started:
//! - a file that is not a PNG, a JPEG, a GIF or a WebP BY ITS CONTENT (whatever its name says) is not handed to
//!   a decoder at all: the toolkit picks a decoder by content, and only these four are the viewer's;
//! - a side over [`SIDE_MAX`], more pixels than [`PIXELS_MAX`], or (an animation) more decoded bytes over all
//!   frames than [`DECODED_BYTES_MAX`] is refused with the figures, for the view to say.
//!
//! The decode itself cannot be stopped part-way (the toolkit has no call for it). It is bounded by the limits
//! here; the worker asks whether it was cancelled before it starts one and drops the pixels after one that was.
//!
//! Must not: allocate from a declared size; read past the slice; log a byte.

/// The longest side decoded (the toolkit's own limit as well).
pub const SIDE_MAX: u32 = 8192;
/// The most pixels of one frame: a 48-megapixel photo fits; 200 MB as RGBA.
pub const PIXELS_MAX: u64 = 50_000_000;
/// The most decoded bytes over every frame of an animation (RGBA, 4 bytes a pixel).
pub const DECODED_BYTES_MAX: u64 = 256 * 1024 * 1024;

/// What a picture's header declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Declared {
    pub width: u32,
    pub height: u32,
    /// 1 for a still.
    pub frames: u32,
}

/// Why a file is not decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not a PNG, JPEG, GIF or WebP by its content, or its header cannot be read.
    NotAPicture,
    /// It declares more than the viewer decodes.
    TooBig(Declared),
}

/// What `bytes` declares, held to the limits. `Ok`: it may be decoded.
pub fn check(bytes: &[u8]) -> Result<Declared, Refused> {
    let declared = declared(bytes).ok_or(Refused::NotAPicture)?;
    if declared.width == 0 || declared.height == 0 || declared.frames == 0 {
        return Err(Refused::NotAPicture);
    }
    let pixels = u64::from(declared.width) * u64::from(declared.height);
    let over = declared.width > SIDE_MAX
        || declared.height > SIDE_MAX
        || pixels > PIXELS_MAX
        || (declared.frames > 1
            && pixels.saturating_mul(4).saturating_mul(u64::from(declared.frames)) > DECODED_BYTES_MAX);
    if over {
        return Err(Refused::TooBig(declared));
    }
    Ok(declared)
}

/// What the header of `bytes` declares, by the file's own content. `None`: not one of the four, or cut short.
pub fn declared(bytes: &[u8]) -> Option<Declared> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg(bytes)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        gif(bytes)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        webp(bytes)
    } else {
        None
    }
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    )))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    )))
}

/// IHDR is the first chunk; an animation says how many frames in `acTL`, which comes before the first `IDAT`.
fn png(bytes: &[u8]) -> Option<Declared> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let (width, height) = (be32(bytes, 16)?, be32(bytes, 20)?);
    let mut frames = 1;
    let mut at = 8usize;
    // Chunks: length, type, data, CRC.
    while let (Some(length), Some(kind)) = (be32(bytes, at), bytes.get(at + 4..at + 8)) {
        match kind {
            b"IDAT" | b"IEND" => break,
            b"acTL" => frames = be32(bytes, at + 8)?.max(1),
            _ => {}
        }
        at = at.checked_add(12)?.checked_add(usize::try_from(length).ok()?)?;
    }
    Some(Declared { width, height, frames })
}

/// The first frame header (SOF) names the size; it comes before the first scan.
fn jpeg(bytes: &[u8]) -> Option<Declared> {
    let mut at = 2usize;
    loop {
        // A marker is 0xFF and a code; 0xFF bytes before it are fill.
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let mut code = *bytes.get(at + 1)?;
        while code == 0xFF {
            at += 1;
            code = *bytes.get(at + 1)?;
        }
        at += 2;
        match code {
            // Markers that stand alone.
            0x01 | 0xD0..=0xD8 => continue,
            // The end, or a scan before any frame header.
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let length = usize::try_from(be16(bytes, at)?).ok()?;
        if length < 2 {
            return None;
        }
        let is_frame_header = matches!(code, 0xC0..=0xCF) && !matches!(code, 0xC4 | 0xC8 | 0xCC);
        if is_frame_header {
            // Precision, then the height and the width.
            return Some(Declared {
                height: be16(bytes, at + 3)?,
                width: be16(bytes, at + 5)?,
                frames: 1,
            });
        }
        at = at.checked_add(length)?;
    }
}

/// The logical screen, and every image in the file: an image may be placed past the screen, so the canvas
/// counted is the larger of the two. A file cut short is what was read until then (its decoder will fail).
fn gif(bytes: &[u8]) -> Option<Declared> {
    let (mut width, mut height) = (le16(bytes, 6)?, le16(bytes, 8)?);
    let flags = *bytes.get(10)?;
    let table = |flags: u8| {
        if flags & 0x80 != 0 {
            3usize << ((flags & 7) + 1)
        } else {
            0
        }
    };
    let mut at = 13usize + table(flags);
    let mut frames = 0u32;
    // Sub-blocks: a length byte and that many bytes, ended by a zero length.
    let skip_blocks = |mut at: usize| -> Option<usize> {
        loop {
            let length = usize::from(*bytes.get(at)?);
            at += 1;
            if length == 0 {
                return Some(at);
            }
            at = at.checked_add(length)?;
        }
    };
    while let Some(&block) = bytes.get(at) {
        let next = match block {
            // An extension: its label, then sub-blocks.
            0x21 => skip_blocks(at + 2),
            // An image: where and how large, its own colour table, the code size, then sub-blocks.
            0x2C => (|| {
                let (left, top) = (le16(bytes, at + 1)?, le16(bytes, at + 3)?);
                let (w, h) = (le16(bytes, at + 5)?, le16(bytes, at + 7)?);
                width = width.max(left + w);
                height = height.max(top + h);
                frames = frames.saturating_add(1);
                skip_blocks(at + 10 + table(*bytes.get(at + 9)?) + 1)
            })(),
            _ => None,
        };
        match next {
            Some(next) => at = next,
            None => break,
        }
    }
    Some(Declared {
        width,
        height,
        frames: frames.max(1),
    })
}

/// The size a lossy bitstream states: a 3-byte frame tag, the start code, then 14 bits of width and of height.
/// `at`: where the chunk's data begins. `None`: not a key frame's header, or cut short.
fn vp8(bytes: &[u8], at: usize) -> Option<(u32, u32)> {
    if bytes.get(at + 3..at + 6)? != [0x9D, 0x01, 0x2A] {
        return None;
    }
    Some((le16(bytes, at + 6)? & 0x3FFF, le16(bytes, at + 8)? & 0x3FFF))
}

/// The size a lossless bitstream states: a signature byte, then 14 bits of width - 1 and 14 of height - 1.
fn vp8l(bytes: &[u8], at: usize) -> Option<(u32, u32)> {
    if *bytes.get(at)? != 0x2F {
        return None;
    }
    let packed = le32(bytes, at + 1)?;
    Some(((packed & 0x3FFF) + 1, ((packed >> 14) & 0x3FFF) + 1))
}

/// The RIFF chunks in `bytes[from..to]`: for each, its name and where its data begins and ends (cut at `to`).
/// A chunk is a name, a length, and the data padded to an even length.
fn chunks(bytes: &[u8], from: usize, to: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut found = Vec::new();
    let mut at = from;
    while let (Some(name), Some(length)) = (bytes.get(at..at + 4), le32(bytes, at + 4)) {
        if at + 8 > to {
            break;
        }
        let Some(end) = usize::try_from(length)
            .ok()
            .and_then(|length| (at + 8).checked_add(length))
        else {
            break;
        };
        found.push(([name[0], name[1], name[2], name[3]], at + 8, end.min(to)));
        match end.checked_add(end & 1) {
            Some(next) if next > at => at = next,
            _ => break,
        }
    }
    found
}

/// A plain file: the size in its one bitstream. An extended file (`VP8X`): the LARGEST of the canvas and of
/// every size stated inside it, because a decoder decodes each bitstream at the size that bitstream states and
/// compares it with the canvas only afterwards. Counted: the canvas, every animation frame's own size (`ANMF`),
/// and every lossy or lossless bitstream, at the top level and inside a frame. A bitstream whose header cannot
/// be read makes the file not a picture: nothing whose size is unknown reaches a decoder.
fn webp(bytes: &[u8]) -> Option<Declared> {
    let data = 20usize;
    let still = |(width, height)| Declared {
        width,
        height,
        frames: 1,
    };
    match bytes.get(12..16)? {
        b"VP8X" => {
            let three = |at: usize| Some(le32(bytes, at)? & 0x00FF_FFFF);
            let (mut width, mut height) = (three(data + 4)? + 1, three(data + 7)? + 1);
            let mut frames = 0u32;
            let mut widen = |(w, h): (u32, u32)| {
                width = width.max(w);
                height = height.max(h);
            };
            let stated = |name: &[u8; 4], at: usize| match name {
                b"VP8 " => Some(vp8(bytes, at)),
                b"VP8L" => Some(vp8l(bytes, at)),
                _ => None,
            };
            for (name, at, end) in chunks(bytes, 12, bytes.len()) {
                if let Some(size) = stated(&name, at) {
                    widen(size?);
                } else if &name == b"ANMF" {
                    frames = frames.saturating_add(1);
                    // The frame's own size (after its offset), then its chunks: alpha, and a bitstream.
                    widen((three(at + 6)? + 1, three(at + 9)? + 1));
                    for (inner, inner_at, _) in chunks(bytes, at + 16, end) {
                        if let Some(size) = stated(&inner, inner_at) {
                            widen(size?);
                        }
                    }
                }
            }
            Some(Declared {
                width,
                height,
                frames: frames.max(1),
            })
        }
        b"VP8 " => vp8(bytes, data).map(still),
        b"VP8L" => vp8l(bytes, data).map(still),
        _ => None,
    }
}

#[cfg(test)]
#[path = "viewer_picture_tests.rs"]
mod tests;
