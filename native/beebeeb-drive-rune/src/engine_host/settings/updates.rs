//! About › Updates (DW-0751) and the Release channel (DW-0789): this PC's updater (`beebeeb-updater`) behind
//! Check now and the channel menu.
//!
//! The updater is one state machine (`beebeeb_updater::Updater`). Here is its host and the part of Settings that
//! drives it:
//! - **The host.** The network is the crate's `HttpsClient` (feature `net`: https only, no redirect followed, a fixed
//!   user agent and nothing else about the person); the clock is the system's; the record is the crate's JSON file
//!   (`updater.json`) and the staging folder (`updates`) in this app's local data folder; the hand-off is
//!   [`EngineApply`]: the engine stops before the NSIS installer starts, and the installer is started only through the
//!   crate's `VerifiedInstaller` (verified again, and held, at that moment).
//! - **What is trusted is fixed in the build.** One key (`keys/updater-v2.pub`, through
//!   `host::windows_release_trust`), one host (`host::RELEASE_BASE_URL`), never below 0.9.0, nothing signed before the
//!   build itself. Nothing here reads a host, a key or a channel address from a setting, a flag or the environment.
//! - **The route follows this PC's proxy settings; the host and the signatures do not.** The client uses reqwest's
//!   defaults, as the engine's own client does (`HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`, and Windows' system proxy):
//!   a proxy can see that this PC asks the release server and can hold updates back (after 14 days the words say
//!   that no update has been verified), but it cannot change the host TLS checks, nor what the signatures, the
//!   not-before time and the per-channel mark let install.
//! - **One worker owns the updater.** Every request and every chunk of a download runs there, never on the window's
//!   thread; the window takes what it reported in [`UpdatesPart::pump`], and the worker wakes the window after each
//!   report (`Engine::wake_host`). It checks once at launch and then whenever `Updater::background_check_due` says so
//!   (every 4 hours; 15 minutes after a failure), which is what About's foot line says.
//! - **Nothing is installed by a check, and nothing without "Restart to update".** A check only asks; an older
//!   version on a channel is said (`About::older`) and never offered: its notes are never the dialog's, and neither
//!   press acts on it. "Download update" ([`UpdatesPart::download_update`]) downloads the newer version and checks its
//!   signature, then waits. "Restart to update" ([`UpdatesPart::restart_to_update`]) only once the file is verified:
//!   the worker verifies it again, stops sync, starts the installer, and the window quits (`Pumped::restart`). The
//!   installer did not start: sync starts again if the hand-off had stopped it (`Pumped::start_sync`). Sync did not
//!   confirm its stop: no installer starts, and the words say Beebeeb could not confirm it (not that it is running).
//! - **A build that is not a release has no updater and asks nothing.** Without a build time
//!   (`host::BUILD_NOT_BEFORE`) or a release version the updater would refuse every manifest (`NoBuildTime`), after
//!   two requests: it is not built ([`release_config`] is `None`), not one request leaves this PC, and About draws the
//!   versions only, as it does on macOS and Linux (no Check now beside a foot line that says Beebeeb checks). The
//!   release workflow refuses a binary without the build time, so no published build is in this state.
//! - **Not per account.** The updater is this PC's: no request carries an account, an email, a file or a token, and
//!   its answers are not an account's, so they are not stamped with the session the way account reads are.
//!
//! Windows only: the release line of 0.9.0 is the Windows installer. On macOS and Linux no updater is built
//! ([`UpdatesPart::for_this_pc`] is `None`) and About draws the versions only.
//!
//! Must not: log a path, a URL, or a server's words; show a string from the network other than a release's own
//! version number and notes.

// On macOS and Linux the part is never built in a build that ships (no release line there yet): its code is
// compiled, and unused.
#![cfg_attr(not(any(windows, test)), allow(dead_code))]

use std::cell::Cell;
#[cfg(any(windows, test))]
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

#[cfg(any(windows, test))]
use beebeeb_engine::app::SyncStop;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::config::{DesktopConfig, ReleaseChannel};
use beebeeb_engine::identity::EngineIdentity;
#[cfg(any(windows, test))]
use beebeeb_updater::{Apply, EngineStopped, Offer, VerifiedInstaller};
use beebeeb_updater::{Clock, Event, Failure, Freshness, Http, Intent, Persisted, Refusal, State, Store, Updater};
use chrono::{DateTime, Local};

use super::{STILL_SAVING, log};
use crate::frames::settings::source::{About, Channel, OlderChannel, Press, Pumped, ReleaseNotes, Update, UpdateCheck};

/// How often the worker asks the updater whether a background check is due (the updater's own rule decides).
const LOOK_EVERY: Duration = Duration::from_secs(60);
/// Download progress is reported at most this often; the state it settles in always is.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// What About says, word for word. Fixed sentences: no text from the network is ever shown.
pub const NOT_A_RELEASE: &str = "This build is not a release, so it takes no updates.";
pub const NO_UPDATER: &str = "The updater did not start. Restart Beebeeb to check for updates.";
pub const NOT_CHECKED: &str = "Not checked yet.";
pub const CHANNEL_NOT_SAVED: &str = "Couldn't save the release channel on this PC.";
pub const NOT_VERIFIED: &str = "The update information could not be verified, so nothing was downloaded.";
pub const UNREADABLE: &str = "The update information could not be read, so nothing was downloaded.";
pub const OUT_OF_DATE: &str = "The release server sent out-of-date update information, so nothing was downloaded.";
pub const FUTURE: &str =
    "The update information is dated ahead of this PC's clock, so nothing was downloaded. Check the date and time.";
pub const OTHER_HOST: &str = "The update is not on Beebeeb's release server, so nothing was downloaded.";
pub const NO_BUILD: &str = "This channel has no build for this PC.";
pub const BELOW_FLOOR: &str = "This channel offers a version this app cannot go back to, so nothing was downloaded.";
pub const NOTHING_PUBLISHED: &str = "The release server has no update information for this channel.";
pub const NOT_SIGNED_FILE: &str =
    "The download was not the file Beebeeb signed. It was deleted and nothing was installed.";
pub const STOPPED_DOWNLOAD: &str = "The download stopped before it was complete. Nothing was installed.";
pub const OFFER_CHANGED: &str = "The version on offer changed. Nothing was installed.";
pub const NO_DISK: &str = "The update could not be saved on this PC. Nothing was installed.";
/// The hand-off's stop was not confirmed. On Windows this is also a stop whose task ended while File Explorer's
/// connection did not let go (the engine says `Unconfirmed` for both): so these words never say that sync runs, nor
/// that it stopped, only what is known. A new start of sync needs a restart of Beebeeb in both cases.
pub const SYNC_UNCONFIRMED: &str =
    "Beebeeb could not confirm that sync stopped, so the update was not started. Restart Beebeeb, then update.";
pub const INSTALLER_DID_NOT_START: &str = "The installer did not start. Nothing was installed.";
pub const VERIFYING: &str = "Checking the download's signature…";
pub const RESTARTING: &str = "Restarting to update…";

/// What the worker does with the updater. A trait so the worker runs the real updater and a test's alike; each is
/// built on the worker's thread, so it need not be `Send`.
pub(crate) trait Drive {
    fn step(&mut self, event: Event) -> State;
    fn background_check_due(&self) -> bool;
    fn freshness(&self) -> Freshness;
    /// When the last check that passed every rule was answered (Unix seconds): what "checked <time>" says.
    fn last_verified(&self) -> Option<u64>;
}

/// The updater's record, watched: the time of the last check that passed every rule, as the machine wrote it
/// (`Persisted::last_check`: saved only after such a check, and read when the updater starts). The machine keeps
/// that time to itself; the record it writes is the one place it shows.
pub(crate) struct Watched<S> {
    store: S,
    verified: Rc<Cell<Option<u64>>>,
}

impl<S: Store> Store for Watched<S> {
    fn load(&self) -> Persisted {
        let persisted = self.store.load();
        self.verified.set(persisted.last_check);
        persisted
    }
    fn save(&mut self, persisted: &Persisted) -> bool {
        let saved = self.store.save(persisted);
        if saved {
            self.verified.set(persisted.last_check);
        }
        saved
    }
}

/// The updater, with the time of its last verified check ([`Watched`]).
pub(crate) struct Watching<H: Http, C: Clock, S: Store, A: beebeeb_updater::Apply> {
    updater: Updater<H, C, Watched<S>, A>,
    verified: Rc<Cell<Option<u64>>>,
}

impl<H: Http, C: Clock, S: Store, A: beebeeb_updater::Apply> Watching<H, C, S, A> {
    pub(crate) fn new(config: beebeeb_updater::Config, http: H, clock: C, store: S, apply: A) -> Self {
        let verified = Rc::new(Cell::new(None));
        let store = Watched {
            store,
            verified: verified.clone(),
        };
        Self {
            updater: Updater::new(config, http, clock, store, apply),
            verified,
        }
    }
}

impl<H: Http, C: Clock, S: Store, A: beebeeb_updater::Apply> Drive for Watching<H, C, S, A> {
    fn step(&mut self, event: Event) -> State {
        self.updater.step(event).clone()
    }
    fn background_check_due(&self) -> bool {
        self.updater.background_check_due()
    }
    fn freshness(&self) -> Freshness {
        self.updater.freshness()
    }
    fn last_verified(&self) -> Option<u64> {
        self.verified.get()
    }
}

/// The hand-off stopped a running sync and the installer did not start: [`EngineApply::resume_engine`] asks, the
/// window's [`UpdatesPart::pump`] takes it (`Pumped::start_sync`), and the shell starts sync with its own start.
#[derive(Clone, Default)]
pub(crate) struct SyncBack(Arc<AtomicBool>);

impl SyncBack {
    pub(crate) fn ask(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    fn take(&self) -> bool {
        self.0.swap(false, Ordering::SeqCst)
    }
}

/// Builds the updater on the worker's thread; its hand-off asks for sync back through the [`SyncBack`] it is given.
pub(crate) type MakeDrive = Box<dyn FnOnce(SyncBack) -> Box<dyn Drive> + Send>;

/// How the worker paces itself and tells the window it has something.
pub(crate) struct Tuning {
    /// How often it asks the updater whether a background check is due.
    pub(crate) look_every: Duration,
    /// Wakes the window after a report (the engine's wake target in the app).
    pub(crate) wake: Box<dyn Fn() + Send>,
}

impl Tuning {
    fn standard(wake: Box<dyn Fn() + Send>) -> Self {
        Self {
            look_every: LOOK_EVERY,
            wake,
        }
    }
}

/// One state the worker reached, for the order numbered `serial` (0: the launch check and every background check).
pub(crate) struct Report {
    serial: u64,
    state: State,
    freshness: Freshness,
    /// When the last verified check was answered (Unix seconds), as the record holds it at this report.
    verified: Option<u64>,
    at: DateTime<Local>,
}

/// An order for the worker: an event for the machine, numbered.
struct Order {
    serial: u64,
    event: Event,
}

/// The window's end of the worker.
struct Worker {
    orders: mpsc::Sender<Order>,
    reports: mpsc::Receiver<Report>,
}

enum Mode {
    /// The updater runs on its worker.
    Live(Worker),
    /// The worker could not be started, or it ended.
    NoWorker,
}

/// The release channel the person chose was written to this PC's settings file, or not.
struct Saved {
    channel: Channel,
    saved: bool,
}

/// About's updates, on the updater. See the module's doc.
pub(crate) struct UpdatesPart {
    mode: Mode,
    identity: &'static EngineIdentity,
    /// The channel this PC's settings file holds (what the menu shows).
    channel: Channel,
    /// The channel the running build came from: `installed_release_channel`, else the channel chosen when this app
    /// started (where the running build was taken from, as far as this PC knows).
    from: Channel,
    /// This build's version, as About says it.
    installed: String,
    /// The worker's last report.
    last: Option<Report>,
    /// The number of the last order sent, and the one a check the person asked for (Check now, a channel) waits on.
    sent: u64,
    asked: Option<u64>,
    /// A channel on its way to the settings file: no second one starts until it is answered.
    saving: Option<Channel>,
    saves: mpsc::Sender<Saved>,
    saved: mpsc::Receiver<Saved>,
    toast_next: Option<&'static str>,
    /// The hand-off asks for sync back here.
    sync_back: SyncBack,
    /// `Restarting` was seen and the quit asked for: once.
    quit_asked: bool,
}

fn channel_of(channel: ReleaseChannel) -> Channel {
    match channel {
        ReleaseChannel::Stable => Channel::Stable,
        ReleaseChannel::Beta => Channel::Beta,
        ReleaseChannel::Alpha => Channel::Alpha,
    }
}

fn release_channel(channel: Channel) -> ReleaseChannel {
    match channel {
        Channel::Stable => ReleaseChannel::Stable,
        Channel::Beta => ReleaseChannel::Beta,
        Channel::Alpha => ReleaseChannel::Alpha,
    }
}

fn line(channel: Channel) -> beebeeb_updater::Channel {
    match channel {
        Channel::Stable => beebeeb_updater::Channel::Stable,
        Channel::Beta => beebeeb_updater::Channel::Beta,
        Channel::Alpha => beebeeb_updater::Channel::Alpha,
    }
}

fn busy(state: &State) -> bool {
    matches!(
        state,
        State::Checking { .. } | State::Rechecking { .. } | State::Downloading { .. } | State::Verifying { .. }
    )
}

/// The worker: the launch check, then each order, and a background check whenever one is due. Ends when the
/// window's end is dropped.
fn work(
    make: MakeDrive,
    sync_back: SyncBack,
    channel: beebeeb_updater::Channel,
    orders: mpsc::Receiver<Order>,
    reports: mpsc::Sender<Report>,
    tuning: Tuning,
) {
    let mut updater = make(sync_back);
    let mut channel = channel;
    let wake = &*tuning.wake;
    let background = |channel| Event::Check {
        channel,
        intent: Intent::Background,
    };
    if !run(&mut *updater, 0, background(channel), &reports, wake) {
        return;
    }
    loop {
        let alive = match orders.recv_timeout(tuning.look_every) {
            Ok(Order { serial, event }) => {
                if let Event::Check { channel: asked, .. } = &event {
                    channel = *asked;
                }
                run(&mut *updater, serial, event, &reports, wake)
            }
            Err(mpsc::RecvTimeoutError::Timeout) if updater.background_check_due() => {
                run(&mut *updater, 0, background(channel), &reports, wake)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => false,
        };
        if !alive {
            return;
        }
    }
}

/// One event, then `Tick`s until the machine has no work left; the window is woken after each report. `false`: the
/// window's end is gone.
fn run(updater: &mut dyn Drive, serial: u64, event: Event, reports: &mpsc::Sender<Report>, wake: &dyn Fn()) -> bool {
    let send = |state: &State, updater: &dyn Drive| {
        let sent = reports
            .send(Report {
                serial,
                state: state.clone(),
                freshness: updater.freshness(),
                verified: updater.last_verified(),
                at: Local::now(),
            })
            .is_ok();
        if sent {
            wake();
        }
        sent
    };
    let mut state = updater.step(event);
    if !send(&state, updater) {
        return false;
    }
    let mut said = Instant::now();
    while busy(&state) {
        state = updater.step(Event::Tick);
        if !busy(&state) || said.elapsed() >= PROGRESS_EVERY {
            if !send(&state, updater) {
                return false;
            }
            said = Instant::now();
        }
    }
    true
}

impl UpdatesPart {
    /// The updater of this PC, when this build has one: a release build of the Windows app. `None` on macOS and
    /// Linux, and in every test build of this crate (a test hands in its own with [`UpdatesPart::live`]): no test
    /// can reach the release server through it.
    pub(crate) fn for_this_pc<C: CredentialStores + 'static>(
        engine: &Arc<Engine<C>>,
        identity: &'static EngineIdentity,
        config: Option<&DesktopConfig>,
    ) -> Option<Self> {
        #[cfg(all(windows, not(test)))]
        {
            use beebeeb_updater::host::{BUILD_MARK, BUILD_NOT_BEFORE};
            // Keeps the build's mark in the binary: the release workflow reads it there (as `windows_release_trust`
            // does).
            std::hint::black_box(BUILD_MARK);
            let (channel, from) = channels(config);
            let folder = rune::dirs::AppDirs::new(crate::shell::identity::AppIdentity::CURRENT.state_dir_id())
                .map(|dirs| crate::shell::local_data_of(&dirs, "windows"));
            match release_setup(BUILD_NOT_BEFORE, option_env!("BEEBEEB_RELEASE_VERSION"), folder) {
                ReleaseSetup::NotARelease => None,
                ReleaseSetup::Broken => {
                    log("settings: this release build could not set its updater up");
                    Some(Self::with_mode(
                        Mode::NoWorker,
                        identity,
                        channel,
                        from,
                        super::version_words(),
                    ))
                }
                ReleaseSetup::Ready { config, record } => {
                    let make = release::updater(engine.clone(), identity, config, record);
                    let woken = engine.clone();
                    let tuning = Tuning::standard(Box::new(move || woken.wake_host()));
                    Some(Self::live_tuned(
                        make,
                        identity,
                        channel,
                        from,
                        super::version_words(),
                        tuning,
                    ))
                }
            }
        }
        #[cfg(not(all(windows, not(test))))]
        {
            let _ = (engine, identity, config);
            None
        }
    }

    /// The part on an updater `make` builds on the worker's thread, looking every minute, waking nothing.
    #[cfg(test)]
    pub(crate) fn live(
        make: MakeDrive,
        identity: &'static EngineIdentity,
        channel: Channel,
        from: Channel,
        installed: String,
    ) -> Self {
        Self::live_tuned(
            make,
            identity,
            channel,
            from,
            installed,
            Tuning::standard(Box::new(|| {})),
        )
    }

    /// The part on an updater `make` builds on the worker's thread, paced and waking as `tuning` says.
    #[cfg(any(windows, test))]
    pub(crate) fn live_tuned(
        make: MakeDrive,
        identity: &'static EngineIdentity,
        channel: Channel,
        from: Channel,
        installed: String,
        tuning: Tuning,
    ) -> Self {
        let (orders, inbox) = mpsc::channel();
        let (reports, taken) = mpsc::channel();
        let first = line(channel);
        let sync_back = SyncBack::default();
        let handed = sync_back.clone();
        let started = std::thread::Builder::new()
            .name("settings-updates".into())
            .spawn(move || work(make, handed, first, inbox, reports, tuning));
        let mode = match started {
            Ok(_) => Mode::Live(Worker { orders, reports: taken }),
            Err(_) => {
                log("settings: could not start a thread for the updater");
                Mode::NoWorker
            }
        };
        let mut part = Self::with_mode(mode, identity, channel, from, installed);
        part.sync_back = sync_back;
        part
    }

    fn with_mode(
        mode: Mode,
        identity: &'static EngineIdentity,
        channel: Channel,
        from: Channel,
        installed: String,
    ) -> Self {
        let (saves, saved) = mpsc::channel();
        Self {
            mode,
            identity,
            channel,
            from,
            installed,
            last: None,
            sent: 0,
            asked: None,
            saving: None,
            saves,
            saved,
            toast_next: None,
            sync_back: SyncBack::default(),
            quit_asked: false,
        }
    }

    /// Sends `event` to the worker. `false`: no worker takes it.
    fn send(&mut self, event: Event) -> bool {
        let Mode::Live(worker) = &self.mode else {
            return false;
        };
        self.sent += 1;
        let sent = worker
            .orders
            .send(Order {
                serial: self.sent,
                event,
            })
            .is_ok();
        if !sent {
            log("settings: the updater's worker has ended");
            self.mode = Mode::NoWorker;
        }
        sent
    }

    /// Check now: a check the person asked for (it also reports an older channel build, which is never offered).
    pub(crate) fn check_now(&mut self) {
        let event = Event::Check {
            channel: line(self.channel),
            intent: Intent::Manual,
        };
        if self.send(event) {
            self.asked = Some(self.sent);
        }
    }

    /// "Download update": the newer version on offer (or the rest of an interrupted download) is downloaded and its
    /// signature checked; it then waits for "Restart to update". Nothing while a press or a check is out; never an
    /// older version (the machine would need `DownloadDowngrade`, which nothing here sends).
    pub(crate) fn download_update(&mut self) {
        if self.asked.is_some() {
            return;
        }
        let offered = matches!(
            self.shown(),
            Some(State::Available { offer } | State::Failed { resume: Some(offer), .. }) if !offer.downgrade
        );
        if offered && self.send(Event::Download) {
            self.asked = Some(self.sent);
        }
    }

    /// "Restart to update", only once the download is verified (spec §4: `Apply` is sent by this press alone). The
    /// worker verifies the file again, stops sync, starts the installer; `pump` then asks the window to quit.
    pub(crate) fn restart_to_update(&mut self) {
        if self.asked.is_some() {
            return;
        }
        let ready = matches!(self.shown(), Some(State::ReadyToRestart { offer, .. }) if !offer.downgrade);
        if ready && self.send(Event::Apply) {
            self.asked = Some(self.sent);
        }
    }

    /// Writes the channel to this PC's settings file on a worker; the menu shows it, and the new channel is checked,
    /// only once the file took it. A refusal keeps the old channel and says so.
    pub(crate) fn set_channel(&mut self, channel: Channel) {
        if self.saving.is_some() {
            self.toast_next = Some(STILL_SAVING);
            return;
        }
        if channel == self.channel {
            return;
        }
        let (identity, saves) = (self.identity, self.saves.clone());
        let started = std::thread::Builder::new()
            .name("settings-channel".into())
            .spawn(move || {
                let saved = std::panic::catch_unwind(|| {
                    DesktopConfig::update_for(identity, |config| {
                        let wanted = release_channel(channel);
                        let changed = config.release_channel != wanted;
                        config.release_channel = wanted;
                        (changed, ())
                    })
                    .is_ok()
                })
                .unwrap_or(false);
                let _ = saves.send(Saved { channel, saved });
            });
        if started.is_ok() {
            self.saving = Some(channel);
        } else {
            log("settings: could not start a thread to save the release channel");
            self.toast_next = Some(CHANNEL_NOT_SAVED);
        }
    }

    /// Take what the worker and the channel's writer answered; and, from the hand-off, the quit (the installer
    /// started) or sync back (it did not, after sync was stopped for it).
    pub(crate) fn pump(&mut self) -> Pumped {
        let mut toast = self.toast_next.take();
        let mut changed = toast.is_some();
        while let Ok(Saved { channel, saved }) = self.saved.try_recv() {
            changed = true;
            self.saving = None;
            if saved {
                self.channel = channel;
                // Another channel: what the last check said is not about it. Asked as Check now asks, so that an
                // older build on it is said (and, as ever, not offered).
                self.last = None;
                self.check_now();
            } else {
                log("settings: the release channel was not saved");
                toast = toast.or(Some(CHANNEL_NOT_SAVED));
            }
        }
        let mut ended = false;
        let mut restarting = false;
        if let Mode::Live(worker) = &self.mode {
            loop {
                match worker.reports.try_recv() {
                    Ok(report) => {
                        changed = true;
                        restarting |= matches!(report.state, State::Restarting { .. });
                        if self.asked.is_some_and(|asked| report.serial >= asked) && !busy(&report.state) {
                            self.asked = None;
                        }
                        self.last = Some(report);
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    // The worker ended (a panic in the updater): never left saying Checking… for good.
                    Err(mpsc::TryRecvError::Disconnected) => {
                        ended = true;
                        break;
                    }
                }
            }
        }
        if ended {
            log("settings: the updater's worker has ended");
            self.mode = Mode::NoWorker;
            self.asked = None;
            self.last = None;
            changed = true;
        }
        // Before the launch check's first report About says Checking… too: the window keeps looking until it came.
        let working = matches!(self.mode, Mode::Live(_)) && self.last.as_ref().is_none_or(|report| busy(&report.state));
        let restart = restarting && !self.quit_asked;
        self.quit_asked |= restart;
        let start_sync = self.sync_back.take();
        Pumped {
            changed: changed || restart || start_sync,
            toast,
            busy: self.asked.is_some() || self.saving.is_some() || working,
            restart,
            start_sync,
        }
    }

    /// About's update rows: the check's words, the channel the settings file holds, an older channel build (said,
    /// never offered), and the notes of a NEWER version on offer, with what the dialog's button does now. An older
    /// version's notes are never put here: DW-0789b's button would offer it.
    pub(crate) fn fill(&self, about: &mut About) {
        about.channel = self.channel;
        about.check = self.check();
        about.older = self.older();
        if let Some(offer) = self.upgrade() {
            about.notes = ReleaseNotes {
                version: offer.version.to_string(),
                channel: self.channel,
                released: String::new(),
                lines: offer
                    .notes
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
                press: self.press(),
            };
        }
    }

    /// General's "a new version" (DW-0781): a newer version on offer, what its button does now, and, while a press is
    /// worked on or after one did not go through, the row's words in the caption's place. Never an older version.
    pub(crate) fn update(&self) -> Update {
        let Some(offer) = self.upgrade() else {
            return Update::Current;
        };
        let press = self.press();
        let says = match self.shown() {
            Some(State::Available { .. } | State::ReadyToRestart { .. }) if press != Press::Wait => None,
            _ => Some(self.check().says().to_string()),
        };
        Update::Available {
            version: offer.version.to_string(),
            press,
            says,
        }
    }

    /// The release notes (DW-0789b) have something real to show: a newer version is on offer.
    pub(crate) fn has_notes(&self) -> bool {
        self.upgrade().is_some()
    }

    /// What the update's button does now: wait while a press is out or the machine works; restart once the file is
    /// verified; otherwise download.
    fn press(&self) -> Press {
        if self.asked.is_some() {
            return Press::Wait;
        }
        match self.shown() {
            Some(State::ReadyToRestart { .. }) => Press::Restart,
            Some(State::Downloading { .. } | State::Verifying { .. } | State::Restarting { .. }) => Press::Wait,
            _ => Press::Download,
        }
    }

    /// The state About draws: what a background check keeps on screen while it runs.
    fn shown(&self) -> Option<&State> {
        let mut state = &self.last.as_ref()?.state;
        while let State::Rechecking { kept, .. } = state {
            state = kept;
        }
        Some(state)
    }

    /// A NEWER version on offer, at any step of its way (a failed download it can resume included). Never a downgrade.
    fn upgrade(&self) -> Option<&beebeeb_updater::Offer> {
        let offer = match self.shown()? {
            State::Available { offer }
            | State::Downloading { offer, .. }
            | State::Verifying { offer }
            | State::ReadyToRestart { offer, .. }
            | State::Restarting { offer }
            | State::Failed {
                resume: Some(offer), ..
            } => offer,
            _ => return None,
        };
        (!offer.downgrade).then_some(offer)
    }

    fn older(&self) -> Option<OlderChannel> {
        match self.shown()? {
            State::Available { offer } if offer.downgrade => Some(OlderChannel {
                channel: self.channel,
                version: offer.version.to_string(),
                runs: self.installed.clone(),
                from: self.from,
            }),
            _ => None,
        }
    }

    fn check(&self) -> UpdateCheck {
        match &self.mode {
            Mode::NoWorker => return UpdateCheck::Checked(NO_UPDATER.into()),
            Mode::Live(_) => {}
        }
        if self.asked.is_some() || self.saving.is_some() {
            return UpdateCheck::Checking;
        }
        let (Some(report), Some(state)) = (self.last.as_ref(), self.shown()) else {
            // The launch check is on its way.
            return UpdateCheck::Checking;
        };
        // The time of the last VERIFIED check: a background check that got no answer, or a refused one, keeps the
        // offer on screen and does not make it look newer.
        let at = report
            .verified
            .and_then(|secs| DateTime::from_timestamp(i64::try_from(secs).ok()?, 0))
            .map_or(report.at, |at| at.with_timezone(&Local));
        let when = when(at, Local::now());
        let checked = |words: String| UpdateCheck::Checked(words);
        match state {
            State::Idle => checked(NOT_CHECKED.into()),
            State::Checking { .. } | State::Rechecking { .. } => UpdateCheck::Checking,
            State::UpToDate { installed, .. } => checked(format!("{installed} is the newest version · checked {when}")),
            State::Available { offer } if offer.downgrade => checked(format!(
                "No newer version on the {} channel · checked {when}",
                self.channel.label()
            )),
            State::Available { offer } => checked(format!("Version {} is available · checked {when}", offer.version)),
            State::Downloading {
                offer, received, total, ..
            } => checked(format!(
                "Downloading version {}: {} %",
                offer.version,
                received.saturating_mul(100) / (*total).max(1)
            )),
            State::Verifying { .. } => checked(VERIFYING.into()),
            State::ReadyToRestart { offer, .. } => {
                checked(format!("Version {} is downloaded and verified.", offer.version))
            }
            State::Restarting { .. } => checked(RESTARTING.into()),
            State::Failed { failure, .. } => failed(*failure, report.freshness),
        }
    }
}

/// When a check was answered, as the row says it: "today, 09:12", or "13 Nov, 09:12".
fn when(at: DateTime<Local>, now: DateTime<Local>) -> String {
    if at.date_naive() == now.date_naive() {
        format!("today, {}", at.format("%H:%M"))
    } else {
        at.format("%-d %b, %H:%M").to_string()
    }
}

/// Why a check, a download or the hand-off did not go through, in fixed words. "Try again in a moment" only where a
/// moment can help (no answer, or the server's own trouble); a refusal says what was refused and that nothing was
/// downloaded or installed. With nothing verified for 14 days, it says that too.
pub(crate) fn failed(failure: Failure, freshness: Freshness) -> UpdateCheck {
    let words: &str = match failure {
        Failure::Offline => "",
        Failure::ServerStatus(404) => NOTHING_PUBLISHED,
        Failure::ServerStatus(_) => "",
        Failure::Refused(refusal) => refused(refusal),
        Failure::WrongSize | Failure::BadSignature => NOT_SIGNED_FILE,
        Failure::DownloadInterrupted { .. } => STOPPED_DOWNLOAD,
        Failure::DowngradeChanged => OFFER_CHANGED,
        Failure::Disk => NO_DISK,
        Failure::EngineStillRunning => SYNC_UNCONFIRMED,
        Failure::InstallerDidNotStart => INSTALLER_DID_NOT_START,
    };
    match (words, freshness) {
        ("", Freshness::Verified) => UpdateCheck::Failed,
        ("", Freshness::Unverified { days }) => UpdateCheck::Checked(format!(
            "Couldn’t reach the release server. No update has been verified for {days} days."
        )),
        (words, Freshness::Verified) => UpdateCheck::Checked(words.into()),
        (words, Freshness::Unverified { days }) => {
            UpdateCheck::Checked(format!("{words} No update has been verified for {days} days."))
        }
    }
}

pub(crate) fn refused(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::ManifestSignature
        | Refusal::ArtifactSignature
        | Refusal::SignatureForOtherFile
        | Refusal::KeyNotValidForVersion
        | Refusal::WrongChannel => NOT_VERIFIED,
        Refusal::ManifestUnreadable | Refusal::BadSize => UNREADABLE,
        Refusal::StaleManifest | Refusal::OlderThanThisBuild => OUT_OF_DATE,
        Refusal::SignedInTheFuture => FUTURE,
        Refusal::UntrustedAddress => OTHER_HOST,
        Refusal::NoBuildForThisPc => NO_BUILD,
        Refusal::BelowFloor => BELOW_FLOOR,
        Refusal::NoBuildTime => NOT_A_RELEASE,
    }
}

/// The channel the settings file holds, and the one the running build came from.
fn channels(config: Option<&DesktopConfig>) -> (Channel, Channel) {
    let chosen = config.map_or(ReleaseChannel::Stable, |config| config.release_channel);
    let from = config
        .and_then(|config| config.installed_release_channel)
        .unwrap_or(chosen);
    (channel_of(chosen), channel_of(from))
}

/// Starts the verified installer and says whether it started. A release on Windows: the passive NSIS run
/// ([`start_installer`]). A test hands in its own: no test ever starts an installer.
#[cfg(any(windows, test))]
pub(crate) type StartInstaller = Box<dyn FnMut(&VerifiedInstaller, EngineStopped) -> bool>;

/// Windows: the crate's passive NSIS start (`VerifiedInstaller::start_nsis`, the only way to start one). Elsewhere
/// nothing is started.
#[cfg(any(windows, test))]
// A Windows test build never calls it: no test there may start a real installer.
#[cfg_attr(all(windows, test), allow(dead_code))]
fn start_installer(installer: &VerifiedInstaller, stopped: EngineStopped) -> bool {
    #[cfg(windows)]
    {
        installer.start_nsis(stopped)
    }
    #[cfg(not(windows))]
    {
        let _ = (installer, stopped);
        false
    }
}

/// "Restart to update": the engine stops before the installer starts, and the installer is the crate's
/// `VerifiedInstaller`, verified again and held at this moment by the machine.
#[cfg(any(windows, test))]
pub(crate) struct EngineApply<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    identity: &'static EngineIdentity,
    /// Where a failed start asks for sync back.
    sync_back: SyncBack,
    /// This hand-off's stop ended a running sync: a failed start asks for it back. A sync that was not running is
    /// not started by an update that did not happen.
    stopped_running: bool,
    /// A stop was not confirmed in this process: the engine slot is empty since, so a later stop would say
    /// `WasNotRunning` about a task that may still run. No proof is minted again until Beebeeb restarts.
    unconfirmed: bool,
    start: StartInstaller,
}

#[cfg(any(windows, test))]
impl<C: CredentialStores> EngineApply<C> {
    /// The release's hand-off, with the real installer start ([`start_installer`]).
    #[cfg_attr(all(windows, test), allow(dead_code))]
    pub(crate) fn new(engine: Arc<Engine<C>>, identity: &'static EngineIdentity, sync_back: SyncBack) -> Self {
        Self::starting_with(engine, identity, sync_back, Box::new(start_installer))
    }

    pub(crate) fn starting_with(
        engine: Arc<Engine<C>>,
        identity: &'static EngineIdentity,
        sync_back: SyncBack,
        start: StartInstaller,
    ) -> Self {
        Self {
            engine,
            identity,
            sync_back,
            stopped_running: false,
            unconfirmed: false,
            start,
        }
    }
}

#[cfg(any(windows, test))]
impl<C: CredentialStores> Apply for EngineApply<C> {
    /// The quit path's stop (`EngineActions::stop_for_quit` calls the same `Engine::stop`): the runner stops before
    /// its next operation and the queue in `state.db` stays as it is, so an upload that was cut is sent again by
    /// the next start. A stop the engine did not confirm mints no proof: the installer is not started. (On Windows
    /// the engine also says `Unconfirmed` when its task ended and File Explorer's connection did not let go; the two
    /// are one answer here, so the words say only that the stop was not confirmed: `SYNC_UNCONFIRMED`.)
    fn stop_engine(&mut self) -> Option<EngineStopped> {
        if self.unconfirmed {
            log("settings: an earlier stop of sync was not confirmed; the update waits for a restart of Beebeeb");
            return None;
        }
        let stopped = match self.engine.stop() {
            SyncStop::Stopped => true,
            SyncStop::WasNotRunning => false,
            SyncStop::Unconfirmed => {
                log("settings: sync did not confirm it stopped; the update was not started");
                self.unconfirmed = true;
                return None;
            }
        };
        self.stopped_running = stopped;
        Some(EngineStopped::engine_stopped_and_uploads_parked())
    }

    /// The installer, then the channel that served it is recorded (spec §4: the shell writes
    /// `installed_release_channel` when the installer started).
    fn launch(&mut self, installer: &VerifiedInstaller, offer: &Offer, stopped: EngineStopped) -> bool {
        if !(self.start)(installer, stopped) {
            return false;
        }
        let served = match offer.channel {
            beebeeb_updater::Channel::Stable => ReleaseChannel::Stable,
            beebeeb_updater::Channel::Beta => ReleaseChannel::Beta,
            beebeeb_updater::Channel::Alpha => ReleaseChannel::Alpha,
        };
        let recorded = DesktopConfig::update_for(self.identity, |config| {
            config.installed_release_channel = Some(served);
            (true, ())
        });
        if recorded.is_err() {
            log("settings: the installer started; the channel it came from was not recorded");
        }
        true
    }

    /// The installer did not start after sync was stopped for it: the shell starts sync again with its own start
    /// (the folder and the first-run state are the shell's), through `SyncBack` and the window's poll. Only a sync
    /// this hand-off stopped.
    fn resume_engine(&mut self) {
        if std::mem::take(&mut self.stopped_running) {
            log("settings: the installer did not start; sync is started again");
            self.sync_back.ask();
        }
    }
}

/// What a release build of this PC takes updates with: the pinned key (`keys/updater-v2.pub`) for 0.9.0 and later,
/// signed after `build_time`; the one host; the staging folder in `folder`. `None` without a build time or a release
/// version: no updater at all. (`host::windows_release_trust` is this trust with the build's own time.)
#[cfg(any(windows, test))]
pub(crate) fn release_config(
    build_time: Option<u64>,
    version: Option<&str>,
    folder: &std::path::Path,
) -> Option<beebeeb_updater::Config> {
    let not_before = build_time?;
    let installed = beebeeb_updater::Version::parse(version?)?;
    let trust = beebeeb_updater::host::windows_trust_built_at(not_before).ok()?;
    Some(beebeeb_updater::Config {
        trust,
        base_url: beebeeb_updater::host::RELEASE_BASE_URL.to_string(),
        installed,
        staging_dir: folder.join("updates"),
    })
}

/// What a build sets its updater up with, or why it has none.
#[cfg(any(windows, test))]
pub(crate) enum ReleaseSetup {
    /// Not a release build (no build time, or no release version): no updater, and not one request.
    NotARelease,
    /// A release build that could not set its updater up (no local data folder; a version or a trust it cannot
    /// read): About says the updater did not start, never only the versions.
    Broken,
    /// The release's config, and where its record lives.
    Ready {
        config: beebeeb_updater::Config,
        record: PathBuf,
    },
}

/// The release glue, testable on every platform: the build's time and version (`host::BUILD_NOT_BEFORE`,
/// `BEEBEEB_RELEASE_VERSION`) and this app's local data folder, as [`UpdatesPart::for_this_pc`] passes them.
#[cfg(any(windows, test))]
pub(crate) fn release_setup(build_time: Option<u64>, version: Option<&str>, folder: Option<PathBuf>) -> ReleaseSetup {
    if build_time.is_none() || version.is_none() {
        return ReleaseSetup::NotARelease;
    }
    let Some(folder) = folder else {
        return ReleaseSetup::Broken;
    };
    match release_config(build_time, version, &folder) {
        Some(config) => ReleaseSetup::Ready {
            config,
            record: folder.join("updater.json"),
        },
        None => ReleaseSetup::Broken,
    }
}

/// The release build's updater: the trust baked into this build, the one host, this PC's record and folder.
#[cfg(all(windows, not(test)))]
mod release {
    use std::path::PathBuf;
    use std::sync::Arc;

    use beebeeb_engine::app::{CredentialStores, Engine};
    use beebeeb_engine::identity::EngineIdentity;
    use beebeeb_updater::host::{FileStore, SystemClock};
    use beebeeb_updater::net::HttpsClient;
    use beebeeb_updater::{Http, Response, Unreachable};

    use super::{Drive, EngineApply, MakeDrive, Watching};
    use crate::engine_host::log;

    /// The https client, or none when it could not be built: every check then ends offline, in sight.
    enum Client {
        Https(HttpsClient),
        None,
    }

    impl Http for Client {
        fn get(&mut self, url: &str, from: u64) -> Result<Response, Unreachable> {
            match self {
                Client::Https(client) => client.get(url, from),
                Client::None => Err(Unreachable),
            }
        }
    }

    pub(super) fn updater<C: CredentialStores + 'static>(
        engine: Arc<Engine<C>>,
        identity: &'static EngineIdentity,
        config: beebeeb_updater::Config,
        record: PathBuf,
    ) -> MakeDrive {
        Box::new(move |sync_back| {
            let client = HttpsClient::new().map_or_else(
                |_| {
                    log("settings: the updater's https client could not be built");
                    Client::None
                },
                Client::Https,
            );
            let store = FileStore::new(record);
            let apply = EngineApply::new(engine, identity, sync_back);
            Box::new(Watching::new(config, client, SystemClock, store, apply)) as Box<dyn Drive>
        })
    }
}

#[cfg(test)]
impl UpdatesPart {
    /// Sends any event to the worker, as a press would (the download and the hand-off have no press yet).
    pub(crate) fn send_for_test(&mut self, event: Event) {
        if self.send(event) {
            self.asked = Some(self.sent);
        }
    }

    pub(crate) fn state_for_test(&self) -> Option<&State> {
        self.last.as_ref().map(|report| &report.state)
    }
}

#[cfg(test)]
#[path = "updates_tests.rs"]
pub(super) mod tests;
