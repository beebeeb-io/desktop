//! A decoded frame made what the app asked for: scaled to fit `w × h`, padded with black to exactly `w × h`, and
//! converted to RGBA with the stream's own matrix and range; PCM mixed to the asked channels. Pure functions.
//!
//! Slice M3 (`media-helper`) fills it.
