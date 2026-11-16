//! The file's bytes as FilmCraft reads them: [`PipeReader`] asks the app for each range (`FromHelper::ReadAt`, at
//! most `MAX_READ` at a time and `MAX_IN_FLIGHT_READS` at once) and waits for its `Data` or `DataFailed`, with an
//! 8 MiB read-through cache of the last answers (this process's memory, gone with it).
//!
//! The app does not wait for a read to be answered before it sends its next message: its next `WantVideo`,
//! `WantAudio` or `Seek` can arrive while a read waits for its `Data`. Such a message is kept, in order, and the
//! session loop answers it after the current one ([`Wire::recv`] takes the kept ones first).
//!
//! The helper never asks for a byte beyond the file's length.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use beebeeb_media_protocol::{
    FromHelper, MAX_IN_FLIGHT_READS, MAX_READ, ToHelper, WireError, read_to_helper, write_msg,
};
use filmcraft_media::ByteReader;

/// The cache holds at most this many bytes of the latest answers.
pub const CACHE_MAX: usize = 8 << 20;
/// A small read asks for this much (clamped to the file), so the demuxers' many small reads cost few round trips.
pub const READ_AHEAD: usize = 64 << 10;
/// The most app messages kept while a read waits. The app has at most one `WantVideo` and one `WantAudio` out, and
/// seeks in a row are kept as the last one, so an app that sends more is not following the protocol.
pub const MAX_KEPT: usize = 16;

/// The two ends of the protocol: what the app sends in, what goes out. Shared by the session loop and the reader;
/// neither holds the lock while it decodes.
pub struct Wire {
    input: Box<dyn Read + Send>,
    output: Box<dyn Write + Send>,
    /// What the app sent while a read waited for its answer, in the order it came: taken before the pipe.
    kept: VecDeque<ToHelper>,
}

impl Wire {
    pub fn new(input: impl Read + Send + 'static, output: impl Write + Send + 'static) -> Wire {
        Wire {
            input: Box::new(input),
            output: Box::new(output),
            kept: VecDeque::new(),
        }
    }

    pub fn send(&mut self, msg: &FromHelper) -> io::Result<()> {
        write_msg(&mut self.output, msg)
    }

    /// The app's next message: the oldest one kept while a read waited, else the next one on the pipe.
    pub fn recv(&mut self) -> Result<ToHelper, WireError> {
        match self.kept.pop_front() {
            Some(msg) => Ok(msg),
            None => read_to_helper(&mut self.input),
        }
    }

    /// The answer to a read (`Data` or `DataFailed`), or `Close`. Every other message that comes first is kept for
    /// [`Wire::recv`]. `Close` replaces what was kept (the app ended the playback: nothing it asked is owed), so the
    /// session loop ends once the read gives up, and no later read waits on the pipe. `None`: more messages came than
    /// the protocol lets the app have out ([`MAX_KEPT`]).
    fn recv_answer(&mut self) -> Result<Option<ToHelper>, WireError> {
        if matches!(self.kept.front(), Some(ToHelper::Close)) {
            return Ok(Some(ToHelper::Close));
        }
        loop {
            let msg = read_to_helper(&mut self.input)?;
            match msg {
                ToHelper::Data { .. } | ToHelper::DataFailed { .. } => return Ok(Some(msg)),
                ToHelper::Close => {
                    self.kept.clear();
                    self.kept.push_back(ToHelper::Close);
                    return Ok(Some(ToHelper::Close));
                }
                // A seek right after a seek: only where the last one goes matters.
                ToHelper::Seek { .. } if matches!(self.kept.back(), Some(ToHelper::Seek { .. })) => {
                    if let Some(last) = self.kept.back_mut() {
                        *last = msg;
                    }
                }
                other => {
                    if self.kept.len() >= MAX_KEPT {
                        return Ok(None);
                    }
                    self.kept.push_back(other);
                }
            }
        }
    }
}

pub type SharedWire = Arc<Mutex<Wire>>;

/// Lock the wire; a poisoned lock is still the same pipe.
pub fn lock(wire: &SharedWire) -> MutexGuard<'_, Wire> {
    wire.lock().unwrap_or_else(|e| e.into_inner())
}

/// What went wrong under a decoder, so the session can name it: the decoder only sees an `io::Error`.
#[derive(Default)]
pub struct ReadFlags {
    /// The app answered `DataFailed` (it could not read that range).
    pub failed: AtomicBool,
    /// The app closed or the pipe ended.
    pub closed: AtomicBool,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(u64, Vec<u8>)>,
    bytes: usize,
}

impl Cache {
    fn covering(&self, offset: u64, end: u64) -> Option<(&Vec<u8>, usize)> {
        self.entries
            .iter()
            .rev()
            .find(|(start, bytes)| *start <= offset && end <= *start + bytes.len() as u64)
            .map(|(start, bytes)| (bytes, (offset - *start) as usize))
    }

    fn insert(&mut self, offset: u64, bytes: Vec<u8>) {
        if bytes.len() > CACHE_MAX {
            return;
        }
        self.bytes += bytes.len();
        self.entries.push_back((offset, bytes));
        while self.bytes > CACHE_MAX {
            match self.entries.pop_front() {
                Some((_, old)) => self.bytes -= old.len(),
                None => break,
            }
        }
    }
}

/// The one file this helper was started for, read range by range from the app.
pub struct PipeReader {
    len: u64,
    wire: SharedWire,
    cache: Mutex<Cache>,
    next_id: AtomicU32,
    flags: Arc<ReadFlags>,
}

impl PipeReader {
    pub fn new(wire: SharedWire, len: u64, flags: Arc<ReadFlags>) -> PipeReader {
        PipeReader {
            len,
            wire,
            cache: Mutex::new(Cache::default()),
            next_id: AtomicU32::new(1),
            flags,
        }
    }

    fn gone(&self) -> io::Error {
        self.flags.closed.store(true, Ordering::Relaxed);
        io::ErrorKind::BrokenPipe.into()
    }

    /// `total` bytes at `offset` from the app: chunks of at most `MAX_READ`, at most `MAX_IN_FLIGHT_READS` asked
    /// before an answer is read.
    fn fetch(&self, offset: u64, total: usize) -> io::Result<Vec<u8>> {
        let mut out = vec![0u8; total];
        let mut next = 0usize;
        let mut received = 0usize;
        // (id, start in `out`, length)
        let mut pending: Vec<(u32, usize, usize)> = Vec::new();
        let mut wire = lock(&self.wire);
        while received < total {
            while pending.len() < MAX_IN_FLIGHT_READS && next < total {
                let n = (total - next).min(MAX_READ as usize);
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let ask = FromHelper::ReadAt {
                    id,
                    offset: offset + next as u64,
                    len: n as u32,
                };
                wire.send(&ask).map_err(|_| self.gone())?;
                pending.push((id, next, n));
                next += n;
            }
            match wire.recv_answer() {
                Ok(Some(ToHelper::Data { id, bytes })) => {
                    let Some(at) = pending.iter().position(|p| p.0 == id) else {
                        return Err(io::ErrorKind::InvalidData.into());
                    };
                    let (_, start, n) = pending.remove(at);
                    if bytes.len() != n {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                    out[start..start + n].copy_from_slice(&bytes);
                    received += n;
                }
                Ok(Some(ToHelper::DataFailed { id })) => {
                    if pending.iter().any(|p| p.0 == id) {
                        self.flags.failed.store(true, Ordering::Relaxed);
                        return Err(io::Error::other("the app could not read that range"));
                    }
                    return Err(io::ErrorKind::InvalidData.into());
                }
                // The app ended the playback, or the pipe did: nothing more is read.
                Ok(Some(ToHelper::Close)) | Err(_) => return Err(self.gone()),
                // More messages than the app may have out while this read waits.
                Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            }
        }
        Ok(out)
    }
}

impl ByteReader for PipeReader {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        if buf.is_empty() {
            return Ok(());
        }
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        if end > self.len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((bytes, at)) = cache.covering(offset, end) {
                buf.copy_from_slice(&bytes[at..at + buf.len()]);
                return Ok(());
            }
        }
        let remaining = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let total = buf.len().max(READ_AHEAD).min(remaining);
        let data = self.fetch(offset, total)?;
        buf.copy_from_slice(&data[..buf.len()]);
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(offset, data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the helper sent to the app.
    #[derive(Clone, Default)]
    struct Sent(Arc<Mutex<Vec<u8>>>);

    impl Write for Sent {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The app's messages, encoded back to back as it writes them on the pipe.
    fn app_sends(msgs: &[ToHelper]) -> io::Cursor<Vec<u8>> {
        let mut bytes = Vec::new();
        for msg in msgs {
            write_msg(&mut bytes, msg).expect("a message the app writes");
        }
        io::Cursor::new(bytes)
    }

    /// P1-1 of the #462 review: the app sends its next requests (and a seek) without waiting for the answer to the
    /// read the helper is waiting on. The read still gets its `Data`, and the session loop then takes the requests in
    /// the order they came; two seeks in a row are kept as the last one.
    #[test]
    fn requests_sent_while_a_read_waits_are_kept_for_after_it() {
        const LEN: u64 = 100;
        let file: Vec<u8> = (0..LEN as u8).collect();
        let input = app_sends(&[
            ToHelper::WantAudio {
                rate: 48_000,
                channels: 2,
                frames: 4_800,
            },
            ToHelper::Seek { at_ms: 100 },
            ToHelper::Seek { at_ms: 500 },
            ToHelper::WantVideo { w: 32, h: 24 },
            // The first read of this reader is id 1: the whole (short) file, read ahead.
            ToHelper::Data {
                id: 1,
                bytes: file.clone(),
            },
        ]);
        let wire: SharedWire = Arc::new(Mutex::new(Wire::new(input, Sent::default())));
        let reader = PipeReader::new(wire.clone(), LEN, Arc::new(ReadFlags::default()));
        let mut buf = [0u8; 10];
        reader
            .read_at(0, &mut buf)
            .expect("the read is answered though requests came first");
        assert_eq!(buf[..], file[..10]);
        let mut wire = lock(&wire);
        assert_eq!(
            wire.recv().ok(),
            Some(ToHelper::WantAudio {
                rate: 48_000,
                channels: 2,
                frames: 4_800
            })
        );
        assert_eq!(wire.recv().ok(), Some(ToHelper::Seek { at_ms: 500 }));
        assert_eq!(wire.recv().ok(), Some(ToHelper::WantVideo { w: 32, h: 24 }));
        assert_eq!(wire.recv().err(), Some(WireError::Closed), "nothing else was kept");
    }

    /// A `Close` while a read waits ends the read, and the session loop sees it next (nothing kept before it is
    /// answered, no later read waits on the pipe). More requests than the app may have out end the read too.
    #[test]
    fn a_close_while_a_read_waits_ends_it_and_the_session() {
        let input = app_sends(&[ToHelper::WantVideo { w: 32, h: 24 }, ToHelper::Close]);
        let wire: SharedWire = Arc::new(Mutex::new(Wire::new(input, Sent::default())));
        let flags = Arc::new(ReadFlags::default());
        let reader = PipeReader::new(wire.clone(), 100, flags.clone());
        let mut buf = [0u8; 10];
        let err = reader.read_at(0, &mut buf).expect_err("the app closed");
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        assert!(flags.closed.load(Ordering::Relaxed));
        let err = reader.read_at(50, &mut buf).expect_err("still closed");
        assert_eq!(
            err.kind(),
            io::ErrorKind::BrokenPipe,
            "no read waits on the pipe after a Close"
        );
        assert_eq!(lock(&wire).recv().ok(), Some(ToHelper::Close));

        let flood: Vec<ToHelper> = (0..=MAX_KEPT as u32)
            .map(|w| ToHelper::WantVideo { w: w + 1, h: 1 })
            .collect();
        let wire: SharedWire = Arc::new(Mutex::new(Wire::new(app_sends(&flood), Sent::default())));
        let reader = PipeReader::new(wire, 100, Arc::new(ReadFlags::default()));
        let err = reader.read_at(0, &mut buf).expect_err("more than the app may have out");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// Row 3 (MCP2): a read that reaches even one byte past the end is refused as the end of the file, and nothing
    /// is asked of the app for it; the last byte itself is asked for.
    #[test]
    fn a_read_one_byte_past_the_end_is_refused_without_asking() {
        const LEN: u64 = 1_000;
        let sent = Sent::default();
        // The app never answers: a read that gets as far as asking fails on the closed pipe instead.
        let wire: SharedWire = Arc::new(Mutex::new(Wire::new(io::empty(), sent.clone())));
        let reader = PipeReader::new(wire, LEN, Arc::new(ReadFlags::default()));
        for (offset, len) in [(LEN, 1usize), (LEN - 1, 2), (LEN - 3, 4), (0, LEN as usize + 1)] {
            let mut buf = vec![0u8; len];
            let err = reader.read_at(offset, &mut buf).expect_err("past the end is refused");
            assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof, "{len} bytes at {offset}");
        }
        assert!(
            sent.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty(),
            "nothing was asked for"
        );
        let mut last = [0u8; 1];
        let err = reader.read_at(LEN - 1, &mut last).expect_err("the pipe is closed");
        assert_eq!(
            err.kind(),
            io::ErrorKind::BrokenPipe,
            "the last byte is in the file: it was asked for"
        );
        assert!(!sent.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    }
}
