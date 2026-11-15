//! What the helper sets up before it reads its first frame.
//!
//! - A panic hook that exits with [`EXIT_PANIC`] and writes nothing: a panic's message can quote text from the
//!   document, and the release profile's abort would end in a crash report. The hook never looks at the panic.
//! - No logger, and the log level is left as it starts (off): the readers' warning lines name parts and link targets
//!   taken from the file, and with no logger they go nowhere.

use crate::EXIT_PANIC;

/// Installs the panic hook. Call once, first thing in `main`.
pub fn start() {
    std::panic::set_hook(Box::new(|_| std::process::exit(i32::from(EXIT_PANIC))));
}
