//! The Office helper's executable. It takes no argument, reads no environment variable and opens no file: the
//! sandbox hands it two pipes, and [`serve`] answers on them until the app closes its end.
//!
//! On macOS the helper confines itself before anything else (no network, no file but the system's fonts, no write,
//! no new process), whoever started it, and ends if that does not take effect: it never reads a frame unconfined.
//! On Windows the sandbox crate confines it from outside before it runs, and the call does nothing.

use beebeeb_office_render::{Granted, Office, serve, start};

fn main() {
    beebeeb_office_sandbox::helper::confine();
    start();
    let code = serve(
        Box::new(std::io::stdin()),
        &mut std::io::stdout().lock(),
        &Granted,
        &Office,
    );
    std::process::exit(code.into())
}
