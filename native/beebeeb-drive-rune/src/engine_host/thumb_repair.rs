//! drive-complete-RS-d: "Make missing thumbnails" (Settings > Thumbnails and previews). Files uploaded where no
//! thumbnail was made (a web upload of a folder of photos) show only a placeholder on every device. This run walks the
//! account's pictures with no server thumbnail ([`Engine::files_missing_thumbnails_for`]), downloads each into memory,
//! makes its thumbnails from the bytes ([`super::thumbs::generate_for`], the media helper decodes) and sends them
//! sealed ([`Engine::upload_generated_thumbnails_for`]), at most [`AT_ONCE`] files at a time.
//!
//! - Holds: before every download the engine is asked whether a thumbnail may be sent now (the preview setting, a
//!   metered-network or battery-saver hold, a paused sync, the session): a hold ends the run and says why. Nothing is
//!   downloaded for a thumbnail that would not be sent.
//! - Once: a file is handled at most once in a run (a list that names it twice, two workers), whatever came of it.
//! - Stop: no new file is begun once Stop is pressed; the ones in hand end and the run says it stopped.
//! - A file whose has_thumbnail is not known is asked of the server first: one the server has a thumbnail of is left.
//!
//! Must not: log an id, a name or a byte; keep a downloaded byte past its file (the engine's buffer is wiped).

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::photo_previews_api::MissingThumb;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::photo_previews::thumbs_upload::NotNow;
use beebeeb_engine::power::PausedFor;
use beebeeb_media_protocol::image::MAX_IMAGE_FILE;

use super::log;
use super::thumbs::{Generate, MadeUpload, ThumbMaker, generate_for, make_from_bytes};

/// How many files are downloaded and made at once.
pub const AT_ONCE: usize = 2;

/// The engine as the repair asks it.
pub trait RepairEngine: ThumbMaker {
    /// The session in memory now.
    fn session(&self) -> SessionGeneration;
    /// Wake the window: the run moved on.
    fn wake(&self);
    /// [`Engine::thumbnail_send_hold_for`]: why nothing may be sent now; `None`: it may.
    fn hold(&self, session: SessionGeneration) -> Option<MadeUpload>;
    /// [`Engine::files_missing_thumbnails_for`].
    fn missing(&self, session: SessionGeneration) -> Option<Vec<MissingThumb>>;
    /// The file's bytes in memory for `session` (the engine's download, decrypted; wiped when dropped). `None`: not
    /// read (offline, too large, another session, stopped).
    fn download(
        &self,
        session: SessionGeneration,
        file_id: &str,
        stop: &AtomicBool,
    ) -> Option<zeroize::Zeroizing<Vec<u8>>>;
}

impl<C: CredentialStores + 'static> RepairEngine for Engine<C> {
    fn session(&self) -> SessionGeneration {
        self.session_turn()
    }

    fn wake(&self) {
        self.wake_host();
    }

    fn hold(&self, session: SessionGeneration) -> Option<MadeUpload> {
        self.thumbnail_send_hold_for(session)
    }

    fn missing(&self, session: SessionGeneration) -> Option<Vec<MissingThumb>> {
        self.files_missing_thumbnails_for(session).ok()
    }

    fn download(
        &self,
        session: SessionGeneration,
        file_id: &str,
        stop: &AtomicBool,
    ) -> Option<zeroize::Zeroizing<Vec<u8>>> {
        let opened = self
            .open_file_in_memory(file_id, MAX_IMAGE_FILE, &|_, _| {}, stop)
            .ok()?;
        (opened.session == session).then_some(opened.bytes)
    }
}

/// Where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    /// Never started in this app's life.
    #[default]
    Idle,
    /// Reading which files have none.
    Listing,
    Running,
    /// Every file was handled.
    Done,
    /// Stop was pressed.
    Stopped,
    /// A hold or a failure ended it; the words say which.
    Held(&'static str),
}

/// What the pane draws of the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Status {
    pub phase: Phase,
    /// Files handled, of `total`.
    pub done: u32,
    pub total: u32,
    /// Files whose thumbnails were sent.
    pub made: u32,
}

impl Status {
    pub fn running(&self) -> bool {
        matches!(self.phase, Phase::Listing | Phase::Running)
    }
}

/// Fixed words for why a run ended early.
pub fn held_words(why: MadeUpload) -> &'static str {
    match why {
        MadeUpload::NotNow(NotNow::SettingOff) => "Photo previews are off. Turn them on to make thumbnails.",
        MadeUpload::NotNow(NotNow::Hold(PausedFor::Metered)) => "Waiting: this PC is on a metered network.",
        MadeUpload::NotNow(NotNow::Hold(PausedFor::BatterySaver)) => "Waiting: battery saver is on.",
        MadeUpload::Paused => "Sync is paused. Resume it to make thumbnails.",
        MadeUpload::SessionMoved => "Beebeeb is locked or signed out.",
        MadeUpload::Offline => "Beebeeb can't be reached right now.",
        MadeUpload::Sent | MadeUpload::NothingToSend | MadeUpload::Failed => "Thumbnails could not be made.",
    }
}

struct Shared {
    status: Mutex<Status>,
    stop: AtomicBool,
    changed: AtomicBool,
}

impl Shared {
    fn set(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.lock().unwrap_or_else(PoisonError::into_inner));
        self.changed.store(true, Ordering::SeqCst);
    }
}

/// The one repair run of the app (the Settings pane holds it).
pub struct ThumbRepair {
    engine: Arc<dyn RepairEngine>,
    generate: Generate,
    shared: Arc<Shared>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ThumbRepair {
    pub fn new(engine: Arc<dyn RepairEngine>) -> Self {
        Self::with_generate(engine, generate_for)
    }

    /// With another maker of the pictures (tests: no helper).
    pub fn with_generate(engine: Arc<dyn RepairEngine>, generate: Generate) -> Self {
        Self {
            engine,
            generate,
            shared: Arc::new(Shared {
                status: Mutex::new(Status::default()),
                stop: AtomicBool::new(false),
                changed: AtomicBool::new(false),
            }),
            thread: Mutex::new(None),
        }
    }

    pub fn status(&self) -> Status {
        *self.shared.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Did the run move on since the last call?
    pub fn take_changed(&self) -> bool {
        self.shared.changed.swap(false, Ordering::SeqCst)
    }

    /// Start a run for the session in memory. `false`: one runs already, or no worker started.
    pub fn start(&self) -> bool {
        let mut thread = self.thread.lock().unwrap_or_else(PoisonError::into_inner);
        if self.status().running() {
            return false;
        }
        if let Some(ended) = thread.take() {
            let _ = ended.join();
        }
        self.shared.stop.store(false, Ordering::SeqCst);
        self.shared.set(|status| {
            *status = Status {
                phase: Phase::Listing,
                ..Status::default()
            }
        });
        let (engine, shared, generate) = (self.engine.clone(), self.shared.clone(), self.generate);
        let started = std::thread::Builder::new()
            .name("thumbnail-repair".into())
            .spawn(move || {
                let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&engine, &shared, generate)));
                if ran.is_err() {
                    log("thumbnail repair: the run ended early");
                    shared.set(|status| status.phase = Phase::Held(held_words(MadeUpload::Failed)));
                }
                engine.wake();
            });
        match started {
            Ok(handle) => {
                *thread = Some(handle);
                true
            }
            Err(_) => {
                self.shared
                    .set(|status| status.phase = Phase::Held(held_words(MadeUpload::Failed)));
                false
            }
        }
    }

    /// No new file is begun; the ones in hand end.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }

    /// Tests: wait for the run to end.
    #[cfg(test)]
    pub fn join(&self) {
        if let Some(handle) = self.thread.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ThumbRepair {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The run, on its own thread: the list, then [`AT_ONCE`] workers over it.
fn run(engine: &Arc<dyn RepairEngine>, shared: &Arc<Shared>, generate: Generate) {
    let session = engine.session();
    if let Some(why) = engine.hold(session) {
        shared.set(|status| status.phase = Phase::Held(held_words(why)));
        return;
    }
    let Some(missing) = engine.missing(session) else {
        shared.set(|status| status.phase = Phase::Held(held_words(MadeUpload::Failed)));
        return;
    };
    // Each file once, in the list's order.
    let mut seen = HashSet::new();
    let queue: VecDeque<MissingThumb> = missing.into_iter().filter(|m| seen.insert(m.file_id.clone())).collect();
    let total = u32::try_from(queue.len()).unwrap_or(u32::MAX);
    shared.set(|status| {
        status.phase = Phase::Running;
        status.total = total;
    });
    engine.wake();
    let queue = Mutex::new(queue);
    let handled: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    let held: Mutex<Option<MadeUpload>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..AT_ONCE {
            scope.spawn(|| work(engine, shared, generate, session, &queue, &handled, &held));
        }
    });
    let held = *held.lock().unwrap_or_else(PoisonError::into_inner);
    let stopped = shared.stop.load(Ordering::SeqCst);
    shared.set(|status| {
        status.phase = match held {
            Some(why) => Phase::Held(held_words(why)),
            None if stopped && status.done < status.total => Phase::Stopped,
            None => Phase::Done,
        }
    });
}

/// One worker: the next file not handled yet, until the list is empty, Stop, or a hold.
fn work(
    engine: &Arc<dyn RepairEngine>,
    shared: &Shared,
    generate: Generate,
    session: SessionGeneration,
    queue: &Mutex<VecDeque<MissingThumb>>,
    handled: &Mutex<HashSet<String>>,
    held: &Mutex<Option<MadeUpload>>,
) {
    loop {
        if shared.stop.load(Ordering::SeqCst) || held.lock().unwrap_or_else(PoisonError::into_inner).is_some() {
            return;
        }
        let Some(file) = queue.lock().unwrap_or_else(PoisonError::into_inner).pop_front() else {
            return;
        };
        if !handled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(file.file_id.clone())
        {
            continue;
        }
        // Asked again before each download: a hold that began during the run ends it.
        if let Some(why) = engine.hold(session) {
            held.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(why);
            return;
        }
        let made = one(&**engine, session, &file, generate, &shared.stop);
        match made {
            Some(MadeUpload::Sent) => shared.set(|status| status.made += 1),
            Some(
                why @ (MadeUpload::NotNow(_) | MadeUpload::Paused | MadeUpload::SessionMoved | MadeUpload::Offline),
            ) => {
                held.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(why);
            }
            _ => {}
        }
        shared.set(|status| status.done = status.done.saturating_add(1));
        engine.wake();
    }
}

/// One file: asked of the server when its flag is not known, downloaded, made and sent.
fn one(
    engine: &dyn RepairEngine,
    session: SessionGeneration,
    file: &MissingThumb,
    generate: Generate,
    stop: &AtomicBool,
) -> Option<MadeUpload> {
    let mime = beebeeb_engine::drive_v2::model::mime_for_name(&file.name)?;
    if !file.server_said_none && !engine.server_has_none(session, &file.file_id) {
        return None;
    }
    let bytes = engine.download(session, &file.file_id, stop)?;
    // The flag was asked above: the engine's send takes the server's answer as said.
    struct Said<'a>(&'a dyn RepairEngine);
    impl ThumbMaker for Said<'_> {
        fn known(&self, _: SessionGeneration, _: &str) -> Option<bool> {
            Some(false)
        }
        fn server_has_none(&self, _: SessionGeneration, _: &str) -> bool {
            true
        }
        fn send(&self, session: SessionGeneration, id: &str, thumbs: &super::thumbs::Thumbs, _: bool) -> MadeUpload {
            self.0.send(session, id, thumbs, true)
        }
    }
    make_from_bytes(&Said(engine), session, &file.file_id, mime, &bytes, generate)
}

#[cfg(test)]
#[path = "thumb_repair_tests.rs"]
mod tests;
