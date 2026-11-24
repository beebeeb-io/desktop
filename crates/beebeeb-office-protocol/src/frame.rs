//! Frames on the two pipes between the app and the Office helper: a length, a kind, an id, a payload.
//!
//! Layout, all little-endian: a `u32` length of everything after it, a `u16` kind, a `u32` id, then the payload
//! (at most [`MAX_PAYLOAD`] bytes). [`read_frame`] reads the length first and refuses a length outside
//! `6 ..= 6 + MAX_PAYLOAD` before it allocates or reads anything else.
//!
//! **Ids.** Requests go both ways, so the id space is split: the app's requests carry even ids, the helper's (for
//! example a request for font bytes) odd ids. An answer carries the id of the request it answers, so the app knows an
//! answer by its even id and refuses one to an id it never sent ([`crate::check::check_answer_id`]).

use std::fmt;
use std::io::{self, Read, Write};

/// Bytes after the length that are not payload: the `u16` kind and the `u32` id.
pub const HEADER_LEN: usize = 6;

/// The largest payload of one frame: 64 KiB. Bulk bytes never travel in a frame; they go in a shared section.
pub const MAX_PAYLOAD: usize = 64 * 1024;

/// The largest value the length field may hold.
const MAX_LEN: u32 = (HEADER_LEN + MAX_PAYLOAD) as u32;

/// Whether a payload of `len` bytes fits one frame. [`write_frame`] and every message's `encode` hold this one check,
/// so a message that encodes can always be written.
pub const fn fits_one_frame(len: usize) -> bool {
    len <= MAX_PAYLOAD
}

/// One message on a pipe.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    /// What the payload is: one of the `KIND_*` constants of [`crate::office`].
    pub kind: u16,
    /// The request this frame is or answers. Even: opened by the app. Odd: opened by the helper.
    pub id: u32,
    /// The encoded message, at most [`MAX_PAYLOAD`] bytes.
    pub payload: Vec<u8>,
}

/// The payload may hold document text, so `Debug` shows its length only.
impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// Why no frame could be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The stream ended cleanly, at a frame boundary.
    Closed,
    /// The length field is over `6 + MAX_PAYLOAD`. Nothing of that size was allocated or read.
    TooLong,
    /// The length field is under 6: no room for the kind and the id.
    TooShort,
    /// The stream ended inside a frame.
    Truncated,
    /// The pipe failed.
    Io(io::ErrorKind),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            FrameError::Closed => "the pipe closed",
            FrameError::TooLong => "a frame longer than the cap",
            FrameError::TooShort => "a frame shorter than its header",
            FrameError::Truncated => "the pipe closed inside a frame",
            FrameError::Io(_) => "the pipe failed",
        };
        f.write_str(text)
    }
}

impl std::error::Error for FrameError {}

/// Writes one frame and flushes. Refuses a payload over [`MAX_PAYLOAD`] without writing anything.
pub fn write_frame(w: &mut impl Write, kind: u16, id: u32, payload: &[u8]) -> io::Result<()> {
    if !fits_one_frame(payload.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame payload over the cap",
        ));
    }
    // Fits: HEADER_LEN + MAX_PAYLOAD is far below u32::MAX.
    let len = (HEADER_LEN + payload.len()) as u32;
    let mut bytes = Vec::with_capacity(4 + HEADER_LEN + payload.len());
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.extend_from_slice(payload);
    w.write_all(&bytes)?;
    w.flush()
}

/// Reads one frame. The length comes first and is refused (as [`FrameError::TooLong`] or [`FrameError::TooShort`])
/// before anything of its size is allocated or read.
pub fn read_frame(r: &mut impl Read) -> Result<Frame, FrameError> {
    let mut len_bytes = [0u8; 4];
    let mut got = 0usize;
    // Not `read_exact`: a stream that ends before the first byte is `Closed`, one that ends inside is `Truncated`.
    while got < len_bytes.len() {
        match r.read(&mut len_bytes[got..]) {
            Ok(0) if got == 0 => return Err(FrameError::Closed),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got = got.saturating_add(n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FrameError::Io(e.kind())),
        }
    }
    let len = u32::from_le_bytes(len_bytes);
    if len < HEADER_LEN as u32 {
        return Err(FrameError::TooShort);
    }
    if len > MAX_LEN {
        return Err(FrameError::TooLong);
    }

    let mut header = [0u8; HEADER_LEN];
    read_all(r, &mut header)?;
    let kind = u16::from_le_bytes([header[0], header[1]]);
    let id = u32::from_le_bytes([header[2], header[3], header[4], header[5]]);

    // At most MAX_PAYLOAD bytes: the length was checked above.
    let mut payload = vec![0u8; len as usize - HEADER_LEN];
    read_all(r, &mut payload)?;
    Ok(Frame { kind, id, payload })
}

fn read_all(r: &mut impl Read, buf: &mut [u8]) -> Result<(), FrameError> {
    r.read_exact(buf).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => FrameError::Truncated,
        kind => FrameError::Io(kind),
    })
}

/// Which side opened a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The app: even ids.
    App,
    /// The helper: odd ids.
    Helper,
}

impl Side {
    /// The side that opened the request with this id.
    pub const fn of(id: u32) -> Side {
        if id & 1 == 0 { Side::App } else { Side::Helper }
    }
}

/// Hands out request ids for one side: even ids for the app, odd ids for the helper.
///
/// Ids wrap after 2^31 requests and keep their parity; the app still refuses an answer to an id that is not
/// outstanding, so a wrapped id cannot be mistaken for a stale one.
#[derive(Debug, Clone)]
pub struct Ids {
    next: u32,
}

impl Ids {
    /// Ids for `side`, starting at 0 (app) or 1 (helper).
    pub const fn new(side: Side) -> Ids {
        let next = match side {
            Side::App => 0,
            Side::Helper => 1,
        };
        Ids { next }
    }

    /// The next id of this side.
    pub fn next_id(&mut self) -> u32 {
        let id = self.next;
        self.next = self.next.wrapping_add(2);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encoded(kind: u16, id: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_frame(&mut out, kind, id, payload).expect("a payload under the cap is written");
        out
    }

    /// Serves `head` and then zero bytes for ever, panics when asked to fill a buffer over 70 KiB, and counts what it
    /// handed out. A reader that allocated the declared length and read into it would ask for far more than 70 KiB.
    struct Greedy {
        head: Vec<u8>,
        served: usize,
    }

    impl Read for Greedy {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert!(
                buf.len() <= 70 * 1024,
                "asked to fill {} bytes: the declared length was trusted",
                buf.len()
            );
            let n = if self.served < self.head.len() {
                let rest = &self.head[self.served..];
                let n = rest.len().min(buf.len());
                buf[..n].copy_from_slice(&rest[..n]);
                n
            } else {
                buf.fill(0);
                buf.len()
            };
            self.served += n;
            Ok(n)
        }
    }

    #[test]
    fn frame_length_over_the_cap_is_refused_without_allocating() {
        for declared in [6 + 1024 * 1024u32, MAX_LEN + 1, u32::MAX] {
            let mut head = declared.to_le_bytes().to_vec();
            head.extend_from_slice(&[0x01, 0x01, 2, 0, 0, 0]);
            let mut r = Greedy { head, served: 0 };
            assert_eq!(read_frame(&mut r), Err(FrameError::TooLong), "declared {declared}");
            assert_eq!(
                r.served, 4,
                "only the length is read before the refusal (declared {declared})"
            );
        }

        // The cap itself is allowed: exactly 6 + MAX_PAYLOAD.
        let payload = vec![0xAB; MAX_PAYLOAD];
        let mut r = Greedy {
            head: encoded(0x0101, 2, &payload),
            served: 0,
        };
        let frame = read_frame(&mut r).expect("a frame at the cap is read");
        assert_eq!(frame.payload.len(), MAX_PAYLOAD);
        assert_eq!(r.served, 4 + HEADER_LEN + MAX_PAYLOAD);

        // And under 6 is too short.
        for declared in 0..HEADER_LEN as u32 {
            let mut r = Cursor::new(declared.to_le_bytes().to_vec());
            assert_eq!(read_frame(&mut r), Err(FrameError::TooShort), "declared {declared}");
        }
    }

    #[test]
    fn frame_round_trips() {
        let cases: [(u16, u32, Vec<u8>); 4] = [
            (0x0101, 0, Vec::new()),
            (0x0203, 2, vec![1, 2, 3]),
            (0x02FF, u32::MAX, vec![0xFF; 17]),
            (0x0204, 7, (0..MAX_PAYLOAD).map(|i| (i % 251) as u8).collect()),
        ];
        let mut stream = Vec::new();
        for (kind, id, payload) in &cases {
            stream.extend_from_slice(&encoded(*kind, *id, payload));
        }
        let mut r = Cursor::new(stream);
        for (kind, id, payload) in cases {
            let frame = read_frame(&mut r).expect("each frame reads back");
            assert_eq!(frame, Frame { kind, id, payload });
        }
        assert_eq!(read_frame(&mut r), Err(FrameError::Closed));
    }

    #[test]
    fn frame_write_refuses_a_payload_over_the_cap() {
        let mut out = Vec::new();
        let err = write_frame(&mut out, 0x0204, 2, &vec![0; MAX_PAYLOAD + 1]).expect_err("over the cap");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(out.is_empty(), "nothing is written");
    }

    #[test]
    fn frame_truncated_is_reported() {
        let whole = encoded(0x0203, 4, &[9; 12]);
        // Every cut inside the frame: in the length, in the header, in the payload.
        for cut in 1..whole.len() {
            let mut r = Cursor::new(whole[..cut].to_vec());
            assert_eq!(read_frame(&mut r), Err(FrameError::Truncated), "cut at {cut}");
        }
    }

    #[test]
    fn frame_closed_at_a_boundary() {
        assert_eq!(read_frame(&mut Cursor::new(Vec::new())), Err(FrameError::Closed));

        let mut r = Cursor::new(encoded(0x0105, 6, &[]));
        assert!(read_frame(&mut r).is_ok());
        assert_eq!(read_frame(&mut r), Err(FrameError::Closed));
    }

    /// Fails with `Interrupted` before every successful read, one byte at a time.
    struct Stutter {
        bytes: Vec<u8>,
        at: usize,
        interrupt_next: bool,
    }

    impl Read for Stutter {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.interrupt_next {
                self.interrupt_next = false;
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            self.interrupt_next = true;
            if self.at >= self.bytes.len() || buf.is_empty() {
                return Ok(0);
            }
            buf[0] = self.bytes[self.at];
            self.at += 1;
            Ok(1)
        }
    }

    #[test]
    fn frame_interrupted_reads_are_retried() {
        let mut r = Stutter {
            bytes: encoded(0x0206, 8, &[4, 3, 2, 1]),
            at: 0,
            interrupt_next: true,
        };
        let frame = read_frame(&mut r).expect("interruptions are retried");
        assert_eq!(
            frame,
            Frame {
                kind: 0x0206,
                id: 8,
                payload: vec![4, 3, 2, 1]
            }
        );
    }

    struct Broken;

    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn frame_pipe_failure_is_reported() {
        assert_eq!(read_frame(&mut Broken), Err(FrameError::Io(io::ErrorKind::BrokenPipe)));
    }

    #[test]
    fn frame_debug_shows_no_payload() {
        let frame = Frame {
            kind: 0x0204,
            id: 2,
            payload: b"quarterly salaries".to_vec(),
        };
        let shown = format!("{frame:?}");
        assert!(!shown.contains("salaries") && !shown.contains("113"), "{shown}");
        assert!(shown.contains("payload_len: 18"), "{shown}");
    }

    #[test]
    fn ids_are_split_by_side() {
        let mut app = Ids::new(Side::App);
        let mut helper = Ids::new(Side::Helper);
        for _ in 0..4 {
            let a = app.next_id();
            let h = helper.next_id();
            assert_eq!(Side::of(a), Side::App, "{a}");
            assert_eq!(Side::of(h), Side::Helper, "{h}");
        }

        // Wrapping keeps the parity.
        let mut app = Ids { next: u32::MAX - 1 };
        assert_eq!(app.next_id(), u32::MAX - 1);
        assert_eq!(app.next_id(), 0);
        let mut helper = Ids { next: u32::MAX };
        assert_eq!(helper.next_id(), u32::MAX);
        assert_eq!(helper.next_id(), 1);
    }
}
