//! Slice M1 (`media-range`): the bridge's half of the media player's range reader (`app::bindings::media_stream`).
//!
//! What M1 puts here: ONE metadata read that pins the file's version (its file key, its `PinnedVersion`, its
//! `ContentRecord`, its chunk size and count, its plaintext size), and a read of a span of chunks through the
//! index-bound partial read of `chunk_frames.rs`, each chunk opened at its own index against the pinned version. Not
//! cfg-gated, no row status written, no transfer-board entry, nothing written to disk.

use super::EngineBridge;

impl EngineBridge {}
