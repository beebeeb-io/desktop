//! Files' starts, each bound to the session of its press (review of #467, P3-1).
//!
//! Open, Show in File Explorer and Open in File Explorer start on the launch worker (`launch::Later`, ruling R-OW1),
//! and Open With ▸ an app and ▸ Other… on a thread of their own. Each may wait for the launch lock: Windows' app
//! chooser holds it for as long as it is open. Two rules keep a start from acting for a session that ended meanwhile:
//!
//! - Each start asks "is the session of the press still the one now?" INSIDE the launch lock, right before it starts
//!   (`still`, handed to every start below), and starts nothing when it moved. Every lock, sign-out, sign-in and
//!   unlock moves the session's turn (`Engine::session_turn`).
//! - The launch worker's starts wait their turn in [`Pending`], which a session end empties
//!   (`FilesSource::forget_starts`, from `frames/files/reset.rs`): what the ended session asked for is dropped, not
//!   kept until the lock is let go.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::frames::files::source::ActionError;

/// A start of a file or a folder: it takes the launch lock itself, and starts only while `still()` holds inside it.
pub(super) type Start = fn(&Path, &dyn Fn() -> bool) -> Result<(), ActionError>;

/// What starts a file or a folder: the system's own ([`super::open_path`], [`super::reveal_in_file_explorer`],
/// Open With's handler, [`super::open_with_apps::SYSTEM`]); a test's stand-in.
#[derive(Clone, Copy)]
pub(super) struct Starts {
    pub(super) open: Start,
    pub(super) reveal: Start,
    /// Open With ▸ an app, on the copy kept on this PC. `None`: this system has no handlers to start.
    pub(super) app: Option<super::open_with_apps::Invoke>,
}

impl Default for Starts {
    fn default() -> Self {
        Starts {
            open: super::open_path,
            reveal: super::reveal_in_file_explorer,
            app: super::open_with_apps::SYSTEM,
        }
    }
}

type Job = Box<dyn FnOnce() + Send>;

/// The launch worker's starts not begun yet, oldest first. Each job handed to the worker takes the oldest one; a
/// session end empties it ([`Pending::forget`]).
#[derive(Clone, Default)]
pub(super) struct Pending(Arc<Mutex<VecDeque<Job>>>);

impl Pending {
    fn held(&self) -> MutexGuard<'_, VecDeque<Job>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue `start`, then hand the worker one job that runs the oldest start queued. `Err`: the worker could not be
    /// started; `start` is taken back out, and nothing will run.
    pub(super) fn run_later(
        &self,
        later: &mut super::launch::Later,
        start: impl FnOnce() + Send + 'static,
    ) -> Result<(), ()> {
        self.held().push_back(Box::new(start));
        let pending = self.clone();
        let handed = later.run(move || {
            // Bound before it runs: the queue is not held while a start waits for the launch lock.
            let oldest = pending.held().pop_front();
            if let Some(start) = oldest {
                start();
            }
        });
        if handed.is_err() {
            // The newest is ours: the worker takes from the front, and no job was handed for this one.
            let _ours = self.held().pop_back();
        }
        handed
    }

    /// A session ended: every start not begun yet goes. A start that is running already compares its session inside
    /// the launch lock.
    pub(super) fn forget(&self) {
        let gone = std::mem::take(&mut *self.held());
        drop(gone);
    }

    /// How many starts wait.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.held().len()
    }
}
