//! The Office viewer's window proof: page 1 of an Office document, rendered by the confined helper, shown in a Rune
//! window, with the helper alive for as long as the window is open.
//!
//! ```text
//! office_page <file.docx> [--zoom <percent>] [--dpi <n>] [--helper <exe>]
//! ```
//!
//! It opens the file through the app's own host (`native/beebeeb-drive-rune/src/engine_host/office_host.rs`, compiled
//! in here as it is), asks for page 1 at `check::render_scale(zoom / 100, dpi)` (defaults: 100 %, 96 DPI), prints ONE
//! line and then shows the page:
//!
//! ```text
//! office_page: state=<variant> pages=<n> first_page_ms=<t> w=<w> h=<h> missing_content=<bool> helper_pid=<pid>
//! ```
//!
//! No file name and no text of the document is printed. `helper_pid` is the live helper's process id (0 when there is
//! none). A state other than `Pages` (or no page within 60 s) prints that state and exits 1. It is a development tool
//! for test fixtures only; the app never reads a file this way.
//!
//! The host stays open while the window is up, so the helper does too: it is closed (and the helper killed) when the
//! window has closed and Rune's event loop has returned. Were the process to end any other way, the helper ends with
//! it: its job is closed with the process.
//!
//! How the lead runs it on the Windows test machine (lanes cannot drive PowerShell there), from the desktop session.
//! It builds in release: this package links the engine without test support (the integration lane writes its lockfile):
//!
//! ```text
//! cargo build --release --locked                                    # in native/beebeeb-office-render
//! cargo run --release --locked -- <fixtures>/system-font.docx --zoom 100 --helper <release>/beebeeb-office-render.exe
//!                                                                   # in native/beebeeb-office-page
//! ```
//!
//! The O-CP3 rung (the helper lives exactly as long as the open document): while the window shows the page, count the
//! processes named `beebeeb-office-render` (1) and check that the printed `helper_pid` is one of them; close the window,
//! and count again (0, and that PID gone).
//!
//! Without `--helper` the helper is looked for beside this program's executable (`office_host::helper_exe`). The helper
//! starts confined exactly as in the app: off Windows this prints `state=NoPreview(NoSandbox)` and exits 1.

// The test build of this package is not a test of anything: `office_host` carries its own unit tests, and they need the
// engine's test support and `tempfile` (dev-dependencies of beebeeb-drive-rune, where they run). So the program is
// included for every build but a test build (`cargo check --all-targets`), which gets an empty `main`.
#[cfg(not(test))]
include!("page.rs");

#[cfg(test)]
fn main() {}
