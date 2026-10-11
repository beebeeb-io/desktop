//! Spec 2026-10-10 (task 1887): a sign-out by choice never loses a Finder save that is still waiting.
//! The pure parts compile on every platform; the keep step and the folder writer are macOS only (W5).
// The sign-out state this module holds is read on macOS and in tests only; elsewhere it is built and never read.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

pub(crate) mod kept_names;
#[cfg(target_os = "macos")]
pub(crate) mod kept_writer;
pub(crate) mod waiting;
