//! The Office helper's executable. It takes no argument, reads no environment variable and opens no file: the
//! sandbox hands it two pipes, and [`serve`] answers on them until the app closes its end.
//!
//! On macOS the helper confines itself before anything else (no network, no file but the system's fonts, no write,
//! no new process), whoever started it, and ends if that does not take effect: it never reads a frame unconfined.
//! Inside the signed app it is the app's XPC service instead (`xpc.rs`): the system confines it, and the app checks
//! that before it sends anything. On Windows the sandbox crate confines it from outside before it runs, and the call
//! does nothing.

use beebeeb_office_render::{Granted, Office, serve, start};

#[cfg(target_os = "macos")]
mod xpc;

fn main() {
    start();
    #[cfg(target_os = "macos")]
    if xpc::is_service() {
        xpc::run_as_service(work);
    }
    std::process::exit(work())
}

/// Confines this process, then answers on the two pipes until the app closes its end.
fn work() -> i32 {
    beebeeb_office_sandbox::helper::confine();
    let code = serve(
        Box::new(std::io::stdin()),
        &mut std::io::stdout().lock(),
        &Granted,
        &Office,
    );
    code.into()
}
