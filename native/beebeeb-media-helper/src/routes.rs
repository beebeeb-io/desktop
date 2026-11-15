//! The route check before a stream's decoder opens ([`crate::ROUTES`], three kinds: [`crate::Route`]): `Rust` opens
//! FilmCraft's decoder, `System` asks [`crate::system`], `Unavailable` answers `Error(Unsupported(codec))`.
//!
//! One fallback, one way only: a `Rust` stream FilmCraft cannot decode (H.264 interlaced or 10-bit, the kinds its
//! decoder refuses) goes to [`crate::system`] when its codec is in [`crate::SYSTEM_FALLBACK`]; otherwise it is
//! `Error(NoDecoder)`. Never a fallback from `System` to FilmCraft's decoder (HEVC above all), and never to a decoder
//! this build does not contain: a codec whose Cargo feature is off is not in the binary.
//!
//! Slice M3 (`media-helper`) fills it.
