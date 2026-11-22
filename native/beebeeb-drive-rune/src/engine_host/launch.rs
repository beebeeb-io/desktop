//! Every process the app starts is started under ONE lock (#460 review F11).
//!
//! The Office and media helpers are started with pipe ends that Windows lets a child inherit, for the moment of
//! their start. Any OTHER process started in that moment (an app opening a file, File Explorer) would inherit them
//! too, and could read what a helper reads or writes. So every start in this crate holds the same lock the helpers'
//! start holds: [`spawn`] for a `std::process::Command`, [`locked`] for a system call that starts one
//! (`ShellExecuteW`, `SHOpenWithDialog`, an Open With handler's `Invoke`). `launch_tests.rs` scans the crate for a
//! start outside them.
//!
//! The lock is taken around the start itself, never across an enumeration or a wait for an answer. It is NOT
//! re-entrant: nothing that holds it starts another process.
//!
//! The window's thread never waits for it: Windows' app chooser holds it for as long as it is open (it
//! starts the app picked inside its own call), so the starts the window asks for go to one worker, [`Later`].

#[cfg(any(test, windows))]
use std::io;
#[cfg(windows)]
use std::process::Child;
#[cfg(any(test, windows))]
use std::process::Command;
use std::sync::mpsc;

/// The lock every start of the app holds: the one `Sandbox::spawn` takes before it starts a helper (H1-S).
fn hold() -> beebeeb_office_sandbox::LaunchGuard {
    beebeeb_office_sandbox::launch_lock()
}

/// Start `cmd`, holding the launch lock while it starts.
#[cfg(windows)]
pub(crate) fn spawn(cmd: &mut Command) -> io::Result<Child> {
    spawn_by(cmd, |cmd| cmd.spawn())
}

/// Start `cmd` with `start`, holding the launch lock while it starts; handed in so a test can look at the lock from inside it.
#[cfg(any(test, windows))]
pub(crate) fn spawn_by<T>(cmd: &mut Command, start: impl FnOnce(&mut Command) -> io::Result<T>) -> io::Result<T> {
    let _held = hold();
    start(cmd)
}

/// Run `f`, a system call that starts a process, holding the launch lock.
pub(crate) fn locked<T>(f: impl FnOnce() -> T) -> T {
    let _held = hold();
    f()
}

type Job = Box<dyn FnOnce() + Send>;

/// One worker for the starts the window asks for (Open, Show in File Explorer): they run there one at a time, in the
/// order asked, each under the launch lock (its start takes it). The window's thread hands a start over and returns:
/// it never waits for the lock. The worker starts with the first start, and again if it ever ended.
#[derive(Default)]
pub(crate) struct Later {
    jobs: Option<mpsc::Sender<Job>>,
}

impl Later {
    /// Hand `job` to the worker. `Err`: no worker could be started; nothing will run.
    pub(crate) fn run(&mut self, job: impl FnOnce() + Send + 'static) -> Result<(), ()> {
        let mut job: Job = Box::new(job);
        for _ in 0..2 {
            let jobs = match &self.jobs {
                Some(jobs) => jobs.clone(),
                None => {
                    let (jobs, inbox) = mpsc::channel::<Job>();
                    std::thread::Builder::new()
                        .name("files-launch".into())
                        .spawn(move || {
                            while let Ok(job) = inbox.recv() {
                                job();
                            }
                        })
                        .map_err(|_| ())?;
                    self.jobs = Some(jobs.clone());
                    jobs
                }
            };
            match jobs.send(job) {
                Ok(()) => return Ok(()),
                // The worker ended (a start panicked): start another, once.
                Err(mpsc::SendError(back)) => {
                    job = back;
                    self.jobs = None;
                }
            }
        }
        Err(())
    }
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
