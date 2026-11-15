//! The route check before a stream's decoder opens ([`crate::ROUTES`], three kinds: [`crate::Route`]): `Rust` opens
//! FilmCraft's decoder, `System` asks [`crate::system`], `Unavailable` answers `Error(Unsupported(codec))`; and
//! FilmCraft's own "no decoder" (MPEG-4 Part 2, WMV, WMA, interlaced or 10-bit H.264, 4:2:2 HEVC) answers
//! `Error(NoDecoder)`. Never a fallback from `System` to FilmCraft's decoder.
//!
//! Slice M3 (`media-helper`) fills it.
