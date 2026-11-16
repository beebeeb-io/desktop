//! Every process the app starts is started under ONE lock (#460 review F11).
//!
//! The Office and media helpers are started with pipe ends that Windows lets a child inherit, for the moment of
//! their start. Any OTHER process started in that moment (an app opening a file, File Explorer) would inherit them
//! too, and could read what a helper reads or writes. So every start in this crate holds the same lock the helpers'
//! start holds: [`spawn`] for a `std::process::Command`, [`locked`] for a system call that starts one
//! (`ShellExecuteW`, `SHOpenWithDialog`, an Open With handler's `Invoke`). `launch_tests.rs` scans the crate for a
//! start outside them.
//!
//! The lock is taken around the start itself, never across an enumeration or a wait for an answer.

use std::io;
use std::process::{Child, Command};

/// The lock every start of the app holds.
///
/// SEAM H1-S (`origin/native/ov-hardening-sandbox`, not pushed when this was written): once it is merged, this body
/// is ONE line, `beebeeb_office_sandbox::launch_lock()`, the lock `Sandbox::spawn` takes before it starts a helper.
/// Until then this is a lock of this crate's own: it orders the app's own starts among themselves, and NOT against a
/// helper's start, so F11 stays open until H1-S is in. A poisoned lock is still a lock (as the sandbox's own).
fn hold() -> impl Sized {
    static LAUNCH: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LAUNCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Start `cmd`, holding the launch lock while it starts.
pub(crate) fn spawn(cmd: &mut Command) -> io::Result<Child> {
    spawn_by(cmd, |cmd| cmd.spawn())
}

/// [`spawn`] with the start itself handed in, so a test can look at the lock from inside it.
pub(crate) fn spawn_by<T>(cmd: &mut Command, start: impl FnOnce(&mut Command) -> io::Result<T>) -> io::Result<T> {
    let _held = hold();
    start(cmd)
}

/// Run `f`, a system call that starts a process, holding the launch lock.
pub(crate) fn locked<T>(f: impl FnOnce() -> T) -> T {
    let _held = hold();
    f()
}

/// Is the launch lock free right now? Asked from ANOTHER thread, waiting at most `wait` for it: `false` when it is
/// held all that time. For the tests only; the app never asks.
#[cfg(test)]
pub(crate) fn free_within(wait: std::time::Duration) -> bool {
    let (told, heard) = std::sync::mpsc::channel();
    let asker = std::thread::spawn(move || {
        let _held = hold();
        let _ = told.send(());
    });
    let free = heard.recv_timeout(wait).is_ok();
    // Not joined: while the lock is held (the caller may hold it) the asker waits for it, and ends once it is let go.
    drop(asker);
    free
}
