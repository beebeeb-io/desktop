//! The Office viewer's helper: a separate, confined process that is handed a document's bytes and a buffer per page,
//! and answers only what the app asks.
//!
//! The app starts it through `beebeeb-office-sandbox` (no network, no files, no child process, gone with the app)
//! and talks to it over two pipes in the frames of `beebeeb-office-protocol`. Bulk bytes travel in sections the app
//! grants: the document read-only, each rendered page read-write. The helper:
//!
//! - writes nothing but frames to its standard output, and nothing anywhere else (its standard error is not
//!   connected, and it installs no logger, so the readers' warnings, which can quote names from the file, go nowhere);
//! - reads no argument, no environment variable and no file;
//! - answers each request exactly once, in the order received, with the request's id ([`serve`]);
//! - ends with [`EXIT_PROTOCOL`] on any frame it cannot trust, and turns a panic into a plain exit ([`start`]).
//!
//! The worst a hostile document can achieve is a viewer that says "Renderer stopped".

#![forbid(unsafe_code)]

mod edit_map;
mod engine;
mod fit;
mod memory;
mod save;
mod serve;
mod start;

pub use engine::{EditingDoc, Engine, Office, OpenDoc, Opening, block_cells, open_code, render_code};
pub use memory::{Granted, Mapped, MappedMut, Memory};
pub use serve::serve;
pub use start::start;

/// The input ended (the app closed the pipe), or the document could not be opened and that answer was written.
pub const EXIT_OK: u8 = 0;
/// A frame the helper cannot trust: an unknown kind, a payload that does not decode, a helper-side id, an answer
/// from the app, a request before `Open`, a second `Open`, a truncated or over-long frame.
pub const EXIT_PROTOCOL: u8 = 2;
/// A panic: the hook [`start`] installs exits with this code and writes nothing.
pub const EXIT_PANIC: u8 = 3;
/// A pipe failed: an answer could not be written, or the input could not be read.
pub const EXIT_IO: u8 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_distinct() {
        let codes = [EXIT_OK, EXIT_PROTOCOL, EXIT_PANIC, EXIT_IO];
        let mut checked = 0;
        for (i, a) in codes.iter().enumerate() {
            for b in &codes[i + 1..] {
                assert_ne!(a, b, "two exit codes share a value");
                checked += 1;
            }
        }
        assert_eq!(checked, 6, "every pair was compared");
        // A Rust panic without the hook exits 101; the hook's code must be told apart from it.
        assert!(codes.iter().all(|c| *c != 101));
    }
}
