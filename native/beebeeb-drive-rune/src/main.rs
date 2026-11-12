//! Beebeeb Drive — the native app (Rune, no web view).
//!
//! This file is the entry point and nothing else: it names the crate's modules and hands over
//! to the shell. Everything about lifetime, windows, the tray and routing is in `shell/`
//! (`src/shell/README.md`); the root state is `shell::state::State`; the Files window
//! (DW-0401) is `ui/` (`ui::window::files_window`), registered in `shell/registry.rs`.
//!
//! ```sh
//! cargo run -p beebeeb-drive-rune
//! cargo run -p beebeeb-drive-rune -- --snap DW-0401.png --scene DW-0401 --size 1280x800 --scale 2
//! ```
//!
//! Lanes: add ONE `mod` line for your module below, in alphabetical order, and nothing else.

mod engine_host;
#[cfg(feature = "feedback")]
mod feedback;
mod frames;
mod shell;
mod source;
// The engine lane's read API (PR 280): only `EngineSource::open` and `list` have a caller
// outside its tests so far (`--db`, dev builds).
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
mod source_engine;
mod theme;
mod ui;

// `ui/**` names these at the crate root.
pub use shell::registry::view;
pub use shell::state::{State, init, load};
pub use shell::{FAMILIES, FONTS, TITLEBAR_H};

fn main() {
    // The packager reads this build's identity out of the binary (`shell/identity.rs`, `BUILD_MARK`).
    std::hint::black_box(shell::identity::BUILD_MARK);
    // The test build does not run on macOS (`shell/identity.rs`, `start_refusal`). Asked before any state,
    // window or control channel exists.
    if let Some(refusal) = shell::identity::AppIdentity::CURRENT.start_refusal(std::env::consts::OS) {
        eprintln!("{refusal}");
        std::process::exit(2);
    }
    shell::run();
}
