//! The Files window's "Downloading" rows (DW-0412) on the real engine: [`DownloadCalls`], the part of
//! `EngineFilesSource` behind `downloads`.
//!
//! - What the rows show is ONE read of the engine ([`DownloadsEngine::downloads_for`]) held here with the session and
//!   account it was read under. [`DownloadCalls::downloads`] only answers from what is held, never reads (it is on a
//!   draw path), and says nothing under another stamp.
//! - A read is asked on a status press, after a copy was kept on this PC, and when a stamp is first seen (a watch of
//!   [`WATCH`] starts then and at every ask: the wake reads while it lasts). In [`DownloadCalls::take`], on the
//!   engine's wake, it reads again (a) at once, whatever the held read's age, when the engine's own status word moved
//!   while rows are held or it moved to Syncing: a copy that ended leaves the row at the pulse that says so (the
//!   pattern of `files_facts`); (b) while the engine reports Syncing or rows are held, when the held read is older
//!   than a second: a download that starts after the watch still gets its bar. Idle and nothing held: no read. The
//!   engine wakes the host when a download row begins, fails or leaves the board (`TransferBoard::set_wake`), so
//!   Paused, Offline and Sync stopped, which send no pulse, read on that wake too. No timer thread.
//! - An answer under another session or account, or of an earlier forget, is dropped. Nothing is drawn before the
//!   engine answered: the figures are the board's, never a made-up bar.
//!
//! Must not: block the window; log or print a name or a file id.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::downloads_api::DownloadRow;
use beebeeb_engine::app::{CredentialStores, Engine, LocalStateRefused};

use super::log;
use super::stamp::{Stamp, stamp};
use crate::frames::files::keeping::Download;
use crate::ui::status_line as words;

#[cfg(test)]
#[path = "files_downloads_tests.rs"]
mod tests;

/// How old a held read may be before the engine's wake reads again, while a copy comes down.
const STALE: Duration = Duration::from_secs(1);
/// After an ask, how long the wake keeps reading although nothing came down yet: a kept copy starts a moment later.
const WATCH: Duration = Duration::from_secs(30);

/// What the calls need of the engine: the real handle, or a double in a test.
pub trait DownloadsEngine: Send + Sync + 'static {
    fn stamp(&self) -> Stamp;
    fn has_session(&self) -> bool;
    fn downloads_for(&self, session: SessionGeneration) -> Result<Vec<DownloadRow>, LocalStateRefused>;
    /// The engine's own status word (`sync::sync_word` of its view), read without waiting.
    fn word(&self) -> Option<&'static str>;
    fn wake(&self);
}

impl<C: CredentialStores> DownloadsEngine for Engine<C> {
    fn stamp(&self) -> Stamp {
        stamp(self)
    }
    fn has_session(&self) -> bool {
        Engine::has_session(self)
    }
    fn downloads_for(&self, session: SessionGeneration) -> Result<Vec<DownloadRow>, LocalStateRefused> {
        Engine::downloads_for(self, session)
    }
    fn word(&self) -> Option<&'static str> {
        super::sync::sync_word(&self.view())
    }
    fn wake(&self) {
        self.wake_host();
    }
}

struct Answer {
    pressed: Stamp,
    epoch: u64,
    read: Result<Vec<DownloadRow>, LocalStateRefused>,
}

/// See the module's doc.
pub struct DownloadCalls<E: DownloadsEngine> {
    engine: Arc<E>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    /// The session and account the last look saw: when they move, everything held goes.
    seen: Option<Stamp>,
    /// The last read, with the stamp it was made under and when it landed.
    held: Option<(Stamp, Vec<DownloadRow>, Instant)>,
    /// A read is on its way.
    reading: bool,
    /// Until when the wake keeps reading although nothing comes down.
    watch_until: Option<Instant>,
    /// Moves at every forget: an answer of an earlier one is dropped.
    epoch: u64,
    /// How long a watch lasts.
    watch: Duration,
    /// The engine's status word the last take saw.
    word: Option<&'static str>,
    /// A read is wanted: the engine's word moved while rows were held (or to Syncing) and none could start yet.
    dirty: bool,
}

impl<E: DownloadsEngine> DownloadCalls<E> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<E>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            held: None,
            reading: false,
            watch_until: None,
            epoch: 0,
            watch: WATCH,
            word: None,
            dirty: false,
        }
    }

    #[cfg(test)]
    fn with_watch(mut self, watch: Duration) -> Self {
        self.watch = watch;
        self
    }

    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.held = None;
        self.reading = false;
        self.dirty = false;
        // The first sighting under a stamp is no move of the word.
        self.word = self.engine.word();
        // A copy may already come down under this session: the wake reads for a while.
        self.watch_until = Some(Instant::now() + self.watch);
    }

    /// Read now (a status press, a copy that was kept) and keep watching for a while. One read at a time.
    pub fn ask(&mut self) {
        let now = self.engine.stamp();
        self.look(&now);
        self.watch_until = Some(Instant::now() + self.watch);
        self.start_read(now);
    }

    fn start_read(&mut self, pressed: Stamp) {
        if self.reading || !self.engine.has_session() {
            return;
        }
        self.reading = true;
        self.dirty = false;
        let session = pressed.0;
        let (engine, answers, epoch) = (self.engine.clone(), self.answers.clone(), self.epoch);
        let started = std::thread::Builder::new()
            .name("files-downloads".into())
            .spawn(move || {
                let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.downloads_for(session)))
                    .unwrap_or_else(|_| {
                        log("files: a downloads read ended early");
                        Err(LocalStateRefused::Unreadable)
                    });
                let _ = answers.send(Answer { pressed, epoch, read });
                engine.wake();
            });
        if started.is_err() {
            self.reading = false;
            log("files: could not start the worker");
        }
    }

    /// The copies coming down: from the held read only, and only under the stamp it was read for.
    pub fn downloads(&self) -> Vec<Download> {
        let Some((read_under, rows, _)) = self.held.as_ref() else {
            return Vec::new();
        };
        if *read_under != self.engine.stamp() {
            return Vec::new();
        }
        rows.iter()
            .map(|row| Download {
                id: row.file_id.clone(),
                done: row.done,
                total: row.total,
                kept: row.kept,
            })
            .collect()
    }

    /// Drop everything held. Nothing is sent to the engine.
    pub fn forget(&mut self) {
        self.held = None;
        self.reading = false;
        self.dirty = false;
        self.watch_until = None;
        // Whatever is on its way now carries the earlier epoch: dropped.
        self.epoch += 1;
    }

    /// Take in what the worker answered, and read again while a copy comes down (or is expected to).
    pub fn take(&mut self) {
        let now = self.engine.stamp();
        self.look(&now);
        let here = self.engine.has_session();
        while let Ok(answer) = self.inbox.try_recv() {
            if !(here && answer.pressed == now && answer.epoch == self.epoch) {
                continue;
            }
            self.reading = false;
            match answer.read {
                Ok(rows) => self.held = Some((now.clone(), rows, Instant::now())),
                // An unreadable board is held as "nothing known" (no bar), so a failing read is not asked again at once.
                Err(refused) => {
                    // Only a state that could not be read is logged: a locked screen, a new account with nothing
                    // synced yet and a session that moved are the ordinary refusals.
                    if refused == LocalStateRefused::Unreadable {
                        log("files: the downloads could not be read");
                    }
                    self.held = Some((now.clone(), Vec::new(), Instant::now()));
                }
            }
        }
        let word = self.engine.word();
        let syncing = word == Some(words::SYNCING);
        let holding = self.held.as_ref().is_some_and(|(_, rows, _)| !rows.is_empty());
        if word != self.word && (holding || syncing) {
            self.dirty = true;
        }
        self.word = word;
        let watching = self.watch_until.is_some_and(|until| Instant::now() < until);
        let old = self.held.as_ref().is_none_or(|(_, _, at)| at.elapsed() > STALE);
        // Idle ("Up to date") with nothing held and no watch: no read. Every other word, Paused, Offline and Sync
        // stopped included, reads on a wake (age-gated): the engine wakes the host when a download row begins or
        // leaves the board, as it sends no pulse then.
        let idle = word == Some(words::UP_TO_DATE);
        let again = self.dirty || (old && (holding || watching || !idle));
        if again {
            self.start_read(now);
        }
    }
}

/// The status line's word for the engine's own, with the copies: never "Up to date" while one comes down (DW-0412).
pub fn with_downloads(word: Option<&'static str>, coming_down: bool) -> Option<&'static str> {
    match word {
        Some(words::UP_TO_DATE) if coming_down => Some(words::SYNCING),
        other => other,
    }
}
