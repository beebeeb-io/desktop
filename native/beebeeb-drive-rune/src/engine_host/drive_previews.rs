//! Thumbnails and previews (DW-0744, DW-0744b, DW-0745) on the real engine: [`PreviewsHost`], the part of
//! `EngineDriveSettings` that answers `photo_previews`, `set_photo_previews`, `previews_cost`, `apply_previews`,
//! `rebuild_previews` and `reload_previews`.
//!
//! What it draws, and from where (every figure the server's or measured on this PC; never a typical size):
//! - the switch, the size and the format: the ACCOUNT's preview setting (the one the iPhone app reads), through
//!   [`Engine::photo_previews_for`];
//! - "What it uses": the account's stored previews (their count and bytes); the fallback and grid-thumbnail rows are
//!   not drawn (no figure exists for them); "On this PC" is not drawn (this PC keeps no previews yet: PREV-2);
//! - the average per size and format: from each stored preview's bytes, where some exist;
//! - the sample: the newest of the person's own photos on this PC that decodes here, measured on this PC
//!   ([`Engine::measure_sample_for`]); with none, the picture that ships inside the app.
//!
//! What a press does: the switch, Apply and Rebuild write the account's setting (`Engine::set_photo_previews_for`),
//! under the session of the press, then re-read; a run makes JPEG previews of this PC's photos in the background
//! (one run at a time, shared with the first run: [`shared_run`]). Off writes `enabled: false`, stops the run and
//! deletes NOTHING. A run also starts once per unlocked session when the setting is on and JPEG (the engine reads
//! the setting and decides). Never on a timer.
//!
//! Whose answer: every job carries the session and the account it was asked for and a generation; `pump` starts a
//! new generation when the session turn, the gate or the account moved (a lock, an unlock, a sign-out, another
//! account), drops what it held, and takes in only answers of the current generation. Nothing is handed out while
//! the vault is not open for the account the window holds things for.
//!
//! When it reads: when the pane opens (and once more when a run ends), and while a run is making previews the summary
//! alone (not the per-file list the averages come from) at most every [`READ_EVERY_WHILE_MAKING`] (on the run's own
//! progress, never a timer). Never at launch.
//!
//! Must not: log a name, a path, an id or a byte value; keep the person's pictures past a change of account.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::photo_previews_api::{PreviewsFacts, PreviewsRefused, SampleFacts};
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::photo_previews::runner::{PreviewRun, RunEnd, RunState, Stopped};
use beebeeb_engine::photo_previews::{PreviewFormat as Wire, PreviewSetting, PreviewsSummary};
use beebeeb_engine::signin::gate::GateState;

use super::log;
use crate::frames::settings::previews::{
    NOT_MEASURED, PhotoPreviews, PreviewFormat, PreviewSize, Previews, REBUILDING,
};
use crate::frames::signin::setup::source::{Sample, wipe_if_last};

#[cfg(test)]
#[path = "drive_previews_tests.rs"]
pub(crate) mod tests;

/// While a run makes previews, what the pane shows is read again at most this often.
pub const READ_EVERY_WHILE_MAKING: Duration = Duration::from_secs(10);

/// The Format caption's added sentence while HEIC is chosen (decision 2527, Q1).
pub const HEIC_NOTE: &str = "This PC makes previews in JPEG only; with HEIC it makes none.";
/// Fixed sentences: never a server's words.
pub const READ_OFFLINE: &str = "Couldn't reach Beebeeb. Check your connection and try again.";
pub const READ_FAILED: &str = "Beebeeb didn't answer as expected. Try again.";
pub const STORAGE_FULL: &str = "Your Beebeeb storage is full, so no more previews were made.";
pub const RUN_STOPPED: &str = "Making previews stopped: Beebeeb couldn't be reached.";
pub const RUN_PAUSED: &str = "Sync is paused, so no previews are being made.";
pub const CHANGE_FAILED: &str = "Couldn't change photo previews. Try again.";
pub const CHANGE_BUSY: &str = "A change to photo previews is still on its way.";
/// Said where the pane would otherwise load for ever.
pub const NOT_BOUND: &str = "Unlock Beebeeb with this account to see and change these settings.";
pub const CHANGE_LOCKED: &str = "Unlock Beebeeb to change this.";

/// The photo picker: the platform's (`Platform::pick_photo`), or a test's. BLOCKS: only a worker calls it.
pub type Picker =
    Arc<dyn Fn(&str, &std::path::Path) -> Result<Option<PathBuf>, crate::shell::platform::PlatformError> + Send + Sync>;

/// What the host asks of the engine. The app's is [`Engine`]; a test's is a fake.
pub trait PreviewsEngine: Send + Sync + 'static {
    /// The session turn (`Engine::session_turn`). Never waits.
    fn session(&self) -> SessionGeneration;
    /// The vault is open and the window may show the account (the gate is Ready). Never waits.
    fn ready(&self) -> bool;
    /// Whose account is signed in. Never waits.
    fn account(&self) -> Option<String>;
    /// Wake the window: an answer or a run's progress is in.
    fn wake(&self);
    fn facts(&self, session: SessionGeneration) -> Result<PreviewsFacts, PreviewsRefused>;
    /// What the stored previews take, alone.
    fn summary(&self, session: SessionGeneration) -> Result<PreviewsSummary, PreviewsRefused>;
    fn setting(&self, session: SessionGeneration) -> Result<PreviewSetting, PreviewsRefused>;
    fn set_setting(&self, session: SessionGeneration, setting: PreviewSetting) -> Result<(), PreviewsRefused>;
    fn make(&self, session: SessionGeneration, force: bool, stop: &AtomicBool, progress: &dyn Fn(u32, u32)) -> RunEnd;
    fn measure(&self, session: SessionGeneration, picked: Option<PathBuf>) -> Result<SampleFacts, PreviewsRefused>;
}

impl<C: CredentialStores> PreviewsEngine for Engine<C> {
    fn session(&self) -> SessionGeneration {
        self.session_turn()
    }
    fn ready(&self) -> bool {
        self.view().gate == GateState::Ready
    }
    fn account(&self) -> Option<String> {
        self.account_email().or_else(|| self.user_id())
    }
    fn wake(&self) {
        self.wake_host();
    }
    fn facts(&self, session: SessionGeneration) -> Result<PreviewsFacts, PreviewsRefused> {
        self.photo_previews_for(session)
    }
    fn summary(&self, session: SessionGeneration) -> Result<PreviewsSummary, PreviewsRefused> {
        self.photo_previews_summary_for(session)
    }
    fn setting(&self, session: SessionGeneration) -> Result<PreviewSetting, PreviewsRefused> {
        self.photo_previews_setting_for(session)
    }
    fn set_setting(&self, session: SessionGeneration, setting: PreviewSetting) -> Result<(), PreviewsRefused> {
        self.set_photo_previews_for(session, setting)
    }
    fn make(&self, session: SessionGeneration, force: bool, stop: &AtomicBool, progress: &dyn Fn(u32, u32)) -> RunEnd {
        self.make_previews_for(session, force, stop, progress)
    }
    fn measure(&self, session: SessionGeneration, picked: Option<PathBuf>) -> Result<SampleFacts, PreviewsRefused> {
        self.measure_sample_for(session, picked)
    }
}

/// The one preview run of this app: the Settings pane and the first run share it (one run at a time).
pub fn shared_run() -> PreviewRun {
    static RUN: std::sync::OnceLock<PreviewRun> = std::sync::OnceLock::new();
    RUN.get_or_init(PreviewRun::new).clone()
}

/// Start a run on `run` for `session` (`force`: a rebuild). The run asks the session before every upload.
pub fn start_run<E: ?Sized + PreviewsEngine>(
    engine: Arc<E>,
    run: &PreviewRun,
    session: SessionGeneration,
    force: bool,
) {
    let waker = engine.clone();
    run.on_change(Arc::new(move || waker.wake()));
    let started = run.start(
        session,
        Box::new(move |stop, progress| engine.make(session, force, stop, progress)),
    );
    if !started {
        log("photo previews: could not start a run");
    }
}

/// The server's long edge of a size on the board (720p is 1280 by 720).
pub fn px_of(size: PreviewSize) -> u32 {
    match size {
        PreviewSize::P720 => 1280,
        PreviewSize::P1080 => 1920,
        PreviewSize::K2 => 2560,
        PreviewSize::K4 => 3840,
    }
}

/// The board's size of a long edge. `None`: not one of the four.
pub fn size_of(px: u32) -> Option<PreviewSize> {
    PreviewSize::ALL.into_iter().find(|size| px_of(*size) == px)
}

pub fn format_of(format: Wire) -> PreviewFormat {
    match format {
        Wire::Heic => PreviewFormat::Heic,
        Wire::Jpeg => PreviewFormat::Jpeg,
    }
}

pub fn wire_of(format: PreviewFormat) -> Wire {
    match format {
        PreviewFormat::Heic => Wire::Heic,
        PreviewFormat::Jpeg => Wire::Jpeg,
    }
}

/// "2,431".
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn bytes_words(bytes: u64) -> String {
    super::drive_settings::size_words(i64::try_from(bytes).unwrap_or(i64::MAX))
}

/// The sample, measured on this PC: its pictures (shared, so a window compares them without reading their bytes)
/// and its weights. Its `Debug` prints sizes only.
#[derive(Clone, PartialEq)]
pub struct Measured {
    pub original_bytes: u64,
    /// A JPEG preview's weight at each long edge.
    pub weights: Vec<(u32, u64)>,
    pub original_png: Arc<[u8]>,
    pub previews_png: Vec<(u32, Arc<[u8]>)>,
}

/// The pictures are the person's photo: whoever lets go of them last wipes them (a window's [`Sample`] does too).
impl Drop for Measured {
    fn drop(&mut self) {
        wipe_if_last(&mut self.original_png);
        for (_, png) in &mut self.previews_png {
            wipe_if_last(png);
        }
    }
}

impl std::fmt::Debug for Measured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Measured")
            .field("original_bytes", &self.original_bytes)
            .field("weights", &self.weights)
            .finish_non_exhaustive()
    }
}

impl Measured {
    /// `None`: no photo of the account on this PC decodes here.
    pub fn of(facts: SampleFacts) -> Option<Self> {
        let photo = facts.photo?;
        Some(Self {
            original_bytes: photo.original_bytes,
            weights: photo.weights,
            original_png: Arc::from(&photo.original_png[..]),
            previews_png: photo
                .previews_png
                .iter()
                .map(|(px, png)| (*px, Arc::from(&png[..])))
                .collect(),
        })
    }

    /// The JPEG weight at `px`, measured here.
    pub fn weight(&self, px: u32) -> Option<u64> {
        self.weights.iter().find(|(at, _)| *at == px).map(|(_, bytes)| *bytes)
    }

    /// The pictures to compare at this size and format: the JPEG preview at that size; with HEIC (no HEIC
    /// encoder here) the original, scaled down the same way (decision 2527, Q1).
    pub fn sample(&self, px: u32, format: Wire) -> Sample {
        let preview = match format {
            Wire::Jpeg => self
                .previews_png
                .iter()
                .find(|(at, _)| *at == px)
                .map(|(_, png)| png.clone()),
            Wire::Heic => None,
        };
        Sample {
            original_png: self.original_png.clone(),
            preview_png: preview.unwrap_or_else(|| self.original_png.clone()),
            bundled: false,
        }
    }
}

enum Ask {
    /// The setting, the summary and the averages.
    Facts,
    /// The summary alone (while a run makes previews).
    Summary,
    /// The sample, measured on this PC.
    Sample,
    /// Write the setting; then start a run (`Some(force)`) or stop nothing more (`None`).
    Set(PreviewSetting, Option<bool>),
    /// Rebuild: a forced run at the stored setting.
    Rebuild,
    /// A new unlocked session: start a run when the setting is on and JPEG.
    SessionStart,
    /// "Choose photo": the picker, then the pick measured.
    Choose(Picker),
}

struct Job {
    ask: Ask,
    session: SessionGeneration,
    generation: u64,
}

enum Answer {
    Facts(Result<PreviewsFacts, PreviewsRefused>),
    Summary(Result<PreviewsSummary, PreviewsRefused>),
    Sample(Option<Measured>),
    /// "Choose photo" ended: the measured pick (`None`: closed without one, or it could not be measured).
    Chosen(Option<Measured>),
    /// A change was answered (`Err`: it was not made).
    Set(Result<(), PreviewsRefused>),
}

struct Tagged {
    generation: u64,
    answer: Answer,
}

/// The preview part of Drive's settings on the real engine. See the module's doc.
pub struct PreviewsHost {
    engine: Option<Arc<dyn PreviewsEngine>>,
    run: PreviewRun,
    jobs: Option<mpsc::Sender<Job>>,
    answers: mpsc::Sender<Tagged>,
    inbox: mpsc::Receiver<Tagged>,
    /// The session and account the window holds things for (`None`: no one with the vault open).
    seen: Option<(SessionGeneration, String)>,
    generation: u64,
    facts: Option<PreviewsFacts>,
    /// The last read failed, and why (said until a read answers).
    failed: Option<&'static str>,
    /// `pump` has run at least once (what it saw first is known).
    pumped: bool,
    reading: bool,
    read_at: Option<Instant>,
    sample: Option<Measured>,
    /// The sample was asked in this generation.
    sampled: bool,
    /// The picker is open or its pick is being measured: no second one is asked.
    choosing: bool,
    /// The pane is the one open.
    open: bool,
    /// A change is on its way: no second one is sent.
    changing: bool,
    toast: Option<&'static str>,
    /// Where the run stood at the last pump.
    last_run: RunState,
}

impl PreviewsHost {
    /// Nothing bound: the pane draws nothing and the shell's frame shows.
    pub fn off() -> Self {
        Self::make(None, PreviewRun::new())
    }

    /// On the engine, sharing `run` (the app's: [`shared_run`]).
    pub fn new(engine: Arc<dyn PreviewsEngine>, run: PreviewRun) -> Self {
        Self::make(Some(engine), run)
    }

    fn make(engine: Option<Arc<dyn PreviewsEngine>>, run: PreviewRun) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            run,
            jobs: None,
            answers,
            inbox,
            seen: None,
            generation: 0,
            facts: None,
            failed: None,
            pumped: false,
            reading: false,
            read_at: None,
            sample: None,
            sampled: false,
            choosing: false,
            open: false,
            changing: false,
            toast: None,
            last_run: RunState::Idle,
        }
    }

    /// Whether an engine answers for the pane.
    pub fn bound(&self) -> bool {
        self.engine.is_some()
    }

    /// The session and account the engine has now, while the vault is open.
    fn now(&self) -> Option<(SessionGeneration, String)> {
        let engine = self.engine.as_ref()?;
        if !engine.ready() {
            return None;
        }
        Some((engine.session(), engine.account()?))
    }

    /// What is held may be handed out: the engine is still at the session and account it was read for.
    fn mine(&self) -> bool {
        self.seen.is_some() && self.now() == self.seen
    }

    fn send(&mut self, ask: Ask) -> bool {
        let (Some(engine), Some((session, _))) = (self.engine.clone(), self.seen.clone()) else {
            return false;
        };
        if self.jobs.is_none() {
            let (tx, rx) = mpsc::channel::<Job>();
            let (answers, run) = (self.answers.clone(), self.run.clone());
            let started = std::thread::Builder::new()
                .name("photo-previews-settings".into())
                .spawn(move || {
                    for job in rx {
                        let generation = job.generation;
                        let answer =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&engine, &run, job)));
                        match answer {
                            Ok(Some(answer)) => {
                                let _ = answers.send(Tagged { generation, answer });
                            }
                            Ok(None) => {}
                            Err(_) => log("photo previews: a read or a change ended early"),
                        }
                        engine.wake();
                    }
                });
            match started {
                Ok(_) => self.jobs = Some(tx),
                Err(_) => {
                    log("photo previews: could not start its worker");
                    return false;
                }
            }
        }
        let job = Job {
            ask,
            session,
            generation: self.generation,
        };
        self.jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok())
    }

    fn read(&mut self) {
        if !self.reading {
            self.reading = self.send(Ask::Facts);
        }
    }

    fn read_summary(&mut self) {
        if !self.reading {
            self.reading = self.send(Ask::Summary);
        }
    }

    /// The pane opened: read what it shows (and the sample, once per generation).
    pub fn opened(&mut self) {
        self.open = true;
        if !self.mine() {
            return;
        }
        self.read();
        if !self.sampled {
            self.sampled = self.send(Ask::Sample);
        }
    }

    /// Another pane opened.
    pub fn closed(&mut self) {
        self.open = false;
    }

    /// Take in what arrived; start a new generation when the session, the gate or the account moved. `true`:
    /// something the pane shows changed.
    pub fn pump(&mut self) -> bool {
        if self.engine.is_none() {
            return false;
        }
        let mut changed = false;
        self.pumped = true;
        let now = self.now();
        if now != self.seen {
            if self.seen.is_some() {
                // A lock, a vault close, a sign-out or another account: the run of the earlier session ends (after
                // the photo in hand; the engine sends nothing more for it either).
                self.run.stop();
            }
            self.seen = now;
            self.generation += 1;
            self.facts = None;
            self.failed = None;
            self.reading = false;
            self.read_at = None;
            self.sample = None;
            self.sampled = false;
            self.choosing = false;
            self.changing = false;
            self.toast = None;
            if self.seen.is_some() {
                self.send(Ask::SessionStart);
                if self.open {
                    self.opened();
                }
            }
            changed = true;
        }
        while let Ok(Tagged { generation, answer }) = self.inbox.try_recv() {
            if generation != self.generation {
                // Asked before the session or the account moved: dropped, never drawn.
                continue;
            }
            changed = true;
            match answer {
                Answer::Facts(read) => {
                    self.reading = false;
                    match read {
                        Ok(facts) => {
                            self.read_at = Some(Instant::now());
                            self.facts = Some(facts);
                            self.failed = None;
                        }
                        // A failed read never stamps freshness, and never reads as "off".
                        Err(PreviewsRefused::Offline) => self.failed = Some(READ_OFFLINE),
                        Err(PreviewsRefused::NotSignedIn | PreviewsRefused::SessionEnded) => {
                            self.failed = Some(CHANGE_LOCKED)
                        }
                        Err(PreviewsRefused::Failed) => self.failed = Some(READ_FAILED),
                    }
                }
                Answer::Summary(read) => {
                    self.reading = false;
                    // Asked again only after the same wait, answered or not: a failed one keeps what is shown.
                    self.read_at = Some(Instant::now());
                    if let (Ok(summary), Some(facts)) = (read, self.facts.as_mut()) {
                        facts.summary = summary;
                    }
                }
                Answer::Sample(sample) => self.sample = sample,
                Answer::Chosen(picked) => {
                    self.choosing = false;
                    // A closed picker or an unreadable photo keeps the sample that was shown.
                    if picked.is_some() {
                        self.sample = picked;
                    }
                }
                Answer::Set(done) => {
                    self.changing = false;
                    if done.is_err() {
                        self.toast = Some(CHANGE_FAILED);
                    }
                    // What the server holds now, changed or not: never a change drawn before it was made.
                    self.read();
                }
            }
        }
        let run = self.run_state();
        if run != self.last_run {
            changed = true;
            // A run ended: what it made is read again.
            if matches!(self.last_run, RunState::Making { .. }) && !matches!(run, RunState::Making { .. }) && self.open
            {
                self.read();
            }
            self.last_run = run;
        }
        if matches!(run, RunState::Making { .. })
            && self.open
            && self.read_at.is_some_and(|at| at.elapsed() >= READ_EVERY_WHILE_MAKING)
        {
            self.read_summary();
        }
        changed
    }

    fn run_state(&self) -> RunState {
        match &self.seen {
            Some((session, _)) => self.run.state_for(*session),
            None => RunState::Idle,
        }
    }

    pub fn take_toast(&mut self) -> Option<&'static str> {
        self.toast.take()
    }

    /// "Choose photo": `picker` opens on a worker, and the photo picked is measured on this PC (one at a time).
    pub fn choose_photo(&mut self, picker: Picker) {
        if self.choosing || !self.mine() {
            return;
        }
        self.choosing = self.send(Ask::Choose(picker));
    }

    /// What the pane draws. See the module's doc: `Unavailable` only where no engine answers; a vault that is not
    /// this account's yet (locked, first pump not run) is a pane still loading, never the "not built" frame.
    pub fn photo_previews(&self) -> PhotoPreviews {
        if self.engine.is_none() {
            return PhotoPreviews::Unavailable;
        }
        if !self.mine() {
            // Pumped and still no vault of this account: locked or signed out, which does not end by waiting.
            // Before the first pump, or while another account's read turns over, it is a pane still loading.
            if self.seen.is_none() && self.pumped && self.now().is_none() {
                return PhotoPreviews::NotBound(NOT_BOUND);
            }
            return PhotoPreviews::Loading;
        }
        match (&self.facts, self.failed) {
            (Some(facts), _) => PhotoPreviews::Ready(self.drawn(facts)),
            (None, Some(why)) => PhotoPreviews::Failed(why),
            (None, None) => PhotoPreviews::Loading,
        }
    }

    fn drawn(&self, facts: &PreviewsFacts) -> Previews {
        let setting = facts.setting;
        let average = |format: Wire, px: u32| {
            facts
                .averages
                .iter()
                .find(|(at, _, _)| *at == (format, px))
                .map(|(_, bytes, count)| (*bytes, *count))
        };
        let mut weights = Vec::new();
        for ((format, px), bytes, _) in &facts.averages {
            if let Some(size) = size_of(*px) {
                weights.push((size, format_of(*format), bytes_words(*bytes)));
            }
        }
        // "Making" only once the run counted what it makes: a run that waits (for sync's first pass) says nothing.
        let notice = match self.run_state() {
            RunState::Making { total, .. } if total > 0 => Some(REBUILDING),
            RunState::Making { .. } => None,
            RunState::Stopped(Stopped::StorageFull) => Some(STORAGE_FULL),
            RunState::Stopped(Stopped::Offline | Stopped::Failed) => Some(RUN_STOPPED),
            RunState::Stopped(Stopped::Paused) => Some(RUN_PAUSED),
            RunState::Stopped(Stopped::HeicNotMadeHere) | RunState::Idle => None,
        };
        Previews {
            on: setting.enabled,
            // A size the server stores that the board has no name for reads as the server's default, 2K.
            size: size_of(setting.px).unwrap_or(PreviewSize::K2),
            format: format_of(setting.format),
            photos: grouped(facts.summary.count),
            original: self
                .sample
                .as_ref()
                .map_or_else(|| NOT_MEASURED.to_string(), |sample| bytes_words(sample.original_bytes)),
            weights,
            measured_on: average(setting.format, setting.px)
                .map_or(0, |(_, count)| u32::try_from(count).unwrap_or(u32::MAX)),
            total: bytes_words(facts.summary.total_bytes),
            uses: [Some(bytes_words(facts.summary.total_bytes)), None, None],
            on_pc: None,
            format_note: (setting.format == Wire::Heic).then_some(HEIC_NOTE),
            fallback: false,
            notice,
            sample: self
                .sample
                .as_ref()
                .map(|sample| sample.sample(setting.px, setting.format)),
        }
    }

    /// What all previews would take at a size and format: the account's own average there times its previews,
    /// else (JPEG only) the sample's weight measured here. `None`: nothing is known (the sentence says no figure).
    pub fn cost(&self, size: PreviewSize, format: PreviewFormat) -> Option<String> {
        let facts = self.facts.as_ref().filter(|_| self.mine())?;
        let (px, format) = (px_of(size), wire_of(format));
        let average = facts
            .averages
            .iter()
            .find(|(at, _, _)| *at == (format, px))
            .map(|(_, bytes, _)| *bytes)
            .or_else(|| {
                (format == Wire::Jpeg)
                    .then(|| self.sample.as_ref()?.weight(px))
                    .flatten()
            })?;
        if facts.summary.count == 0 {
            return None;
        }
        Some(bytes_words(average.saturating_mul(facts.summary.count)))
    }

    /// Send a change of the setting; one at a time, only while the account is the one the pane holds.
    fn change(&mut self, setting: PreviewSetting, then_run: Option<bool>) {
        if self.changing {
            self.toast = Some(CHANGE_BUSY);
            return;
        }
        if !self.mine() || !self.send(Ask::Set(setting, then_run)) {
            self.toast = Some(CHANGE_LOCKED);
            return;
        }
        self.changing = true;
    }

    /// The switch. Off stops the run at once and deletes nothing; On makes previews in the background.
    pub fn set_on(&mut self, on: bool) {
        let Some(stored) = self.facts.as_ref().filter(|_| self.mine()).map(|facts| facts.setting) else {
            self.toast = Some(CHANGE_LOCKED);
            return;
        };
        if !on {
            self.run.stop();
        }
        self.change(PreviewSetting { enabled: on, ..stored }, on.then_some(false));
    }

    /// Apply a new size and format: stored (on), then a run makes the new previews; the old ones stay until each
    /// new one replaced it.
    pub fn apply(&mut self, size: PreviewSize, format: PreviewFormat) {
        let setting = PreviewSetting {
            enabled: true,
            format: wire_of(format),
            px: px_of(size),
        };
        self.change(setting, Some(false));
    }

    /// Make every preview again at the stored setting (a forced run).
    pub fn rebuild(&mut self) {
        if !self.mine() || !self.send(Ask::Rebuild) {
            self.toast = Some(CHANGE_LOCKED);
        }
    }

    /// Try again after a failed read.
    pub fn reload(&mut self) {
        self.failed = None;
        self.read();
        if !self.sampled {
            self.sampled = self.send(Ask::Sample);
        }
    }
}

/// One job, on the worker, for its session and no other.
fn work(engine: &Arc<dyn PreviewsEngine>, run: &PreviewRun, job: Job) -> Option<Answer> {
    let session = job.session;
    match job.ask {
        Ask::Facts => Some(Answer::Facts(engine.facts(session))),
        Ask::Summary => Some(Answer::Summary(engine.summary(session))),
        Ask::Sample => Some(Answer::Sample(match engine.measure(session, None) {
            Ok(facts) => Measured::of(facts),
            Err(why) => {
                log(format!("photo previews: no sample was measured ({})", why.kind()));
                None
            }
        })),
        Ask::Choose(picker) => Some(Answer::Chosen(match picker("Choose a photo to compare", std::path::Path::new("")) {
            Ok(Some(picked)) => match engine.measure(session, Some(picked)) {
                Ok(facts) => Measured::of(facts),
                Err(why) => {
                    log(format!("photo previews: the chosen photo was not measured ({})", why.kind()));
                    None
                }
            },
            Ok(None) => None,
            Err(_) => {
                log("photo previews: the photo picker did not open");
                None
            }
        })),
        Ask::Set(setting, then_run) => {
            let done = engine.set_setting(session, setting);
            match (&done, then_run) {
                (Ok(()), Some(force)) => start_run(engine.clone(), run, session, force),
                // Off, once written: the run ends, also one a Rebuild asked for just before it (its start came
                // through this worker first, so this stop comes after it).
                (Ok(()), None) if !setting.enabled => run.stop(),
                (Ok(()), None) => {}
                (Err(why), _) => log(format!("photo previews: the setting was not changed ({})", why.kind())),
            }
            Some(Answer::Set(done))
        }
        Ask::Rebuild => {
            start_run(engine.clone(), run, session, true);
            None
        }
        Ask::SessionStart => {
            // Once per unlocked session: the engine's setting decides; a failed read starts nothing.
            match engine.setting(session) {
                Ok(setting) if setting.enabled && setting.format == Wire::Jpeg => {
                    start_run(engine.clone(), run, session, false)
                }
                Ok(_) => {}
                Err(why) => log(format!("photo previews: the setting was not read ({})", why.kind())),
            }
            None
        }
    }
}
