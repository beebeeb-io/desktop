//! The file's bytes as FilmCraft reads them: [`PipeReader`] asks the app for each range (`FromHelper::ReadAt`, at
//! most `MAX_READ` at a time and `MAX_IN_FLIGHT_READS` at once) and waits for its `Data` or `DataFailed`.
//!
//! Slice M3 (`media-helper`) fills it: the asks, the waits, an 8 MiB read-through cache of the last answers (this
//! process's memory, gone with it). The scaffold's reader answers every read with `Unsupported`.

use std::io;

use filmcraft_media::ByteReader;

/// The one file this helper was started for, read range by range from the app.
pub struct PipeReader;

impl ByteReader for PipeReader {
    fn len(&self) -> u64 {
        0
    }

    fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}
