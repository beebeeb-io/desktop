//! The first reads of an opened item, before its helper starts (RE-d): the item's LAST bytes first, then its first
//! ones, so the chunks a player needs to know a file (an MP4 or a MOV whose `moov` is written at the end, as a phone
//! writes it) are fetched at once, and only those. The engine's reader ([`MediaBytes`] over
//! `beebeeb_engine::app::bindings::media_stream::MediaReader`) fetches only the chunks that cover a range and keeps
//! them in its cache, so the helper's own reads of the head and of the `moov` that follow are answered from memory;
//! nothing here depends on the Windows streaming switch (`cf_range::W2_STREAMING`).
//!
//! [`duration_ms`] reads the file's length from its `mvhd` box: box headers and one `mvhd` read, every size and
//! offset checked, at most [`MAX_BOXES`] boxes a level, at most 64 bytes a read. It decodes nothing (the helper
//! still decodes every byte that plays, confined); a file that is not an ISO media file, or one whose boxes do not
//! add up, has no length here and waits for the helper's `Opened` as before.

use std::sync::atomic::AtomicBool;

use super::{MAX_DURATION_MS, MediaBytes};

/// The most boxes read at one level (the file's top level, the `moov`'s children).
pub(super) const MAX_BOXES: u32 = 64;

/// The reads that make the file's two ends arrive first: its last byte (the chunk that holds a trailing `moov`), then
/// its first (the header and the first frames). A failed read is left to the helper's own reads to say.
pub(super) fn prime(bytes: &mut dyn MediaBytes, cancel: &AtomicBool) {
    let len = bytes.len();
    if len == 0 {
        return;
    }
    let _ = bytes.read_at(len - 1, 1, cancel);
    let _ = bytes.read_at(0, 1, cancel);
}

/// [`prime`], then [`duration_ms`]: the item's length as its own header says it, read from its two ends.
pub(super) fn prime_and_probe(bytes: &mut dyn MediaBytes, cancel: &AtomicBool) -> Option<u64> {
    prime(bytes, cancel);
    duration_ms(bytes, cancel)
}

/// One box: its type, where its content starts, and where it ends.
struct BoxAt {
    kind: [u8; 4],
    body: u64,
    end: u64,
}

/// `n` bytes at `offset` (n ≤ 64), or `None`.
fn read(bytes: &mut dyn MediaBytes, offset: u64, n: u32, cancel: &AtomicBool) -> Option<Vec<u8>> {
    let got = bytes.read_at(offset, n, cancel).ok()?;
    (got.len() == n as usize).then(|| got.to_vec())
}

fn be32(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?)))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The box whose header is at `at`, inside `..limit`. `None`: no header fits, its type is not four printable
/// characters, or its size does not fit inside `limit`.
fn box_at(bytes: &mut dyn MediaBytes, at: u64, limit: u64, cancel: &AtomicBool) -> Option<BoxAt> {
    let room = limit.checked_sub(at)?;
    if room < 8 {
        return None;
    }
    let head = read(bytes, at, room.min(16) as u32, cancel)?;
    let kind: [u8; 4] = head.get(4..8)?.try_into().ok()?;
    if !kind.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
        return None;
    }
    let (size, header) = match be32(&head, 0)? {
        // To the end of what holds it.
        0 => (room, 8),
        // A 64-bit size follows the type.
        1 => (be64(&head, 8)?, 16),
        size => (size, 8),
    };
    if size < header || size > room {
        return None;
    }
    Some(BoxAt {
        kind,
        body: at + header,
        end: at + size,
    })
}

/// The first box of type `kind` among the boxes from `from` to `limit`.
fn find(bytes: &mut dyn MediaBytes, from: u64, limit: u64, kind: &[u8; 4], cancel: &AtomicBool) -> Option<BoxAt> {
    let mut at = from;
    for _ in 0..MAX_BOXES {
        let found = box_at(bytes, at, limit, cancel)?;
        if &found.kind == kind {
            return Some(found);
        }
        if found.end >= limit {
            return None;
        }
        at = found.end;
    }
    None
}

/// The length the file's `moov` → `mvhd` states, in ms. `None`: not an ISO media file, no `mvhd`, a time scale of
/// 0, an unknown duration (all ones), or one over the player's longest.
pub(super) fn duration_ms(bytes: &mut dyn MediaBytes, cancel: &AtomicBool) -> Option<u64> {
    let len = bytes.len();
    let moov = find(bytes, 0, len, b"moov", cancel)?;
    let mvhd = find(bytes, moov.body, moov.end, b"mvhd", cancel)?;
    let room = mvhd.end.checked_sub(mvhd.body)?;
    let head = read(bytes, mvhd.body, room.min(32) as u32, cancel)?;
    let (scale, duration) = match head.first()? {
        // version, flags, created, modified (32-bit), time scale, duration (32-bit)
        0 => (be32(&head, 12)?, be32(&head, 16).filter(|d| *d != u64::from(u32::MAX))?),
        // version, flags, created, modified (64-bit), time scale, duration (64-bit)
        1 => (be32(&head, 20)?, be64(&head, 24).filter(|d| *d != u64::MAX)?),
        _ => return None,
    };
    if scale == 0 {
        return None;
    }
    let ms = u128::from(duration) * 1000 / u128::from(scale);
    u64::try_from(ms).ok().filter(|ms| *ms > 0 && *ms <= MAX_DURATION_MS)
}

#[cfg(test)]
#[path = "ranged_tests.rs"]
mod tests;
