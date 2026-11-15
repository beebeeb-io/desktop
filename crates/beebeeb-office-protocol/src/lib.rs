//! The wire format between the app and the Office helper, and the checks the app runs on every answer.
//!
//! The helper parses documents nobody trusts, inside the sandbox (`beebeeb-office-sandbox`). It talks to the app over
//! two pipes in binary [`frame`]s; bulk bytes (the document, rendered pixels) travel in shared memory sections the app
//! creates and hands over, named on the wire by a [`office::GrantRef`]. The worst a hostile document may achieve is a
//! viewer that says "Renderer stopped":
//!
//! - every length is read first and refused before anything of that size is allocated ([`frame::read_frame`],
//!   [`office::Message::decode`]);
//! - every message fits one frame: `encode` refuses one that does not, with the check `write_frame` holds
//!   ([`frame::fits_one_frame`]);
//! - every answer is checked against what the app asked before it is used ([`check`]): sizes equal the ask and stay
//!   under the limits, counts and text lengths are capped, an error is a code from a fixed list, never the helper's
//!   own words;
//! - text from the helper is only ever text: never a path, a URL or a format string.
//!
//! Anything that fails a check is a protocol error: the app kills the helper and shows "Renderer stopped".
//!
//! Media playback keeps its own codec (`beebeeb-media-protocol`) with the same conventions: a little-endian length
//! first, refused before allocating, error codes from a fixed list, sizes equal to the ask, text capped. Merging the
//! two codecs is a clean-up after 0.9.0; neither crate depends on the other.

#![forbid(unsafe_code)]

pub mod check;
pub mod frame;
pub mod office;

mod wire;
