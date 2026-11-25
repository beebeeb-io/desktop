//! RE-d: a video's length comes from its two ends. A file of chunks (a fake of the engine's reader: every read
//! counts the chunks it covers) with its `moov` at the end says its length after the head and the tail chunks only.

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;

use zeroize::Zeroizing;

use super::{duration_ms, prime_and_probe};
use crate::media::player::MediaBytes;

/// A file in chunks of `stride` bytes; `touched` is every chunk a read covered, in the order of the first touch.
struct Chunked {
    data: Vec<u8>,
    stride: u64,
    touched: Vec<u64>,
}

impl MediaBytes for Chunked {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_at(&mut self, offset: u64, len: u32, _cancel: &AtomicBool) -> Result<Zeroizing<Vec<u8>>, ()> {
        let end = offset + u64::from(len);
        if len == 0 || end > self.len() {
            return Err(());
        }
        for chunk in offset / self.stride..=(end - 1) / self.stride {
            if !self.touched.contains(&chunk) {
                self.touched.push(chunk);
            }
        }
        Ok(Zeroizing::new(self.data[offset as usize..end as usize].to_vec()))
    }
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// `mvhd` version 0: `scale` units a second, `duration` units long.
fn mvhd_v0(scale: u32, duration: u32) -> Vec<u8> {
    let mut body = vec![0u8; 4 + 8];
    body.extend_from_slice(&scale.to_be_bytes());
    body.extend_from_slice(&duration.to_be_bytes());
    body.resize(100, 0);
    boxed(b"mvhd", &body)
}

/// `mvhd` version 1.
fn mvhd_v1(scale: u32, duration: u64) -> Vec<u8> {
    let mut body = vec![1u8, 0, 0, 0];
    body.extend_from_slice(&[0u8; 16]);
    body.extend_from_slice(&scale.to_be_bytes());
    body.extend_from_slice(&duration.to_be_bytes());
    body.resize(112, 0);
    boxed(b"mvhd", &body)
}

/// A phone's MOV: `ftyp`, `wide`, an `mdat` of `mdat_len` bytes (a 64-bit size when `large`), then `moov`
/// (a `trak` stand-in before its `mvhd`, as some writers order them).
fn moov_at_end(mdat_len: usize, large: bool, mvhd: Vec<u8>) -> Vec<u8> {
    let mut file = boxed(b"ftyp", b"qt  \0\0\0\0qt  ");
    file.extend(boxed(b"wide", b""));
    if large {
        file.extend_from_slice(&1u32.to_be_bytes());
        file.extend_from_slice(b"mdat");
        file.extend_from_slice(&((mdat_len + 16) as u64).to_be_bytes());
    } else {
        file.extend_from_slice(&((mdat_len + 8) as u32).to_be_bytes());
        file.extend_from_slice(b"mdat");
    }
    file.extend(std::iter::repeat_n(0xAB, mdat_len));
    let mut moov = boxed(b"udta", &[0u8; 40]);
    moov.extend(mvhd);
    moov.extend(boxed(b"trak", &[0u8; 200]));
    file.extend(boxed(b"moov", &moov));
    file
}

/// The red test of RE-d: 83.5 s at a time scale of 600, `moov` at the end of a 40-chunk file. The length is known
/// after the head chunk and the tail chunk, the head one first (RE3-b: the engine fetches the tail beside it); no
/// chunk between them is read.
#[test]
fn moov_at_end_says_its_duration_from_the_head_and_tail_chunks() {
    const STRIDE: u64 = 4096;
    let data = moov_at_end(40 * STRIDE as usize, false, mvhd_v0(600, 600 * 835 / 10));
    let last = (data.len() as u64 - 1) / STRIDE;
    let mut file = Chunked {
        data,
        stride: STRIDE,
        touched: Vec::new(),
    };
    let cancel = AtomicBool::new(false);
    assert_eq!(prime_and_probe(&mut file, &cancel), Some(83_500));
    assert_eq!(
        file.touched.first(),
        Some(&0),
        "the head chunk is read first, the tail fetched beside it"
    );
    let touched: BTreeSet<u64> = file.touched.iter().copied().collect();
    assert_eq!(
        touched,
        BTreeSet::from([0, last]),
        "only the head and the tail chunks are read"
    );
}

/// A file over 4 GiB is written with a 64-bit `mdat` size and often a version 1 `mvhd`.
#[test]
fn a_64_bit_mdat_and_a_version_1_mvhd() {
    let mut file = Chunked {
        data: moov_at_end(10_000, true, mvhd_v1(90_000, 90_000 * 3_725)),
        stride: 1024,
        touched: Vec::new(),
    };
    assert_eq!(duration_ms(&mut file, &AtomicBool::new(false)), Some(3_725_000));
}

/// What is not an ISO media file, or one whose sizes do not add up, has no length here (the helper's `Opened` says
/// it): no panic, no read outside the file.
#[test]
fn no_length_from_what_does_not_add_up() {
    let cancel = AtomicBool::new(false);
    let mut cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"\x1aE\xdf\xa3 matroska and more bytes".to_vec(),
        // An mdat that says it is longer than the file.
        {
            let mut f = boxed(b"ftyp", b"isom");
            f.extend_from_slice(&u32::MAX.to_be_bytes());
            f.extend_from_slice(b"mdat");
            f
        },
        // A time scale of 0, and an unknown duration.
        moov_at_end(100, false, mvhd_v0(0, 100)),
        moov_at_end(100, false, mvhd_v0(600, u32::MAX)),
    ];
    // A size smaller than its own header.
    let mut tiny = moov_at_end(100, false, mvhd_v0(600, 600));
    tiny[0..4].copy_from_slice(&4u32.to_be_bytes());
    cases.push(tiny);
    for (i, data) in cases.into_iter().enumerate() {
        let mut file = Chunked {
            data,
            stride: 64,
            touched: Vec::new(),
        };
        assert_eq!(prime_and_probe(&mut file, &cancel), None, "case {i}");
    }
}
