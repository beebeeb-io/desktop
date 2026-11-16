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
//!   (`FilesSource::forget_starts`, from `frames/files/reset.rs`), and so do the queues of the session
//!   [`super::launcher::Launcher`]s (tray, viewer): what the ended session asked for is dropped, not kept until the
//!   lock is let go.

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

/// The launch worker's starts not begun yet, in the order asked, each under the number of the job that runs it. A
/// session end empties it ([`Pending::forget`]).
#[derive(Clone, Default)]
pub(super) struct Pending(Arc<Mutex<Queue>>);

/// The queues a session end empties besides the one asked: every session [`super::launcher::Launcher`]'s (the tray's,
/// the viewer's), so one session end drops all of the ended session's starts, not only Files'. In a test only the
/// queues joined by the same thread (tests run in parallel and each is its own session).
static JOINED: Mutex<Vec<(std::thread::ThreadId, std::sync::Weak<Mutex<Queue>>)>> = Mutex::new(Vec::new());

#[derive(Default)]
struct Queue {
    next: u64,
    starts: VecDeque<(u64, Job)>,
}

impl Pending {
    fn held(&self) -> MutexGuard<'_, Queue> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The start queued under `number`, taken out. `None`: a session end dropped it.
    fn take(&self, number: u64) -> Option<Job> {
        let mut queue = self.held();
        let at = queue.starts.iter().position(|(queued, _)| *queued == number)?;
        queue.starts.remove(at).map(|(_, start)| start)
    }

    /// Queue `start`, then hand the worker the one job that runs it, by its number (a job never runs another press's
    /// start, also not after a worker that ended lost the jobs it held). `Err`: the worker could not be started;
    /// `start` is taken back out, and nothing will run.
    pub(super) fn run_later(
        &self,
        later: &mut super::launch::Later,
        start: impl FnOnce() + Send + 'static,
    ) -> Result<(), ()> {
        let number = {
            let mut queue = self.held();
            let number = queue.next;
            queue.next += 1;
            queue.starts.push_back((number, Box::new(start)));
            number
        };
        let pending = self.clone();
        let handed = later.run(move || {
            // Taken out first: the queue is not held while the start waits for the launch lock.
            let ours = pending.take(number);
            if let Some(start) = ours {
                start();
            }
        });
        if handed.is_err() {
            let _ours = self.take(number);
        }
        handed
    }

    /// This queue is emptied by every session end ([`Pending::forget`], whichever queue it is called on).
    pub(super) fn join_session_end(&self) {
        let me = std::thread::current().id();
        let mut joined = JOINED.lock().unwrap_or_else(PoisonError::into_inner);
        joined.retain(|(_, queue)| queue.strong_count() > 0);
        if !joined.iter().any(|(_, queue)| queue.as_ptr() == Arc::as_ptr(&self.0)) {
            joined.push((me, Arc::downgrade(&self.0)));
        }
    }

    /// A session ended: every start not begun yet goes, in this queue and in every joined one. A start that is
    /// running already compares its session inside the launch lock.
    pub(super) fn forget(&self) {
        let me = std::thread::current().id();
        let others: Vec<_> = JOINED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(thread, _)| !cfg!(test) || *thread == me)
            .filter_map(|(_, queue)| queue.upgrade())
            .collect();
        for queue in others.iter().chain(std::iter::once(&self.0)) {
            let gone = std::mem::take(&mut queue.lock().unwrap_or_else(PoisonError::into_inner).starts);
            drop(gone);
        }
    }

    /// How many starts wait.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.held().starts.len()
    }
}
