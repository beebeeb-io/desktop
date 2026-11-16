//! The starts the window asks for outside Files: the tray's and the menu's Open in File Explorer, a page opened in
//! the browser, the viewer's Open in another app (ruling R-OW1).
//!
//! Windows' app chooser holds the launch lock for as long as it is open, so none of them starts on the window's
//! thread: each is handed to the launch worker (`launch::Later`, FIFO with the other starts of its [`Launcher`]) and
//! the window's thread returns at once. The start asks "is the session of the press still the one now?" INSIDE the
//! launch lock (`still`, as Files' starts do, `files_launch`), starts nothing when it moved, and says nothing of it.
//! What the start answered reaches `done` on the worker, only while the session still holds.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use super::files_launch::{Pending, Start};
use crate::frames::files::source::ActionError;
use beebeeb_engine::app::{CredentialStores, Engine};

/// One launch worker and its queue. Clones share both.
#[derive(Clone, Default)]
pub(super) struct Launcher {
    later: Arc<Mutex<super::launch::Later>>,
    pending: Pending,
}

impl Launcher {
    /// Start `path` with `start` on the worker, for the press whose session `still` tells about. `done` gets what
    /// the start answered, on the worker, unless the session moved on (then nothing is said). `Err`: no worker could
    /// be started; nothing will run.
    pub(super) fn open(
        &self,
        start: Start,
        path: PathBuf,
        still: impl Fn() -> bool + Send + 'static,
        done: impl FnOnce(Result<(), ActionError>) + Send + 'static,
    ) -> Result<(), ()> {
        let job = move || {
            let answer = start(&path, &still);
            if still() {
                done(answer);
            }
        };
        let mut later = self.later.lock().unwrap_or_else(PoisonError::into_inner);
        self.pending.run_later(&mut later, job)
    }

    /// [`Launcher::open`] for a press of a session: the queue is one with Files' (`Pending::forget`), so a session end
    /// drops what is not begun yet here too. A page has no session and uses [`Launcher::open`].
    pub(super) fn open_session(
        &self,
        start: Start,
        path: PathBuf,
        still: impl Fn() -> bool + Send + 'static,
        done: impl FnOnce(Result<(), ActionError>) + Send + 'static,
    ) -> Result<(), ()> {
        self.pending.join_session_end();
        self.open(start, path, still, done)
    }

    /// [`Launcher::open`] for the session `engine` is in NOW (the press's): the start compares it inside the lock, and a
    /// failure that is still this session's is logged as `failed`.
    pub(super) fn open_for<C: CredentialStores + 'static>(
        &self,
        engine: &Arc<Engine<C>>,
        start: Start,
        path: PathBuf,
        failed: &'static str,
    ) -> Result<(), ()> {
        let (engine, turn) = (engine.clone(), engine.session_turn());
        self.open_session(
            start,
            path,
            move || engine.session_turn() == turn,
            move |answer| {
                if answer.is_err() {
                    super::log(failed);
                }
            },
        )
    }

    /// How many starts wait.
    #[cfg(test)]
    pub(super) fn waiting(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod session_end_tests {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    static STARTED: AtomicUsize = AtomicUsize::new(0);
    fn counting(_: &Path, _: &dyn Fn() -> bool) -> Result<(), ActionError> {
        STARTED.fetch_add(1, SeqCst);
        Ok(())
    }

    /// A session end (Files' `forget`) empties the queues of the tray's and the viewer's launchers too; a page's queue
    /// (no session) is kept.
    #[test]
    fn a_session_end_empties_every_session_launcher_queue_but_not_the_pages() {
        let (tray, viewer, page) = (Launcher::default(), Launcher::default(), Launcher::default());
        let files = Pending::default();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (holding, locked) = std::sync::mpsc::channel();
        let chooser = std::thread::spawn(move || {
            super::super::launch::locked(|| {
                holding.send(()).unwrap();
                let _ = held.recv_timeout(std::time::Duration::from_secs(30));
            })
        });
        locked.recv().unwrap();
        for l in [&tray, &viewer] {
            // Two: the first is taken by the worker and waits for the lock, the second waits in the queue.
            for _ in 0..2 {
                l.open_session(counting, "/x".into(), || true, |_| {}).unwrap();
            }
        }
        page.open(counting, "/p".into(), || true, |_| {}).unwrap();
        page.open(counting, "/p".into(), || true, |_| {}).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let (t, v, p) = (tray.waiting(), viewer.waiting(), page.waiting());
        files.forget();
        assert_eq!((tray.waiting(), viewer.waiting()), (0, 0), "was {t} and {v}");
        assert_eq!(page.waiting(), p, "a page has no session");
        release.send(()).unwrap();
        chooser.join().unwrap();
    }
}

#[cfg(test)]
#[path = "launcher_tests.rs"]
mod launcher_tests;
