//! One playback: `Open` → the container's reader-based opener over [`crate::read::PipeReader`] → `Opened` and `Tags`;
//! then `WantVideo` / `WantAudio` / `Seek` answered in presentation order until the end (`Ended`) or `Close`.
//! Before a stream's decoder opens, its codec's route ([`crate::route`]) decides: `Rust` decodes here, `System` goes
//! to [`crate::system`] (and is `NoSystemDecoder` where the PC has none), `Unavailable` is `Unsupported`.
//!
//! Slice M3 (`media-helper`) fills it. The scaffold names the opener it will use, so the one check proves the
//! dependency resolves and builds.

use std::sync::Arc;

use filmcraft_codecs::{CodecError, Mp4Source};
use filmcraft_media::SharedReader;

use crate::read::PipeReader;

/// An MP4 or MOV over the app's ranges: only the index is read when it opens, samples when they are asked for. The
/// helper has no name for the file: FilmCraft's errors name it "".
pub fn open_mp4(reader: PipeReader) -> Result<Mp4Source, CodecError> {
    let shared: SharedReader = Arc::new(reader);
    Mp4Source::open_reader("", shared)
}
