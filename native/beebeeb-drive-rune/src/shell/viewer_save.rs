//! The viewer's "Save a copy…" and "Download to…" (`Intent::SaveCopy`, `Intent::DownloadTo`).
//!
//! The press asks the platform's save panel for a place, named as the open file is, on a worker of its own (the panel
//! BLOCKS until it is closed). The same worker then writes the copy through [`engine_host::save_copy`]: the whole save
//! is bound to the session of the press (`CopySaver::turn` at the press). An answer that lands after the session moved
//! on (a lock, a sign-out, another account) writes nothing and never starts a copy; a copy that fails or is stopped
//! leaves no partial file. The poll ([`poll`]) hands the result to the viewer's toast and stops a copy whose session
//! ended.
//!
//! The engine is reached through [`install`]: the app's start gives the shell the saver of its one engine handle (a
//! window without one, a board fixture, says the copy is not available). One save at a time.
//!
//! Must not: log a path or a name. Photos' Save to PC calls [`save_with`] (w18).

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use beebeeb_engine::account::SessionGeneration;
use rune::Tx;

use super::host::log;
use super::platform::{self, Platform, SaveAnswer};
use super::session_gate;
use crate::State;
use crate::engine_host::save_copy::{CopySaver, SaveRefused};
use crate::frames::viewers::{self, Intent};

/// How often the poll looks for the result while a save runs.
const POLL: Duration = Duration::from_millis(250);

const SAVED: &str = "Saved a copy";
const NOT_AVAILABLE: &str = "Not saved: saving a copy is not available here";
const BUSY: &str = "A copy is being saved already";
const NO_PANEL: &str = "Not saved: the save window could not open";

/// The end of a save: what the viewer's toast says.
type Done = Result<(), String>;

/// A save that runs: the session of the press, its stop flag and where the result comes.
struct Running {
    session: SessionGeneration,
    saver: Arc<dyn CopySaver>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Option<Done>>,
}

thread_local! {
    /// The engine's saver (the window's thread).
    static SAVER: RefCell<Option<Arc<dyn CopySaver>>> = const { RefCell::new(None) };
    static RUNNING: RefCell<Option<Running>> = const { RefCell::new(None) };
}

/// The app's start: copies are made by this saver.
pub fn install(saver: Arc<dyn CopySaver>) {
    SAVER.set(Some(saver));
}

/// A save runs from this window's thread.
pub fn running() -> bool {
    RUNNING.with(|r| r.borrow().is_some())
}

/// Takes SaveCopy or DownloadTo; `false`: not taken.
pub fn take(s: &mut State, tx: &mut Tx, intent: Intent) -> bool {
    if !matches!(intent, Intent::SaveCopy | Intent::DownloadTo) {
        return false;
    }
    let Some(saver) = SAVER.with(|saver| saver.borrow().clone()) else {
        say(s, NOT_AVAILABLE);
        return true;
    };
    let Some(name) = viewers::ui(s).map(|ui| ui.file.name.clone()) else {
        return true;
    };
    let Some(id) = viewers::ui(s).map(|ui| ui.file.id.clone()) else {
        return true;
    };
    save_with(s, tx, platform::current(), saver, id, name, start_folder());
    true
}

/// Where the panel opens: the person's Downloads folder when there is one.
fn start_folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Downloads"))
        .unwrap_or_default()
}

/// Asks `platform` for a place for `name` and saves the file `id` there, on a worker, for the session `saver` has now
/// (Photos' Save to PC calls this too). `false`: nothing started (the reason is the viewer's toast, or logged).
pub fn save_with(
    s: &mut State,
    tx: &mut Tx,
    platform: Arc<dyn Platform>,
    saver: Arc<dyn CopySaver>,
    id: String,
    name: String,
    start: PathBuf,
) -> bool {
    if !session_gate::open(s) {
        log("a copy was not started: no session the window may act on");
        return false;
    }
    if running() {
        say(s, BUSY);
        return false;
    }
    let session = saver.turn();
    let cancel = Arc::new(AtomicBool::new(false));
    let (send, done) = mpsc::channel();
    let worker = {
        let (saver, cancel) = (saver.clone(), cancel.clone());
        std::thread::Builder::new().name("save-copy".into()).spawn(move || {
            let _ = send.send(ask_and_save(&*platform, &*saver, session, &cancel, &id, &name, start));
        })
    };
    if worker.is_err() {
        log("the copy could not start");
        say(s, NO_PANEL);
        return false;
    }
    RUNNING.set(Some(Running { session, saver, cancel, done }));
    tx.wake_after(POLL);
    true
}

/// The worker: the panel, then the copy, only while the session of the press stands. `None`: nothing to say (the
/// panel was closed without a place).
fn ask_and_save(
    platform: &dyn Platform,
    saver: &dyn CopySaver,
    session: SessionGeneration,
    cancel: &AtomicBool,
    id: &str,
    name: &str,
    start: PathBuf,
) -> Option<Done> {
    let dest = match platform.pick_save_path(name, start) {
        SaveAnswer::Chosen(dest) => dest,
        SaveAnswer::Cancelled => return None,
        SaveAnswer::Failed => {
            log("the save window did not open");
            return Some(Err(NO_PANEL.into()));
        }
    };
    // An answer under another session writes nothing, and no copy starts.
    if saver.turn() != session {
        log("a chosen place was dropped: the session changed while the save window was open");
        return Some(Err(SaveRefused::NotSignedIn.words()));
    }
    match saver.save(session, id, &dest, &|_, _| {}, cancel) {
        Err(SaveRefused::Cancelled) => None,
        other => Some(other.map_err(SaveRefused::words)),
    }
}

/// The shell's poll: the result of the save that ran goes to the viewer's toast; a save whose session moved on is
/// stopped. `true`: something was taken.
pub fn poll(s: &mut State, tx: &mut Tx) -> bool {
    let step = RUNNING.with(|running| {
        let mut running = running.borrow_mut();
        let run = running.as_mut()?;
        if run.saver.turn() != run.session {
            run.cancel.store(true, Ordering::SeqCst);
        }
        match run.done.try_recv() {
            Err(mpsc::TryRecvError::Empty) => {
                tx.wake_after(POLL);
                None
            }
            got => {
                running.take();
                Some(got.ok().flatten())
            }
        }
    });
    let Some(done) = step else {
        return false;
    };
    match done {
        Some(Ok(())) => say(s, SAVED),
        Some(Err(words)) => say(s, &words),
        None => {}
    }
    true
}

fn say(s: &mut State, words: &str) {
    if let Some(ui) = viewers::ui(s) {
        ui.toast = Some(words.to_string());
    }
}

#[cfg(test)]
#[path = "viewer_save_tests.rs"]
mod tests;
