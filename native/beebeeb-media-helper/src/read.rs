//! The file's bytes as FilmCraft reads them: [`PipeReader`] asks the app for each range (`FromHelper::ReadAt`, at
//! most `MAX_READ` at a time and `MAX_IN_FLIGHT_READS` at once) and waits for its `Data` or `DataFailed`, with an
//! 8 MiB read-through cache of the last answers (this process's memory, gone with it).
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

/// The two ends of the protocol: what the app sends in, what goes out. Shared by the session loop and the reader;
/// neither holds the lock while it decodes.
pub struct Wire {
    input: Box<dyn Read + Send>,
    output: Box<dyn Write + Send>,
}

impl Wire {
    pub fn new(input: impl Read + Send + 'static, output: impl Write + Send + 'static) -> Wire {
        Wire {
            input: Box::new(input),
            output: Box::new(output),
        }
    }

    pub fn send(&mut self, msg: &FromHelper) -> io::Result<()> {
        write_msg(&mut self.output, msg)
    }

    pub fn recv(&mut self) -> Result<ToHelper, WireError> {
        read_to_helper(&mut self.input)
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
            match wire.recv() {
                Ok(ToHelper::Data { id, bytes }) => {
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
                Ok(ToHelper::DataFailed { id }) => {
                    if pending.iter().any(|p| p.0 == id) {
                        self.flags.failed.store(true, Ordering::Relaxed);
                        return Err(io::Error::other("the app could not read that range"));
                    }
                    return Err(io::ErrorKind::InvalidData.into());
                }
                // The app ended the playback, or sent what this state does not take: nothing more is read.
                Ok(ToHelper::Close) | Err(_) => return Err(self.gone()),
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
