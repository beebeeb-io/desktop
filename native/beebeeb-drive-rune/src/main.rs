//! Beebeeb Drive — the native app (Rune, no web view).
//!
//! This file is the entry point and nothing else: it names the crate's modules and hands over
//! to the shell. Everything about lifetime, windows, the tray and routing is in `shell/`
//! (`src/shell/README.md`); the root state is `shell::state::State`.
//!
//! ```sh
//! cargo run -p beebeeb-drive-rune
//! cargo run -p beebeeb-drive-rune -- --snap DW-0401.png --size 1280x800 --scale 2 --theme light
//! ```
//!
//! Lanes: add ONE `mod` line for your module below, in alphabetical order, and nothing else.

mod shell;
mod source;
// The engine lane's read API (PR 280): only `EngineSource::open` and `list` have a caller
// outside its tests so far (`--db`, dev builds).
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
mod source_engine;
mod theme;
mod ui;

// `ui/**` names these at the crate root.
pub use shell::TITLEBAR_H;
pub use shell::state::State;

fn main() {
    shell::run();
}
