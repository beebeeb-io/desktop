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
//!
//! Decision (lane, 2026-11-12): release and TEST builds on Windows use the GUI subsystem (PE 2)
//! so Windows does not attach a console window beside the app. Debug builds keep the console
//! subsystem (PE 3) for developer convenience and so compare.sh's debug render path keeps its
//! stdout/stderr visible in the terminal. The cfg_attr guard is `all(windows,
//! not(debug_assertions))`, which is true for every --release build and false for every debug
//! build, matching the profile used for both the installer TEST binary and the shipped binary.
//! eprintln! output is still delivered to a redirected handle (pipes, Start-Process
//! -RedirectStandardError) under a GUI-subsystem binary; only an unredirected terminal drops it.

// Release builds on Windows: GUI subsystem (PE 2) — no console window beside the app.
// Debug builds retain the console subsystem for developer convenience.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(test)]
#[path = "../build_version.rs"]
mod build_version;
mod engine_host;
#[cfg(feature = "feedback")]
mod feedback;
mod frames;
mod log_file;
mod media;
mod shell;
mod source;
// The engine lane's read API (PR 280): only `EngineSource::open` and `list` have a caller
// outside its tests so far (`--db`, dev builds).
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
mod source_engine;
mod text_kit;
mod theme;
mod ui;
mod uninstall_hook;
mod windows_floor;

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
    // drive-complete-006b: the uninstaller's sync-folder teardown, before any state, window or control channel.
    if let Some(code) = uninstall_hook::run(std::env::args().skip(1)) {
        std::process::exit(code);
    }
    // drive-complete-138: a panic leaves one redacted record in the log folder (`log_file::crash`).
    log_file::crash::install_hook();
    // Test builds only: a crash on demand, for the installed-build check of the record.
    if !shell::identity::engine_identity().is_real()
        && std::env::var_os("BEEBEEB_TEST_CRASH").is_some()
    {
        std::thread::spawn(|| {
            while log_file::dir().is_none() {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
            panic!("test crash");
        });
    }
    shell::run();
}
