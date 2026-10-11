//! Spec 2026-10-10 (task 1887): a sign-out by choice never loses a Finder save that is still waiting.
//! The pure parts compile on every platform; the keep step and the folder writer are macOS only (W5).
// Plan review M11: the state this module adds (`SignOutState` and its fields, Tasks 7-14) is read on macOS and in tests
// only; elsewhere it is built and never read.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

pub(crate) mod waiting;
