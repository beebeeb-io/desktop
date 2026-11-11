//! Desktop v2 (batch B02): the `drive_*` commands behind the window's `DriveSource`
//! (`src/v2/source/desktop.ts`; spec `docs/superpowers/specs/2026-11-08-desktop-v2-design.md` 3.1).
//!
//! The boundary (spec 2.1): the session token, the master key and every file key stay in Rust.
//! A command returns view data only (decrypted names, sizes, times, states; thumbnails and file
//! content as binary IPC) and never a key, a wrapped key, a token, a URL, a nonce or a ciphertext.
//! Reads come from the local state DB; changes are made on the sync root so the engine uploads them
//! under core's rules; trash and restore go through the API as today's Trash page does.
//!
//! Errors are `"<code>: <detail>"` with a `DriveSourceError` code; the detail is a fixed word,
//! never a path or a name.

#[cfg(test)]
mod absent_row; // slice 58
mod commands;
pub(crate) use beebeeb_engine::drive_v2::dto;
mod events;
mod fsops;
/// The window's view models, under their old path: the engine crate's, plus the two that read a shell module.
pub(crate) mod model {
    pub(crate) use super::model_shell::*;
    pub(crate) use beebeeb_engine::drive_v2::model::*;
}
mod model_shell;
mod photo_meta; // v2 b02b
mod status; // v2 b02b
mod status_cmds; // v2 b02b
#[cfg(test)]
mod status_tests; // v2 b02b
#[cfg(test)]
mod tests;

pub(crate) use commands::*;
pub(crate) use status_cmds::*; // v2 b02b

pub(crate) use beebeeb_engine::drive_v2::DriveError;
