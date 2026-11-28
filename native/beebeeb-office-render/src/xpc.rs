//! macOS: the helper as its app's XPC service (`Contents/XPCServices/<app id>.office-render.xpc`).
//!
//! The system starts the service in a sandbox of its own, made from its one entitlement
//! (`packaging/macos/office-render.entitlements`). Its main thread belongs to the system: `xpc_main` takes it and
//! hands every new connection to the sandbox crate's handler (`beebeeb_office_sandbox_service_peer`), which takes the
//! first one as the document's and cancels the rest. The helper's own work runs on a second thread and starts, as in
//! every helper, with `beebeeb_office_sandbox::helper::confine`, which in a service returns once the app's first
//! message has given it its two pipes and its memory limit and the service has told the app how it is confined.
//!
//! No block is written here: `xpc_main` takes a plain C function. The one block the service needs (its connection's
//! event handler) is a global block literal laid out in the sandbox crate (`macos_xpc.rs`): no crate, no C file.
#![allow(unsafe_code)]

use std::ffi::c_void;

unsafe extern "C" {
    /// `<xpc/xpc.h>`: runs the service's connection handler on the system's queue; never returns.
    fn xpc_main(handler: unsafe extern "C" fn(*mut c_void)) -> !;
    /// The sandbox crate's exports (`macos_service.rs`): whether this process sits in an XPC service's bundle, and
    /// the handler of its new connections.
    fn beebeeb_office_sandbox_is_service() -> bool;
    fn beebeeb_office_sandbox_service_peer(peer: *mut c_void);
}

/// Whether this process was started as its app's XPC service (its executable is inside a `.xpc` bundle).
pub fn is_service() -> bool {
    // SAFETY: takes nothing, reads only this process's own executable path.
    unsafe { beebeeb_office_sandbox_is_service() }
}

/// Runs `work` on a thread of its own and gives the main thread to the system. The process ends with `work`'s code,
/// or with code 0 when the app closes the connection first (the sandbox crate's handler).
pub fn run_as_service(work: fn() -> i32) -> ! {
    let started = std::thread::Builder::new()
        .name("office-render".into())
        .spawn(move || std::process::exit(work()));
    if started.is_err() {
        std::process::exit(i32::from(beebeeb_office_sandbox::helper::UNCONFINED_EXIT_CODE));
    }
    // SAFETY: the handler is the sandbox crate's connection handler, a C function that lives for the whole process.
    unsafe { xpc_main(beebeeb_office_sandbox_service_peer) }
}
