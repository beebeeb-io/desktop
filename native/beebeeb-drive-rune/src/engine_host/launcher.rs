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
        self.open(
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
#[path = "launcher_tests.rs"]
mod launcher_tests;
